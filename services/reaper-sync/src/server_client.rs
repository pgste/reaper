//! Management Server Client
//!
//! Client for communicating with the Reaper management server (platform)
//! to fetch policies and entity data.
#![allow(dead_code)]

use crate::config::SyncConfig;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument, warn};

/// Server client errors
#[derive(Debug, Error)]
pub enum ServerClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Server error: {status} - {message}")]
    ServerError { status: u16, message: String },
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Policy summary from list endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicySummary {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub checksum: Option<String>,
    pub updated_at: String,
    pub language: Option<String>,
}

/// Response from policy list endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyListResponse {
    pub policies: Vec<PolicySummary>,
    pub total: Option<usize>,
    pub page: Option<usize>,
}

/// Full policy details from get endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: u64,
    pub language: String,
    pub content: String,
    pub rules: Option<Vec<serde_json::Value>>,
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
    pub checksum: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Entity data response
#[derive(Debug, Clone, Deserialize)]
pub struct EntityDataResponse {
    pub entities: Vec<serde_json::Value>,
    pub total: Option<usize>,
}

/// Datastore status (data plane).
#[derive(Debug, Clone, Deserialize)]
pub struct DatastoreStatus {
    pub current_version: i64,
}

/// A published datastore version with its materialized document.
#[derive(Debug, Clone, Deserialize)]
pub struct DatastoreVersion {
    pub version: i64,
    pub checksum: String,
    #[serde(default)]
    pub change_seq: i64,
    pub document: serde_json::Value,
}

/// A page of the change stream.
#[derive(Debug, Clone, Deserialize)]
pub struct DatastoreChanges {
    pub snapshot_required: bool,
    #[serde(default)]
    pub head_seq: i64,
    #[serde(default)]
    pub deltas: Vec<serde_json::Value>,
}

/// Client for communicating with the management server
pub struct ServerClient {
    config: SyncConfig,
    http_client: Client,
    auth_token: Option<String>,
}

impl ServerClient {
    /// Create a new server client
    pub fn new(config: SyncConfig) -> Result<Self, ServerClientError> {
        let timeout = std::time::Duration::from_secs(config.sync.server.timeout_seconds);

        let http_client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(ServerClientError::Http)?;

        // Resolve auth token
        let auth_token = Self::resolve_auth_token(&config)?;

        Ok(Self {
            config,
            http_client,
            auth_token,
        })
    }

    /// Resolve authentication token from config
    fn resolve_auth_token(config: &SyncConfig) -> Result<Option<String>, ServerClientError> {
        match config.sync.auth.auth_type.as_str() {
            "api_token" => {
                // Try direct token first
                if let Some(ref token) = config.sync.auth.token {
                    return Ok(Some(token.clone()));
                }
                // Try token file
                if let Some(ref token_file) = config.sync.auth.token_file {
                    let token = std::fs::read_to_string(token_file).map_err(|e| {
                        ServerClientError::ConfigError(format!(
                            "Failed to read token file {:?}: {}",
                            token_file, e
                        ))
                    })?;
                    return Ok(Some(token.trim().to_string()));
                }
                Err(ServerClientError::ConfigError(
                    "api_token auth requires token or token_file".to_string(),
                ))
            }
            "none" => Ok(None),
            other => {
                warn!("Unsupported auth type: {}, treating as none", other);
                Ok(None)
            }
        }
    }

    /// Build the base URL for API requests
    fn base_url(&self) -> String {
        format!(
            "{}/api/{}",
            self.config.sync.server.url.trim_end_matches('/'),
            self.config.sync.server.api_version
        )
    }

    /// Add authentication headers to a request
    fn add_auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_token {
            Some(token) => request.header(header::AUTHORIZATION, format!("Bearer {}", token)),
            None => request,
        }
    }

    /// Build query parameters for scope filtering
    fn scope_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::new();

        if !self.config.sync.scope.teams.is_empty() {
            params.push(("teams", self.config.sync.scope.teams.join(",")));
        }
        if !self.config.sync.scope.environments.is_empty() {
            params.push((
                "environments",
                self.config.sync.scope.environments.join(","),
            ));
        }
        if !self.config.sync.scope.regions.is_empty() {
            params.push(("regions", self.config.sync.scope.regions.join(",")));
        }

        params
    }

    /// List policies from the server
    #[instrument(skip(self))]
    pub async fn list_policies(&self) -> Result<PolicyListResponse, ServerClientError> {
        let url = format!("{}/policies", self.base_url());

        debug!("Fetching policy list from {}", url);

        let request = self.http_client.get(&url);
        let request = self.add_auth_headers(request);
        let request = request.query(&self.scope_params());

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }

        let policies: PolicyListResponse = response.json().await?;
        debug!("Received {} policies from server", policies.policies.len());

        Ok(policies)
    }

    /// Get full policy details
    #[instrument(skip(self))]
    pub async fn get_policy(&self, policy_id: &str) -> Result<PolicyDetail, ServerClientError> {
        let url = format!("{}/policies/{}", self.base_url(), policy_id);

        debug!("Fetching policy {} from {}", policy_id, url);

        let request = self.http_client.get(&url);
        let request = self.add_auth_headers(request);

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }

        let policy: PolicyDetail = response.json().await?;
        debug!(
            "Received policy: {} (version {})",
            policy.name, policy.version
        );

        Ok(policy)
    }

    /// Check server health
    #[instrument(skip(self))]
    pub async fn health_check(&self) -> Result<bool, ServerClientError> {
        let url = format!(
            "{}/health",
            self.config.sync.server.url.trim_end_matches('/')
        );

        let response = self.http_client.get(&url).send().await?;

        Ok(response.status().is_success())
    }

    /// Get entity data from the server
    #[instrument(skip(self))]
    /// Current datastore status for a namespace (version counter only —
    /// one tiny COUNT-backed response per poll).
    pub async fn get_datastore_status(
        &self,
        org: &str,
        namespace: &str,
    ) -> Result<DatastoreStatus, ServerClientError> {
        let url = format!(
            "{}/orgs/{}/namespaces/{}/datastore",
            self.base_url(),
            org,
            namespace
        );
        let request = self.add_auth_headers(self.http_client.get(&url));
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }
        Ok(response.json().await?)
    }

    /// Fetch a published datastore version's materialized document.
    pub async fn get_datastore_version(
        &self,
        org: &str,
        namespace: &str,
        version: i64,
    ) -> Result<DatastoreVersion, ServerClientError> {
        let url = format!(
            "{}/orgs/{}/namespaces/{}/datastore/versions/{}",
            self.base_url(),
            org,
            namespace,
            version
        );
        let request = self.add_auth_headers(self.http_client.get(&url));
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }
        Ok(response.json().await?)
    }

    /// Pull the change stream after `since` (durable delta path — the log
    /// is the source, notifications are only wake-ups).
    pub async fn get_datastore_changes(
        &self,
        org: &str,
        namespace: &str,
        since: i64,
    ) -> Result<DatastoreChanges, ServerClientError> {
        let url = format!(
            "{}/orgs/{}/namespaces/{}/datastore/changes?since={}",
            self.base_url(),
            org,
            namespace,
            since
        );
        let request = self.add_auth_headers(self.http_client.get(&url));
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }
        Ok(response.json().await?)
    }

    pub async fn get_entities(&self) -> Result<EntityDataResponse, ServerClientError> {
        let url = format!("{}/entities", self.base_url());

        debug!("Fetching entities from {}", url);

        let request = self.http_client.get(&url);
        let request = self.add_auth_headers(request);
        let request = request.query(&self.scope_params());

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ServerClientError::ServerError { status, message });
        }

        let entities: EntityDataResponse = response.json().await?;
        debug!("Received {} entities from server", entities.entities.len());

        Ok(entities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SyncConfig {
        SyncConfig {
            sync: crate::config::SyncSettings {
                datastore: Default::default(),
                server: crate::config::ServerConfig {
                    url: "http://localhost:8081".to_string(),
                    api_version: "v1".to_string(),
                    timeout_seconds: 30,
                },
                auth: crate::config::AuthConfig {
                    auth_type: "none".to_string(),
                    token: None,
                    token_file: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                },
                scope: crate::config::ScopeConfig {
                    teams: vec!["team-a".to_string()],
                    environments: vec![],
                    regions: vec![],
                    policy_ids: vec![],
                },
                behavior: crate::config::BehaviorConfig {
                    mode: "active".to_string(),
                    poll_interval_seconds: 30,
                    batch_size: 100,
                    retry_max_attempts: 3,
                    retry_backoff_seconds: 5,
                    sync_on_start: true,
                },
                agent: crate::config::AgentConfig {
                    url: "http://localhost:8080".to_string(),
                    health_check_interval_seconds: 10,
                    timeout_seconds: 10,
                },
                cache: crate::config::CacheConfig::default(),
                metrics: crate::config::MetricsConfig::default(),
            },
        }
    }

    #[test]
    fn test_server_client_creation() {
        let config = test_config();
        let client = ServerClient::new(config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_base_url() {
        let config = test_config();
        let client = ServerClient::new(config).unwrap();
        assert_eq!(client.base_url(), "http://localhost:8081/api/v1");
    }

    #[test]
    fn test_scope_params() {
        let config = test_config();
        let client = ServerClient::new(config).unwrap();
        let params = client.scope_params();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], ("teams", "team-a".to_string()));
    }
}
