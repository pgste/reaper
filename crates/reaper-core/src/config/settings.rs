//! Configuration settings structs for Reaper services.
//!
//! This module contains all the individual settings structs that make up
//! the complete agent configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ============================================================================
// Agent Settings
// ============================================================================

/// Agent network and identification settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Agent unique identifier (auto-generated if not specified)
    pub id: Option<String>,

    /// Agent name for display
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Address to bind to
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            id: None,
            name: default_agent_name(),
            port: default_port(),
            bind_address: default_bind_address(),
        }
    }
}

fn default_agent_name() -> String {
    "reaper-agent".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

// ============================================================================
// Policy Settings
// ============================================================================

/// Policy loading settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySettings {
    /// Directory to load bootstrap policies from on startup
    pub bootstrap_dir: Option<PathBuf>,

    /// Directory to cache deployed policies
    pub cache_dir: Option<PathBuf>,

    /// Enable automatic policy reload on file change
    #[serde(default)]
    pub watch_for_changes: bool,

    /// File extensions to recognize as policies
    #[serde(default = "default_policy_extensions")]
    pub extensions: Vec<String>,
}

impl Default for PolicySettings {
    fn default() -> Self {
        Self {
            bootstrap_dir: None,
            cache_dir: None,
            watch_for_changes: false,
            extensions: default_policy_extensions(),
        }
    }
}

fn default_policy_extensions() -> Vec<String> {
    vec![
        "reap".to_string(),
        "yaml".to_string(),
        "yml".to_string(),
        "json".to_string(),
    ]
}

// ============================================================================
// Data Settings
// ============================================================================

/// Entity data settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataSettings {
    /// File to load bootstrap entity data from
    pub bootstrap_file: Option<PathBuf>,

    /// Directory containing entity data files
    pub bootstrap_dir: Option<PathBuf>,

    /// Directory to cache synced entity data
    pub cache_dir: Option<PathBuf>,
}

// ============================================================================
// Performance Settings
// ============================================================================

/// Performance tuning settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSettings {
    /// Target latency in microseconds (for monitoring/alerting)
    #[serde(default = "default_target_latency")]
    pub target_latency_microseconds: f64,

    /// Number of worker threads (0 = auto-detect)
    #[serde(default)]
    pub worker_threads: usize,

    /// Enable SIMD optimizations
    #[serde(default = "default_true")]
    pub enable_simd: bool,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            target_latency_microseconds: default_target_latency(),
            worker_threads: 0,
            enable_simd: true,
        }
    }
}

fn default_target_latency() -> f64 {
    1.0
}

// ============================================================================
// Cache Settings
// ============================================================================

/// Decision cache settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSettings {
    /// Enable decision caching
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Maximum number of cached decisions
    #[serde(default = "default_cache_capacity")]
    pub capacity: usize,

    /// TTL for cached decisions in seconds (0 = no TTL)
    #[serde(default = "default_cache_ttl")]
    pub ttl_seconds: u64,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            capacity: default_cache_capacity(),
            ttl_seconds: default_cache_ttl(),
        }
    }
}

fn default_cache_capacity() -> usize {
    10_000
}

fn default_cache_ttl() -> u64 {
    10
}

// ============================================================================
// Observability Settings
// ============================================================================

/// Observability settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilitySettings {
    /// Enable metrics endpoint
    #[serde(default = "default_true")]
    pub enable_metrics: bool,

    /// Enable structured JSON logging
    #[serde(default)]
    pub json_logging: bool,

    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Enable OpenTelemetry tracing
    #[serde(default)]
    pub enable_tracing: bool,

    /// OpenTelemetry collector endpoint
    pub otel_endpoint: Option<String>,

    /// Enable enhanced metrics (HDR histogram for percentiles, CPU/memory monitoring)
    /// When disabled (default), these expensive operations are skipped.
    /// Enable with REAPER_ENHANCED_METRICS=true for detailed metrics.
    #[serde(default)]
    pub enable_enhanced_metrics: bool,
}

impl Default for ObservabilitySettings {
    fn default() -> Self {
        Self {
            enable_metrics: true,
            json_logging: false,
            log_level: default_log_level(),
            enable_tracing: false,
            otel_endpoint: None,
            enable_enhanced_metrics: false, // Off by default for performance
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

// ============================================================================
// Management Settings
// ============================================================================

/// Management plane settings.
///
/// When enabled, the agent will connect to a Reaper Management Server
/// to receive policy bundles and report health status.
/// When disabled (default), the agent runs standalone using local policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementSettings {
    /// Enable connection to management plane (default: false for standalone mode)
    #[serde(default)]
    pub enabled: bool,

    /// Management server URL (e.g., "http://localhost:8081")
    pub url: Option<String>,

    /// Organization slug or ID to register with
    pub org: Option<String>,

    /// API key for authentication with management server
    pub api_key: Option<String>,

    /// How often to poll for bundle updates (seconds)
    /// When SSE is enabled, this is used as a fallback interval (default: 30 without SSE, 300 with SSE)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// How often to send heartbeat (seconds)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Whether to pull promoted bundle on startup
    #[serde(default = "default_true")]
    pub sync_on_startup: bool,

    /// Timeout for HTTP requests to management server (seconds)
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,

    // ========================================================================
    // SSE Push Notification Settings
    // ========================================================================
    /// Enable SSE push notifications for real-time updates (default: true)
    /// When enabled, the agent receives instant notifications of bundle promotions
    /// and data refreshes. Polling is used as a fallback.
    #[serde(default = "default_true")]
    pub sse_enabled: bool,

    /// Initial reconnection delay for SSE in seconds (default: 1)
    /// Uses exponential backoff up to sse_reconnect_max_secs
    #[serde(default = "default_sse_reconnect_initial")]
    pub sse_reconnect_initial_secs: u64,

    /// Maximum reconnection delay for SSE in seconds (default: 60)
    #[serde(default = "default_sse_reconnect_max")]
    pub sse_reconnect_max_secs: u64,

    /// Poll interval when SSE is active (seconds, default: 300)
    /// This is a fallback to catch any events missed during SSE reconnection
    #[serde(default = "default_poll_interval_with_sse")]
    pub poll_interval_with_sse_secs: u64,
}

impl Default for ManagementSettings {
    fn default() -> Self {
        Self {
            enabled: false, // Standalone mode by default
            url: None,
            org: None,
            api_key: None,
            poll_interval_secs: default_poll_interval(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            sync_on_startup: true,
            request_timeout_secs: default_request_timeout(),
            // SSE settings
            sse_enabled: true,
            sse_reconnect_initial_secs: default_sse_reconnect_initial(),
            sse_reconnect_max_secs: default_sse_reconnect_max(),
            poll_interval_with_sse_secs: default_poll_interval_with_sse(),
        }
    }
}

fn default_poll_interval() -> u64 {
    30
}

fn default_heartbeat_interval() -> u64 {
    30
}

fn default_request_timeout() -> u64 {
    10
}

fn default_sse_reconnect_initial() -> u64 {
    1
}

fn default_sse_reconnect_max() -> u64 {
    60
}

fn default_poll_interval_with_sse() -> u64 {
    300
}

// ============================================================================
// Unix Domain Socket Settings
// ============================================================================

/// Unix Domain Socket (UDS) listener settings.
///
/// When enabled, the agent listens on a Unix socket in addition to TCP.
/// UDS bypasses the TCP/IP stack for lower latency same-host IPC.
/// Only applicable on Unix-like systems (Linux, macOS).
///
/// # Deployment models
///
/// - **Shared** (`shards = 0` or `1`, the default): one socket served by the
///   agent's shared multi-threaded runtime. Simple, work-stealing across all
///   cores, best tail latency. Recommended default.
/// - **Sharded / thread-per-core** (`shards = N > 1`): N sockets
///   (`agent-0.sock … agent-{N-1}.sock`), each served by its own single-thread
///   runtime pinned to a core (share-nothing). ~12–17% higher throughput and
///   lower median latency under saturation, at the cost of worse p99 (no
///   cross-core rebalancing). UDS has no `SO_REUSEPORT`, so multiple socket
///   files is how a thread-per-core UDS server is sharded; clients round-robin
///   connections across the sockets.
///
/// # Security
///
/// UDS has **no application-layer authentication** — filesystem permissions are
/// the access-control boundary. The agent creates the socket's parent directory
/// owner-only (`0700`) and chmods every socket to `socket_permissions`
/// (default `0o660`). In sharded mode all N sockets live in that one `0700`
/// directory, so a single directory boundary secures every mount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdsSettings {
    /// Enable UDS listener (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Path to the Unix socket file.
    ///
    /// In sharded mode this is the base path; a shard index is inserted before
    /// the extension, e.g. `/run/reaper/agent.sock` → `agent-0.sock`,
    /// `agent-1.sock`, …
    #[serde(default = "default_uds_path")]
    pub socket_path: PathBuf,

    /// Socket file permissions (octal, e.g. 0o660)
    #[serde(default = "default_socket_permissions")]
    pub socket_permissions: u32,

    /// Number of thread-per-core shards.
    ///
    /// `0` or `1` = shared single-socket model (default). `N > 1` = sharded
    /// thread-per-core model with N pinned single-thread runtimes, each owning
    /// its own socket file.
    #[serde(default)]
    pub shards: usize,

    /// Pin each shard's runtime thread to a CPU core in sharded mode (default:
    /// true). Disable if the agent shares a host with other latency-sensitive
    /// processes and you'd rather let the scheduler balance.
    #[serde(default = "default_true")]
    pub pin_cores: bool,
}

impl Default for UdsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            socket_path: default_uds_path(),
            socket_permissions: default_socket_permissions(),
            shards: 0,
            pin_cores: true,
        }
    }
}

impl UdsSettings {
    /// Whether the sharded (thread-per-core) model is requested.
    pub fn is_sharded(&self) -> bool {
        self.shards > 1
    }

    /// Effective shard count: 1 for the shared model, else `shards`.
    pub fn effective_shards(&self) -> usize {
        self.shards.max(1)
    }

    /// Socket path for shard `i` in sharded mode: inserts `-i` before the file
    /// extension (`agent.sock` → `agent-0.sock`). For the shared model callers
    /// use `socket_path` directly.
    pub fn shard_socket_path(&self, i: usize) -> PathBuf {
        let stem = self
            .socket_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "agent".to_string());
        let ext = self
            .socket_path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let file = format!("{stem}-{i}{ext}");
        match self.socket_path.parent() {
            Some(parent) => parent.join(file),
            None => PathBuf::from(file),
        }
    }
}

fn default_uds_path() -> PathBuf {
    PathBuf::from("/var/run/reaper/agent.sock")
}

fn default_socket_permissions() -> u32 {
    0o660
}

// ============================================================================
// TLS Settings
// ============================================================================

/// TLS/mTLS settings for secure connections.
///
/// Enables HTTPS with optional mutual TLS authentication.
/// When `require_client_cert` is true, clients must present valid certificates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsSettings {
    /// Enable TLS (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Path to server certificate file (PEM format)
    pub cert_file: Option<PathBuf>,

    /// Path to server private key file (PEM format)
    pub key_file: Option<PathBuf>,

    /// Path to CA certificate for client verification (PEM format)
    /// Required when `require_client_cert` is true
    pub ca_file: Option<PathBuf>,

    /// Require client certificate (mTLS mode)
    #[serde(default)]
    pub require_client_cert: bool,
}

// ============================================================================
// Shared Default Functions
// ============================================================================

fn default_true() -> bool {
    true
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_settings_default() {
        let settings = AgentSettings::default();
        assert_eq!(settings.port, 8080);
        assert_eq!(settings.bind_address, "0.0.0.0");
        assert_eq!(settings.name, "reaper-agent");
    }

    #[test]
    fn test_cache_settings_default() {
        let settings = CacheSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.capacity, 10_000);
        assert_eq!(settings.ttl_seconds, 10);
    }

    #[test]
    fn test_management_settings_default() {
        let settings = ManagementSettings::default();
        assert!(!settings.enabled);
        assert!(settings.sse_enabled);
        assert_eq!(settings.poll_interval_secs, 30);
    }

    #[test]
    fn test_tls_settings_default() {
        let settings = TlsSettings::default();
        assert!(!settings.enabled);
        assert!(!settings.require_client_cert);
    }

    #[test]
    fn test_uds_settings_default() {
        let settings = UdsSettings::default();
        assert!(!settings.enabled);
        assert_eq!(
            settings.socket_path,
            PathBuf::from("/var/run/reaper/agent.sock")
        );
        assert_eq!(settings.socket_permissions, 0o660);
        // Shared model by default.
        assert_eq!(settings.shards, 0);
        assert!(!settings.is_sharded());
        assert_eq!(settings.effective_shards(), 1);
        assert!(settings.pin_cores);
    }

    #[test]
    fn test_uds_shard_socket_paths() {
        let settings = UdsSettings {
            socket_path: PathBuf::from("/run/reaper/agent.sock"),
            shards: 4,
            ..UdsSettings::default()
        };
        assert!(settings.is_sharded());
        assert_eq!(settings.effective_shards(), 4);
        assert_eq!(
            settings.shard_socket_path(0),
            PathBuf::from("/run/reaper/agent-0.sock")
        );
        assert_eq!(
            settings.shard_socket_path(3),
            PathBuf::from("/run/reaper/agent-3.sock")
        );
    }

    #[test]
    fn test_uds_shard_socket_path_no_extension() {
        let settings = UdsSettings {
            socket_path: PathBuf::from("/run/reaper/agent"),
            shards: 2,
            ..UdsSettings::default()
        };
        assert_eq!(
            settings.shard_socket_path(1),
            PathBuf::from("/run/reaper/agent-1")
        );
    }
}
