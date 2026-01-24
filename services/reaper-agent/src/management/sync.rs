//! Background sync service for management plane
//!
//! Handles:
//! - Periodic heartbeats
//! - Polling for bundle updates
//! - Automatic bundle deployment

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use policy_engine::PolicyEngine;
use reaper_core::config::ManagementSettings;

use super::client::ManagementClient;
use super::types::{AgentMetrics, ManagementError};
use crate::AgentStats;

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
    /// Agent statistics for metrics collection
    stats: Arc<AgentStats>,
    /// Agent start time for uptime calculation
    started_at: Instant,
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
        stats: Arc<AgentStats>,
        started_at: Instant,
        shutdown_rx: watch::Receiver<bool>,
    ) -> (Self, watch::Receiver<Option<BundleUpdate>>) {
        let (update_tx, update_rx) = watch::channel(None);

        let service = Self {
            client,
            config,
            policy_engine,
            stats,
            started_at,
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
        // Get request stats
        let requests_total = self.stats.requests_processed.load(Ordering::Relaxed);
        let total_eval_time_ns = self.stats.total_evaluation_time_ns.load(Ordering::Relaxed);

        // Calculate uptime
        let uptime_seconds = self.started_at.elapsed().as_secs();

        // Calculate requests per second (avoid division by zero)
        let requests_per_second = if uptime_seconds > 0 {
            requests_total as f64 / uptime_seconds as f64
        } else {
            0.0
        };

        // Calculate average latency (avoid division by zero)
        let avg_latency_us = if requests_total > 0 {
            (total_eval_time_ns as f64 / requests_total as f64) / 1000.0 // ns to µs
        } else {
            0.0
        };

        // Get accurate p50 and p99 from HDR histogram
        let p50_latency_us = self.stats.get_latency_percentile_us(50.0);
        let p99_latency_us = self.stats.get_latency_percentile_us(99.0);

        // Get memory usage from sysinfo (cross-platform)
        let memory_bytes = self.stats.get_memory_bytes();

        // Get CPU usage from sysinfo (cross-platform)
        let cpu_percent = self.stats.get_cpu_percent();

        // Get real allow/deny decision counts
        let decisions_allow = self.stats.decisions_allow.load(Ordering::Relaxed);
        let decisions_deny = self.stats.decisions_deny.load(Ordering::Relaxed);

        // Get current bundle info from client
        let (current_bundle_id, current_bundle_version) = self.client.get_current_bundle_sync();

        AgentMetrics {
            requests_total,
            requests_per_second,
            avg_latency_us,
            p50_latency_us,
            p99_latency_us,
            memory_bytes,
            cpu_percent,
            decisions_allow,
            decisions_deny,
            uptime_seconds,
            current_bundle_id,
            current_bundle_version,
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
