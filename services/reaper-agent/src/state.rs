//! Agent state and statistics management.
//!
//! This module contains the core state structures shared across all handlers.

use parking_lot::Mutex;
use policy_engine::{
    cache_config::CacheConfig, decision_cache::DecisionCache, PolicyEngine, SharedDecisionBuffer,
};
use reaper_core::config::ReaperAgentConfig;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::cache::PolicyCache;
use parking_lot::RwLock;
use std::sync::atomic::AtomicI64;

/// Shared agent state accessible by all request handlers.
///
/// This struct is wrapped in `Arc` and extracted via Axum's `State` extractor
/// for thread-safe sharing across all handlers.
#[derive(Clone)]
pub struct AgentState {
    /// Lock-free policy store for sub-microsecond lookups
    pub policy_engine: PolicyEngine,
    /// Shared entity store for ABAC/ReBAC evaluations
    pub data_store: Arc<policy_engine::DataStore>,
    /// Performance metrics and statistics
    pub stats: Arc<AgentStats>,
    /// Optional decision cache (environment configurable)
    pub decision_cache: Option<Arc<DecisionCache>>,
    /// Cache configuration for logging/metrics
    pub cache_config: CacheConfig,
    /// Full agent configuration
    pub agent_config: ReaperAgentConfig,
    /// Optional disk cache for policies
    pub policy_cache: Option<Arc<PolicyCache>>,
    /// Decision logging buffer (OPA-style audit)
    pub decision_buffer: Option<SharedDecisionBuffer>,
    /// Agent identifier for decision logs
    pub agent_id: String,
    /// Cache of per-policy Prometheus metric handles (avoids re-hashing label
    /// values and re-locking the metric vecs on every request).
    pub decision_metrics: Arc<crate::metrics_cache::DecisionMetrics>,
    /// Data-plane sync state: which datastore version this agent serves and
    /// how the configured staleness budget is enforced.
    pub data_sync: Arc<DataSyncState>,
}

/// What to do when the data-plane staleness budget is exceeded.
///
/// The budget question is availability vs. certainty and belongs to the
/// OPERATOR, not us — so it's configuration, not policy:
/// - `Monitor`: metrics/logs only (default when no budget is set)
/// - `Flag`: keep serving, stamp `data_stale: true` into every decision log
///   entry so audits can see exactly which decisions ran on old data
/// - `Enforce`: FAIL CLOSED — deny everything until data catches up
///   ("should reaper stop returning true": yes, in this mode)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessMode {
    Monitor,
    Flag,
    Enforce,
}

impl StalenessMode {
    pub fn from_env() -> Self {
        match std::env::var("REAPER_DATA_STALENESS_MODE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "enforce" | "deny" => Self::Enforce,
            "flag" => Self::Flag,
            _ => Self::Monitor,
        }
    }
}

/// Read-replica style sync bookkeeping for the data plane.
///
/// Integrity: versions are only recorded after the agent has verified the
/// published sha256 checksum over the canonical document — a corrupt or
/// tampered payload is rejected before it ever reaches the DataStore
/// (same contract as a replica refusing a bad WAL segment).
pub struct DataSyncState {
    /// Last verified datastore version (0 = never synced).
    pub version: AtomicI64,
    /// Checksum of that version ("sha256:…").
    pub checksum: RwLock<String>,
    /// Unix seconds of the last successful sync (0 = never).
    pub last_synced_epoch: AtomicU64,
    /// Position in the control plane's change stream (delta sync). Set to
    /// the snapshot's change_seq on deploy-version; advanced by
    /// apply-deltas. Contiguity is enforced: a delta batch must start
    /// exactly here or the agent 409s with this value so the sync client
    /// self-corrects (pull-based gap repair).
    pub applied_seq: AtomicI64,
    /// Staleness budget in seconds; 0 = no budget configured.
    pub max_staleness_secs: u64,
    /// Behavior when the budget is exceeded.
    pub mode: StalenessMode,
    /// Cold-start gate (REAPER_DATA_REQUIRE_SYNC): until the first
    /// VERIFIED snapshot lands, /ready reports 503 (orchestrators keep the
    /// pod out of rotation) and evaluation fails closed. Off by default —
    /// standalone / bootstrap-file agents have no data plane to wait for.
    pub require_sync: bool,
}

impl DataSyncState {
    pub fn from_env() -> Self {
        Self {
            version: AtomicI64::new(0),
            checksum: RwLock::new(String::new()),
            last_synced_epoch: AtomicU64::new(0),
            applied_seq: AtomicI64::new(0),
            max_staleness_secs: std::env::var("REAPER_DATA_MAX_STALENESS_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            mode: StalenessMode::from_env(),
            require_sync: std::env::var("REAPER_DATA_REQUIRE_SYNC")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false),
        }
    }

    fn now_epoch() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Record a verified sync (called AFTER checksum verification passes).
    pub fn record_sync(&self, version: i64, checksum: String) {
        *self.checksum.write() = checksum;
        self.version.store(version, Ordering::Release);
        self.last_synced_epoch
            .store(Self::now_epoch(), Ordering::Release);
    }

    /// Refresh the staleness clock WITHOUT changing version/checksum — a
    /// verified "you are already current" from the control plane is a
    /// replica heartbeat: staleness measures lag behind the primary, not
    /// time since the last data change. A quiet hour with no publishes
    /// must not make a current agent look stale.
    pub fn record_heartbeat(&self) {
        self.last_synced_epoch
            .store(Self::now_epoch(), Ordering::Release);
    }

    /// Seconds since the last successful sync, if the agent has ever synced.
    /// An agent that never synced (bootstrap-file / standalone mode) has no
    /// staleness clock — budgets only apply once the data plane is in use.
    pub fn staleness_secs(&self) -> Option<u64> {
        let last = self.last_synced_epoch.load(Ordering::Acquire);
        if last == 0 {
            return None;
        }
        Some(Self::now_epoch().saturating_sub(last))
    }

    /// True when a budget is configured AND the last sync is older than it.
    pub fn is_stale(&self) -> bool {
        if self.max_staleness_secs == 0 {
            return false;
        }
        self.staleness_secs()
            .is_some_and(|s| s > self.max_staleness_secs)
    }

    /// True while the cold-start gate is armed and no verified snapshot
    /// has landed yet (version is only set after checksum verification).
    #[inline]
    pub fn awaiting_initial_sync(&self) -> bool {
        self.require_sync && self.version.load(Ordering::Acquire) == 0
    }

    /// Why evaluation must FAIL CLOSED right now, if it must. Two relaxed
    /// atomic loads on the hot path; None when healthy.
    #[inline]
    pub fn deny_reason(&self) -> Option<&'static str> {
        if self.awaiting_initial_sync() {
            return Some("awaiting_initial_data_sync");
        }
        if self.mode == StalenessMode::Enforce && self.is_stale() {
            return Some("data_staleness_exceeded");
        }
        None
    }

    /// Whether evaluation must FAIL CLOSED right now.
    #[inline]
    pub fn must_deny(&self) -> bool {
        self.deny_reason().is_some()
    }

    /// Whether decision entries should be flagged stale right now.
    #[inline]
    pub fn flag_stale(&self) -> bool {
        self.mode != StalenessMode::Monitor && self.is_stale()
    }

    /// (version, checksum) snapshot for stamping decisions; version 0 → None.
    pub fn provenance(&self) -> (i64, Option<String>) {
        let version = self.version.load(Ordering::Acquire);
        if version == 0 {
            (0, None)
        } else {
            (version, Some(self.checksum.read().clone()))
        }
    }
}

impl std::fmt::Debug for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentState")
            .field("policy_engine", &self.policy_engine)
            .field("data_store", &"DataStore")
            .field("stats", &"AgentStats")
            .field(
                "decision_cache",
                &self.decision_cache.as_ref().map(|_| "DecisionCache"),
            )
            .field("cache_config", &self.cache_config)
            .field("agent_config", &self.agent_config)
            .field(
                "policy_cache",
                &self.policy_cache.as_ref().map(|_| "PolicyCache"),
            )
            .field(
                "decision_buffer",
                &self.decision_buffer.as_ref().map(|_| "DecisionBuffer"),
            )
            .field("agent_id", &self.agent_id)
            .finish()
    }
}

/// Thread-safe statistics for metrics collection.
///
/// Uses atomic counters for lock-free updates on the hot path.
/// HDR histogram provides accurate latency percentiles when enhanced metrics are enabled.
pub struct AgentStats {
    /// Total requests processed
    pub requests_processed: AtomicU64,
    /// Cumulative evaluation time in nanoseconds
    pub total_evaluation_time_ns: AtomicU64,
    /// Policy cache hit count
    pub policy_cache_hits: AtomicU64,
    /// Policy cache miss count
    pub policy_cache_misses: AtomicU64,
    /// Decision cache hit count
    pub decision_cache_hits: AtomicU64,
    /// Decision cache miss count
    pub decision_cache_misses: AtomicU64,
    /// Count of allow decisions
    pub decisions_allow: AtomicU64,
    /// Count of deny decisions
    pub decisions_deny: AtomicU64,
    /// Whether enhanced metrics (histogram, CPU, memory) are enabled
    enhanced_metrics_enabled: bool,
    /// HDR histogram for accurate latency percentiles (nanoseconds)
    /// Range: 1ns to 1 second, 3 significant figures
    latency_histogram: Mutex<hdrhistogram::Histogram<u64>>,
    /// System info for CPU/memory monitoring
    system_info: Mutex<sysinfo::System>,
}

impl AgentStats {
    /// Create new AgentStats with optional enhanced metrics.
    ///
    /// When `enhanced_metrics_enabled` is true:
    /// - HDR histogram tracks latency percentiles
    /// - CPU and memory usage are available
    ///
    /// Enhanced metrics add slight overhead, so they're disabled by default.
    pub fn new(enhanced_metrics_enabled: bool) -> Self {
        Self {
            requests_processed: AtomicU64::new(0),
            total_evaluation_time_ns: AtomicU64::new(0),
            policy_cache_hits: AtomicU64::new(0),
            policy_cache_misses: AtomicU64::new(0),
            decision_cache_hits: AtomicU64::new(0),
            decision_cache_misses: AtomicU64::new(0),
            decisions_allow: AtomicU64::new(0),
            decisions_deny: AtomicU64::new(0),
            enhanced_metrics_enabled,
            // Histogram: 1ns to 1s range, 3 significant figures
            latency_histogram: Mutex::new(
                hdrhistogram::Histogram::new_with_bounds(1, 1_000_000_000, 3)
                    .expect("Failed to create histogram"),
            ),
            system_info: Mutex::new(sysinfo::System::new()),
        }
    }

    /// Record a policy evaluation with its duration in nanoseconds.
    pub fn record_evaluation(&self, evaluation_time_ns: u64) {
        self.requests_processed.fetch_add(1, Ordering::Relaxed);
        self.total_evaluation_time_ns
            .fetch_add(evaluation_time_ns, Ordering::Relaxed);

        // Only record to histogram if enhanced metrics are enabled
        if self.enhanced_metrics_enabled {
            if let Some(mut histogram) = self.latency_histogram.try_lock() {
                // Clamp to histogram range (1ns to 1s)
                let clamped = evaluation_time_ns.clamp(1, 1_000_000_000);
                let _ = histogram.record(clamped);
            }
        }
    }

    /// Record an allow decision.
    pub fn record_allow(&self) {
        self.decisions_allow.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a deny decision.
    pub fn record_deny(&self) {
        self.decisions_deny.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a policy cache hit.
    pub fn record_cache_hit(&self) {
        self.policy_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a policy cache miss.
    pub fn record_cache_miss(&self) {
        self.policy_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a decision cache hit.
    pub fn record_decision_cache_hit(&self) {
        self.decision_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a decision cache miss.
    pub fn record_decision_cache_miss(&self) {
        self.decision_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get latency percentile in nanoseconds.
    ///
    /// Returns 0 if enhanced metrics disabled, histogram empty, or lock contended.
    pub fn get_latency_percentile_ns(&self, percentile: f64) -> u64 {
        if !self.enhanced_metrics_enabled {
            return 0;
        }
        if let Some(histogram) = self.latency_histogram.try_lock() {
            if histogram.len() > 0 {
                return histogram.value_at_percentile(percentile);
            }
        }
        0
    }

    /// Get latency percentile in microseconds.
    pub fn get_latency_percentile_us(&self, percentile: f64) -> f64 {
        self.get_latency_percentile_ns(percentile) as f64 / 1000.0
    }

    /// Get current CPU usage percentage for this process.
    ///
    /// Returns 0.0 if enhanced metrics disabled or unable to read CPU info.
    pub fn get_cpu_percent(&self) -> f64 {
        if !self.enhanced_metrics_enabled {
            return 0.0;
        }
        use sysinfo::{Pid, ProcessRefreshKind, RefreshKind};

        if let Some(mut system) = self.system_info.try_lock() {
            let pid = Pid::from_u32(std::process::id());

            // Refresh only process CPU info for efficiency
            let refresh_kind =
                RefreshKind::new().with_processes(ProcessRefreshKind::new().with_cpu());
            system.refresh_specifics(refresh_kind);

            if let Some(process) = system.process(pid) {
                return process.cpu_usage() as f64;
            }
        }
        0.0
    }

    /// Get current memory usage in bytes for this process.
    ///
    /// Returns 0 if enhanced metrics disabled or unable to read memory info.
    pub fn get_memory_bytes(&self) -> u64 {
        if !self.enhanced_metrics_enabled {
            return 0;
        }
        use sysinfo::{Pid, ProcessRefreshKind, RefreshKind};

        if let Some(mut system) = self.system_info.try_lock() {
            let pid = Pid::from_u32(std::process::id());

            // Refresh only process memory info
            let refresh_kind =
                RefreshKind::new().with_processes(ProcessRefreshKind::new().with_memory());
            system.refresh_specifics(refresh_kind);

            if let Some(process) = system.process(pid) {
                return process.memory();
            }
        }
        0
    }

    /// Check if enhanced metrics are enabled.
    pub fn enhanced_metrics_enabled(&self) -> bool {
        self.enhanced_metrics_enabled
    }

    /// Get total requests processed.
    pub fn get_requests_processed(&self) -> u64 {
        self.requests_processed.load(Ordering::Relaxed)
    }

    /// Get total evaluation time in nanoseconds.
    pub fn get_total_evaluation_time_ns(&self) -> u64 {
        self.total_evaluation_time_ns.load(Ordering::Relaxed)
    }

    /// Get average evaluation time in nanoseconds.
    pub fn get_avg_evaluation_time_ns(&self) -> f64 {
        let requests = self.get_requests_processed();
        if requests == 0 {
            return 0.0;
        }
        self.get_total_evaluation_time_ns() as f64 / requests as f64
    }
}

impl Default for AgentStats {
    fn default() -> Self {
        Self::new(false) // Enhanced metrics off by default
    }
}

impl std::fmt::Debug for AgentStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentStats")
            .field("requests_processed", &self.requests_processed)
            .field("decisions_allow", &self.decisions_allow)
            .field("decisions_deny", &self.decisions_deny)
            .field("enhanced_metrics_enabled", &self.enhanced_metrics_enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_stats_new() {
        let stats = AgentStats::new(false);
        assert_eq!(stats.get_requests_processed(), 0);
        assert!(!stats.enhanced_metrics_enabled());
    }

    #[test]
    fn test_agent_stats_with_enhanced_metrics() {
        let stats = AgentStats::new(true);
        assert!(stats.enhanced_metrics_enabled());
    }

    #[test]
    fn test_record_evaluation() {
        let stats = AgentStats::new(false);
        stats.record_evaluation(1000); // 1µs
        stats.record_evaluation(2000); // 2µs
        stats.record_evaluation(3000); // 3µs

        assert_eq!(stats.get_requests_processed(), 3);
        assert_eq!(stats.get_total_evaluation_time_ns(), 6000);
        assert!((stats.get_avg_evaluation_time_ns() - 2000.0).abs() < 0.1);
    }

    #[test]
    fn test_record_evaluation_with_histogram() {
        let stats = AgentStats::new(true);
        stats.record_evaluation(500); // 500ns
        stats.record_evaluation(1000); // 1µs
        stats.record_evaluation(1500); // 1.5µs

        assert_eq!(stats.get_requests_processed(), 3);
        // Histogram should have recorded values
        assert!(stats.get_latency_percentile_ns(50.0) > 0);
    }

    #[test]
    fn test_record_decisions() {
        let stats = AgentStats::new(false);
        stats.record_allow();
        stats.record_allow();
        stats.record_deny();

        assert_eq!(stats.decisions_allow.load(Ordering::Relaxed), 2);
        assert_eq!(stats.decisions_deny.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_cache_hits_misses() {
        let stats = AgentStats::new(false);

        stats.record_cache_hit();
        stats.record_cache_hit();
        stats.record_cache_miss();

        assert_eq!(stats.policy_cache_hits.load(Ordering::Relaxed), 2);
        assert_eq!(stats.policy_cache_misses.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_decision_cache_hits_misses() {
        let stats = AgentStats::new(false);

        stats.record_decision_cache_hit();
        stats.record_decision_cache_miss();
        stats.record_decision_cache_miss();

        assert_eq!(stats.decision_cache_hits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.decision_cache_misses.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_avg_evaluation_time_zero_requests() {
        let stats = AgentStats::new(false);
        assert_eq!(stats.get_avg_evaluation_time_ns(), 0.0);
    }

    #[test]
    fn test_percentile_with_enhanced_disabled() {
        let stats = AgentStats::new(false);
        stats.record_evaluation(1000);
        // Should return 0 when enhanced metrics disabled
        assert_eq!(stats.get_latency_percentile_ns(50.0), 0);
    }

    #[test]
    fn test_percentile_microseconds() {
        let stats = AgentStats::new(true);
        stats.record_evaluation(1000); // 1µs
        stats.record_evaluation(1000);
        stats.record_evaluation(1000);

        let p50_us = stats.get_latency_percentile_us(50.0);
        assert!(p50_us > 0.0);
        assert!(p50_us < 10.0); // Should be around 1µs
    }

    #[test]
    fn test_cpu_memory_without_enhanced() {
        let stats = AgentStats::new(false);
        assert_eq!(stats.get_cpu_percent(), 0.0);
        assert_eq!(stats.get_memory_bytes(), 0);
    }

    #[test]
    fn test_agent_stats_default() {
        let stats = AgentStats::default();
        assert!(!stats.enhanced_metrics_enabled());
    }

    #[test]
    fn test_agent_stats_debug() {
        let stats = AgentStats::new(true);
        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("AgentStats"));
        assert!(debug_str.contains("enhanced_metrics_enabled: true"));
    }

    #[test]
    fn test_concurrent_updates() {
        use std::thread;

        let stats = Arc::new(AgentStats::new(true));
        let mut handles = vec![];

        // Spawn 10 threads each recording 100 evaluations
        for _ in 0..10 {
            let stats_clone = Arc::clone(&stats);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    stats_clone.record_evaluation(500);
                    stats_clone.record_allow();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(stats.get_requests_processed(), 1000);
        assert_eq!(stats.decisions_allow.load(Ordering::Relaxed), 1000);
    }
}
