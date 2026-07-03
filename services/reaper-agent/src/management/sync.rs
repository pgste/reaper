//! Background sync service for management plane
//!
//! Handles:
//! - SSE push notifications (primary - instant)
//! - Periodic heartbeats
//! - Polling for bundle updates (fallback)
//! - Automatic bundle deployment

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use policy_engine::{DataStore, PolicyEngine};
use reaper_core::config::ManagementSettings;

use super::client::ManagementClient;
use super::sse::{ManagementEvent, SseClient, SseConfig};
use super::types::{AgentMetrics, BundleDownload, ManagementError};
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
    /// Shared DataStore for entity data updates via SSE
    data_store: Arc<DataStore>,
    /// Agent statistics for metrics collection
    stats: Arc<AgentStats>,
    /// Agent start time for uptime calculation
    started_at: Instant,
    /// Channel to send bundle updates
    update_tx: watch::Sender<Option<BundleUpdate>>,
    /// Channel to receive shutdown signal
    shutdown_rx: watch::Receiver<bool>,
    /// Whether SSE is currently connected
    sse_connected: bool,
    /// Pinned public key for bundle signature verification (parsed from config).
    verifying_key: Option<reaper_core::bundle_signing::VerifyingKey>,
}

impl SyncService {
    /// Create a new sync service
    pub fn new(
        client: Arc<ManagementClient>,
        config: ManagementSettings,
        policy_engine: Arc<PolicyEngine>,
        data_store: Arc<DataStore>,
        stats: Arc<AgentStats>,
        started_at: Instant,
        shutdown_rx: watch::Receiver<bool>,
    ) -> (Self, watch::Receiver<Option<BundleUpdate>>) {
        let (update_tx, update_rx) = watch::channel(None);

        // Parse the pinned public key once. A bad key/algorithm is a hard
        // configuration error surfaced loudly; verification then fails closed for
        // every bundle.
        let verifying_key = match &config.bundle_public_key {
            Some(hex) => {
                let alg_str = config
                    .bundle_signature_algorithm
                    .as_deref()
                    .unwrap_or(reaper_core::bundle_signing::ALGORITHM);
                match reaper_core::bundle_signing::SigAlgorithm::parse(alg_str)
                    .and_then(|alg| reaper_core::bundle_signing::VerifyingKey::from_hex(alg, hex))
                {
                    Ok(k) => {
                        info!(algorithm = %alg_str, "Bundle signature verification enabled");
                        Some(k)
                    }
                    Err(e) => {
                        error!(error = %e, "Invalid management.bundle_public_key/algorithm; \
                            bundle verification will FAIL CLOSED until fixed");
                        None
                    }
                }
            }
            None => {
                if config.require_signed_bundles {
                    warn!(
                        "require_signed_bundles is true but no bundle_public_key is set — \
                         managed bundles will be REJECTED until a key is configured"
                    );
                }
                None
            }
        };

        let service = Self {
            client,
            config,
            policy_engine,
            data_store,
            stats,
            started_at,
            update_tx,
            shutdown_rx,
            sse_connected: false,
            verifying_key,
        };

        (service, update_rx)
    }

    /// Verify a downloaded bundle's authenticity + integrity before it is
    /// applied. Fail closed: any problem returns an error and the bundle is not
    /// deployed.
    ///
    /// Policy matrix (`require` = `config.require_signed_bundles`):
    /// - key set, signature present  -> verify; reject on failure.
    /// - key set, signature absent   -> reject if `require`, else warn+allow.
    /// - key absent                  -> reject if `require`, else warn+allow.
    fn verify_download(&self, download: &BundleDownload) -> Result<(), ManagementError> {
        verify_bundle_download(
            self.config.require_signed_bundles,
            self.verifying_key.as_ref(),
            self.config.bundle_key_id.as_deref(),
            download,
        )
    }

    /// Run the sync service
    pub async fn run(mut self) {
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

        // Determine poll interval based on SSE configuration
        let heartbeat_interval = Duration::from_secs(self.config.heartbeat_interval_secs);
        let poll_interval = if self.config.sse_enabled {
            Duration::from_secs(self.config.poll_interval_with_sse_secs)
        } else {
            Duration::from_secs(self.config.poll_interval_secs)
        };

        info!(
            sse_enabled = self.config.sse_enabled,
            poll_interval_secs = poll_interval.as_secs(),
            "Sync service configured"
        );

        // Set up SSE event channel
        let (sse_tx, mut sse_rx) = mpsc::channel::<ManagementEvent>(100);

        // Spawn SSE client if enabled
        let sse_shutdown_rx = self.shutdown_rx.clone();
        let sse_task = if self.config.sse_enabled {
            let state = self.client.state().await;
            if let (Some(agent_id), Some(token)) = (state.agent_id, state.token) {
                let sse_config = SseConfig {
                    base_url: self
                        .config
                        .url
                        .clone()
                        .unwrap_or_default()
                        .trim_end_matches('/')
                        .to_string(),
                    org: self.config.org.clone().unwrap_or_default(),
                    agent_id,
                    token,
                    reconnect_initial_secs: self.config.sse_reconnect_initial_secs,
                    reconnect_max_secs: self.config.sse_reconnect_max_secs,
                };

                let sse_client = SseClient::new(sse_config, sse_tx);
                Some(tokio::spawn(async move {
                    sse_client.run(sse_shutdown_rx).await;
                }))
            } else {
                warn!("SSE enabled but agent not registered, skipping SSE");
                None
            }
        } else {
            None
        };

        let mut heartbeat_ticker = tokio::time::interval(heartbeat_interval);
        let mut poll_ticker = tokio::time::interval(poll_interval);

        // Skip the first immediate tick
        heartbeat_ticker.tick().await;
        poll_ticker.tick().await;

        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                // SSE events (primary - instant)
                Some(event) = sse_rx.recv() => {
                    self.handle_sse_event(event).await;
                }
                // Heartbeat (unchanged)
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
                // Poll fallback (longer interval when SSE active)
                _ = poll_ticker.tick() => {
                    debug!(sse_connected = self.sse_connected, "Polling for bundle updates");
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

        // Wait for SSE task to finish
        if let Some(task) = sse_task {
            let _ = task.await;
        }
    }

    /// Handle an SSE event
    async fn handle_sse_event(&mut self, event: ManagementEvent) {
        match event {
            ManagementEvent::Connected => {
                self.sse_connected = true;
                info!("SSE connected - real-time updates active");
            }
            ManagementEvent::Disconnected { error } => {
                self.sse_connected = false;
                if let Some(err) = error {
                    warn!(error = %err, "SSE disconnected");
                } else {
                    info!("SSE disconnected");
                }
            }
            ManagementEvent::BundlePromoted {
                bundle_id, version, ..
            } => {
                info!(
                    bundle_id = %bundle_id,
                    version = %version,
                    "Received BundlePromoted event via SSE"
                );
                // Trigger immediate bundle sync
                if let Err(e) = self.sync_bundle_by_id(bundle_id).await {
                    warn!(error = %e, bundle_id = %bundle_id, "Failed to sync bundle from SSE event");
                }
            }
            ManagementEvent::DataRefresh {
                source_id,
                source_type,
                ..
            } => {
                info!(
                    source_id = %source_id,
                    source_type = %source_type,
                    "Received DataRefresh event via SSE"
                );
                // Data refresh handling will be implemented in Phase 4
                if let Err(e) = self.sync_data_source(source_id, &source_type).await {
                    warn!(error = %e, source_id = %source_id, "Failed to sync data source from SSE event");
                }
            }
            ManagementEvent::PolicyUpdated {
                policy_id, version, ..
            } => {
                info!(
                    policy_id = %policy_id,
                    version = version,
                    "Received PolicyUpdated event via SSE"
                );
                // Trigger bundle sync to get latest policies
                if let Err(e) = self.sync_bundle().await {
                    warn!(error = %e, "Failed to sync bundle after PolicyUpdated event");
                }
            }
            ManagementEvent::Ping { timestamp } => {
                debug!(timestamp = %timestamp, "SSE ping received");
            }
        }
    }

    /// Sync a specific bundle by ID
    async fn sync_bundle_by_id(&self, bundle_id: Uuid) -> Result<(), ManagementError> {
        info!(bundle_id = %bundle_id, "Downloading bundle by ID");

        // Download the bundle
        let download = self.client.download_bundle(bundle_id).await?;

        // Verify authenticity + integrity BEFORE applying (fail closed).
        self.verify_download(&download)?;

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

        info!(bundle_id = %bundle_id, "Bundle sync from SSE complete");
        Ok(())
    }

    /// Sync data from a data source
    ///
    /// Downloads the data bundle and atomically replaces the DataStore contents.
    async fn sync_data_source(
        &self,
        source_id: Uuid,
        source_type: &str,
    ) -> Result<(), ManagementError> {
        info!(
            source_id = %source_id,
            source_type = %source_type,
            "Downloading data bundle from source"
        );

        // Download the data bundle
        let download = self.client.download_data_bundle(source_id).await?;

        info!(
            source_id = %source_id,
            size_bytes = download.data.len(),
            checksum = %download.checksum,
            "Data bundle downloaded, loading into DataStore"
        );

        // Parse the data bundle
        let bundle = policy_engine::DataBundle::from_bytes(&download.data).map_err(|e| {
            ManagementError::DataLoadError(format!("Failed to parse data bundle: {}", e))
        })?;

        let entity_count = bundle.metadata.entity_count;
        let bundle_version = bundle.metadata.version.clone();

        // Atomically replace the DataStore contents with the bundle data
        bundle.replace_store(&self.data_store).map_err(|e| {
            ManagementError::DataLoadError(format!("Failed to load data bundle: {}", e))
        })?;

        info!(
            source_id = %source_id,
            entity_count = entity_count,
            bundle_version = %bundle_version,
            "Data source sync complete - {} entities loaded",
            entity_count
        );

        Ok(())
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

        // Verify authenticity + integrity BEFORE applying (fail closed).
        self.verify_download(&download)?;

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

/// Decide whether a downloaded bundle may be applied, and verify it if a key is
/// present. Fail closed. Extracted as a free function so the policy matrix is
/// unit-testable without a full `SyncService`.
///
/// `require` = `require_signed_bundles`:
/// - key set, signature present -> verify (integrity + authenticity); reject on failure.
/// - key set, signature absent  -> reject if `require`, else warn+allow.
/// - key absent                 -> reject if `require`, else warn+allow.
fn verify_bundle_download(
    require: bool,
    key: Option<&reaper_core::bundle_signing::VerifyingKey>,
    key_id_pin: Option<&str>,
    download: &BundleDownload,
) -> Result<(), ManagementError> {
    match (key, &download.signature) {
        (Some(key), Some(sig)) => {
            reaper_core::bundle_signing::verify_bundle(&download.data, sig, key, key_id_pin)
                .map_err(|e| ManagementError::SignatureVerification(e.to_string()))?;
            info!(bundle_id = %download.bundle_id, key_id = %sig.key_id,
                "Bundle signature verified");
            Ok(())
        }
        (Some(_), None) => {
            if require {
                Err(ManagementError::SignatureVerification(
                    "bundle is unsigned but a verification key is configured and \
                     require_signed_bundles is true"
                        .to_string(),
                ))
            } else {
                warn!(bundle_id = %download.bundle_id,
                    "Applying UNSIGNED bundle (require_signed_bundles=false)");
                Ok(())
            }
        }
        (None, _) => {
            if require {
                Err(ManagementError::SignatureVerification(
                    "require_signed_bundles is true but no bundle_public_key is configured"
                        .to_string(),
                ))
            } else {
                warn!(bundle_id = %download.bundle_id,
                    "Bundle signature verification DISABLED (no key, require_signed_bundles=false)");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaper_core::bundle_signing::{sign_bundle, SigningKey, VerifyingKey};

    fn test_key() -> SigningKey {
        SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[3u8; 32])))
    }

    fn vk(key: &SigningKey) -> VerifyingKey {
        VerifyingKey::from_hex(key.algorithm(), &key.public_key_hex()).unwrap()
    }

    fn dl(
        data: &[u8],
        signature: Option<reaper_core::bundle_signing::BundleSignature>,
    ) -> BundleDownload {
        BundleDownload {
            data: data.to_vec(),
            bundle_id: uuid::Uuid::new_v4(),
            checksum: "x".to_string(),
            signature,
        }
    }

    #[test]
    fn signed_bundle_with_pinned_key_is_accepted() {
        let key = test_key();
        let data = b"bundle-bytes";
        let sig = sign_bundle(data, &key, "k1");
        let d = dl(data, Some(sig));
        verify_bundle_download(true, Some(&vk(&key)), None, &d).unwrap();
    }

    #[test]
    fn tampered_signed_bundle_is_rejected() {
        let key = test_key();
        let sig = sign_bundle(b"original", &key, "k1");
        let d = dl(b"tampered", Some(sig)); // bytes differ from what was signed
        let err = verify_bundle_download(true, Some(&vk(&key)), None, &d).unwrap_err();
        assert!(matches!(err, ManagementError::SignatureVerification(_)));
    }

    #[test]
    fn required_but_unsigned_is_rejected() {
        let key = test_key();
        let d = dl(b"bundle", None);
        let err = verify_bundle_download(true, Some(&vk(&key)), None, &d).unwrap_err();
        assert!(matches!(err, ManagementError::SignatureVerification(_)));
    }

    #[test]
    fn required_but_no_key_is_rejected() {
        let d = dl(b"bundle", None);
        let err = verify_bundle_download(true, None, None, &d).unwrap_err();
        assert!(matches!(err, ManagementError::SignatureVerification(_)));
    }

    #[test]
    fn not_required_and_no_key_allows() {
        let d = dl(b"bundle", None);
        verify_bundle_download(false, None, None, &d).unwrap();
    }

    #[test]
    fn wrong_key_id_pin_is_rejected() {
        let key = test_key();
        let sig = sign_bundle(b"bundle", &key, "k1");
        let d = dl(b"bundle", Some(sig));
        let err = verify_bundle_download(true, Some(&vk(&key)), Some("k2"), &d).unwrap_err();
        assert!(matches!(err, ManagementError::SignatureVerification(_)));
    }

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
