//! HTTP client for Reaper Management Server
#![allow(dead_code)]

use reqwest::Client;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use uuid::Uuid;

use reaper_core::config::ManagementSettings;

use super::types::*;

/// State of the management connection
#[derive(Debug, Clone, Default)]
pub struct ConnectionState {
    /// Agent ID assigned by management server
    pub agent_id: Option<Uuid>,
    /// JWT token for authentication
    pub token: Option<String>,
    /// Token expiration time
    pub token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Currently deployed bundle ID
    pub current_bundle_id: Option<Uuid>,
    /// Currently deployed bundle checksum
    pub current_bundle_checksum: Option<String>,
}

/// HTTP client for communicating with Reaper Management Server
pub struct ManagementClient {
    /// HTTP client
    client: Client,
    /// Management server base URL
    base_url: String,
    /// Organization slug
    org: String,
    /// API key for initial registration
    api_key: String,
    /// Connection state (agent ID, token, etc.)
    state: Arc<RwLock<ConnectionState>>,
    /// Agent name
    agent_name: String,
    /// Agent version
    agent_version: String,
}

impl ManagementClient {
    /// Create a new management client from configuration
    pub fn new(
        config: &ManagementSettings,
        agent_name: String,
        agent_version: String,
    ) -> ManagementResult<Self> {
        let base_url = config
            .url
            .as_ref()
            .ok_or_else(|| ManagementError::NotConfigured("url".to_string()))?
            .trim_end_matches('/')
            .to_string();

        let org = config
            .org
            .as_ref()
            .ok_or_else(|| ManagementError::NotConfigured("org".to_string()))?
            .clone();

        let api_key = config
            .api_key
            .as_ref()
            .ok_or_else(|| ManagementError::NotConfigured("api_key".to_string()))?
            .clone();

        let client = Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()?;

        Ok(Self {
            client,
            base_url,
            org,
            api_key,
            state: Arc::new(RwLock::new(ConnectionState::default())),
            agent_name,
            agent_version,
        })
    }

    /// Get the current connection state
    pub async fn state(&self) -> ConnectionState {
        self.state.read().await.clone()
    }

    /// Check if the agent is registered
    pub async fn is_registered(&self) -> bool {
        let state = self.state.read().await;
        state.agent_id.is_some() && state.token.is_some()
    }

    /// Check if the token needs refresh
    pub async fn needs_token_refresh(&self) -> bool {
        let state = self.state.read().await;
        if let Some(expires_at) = state.token_expires_at {
            // Refresh if token expires in less than 5 minutes
            expires_at < chrono::Utc::now() + chrono::Duration::minutes(5)
        } else {
            true
        }
    }

    /// Register the agent with the management server
    pub async fn register(&self) -> ManagementResult<AgentInfo> {
        let url = format!("{}/orgs/{}/agents/register", self.base_url, self.org);

        let hostname = hostname::get().ok().and_then(|h| h.into_string().ok());

        let request = RegisterAgentRequest {
            name: self.agent_name.clone(),
            hostname,
            version: Some(self.agent_version.clone()),
            labels: serde_json::json!({}),
        };

        info!(url = %url, agent = %self.agent_name, "Registering with management server");

        let response = self
            .client
            .post(&url)
            .header("X-API-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            error!(status = %status, message = %message, "Registration failed");
            return Err(ManagementError::RegistrationFailed(format!(
                "{}: {}",
                status, message
            )));
        }

        let result: RegisterAgentResponse = response.json().await.map_err(|e| {
            ManagementError::Parse(format!("Failed to parse registration response: {}", e))
        })?;

        // Update state with registration info
        {
            let mut state = self.state.write().await;
            state.agent_id = Some(result.agent.id);
            state.token = Some(result.token.clone());
            state.token_expires_at = Some(result.token_expires_at);
        }

        info!(
            agent_id = %result.agent.id,
            "Successfully registered with management server"
        );

        Ok(result.agent)
    }

    /// Send a heartbeat to the management server
    pub async fn heartbeat(
        &self,
        metrics: Option<AgentMetrics>,
    ) -> ManagementResult<HeartbeatResponse> {
        let state = self.state.read().await;
        let agent_id = state.agent_id.ok_or(ManagementError::NotRegistered)?;
        let token = state
            .token
            .as_ref()
            .ok_or(ManagementError::NotRegistered)?
            .clone();
        drop(state);

        let url = format!(
            "{}/orgs/{}/agents/{}/heartbeat",
            self.base_url, self.org, agent_id
        );

        let request = HeartbeatRequest {
            status: Some("healthy".to_string()),
            metrics,
        };

        debug!(url = %url, "Sending heartbeat");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ManagementError::AuthFailed("Token expired".to_string()));
        }

        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ManagementError::ServerError {
                status: status.as_u16(),
                message,
            });
        }

        let result: HeartbeatResponse = response.json().await.map_err(|e| {
            ManagementError::Parse(format!("Failed to parse heartbeat response: {}", e))
        })?;

        debug!("Heartbeat acknowledged");
        Ok(result)
    }

    /// Get the currently promoted bundle info
    pub async fn get_promoted_bundle(&self) -> ManagementResult<Option<BundleInfo>> {
        let state = self.state.read().await;
        let token = state
            .token
            .as_ref()
            .ok_or(ManagementError::NotRegistered)?
            .clone();
        drop(state);

        let url = format!("{}/orgs/{}/bundles/promoted", self.base_url, self.org);

        debug!(url = %url, "Fetching promoted bundle info");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            debug!("No promoted bundle available");
            return Ok(None);
        }

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ManagementError::AuthFailed("Token expired".to_string()));
        }

        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ManagementError::ServerError {
                status: status.as_u16(),
                message,
            });
        }

        let bundle: BundleInfo = response
            .json()
            .await
            .map_err(|e| ManagementError::Parse(format!("Failed to parse bundle info: {}", e)))?;

        debug!(bundle_id = %bundle.id, name = %bundle.name, "Found promoted bundle");
        Ok(Some(bundle))
    }

    /// Download a bundle by ID
    pub async fn download_bundle(&self, bundle_id: Uuid) -> ManagementResult<BundleDownload> {
        let state = self.state.read().await;
        let token = state
            .token
            .as_ref()
            .ok_or(ManagementError::NotRegistered)?
            .clone();
        drop(state);

        let url = format!(
            "{}/orgs/{}/bundles/{}/download",
            self.base_url, self.org, bundle_id
        );

        info!(bundle_id = %bundle_id, "Downloading bundle");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ManagementError::BundleNotFound);
        }

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ManagementError::AuthFailed("Token expired".to_string()));
        }

        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ManagementError::ServerError {
                status: status.as_u16(),
                message,
            });
        }

        let data = response.bytes().await?.to_vec();

        // Calculate checksum
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let checksum = format!("{:x}", hasher.finalize());

        info!(
            bundle_id = %bundle_id,
            size_bytes = data.len(),
            checksum = %checksum,
            "Bundle downloaded successfully"
        );

        Ok(BundleDownload {
            data,
            bundle_id,
            checksum,
        })
    }

    /// Check if a bundle update is available
    pub async fn check_for_update(&self) -> ManagementResult<Option<BundleInfo>> {
        let promoted = self.get_promoted_bundle().await?;

        if let Some(ref bundle) = promoted {
            let state = self.state.read().await;

            // Check if we already have this bundle
            if state.current_bundle_id == Some(bundle.id) {
                if let (Some(current_checksum), Some(new_checksum)) =
                    (&state.current_bundle_checksum, &bundle.checksum)
                {
                    if current_checksum == new_checksum {
                        debug!("Bundle is up to date");
                        return Ok(None);
                    }
                }
            }

            debug!(
                bundle_id = %bundle.id,
                current_bundle_id = ?state.current_bundle_id,
                "Bundle update available"
            );
            return Ok(Some(bundle.clone()));
        }

        Ok(None)
    }

    /// Update the current bundle tracking
    pub async fn set_current_bundle(&self, bundle_id: Uuid, checksum: String) {
        let mut state = self.state.write().await;
        state.current_bundle_id = Some(bundle_id);
        state.current_bundle_checksum = Some(checksum);
    }

    /// Get current bundle info synchronously (for metrics collection)
    /// Returns (bundle_id, bundle_version/checksum)
    pub fn get_current_bundle_sync(&self) -> (Option<Uuid>, Option<String>) {
        // Use try_read to avoid blocking, return None if lock is contended
        match self.state.try_read() {
            Ok(state) => (state.current_bundle_id, state.current_bundle_checksum.clone()),
            Err(_) => (None, None),
        }
    }

    /// Download a data bundle by source ID
    ///
    /// Used to sync data from a data source (e.g., after receiving a DataRefresh SSE event).
    pub async fn download_data_bundle(&self, source_id: Uuid) -> ManagementResult<DataBundleDownload> {
        let state = self.state.read().await;
        let token = state
            .token
            .as_ref()
            .ok_or(ManagementError::NotRegistered)?
            .clone();
        drop(state);

        let url = format!(
            "{}/orgs/{}/sources/{}/data",
            self.base_url, self.org, source_id
        );

        info!(source_id = %source_id, "Downloading data bundle");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/octet-stream")
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ManagementError::DataSourceNotFound(source_id));
        }

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ManagementError::AuthFailed("Token expired".to_string()));
        }

        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ManagementError::ServerError {
                status: status.as_u16(),
                message,
            });
        }

        let data = response.bytes().await?.to_vec();

        // Calculate checksum
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let checksum = format!("{:x}", hasher.finalize());

        info!(
            source_id = %source_id,
            size_bytes = data.len(),
            checksum = %checksum,
            "Data bundle downloaded successfully"
        );

        Ok(DataBundleDownload {
            data,
            source_id,
            checksum,
        })
    }
}

impl std::fmt::Debug for ManagementClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementClient")
            .field("base_url", &self.base_url)
            .field("org", &self.org)
            .field("agent_name", &self.agent_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_default() {
        let state = ConnectionState::default();
        assert!(state.agent_id.is_none());
        assert!(state.token.is_none());
        assert!(state.current_bundle_id.is_none());
    }
}
