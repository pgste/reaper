//! Sync Engine
//!
//! Orchestrates the synchronization of policies from the management server
//! to the agent. Handles change detection, retries, and continuous polling.

use crate::agent_client::{AgentClient, AgentClientError};
use crate::config::SyncConfig;
use crate::server_client::{PolicySummary, ServerClient, ServerClientError};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, error, info, instrument, warn};

/// Sync engine errors
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Server error: {0}")]
    Server(#[from] ServerClientError),
    #[error("Agent error: {0}")]
    Agent(#[from] AgentClientError),
    #[error("Sync failed: {0}")]
    SyncFailed(String),
}

/// Result of a sync operation
#[derive(Debug, Default)]
pub struct SyncResult {
    /// Number of policies deployed
    pub deployed: usize,
    /// Number of policies skipped (no changes)
    pub skipped: usize,
    /// Number of policies that failed to deploy
    pub failed: usize,
    /// Number of entities synced
    pub entities_synced: usize,
    /// Whether the sync completed successfully
    pub success: bool,
    /// Error message if sync failed
    pub error: Option<String>,
}

impl SyncResult {
    /// Create a successful result
    pub fn success(deployed: usize, skipped: usize) -> Self {
        Self {
            deployed,
            skipped,
            failed: 0,
            entities_synced: 0,
            success: true,
            error: None,
        }
    }

    /// Create a partial success result
    pub fn partial(deployed: usize, skipped: usize, failed: usize) -> Self {
        Self {
            deployed,
            skipped,
            failed,
            entities_synced: 0,
            success: failed == 0,
            error: if failed > 0 {
                Some(format!("{} policies failed to deploy", failed))
            } else {
                None
            },
        }
    }

    /// Create a failure result
    pub fn failure(error: String) -> Self {
        Self {
            deployed: 0,
            skipped: 0,
            failed: 0,
            entities_synced: 0,
            success: false,
            error: Some(error),
        }
    }
}

/// Tracked policy state for change detection
#[derive(Debug, Clone)]
struct PolicyState {
    version: u64,
    checksum: Option<String>,
}

/// Sync engine for orchestrating policy synchronization
pub struct SyncEngine {
    config: SyncConfig,
    server_client: ServerClient,
    agent_client: AgentClient,
    /// Last known state of policies (for change detection)
    last_synced: HashMap<String, PolicyState>,
    /// Statistics
    total_syncs: u64,
    total_policies_deployed: u64,
}

impl SyncEngine {
    /// Create a new sync engine
    pub fn new(config: SyncConfig) -> Result<Self, SyncError> {
        let server_client = ServerClient::new(config.clone())?;
        let agent_client = AgentClient::new(&config)?;

        Ok(Self {
            config,
            server_client,
            agent_client,
            last_synced: HashMap::new(),
            total_syncs: 0,
            total_policies_deployed: 0,
        })
    }

    /// Check if a policy needs to be synced
    fn needs_sync(&self, policy: &PolicySummary) -> bool {
        match self.last_synced.get(&policy.id) {
            Some(state) => {
                // Check version change
                if state.version != policy.version {
                    return true;
                }
                // Check checksum change (if available)
                if let (Some(old_checksum), Some(new_checksum)) =
                    (&state.checksum, &policy.checksum)
                {
                    if old_checksum != new_checksum {
                        return true;
                    }
                }
                false
            }
            None => true, // Never synced
        }
    }

    /// Mark a policy as synced
    fn mark_synced(&mut self, policy: &PolicySummary) {
        self.last_synced.insert(
            policy.id.clone(),
            PolicyState {
                version: policy.version,
                checksum: policy.checksum.clone(),
            },
        );
    }

    /// Perform a single sync operation
    #[instrument(skip(self))]
    pub async fn sync_once(&mut self) -> SyncResult {
        info!("Starting policy synchronization");
        self.total_syncs += 1;

        // 1. Check agent health
        match self.agent_client.health_check().await {
            Ok(true) => debug!("Agent is healthy"),
            Ok(false) => {
                warn!("Agent health check returned unhealthy");
                return SyncResult::failure("Agent is unhealthy".to_string());
            }
            Err(e) => {
                warn!("Agent health check failed: {}", e);
                return SyncResult::failure(format!("Agent unavailable: {}", e));
            }
        }

        // 2. Fetch policy list from server
        let policies = match self.server_client.list_policies().await {
            Ok(response) => response.policies,
            Err(e) => {
                error!("Failed to fetch policies from server: {}", e);
                return SyncResult::failure(format!("Server error: {}", e));
            }
        };

        info!("Server has {} policies in scope", policies.len());

        // 3. Check which policies need syncing
        let mut deployed = 0;
        let mut skipped = 0;
        let mut failed = 0;

        for policy_summary in &policies {
            if !self.needs_sync(policy_summary) {
                debug!("Policy {} unchanged, skipping", policy_summary.name);
                skipped += 1;
                continue;
            }

            // 4. Fetch full policy details
            let policy_detail = match self.server_client.get_policy(&policy_summary.id).await {
                Ok(detail) => detail,
                Err(e) => {
                    warn!(
                        "Failed to fetch policy {} details: {}",
                        policy_summary.id, e
                    );
                    failed += 1;
                    continue;
                }
            };

            // 5. Deploy to agent
            match self.agent_client.deploy_policy(&policy_detail).await {
                Ok(response) => {
                    if response.error.is_some() {
                        warn!(
                            "Policy {} deployment returned error: {:?}",
                            policy_summary.name, response.error
                        );
                        failed += 1;
                    } else {
                        info!(
                            "Deployed policy: {} (v{}) -> agent",
                            policy_summary.name, policy_summary.version
                        );
                        self.mark_synced(policy_summary);
                        self.total_policies_deployed += 1;
                        deployed += 1;
                    }
                }
                Err(e) => {
                    error!("Failed to deploy policy {}: {}", policy_summary.name, e);
                    failed += 1;
                }
            }
        }

        let result = SyncResult::partial(deployed, skipped, failed);

        info!(
            "Sync complete: deployed={}, skipped={}, failed={}",
            deployed, skipped, failed
        );

        result
    }

    /// Sync entity data from server to agent
    #[instrument(skip(self))]
    pub async fn sync_entities(&mut self, replace_all: bool) -> SyncResult {
        info!("Starting entity data synchronization");

        // Fetch entities from server
        let entities = match self.server_client.get_entities().await {
            Ok(response) => response.entities,
            Err(e) => {
                error!("Failed to fetch entities from server: {}", e);
                return SyncResult::failure(format!("Server error: {}", e));
            }
        };

        if entities.is_empty() {
            debug!("No entities to sync");
            return SyncResult::success(0, 0);
        }

        info!("Syncing {} entities to agent", entities.len());

        // Sync to agent
        match self.agent_client.sync_data(entities.clone(), replace_all).await {
            Ok(response) => {
                let mut result = SyncResult::success(0, 0);
                result.entities_synced = response.inserted;
                if response.failed > 0 {
                    result.error = Some(format!("{} entities failed to sync", response.failed));
                }
                result
            }
            Err(e) => {
                error!("Failed to sync entities to agent: {}", e);
                SyncResult::failure(format!("Agent error: {}", e))
            }
        }
    }

    /// Run continuous synchronization
    #[instrument(skip(self))]
    pub async fn run_continuous(&mut self) -> Result<(), SyncError> {
        let poll_interval =
            std::time::Duration::from_secs(self.config.sync.behavior.poll_interval_seconds);

        info!(
            "Starting continuous sync (poll interval: {}s)",
            self.config.sync.behavior.poll_interval_seconds
        );

        // Initial sync
        if self.config.sync.behavior.sync_on_start {
            let result = self.sync_once().await;
            if !result.success {
                warn!("Initial sync failed: {:?}", result.error);
            }
        }

        // Continuous polling
        loop {
            tokio::time::sleep(poll_interval).await;

            let result = self.sync_once().await;

            if !result.success {
                warn!("Sync iteration failed: {:?}", result.error);

                // Apply retry backoff on failure
                let backoff =
                    std::time::Duration::from_secs(self.config.sync.behavior.retry_backoff_seconds);
                tokio::time::sleep(backoff).await;
            }
        }
    }

    /// Get sync statistics
    pub fn stats(&self) -> SyncStats {
        SyncStats {
            total_syncs: self.total_syncs,
            total_policies_deployed: self.total_policies_deployed,
            tracked_policies: self.last_synced.len(),
        }
    }
}

/// Sync statistics
#[derive(Debug)]
pub struct SyncStats {
    pub total_syncs: u64,
    pub total_policies_deployed: u64,
    pub tracked_policies: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SyncConfig {
        SyncConfig {
            sync: crate::config::SyncSettings {
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
                    teams: vec![],
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
    fn test_sync_engine_creation() {
        let config = test_config();
        let engine = SyncEngine::new(config);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_sync_result_success() {
        let result = SyncResult::success(5, 3);
        assert!(result.success);
        assert_eq!(result.deployed, 5);
        assert_eq!(result.skipped, 3);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_sync_result_partial() {
        let result = SyncResult::partial(3, 2, 1);
        assert!(!result.success);
        assert_eq!(result.deployed, 3);
        assert_eq!(result.failed, 1);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_sync_result_failure() {
        let result = SyncResult::failure("Test error".to_string());
        assert!(!result.success);
        assert_eq!(result.error, Some("Test error".to_string()));
    }
}
