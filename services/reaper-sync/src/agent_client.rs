//! Agent Client
//!
//! Client for communicating with the Reaper Agent to deploy policies
//! and sync entity data.
#![allow(dead_code)]

use crate::config::SyncConfig;
use crate::server_client::PolicyDetail;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

/// Agent client errors
#[derive(Debug, Error)]
pub enum AgentClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Agent error: {status} - {message}")]
    AgentError { status: u16, message: String },
    #[error("Agent unavailable")]
    Unavailable,
}

/// Request to deploy a policy to the agent
#[derive(Debug, Serialize)]
pub struct DeployPolicyRequest {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<DeployMetadata>,
}

/// Metadata about the deployment source
#[derive(Debug, Serialize)]
pub struct DeployMetadata {
    pub source: String,
    pub server_version: String,
    pub deployed_by: String,
}

/// Response from policy deployment
#[derive(Debug, Deserialize)]
pub struct DeployPolicyResponse {
    pub status: String,
    pub policy_id: String,
    #[serde(default)]
    pub policy_name: Option<String>,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Request to sync entity data to the agent
#[derive(Debug, Serialize)]
pub struct SyncDataRequest {
    pub entities: Vec<serde_json::Value>,
    #[serde(default)]
    pub replace_all: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SyncSource>,
}

/// Source information for data sync
#[derive(Debug, Serialize)]
pub struct SyncSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub server_url: Option<String>,
    pub server_version: Option<String>,
    pub team: Option<String>,
}

/// Response from data sync
#[derive(Debug, Deserialize)]
pub struct SyncDataResponse {
    pub status: String,
    pub inserted: usize,
    pub failed: usize,
    pub replaced: bool,
    pub total_entities: usize,
}

/// Policy list response from agent
#[derive(Debug, Deserialize)]
pub struct AgentPolicyListResponse {
    pub policies: Vec<AgentPolicy>,
}

/// Policy info from agent
#[derive(Debug, Deserialize)]
pub struct AgentPolicy {
    pub id: String,
    pub name: String,
    pub version: u64,
}

/// Client for communicating with the Reaper Agent
pub struct AgentClient {
    agent_url: String,
    http_client: Client,
    server_url: String,
    teams: Vec<String>,
}

impl AgentClient {
    /// Create a new agent client
    pub fn new(config: &SyncConfig) -> Result<Self, AgentClientError> {
        let timeout = std::time::Duration::from_secs(config.sync.agent.timeout_seconds);

        let http_client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(AgentClientError::Http)?;

        Ok(Self {
            agent_url: config.sync.agent.url.clone(),
            http_client,
            server_url: config.sync.server.url.clone(),
            teams: config.sync.scope.teams.clone(),
        })
    }

    /// Deploy a policy to the agent
    #[instrument(skip(self, policy))]
    pub async fn deploy_policy(
        &self,
        policy: &PolicyDetail,
    ) -> Result<DeployPolicyResponse, AgentClientError> {
        let url = format!(
            "{}/api/v1/policies/deploy",
            self.agent_url.trim_end_matches('/')
        );

        debug!("Deploying policy {} to agent at {}", policy.name, url);

        // Convert rules from policy
        let rules = policy.rules.clone().unwrap_or_default();

        let request = DeployPolicyRequest {
            policy_id: policy.id.clone(),
            name: policy.name.clone(),
            description: policy.description.clone(),
            rules,
            metadata: Some(DeployMetadata {
                source: "sync-client".to_string(),
                server_version: format!("v{}", policy.version),
                deployed_by: format!("sync-client:{}", self.teams.join(",")),
            }),
        };

        let response = self.http_client.post(&url).json(&request).send().await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(AgentClientError::AgentError {
                status: status_code,
                message,
            });
        }

        let deploy_response: DeployPolicyResponse = response.json().await?;

        if deploy_response.error.is_some() {
            warn!(
                "Policy deployment returned error: {:?}",
                deploy_response.error
            );
        } else {
            info!(
                "Deployed policy {} (version {}) to agent",
                policy.name, policy.version
            );
        }

        Ok(deploy_response)
    }

    /// Deploy a verified, versioned data bundle (full replication push).
    pub async fn deploy_data_version(
        &self,
        version: i64,
        checksum: &str,
        change_seq: i64,
        model_version: i64,
        document: &serde_json::Value,
    ) -> Result<(), AgentClientError> {
        let url = format!(
            "{}/api/v1/data/deploy-version",
            self.agent_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "version": version,
            "checksum": checksum,
            "change_seq": change_seq,
            "model_version": model_version,
            "document": document,
        });
        let response = self.http_client.post(&url).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(AgentClientError::AgentError {
                status: status_code,
                message,
            });
        }
        Ok(())
    }

    /// Apply a contiguous delta batch. Ok(Ok(head)) = applied to head;
    /// Ok(Err(agent_seq)) = 409 seq mismatch, the agent reports where it
    /// actually is — pull from THERE (self-correcting, gap-proof).
    pub async fn apply_data_deltas(
        &self,
        from_seq: i64,
        head_seq: i64,
        deltas: &[serde_json::Value],
    ) -> Result<Result<i64, i64>, AgentClientError> {
        let url = format!(
            "{}/api/v1/data/apply-deltas",
            self.agent_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "from_seq": from_seq,
            "head_seq": head_seq,
            "deltas": deltas,
        });
        let response = self.http_client.post(&url).json(&body).send().await?;
        match response.status().as_u16() {
            200 => Ok(Ok(head_seq)),
            409 => {
                let text = response.text().await.unwrap_or_default();
                let agent_seq = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v.get("applied_seq").and_then(|s| s.as_i64()))
                    .unwrap_or(-1);
                Ok(Err(agent_seq))
            }
            status => {
                let message = response.text().await.unwrap_or_default();
                Err(AgentClientError::AgentError { status, message })
            }
        }
    }

    /// Replica heartbeat: confirm the agent is still on the current version
    /// without shipping the document. 409 = agent behind or diverged; the
    /// caller responds with a full deploy_data_version.
    pub async fn confirm_data_version(
        &self,
        version: i64,
        checksum: &str,
    ) -> Result<bool, AgentClientError> {
        let url = format!(
            "{}/api/v1/data/confirm-version",
            self.agent_url.trim_end_matches('/')
        );
        let body = serde_json::json!({"version": version, "checksum": checksum});
        let response = self.http_client.post(&url).json(&body).send().await?;
        match response.status().as_u16() {
            200 => Ok(true),
            409 => Ok(false), // behind/diverged: caller pushes a full deploy
            status => {
                let message = response.text().await.unwrap_or_default();
                Err(AgentClientError::AgentError { status, message })
            }
        }
    }

    /// Sync entity data to the agent
    #[instrument(skip(self, entities))]
    pub async fn sync_data(
        &self,
        entities: Vec<serde_json::Value>,
        replace_all: bool,
    ) -> Result<SyncDataResponse, AgentClientError> {
        let url = format!("{}/api/v1/data/sync", self.agent_url.trim_end_matches('/'));

        debug!(
            "Syncing {} entities to agent (replace_all={})",
            entities.len(),
            replace_all
        );

        let request = SyncDataRequest {
            entities,
            replace_all,
            source: Some(SyncSource {
                source_type: "sync-client".to_string(),
                server_url: Some(self.server_url.clone()),
                server_version: None,
                team: self.teams.first().cloned(),
            }),
        };

        let response = self.http_client.post(&url).json(&request).send().await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(AgentClientError::AgentError {
                status: status_code,
                message,
            });
        }

        let sync_response: SyncDataResponse = response.json().await?;

        info!(
            "Synced data to agent: {} inserted, {} failed, {} total",
            sync_response.inserted, sync_response.failed, sync_response.total_entities
        );

        Ok(sync_response)
    }

    /// Check agent health
    #[instrument(skip(self))]
    pub async fn health_check(&self) -> Result<bool, AgentClientError> {
        let url = format!("{}/health", self.agent_url.trim_end_matches('/'));

        let response = self.http_client.get(&url).send().await?;

        Ok(response.status().is_success())
    }

    /// List policies currently deployed on the agent
    #[instrument(skip(self))]
    pub async fn list_policies(&self) -> Result<AgentPolicyListResponse, AgentClientError> {
        let url = format!("{}/api/v1/policies", self.agent_url.trim_end_matches('/'));

        let response = self.http_client.get(&url).send().await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(AgentClientError::AgentError {
                status: status_code,
                message,
            });
        }

        let list_response: AgentPolicyListResponse = response.json().await?;

        debug!(
            "Agent has {} policies deployed",
            list_response.policies.len()
        );

        Ok(list_response)
    }

    /// Wait for agent to become available
    pub async fn wait_for_agent(&self, max_attempts: u32, delay_secs: u64) -> bool {
        for attempt in 1..=max_attempts {
            match self.health_check().await {
                Ok(true) => {
                    info!("Agent is available");
                    return true;
                }
                Ok(false) => {
                    debug!(
                        "Agent health check returned false, attempt {}/{}",
                        attempt, max_attempts
                    );
                }
                Err(e) => {
                    debug!(
                        "Agent health check failed: {}, attempt {}/{}",
                        e, attempt, max_attempts
                    );
                }
            }

            if attempt < max_attempts {
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            }
        }

        warn!("Agent not available after {} attempts", max_attempts);
        false
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
    fn test_agent_client_creation() {
        let config = test_config();
        let client = AgentClient::new(&config);
        assert!(client.is_ok());
    }
}
