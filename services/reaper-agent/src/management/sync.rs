//! Background sync service for management plane
//!
//! Handles:
//! - Periodic heartbeats
//! - Polling for bundle updates
//! - Automatic bundle deployment

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use policy_engine::PolicyEngine;
use reaper_core::config::ManagementSettings;

use super::client::ManagementClient;
use super::types::{AgentMetrics, ManagementError};

/// Bundle update notification
#[derive(Debug, Clone)]
pub struct BundleUpdate {
    pub bundle_id: uuid::Uuid,
    pub checksum: String,
    pub data: Arc<Vec<u8>>,
}

/// Sync service for management plane communication
pub struct SyncService {
    client: Arc<ManagementClient>,
    config: ManagementSettings,
    policy_engine: Arc<PolicyEngine>,
    /// Channel to send bundle updates
    update_tx: watch::Sender<Option<BundleUpdate>>,
    /// Channel to receive shutdown signal
    shutdown_rx: watch::Receiver<bool>,
}

impl SyncService {
    /// Create a new sync service
    pub fn new(
        client: Arc<ManagementClient>,
        config: ManagementSettings,
        policy_engine: Arc<PolicyEngine>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> (Self, watch::Receiver<Option<BundleUpdate>>) {
        let (update_tx, update_rx) = watch::channel(None);

        let service = Self {
            client,
            config,
            policy_engine,
            update_tx,
            shutdown_rx,
        };

        (service, update_rx)
    }

    /// Run the sync service
    pub async fn run(self) {
        info!("Starting management sync service");

        // Register with management server
        if let Err(e) = self.register_with_retry(3).await {
            error!(error = %e, "Failed to register with management server");
            return;
        }

        // Initial bundle sync
        if self.config.sync_on_startup {
            if let Err(e) = self.sync_bundle().await {
                warn!(error = %e, "Initial bundle sync failed");
            }
        }

        // Start background tasks
        let heartbeat_interval = Duration::from_secs(self.config.heartbeat_interval_secs);
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);

        let mut heartbeat_ticker = tokio::time::interval(heartbeat_interval);
        let mut poll_ticker = tokio::time::interval(poll_interval);

        // Skip the first immediate tick
        heartbeat_ticker.tick().await;
        poll_ticker.tick().await;

        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                _ = heartbeat_ticker.tick() => {
                    if let Err(e) = self.send_heartbeat().await {
                        warn!(error = %e, "Heartbeat failed");
                        // If auth failed, try to re-register
                        if matches!(e, ManagementError::AuthFailed(_)) {
                            if let Err(e) = self.register_with_retry(1).await {
                                error!(error = %e, "Re-registration failed");
                            }
                        }
                    }
                }
                _ = poll_ticker.tick() => {
                    if let Err(e) = self.sync_bundle().await {
                        warn!(error = %e, "Bundle sync failed");
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Sync service shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// Register with the management server with retries
    async fn register_with_retry(&self, max_retries: u32) -> Result<(), ManagementError> {
        let mut attempts = 0;

        loop {
            match self.client.register().await {
                Ok(agent) => {
                    info!(agent_id = %agent.id, "Registered with management server");
                    return Ok(());
                }
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_retries {
                        return Err(e);
                    }
                    warn!(
                        error = %e,
                        attempt = attempts,
                        max_retries = max_retries,
                        "Registration failed, retrying..."
                    );
                    tokio::time::sleep(Duration::from_secs(2_u64.pow(attempts))).await;
                }
            }
        }
    }

    /// Send a heartbeat with current metrics
    async fn send_heartbeat(&self) -> Result<(), ManagementError> {
        let metrics = self.collect_metrics();
        self.client.heartbeat(Some(metrics)).await?;
        debug!("Heartbeat sent");
        Ok(())
    }

    /// Collect current agent metrics
    fn collect_metrics(&self) -> AgentMetrics {
        let policy_count = self.policy_engine.list_policies().len();

        // TODO: Get actual metrics from prometheus/stats
        AgentMetrics {
            requests_per_second: 0.0,
            avg_latency_us: 0.0,
            p99_latency_us: 0.0,
            policy_count,
            memory_bytes: None,
            cache_hit_rate: None,
        }
    }

    /// Sync bundle from management server
    async fn sync_bundle(&self) -> Result<(), ManagementError> {
        // Check for updates
        let update = match self.client.check_for_update().await? {
            Some(bundle) => bundle,
            None => {
                debug!("No bundle updates available");
                return Ok(());
            }
        };

        info!(
            bundle_id = %update.id,
            name = %update.name,
            "Bundle update available, downloading..."
        );

        // Download the bundle
        let download = self.client.download_bundle(update.id).await?;

        // Verify checksum if provided
        if let Some(expected_checksum) = &update.checksum {
            if &download.checksum != expected_checksum {
                return Err(ManagementError::ChecksumMismatch {
                    expected: expected_checksum.clone(),
                    actual: download.checksum,
                });
            }
        }

        // Update tracking
        self.client
            .set_current_bundle(download.bundle_id, download.checksum.clone())
            .await;

        // Notify about the update
        let bundle_update = BundleUpdate {
            bundle_id: download.bundle_id,
            checksum: download.checksum,
            data: Arc::new(download.data),
        };

        // Send update notification (ignoring errors if no receivers)
        let _ = self.update_tx.send(Some(bundle_update));

        info!(bundle_id = %update.id, "Bundle sync complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_update_clone() {
        let update = BundleUpdate {
            bundle_id: uuid::Uuid::new_v4(),
            checksum: "abc123".to_string(),
            data: Arc::new(vec![1, 2, 3]),
        };
        let cloned = update.clone();
        assert_eq!(update.bundle_id, cloned.bundle_id);
        assert_eq!(update.checksum, cloned.checksum);
    }
}
