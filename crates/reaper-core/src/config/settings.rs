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

/// Loopback by default: exposing the enforcement API beyond the host is an
/// explicit decision that pairs with inbound auth (see [`AgentAuthSettings`]),
/// not something a default config does silently.
fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

/// Is this bind address loopback-only (unreachable from other hosts)?
pub fn is_loopback_bind(addr: &str) -> bool {
    if addr.eq_ignore_ascii_case("localhost") {
        return true;
    }
    addr.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

// ============================================================================
// Agent Inbound Auth Settings
// ============================================================================

/// Which credential(s) the agent's inbound auth accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuthMode {
    /// mTLS-verified client identity only: either TLS terminates at the agent
    /// with `tls.require_client_cert`, or a trusted reverse proxy verified the
    /// client cert and forwards its fingerprint in `mtls_fingerprint_header`.
    Mtls,
    /// Bearer credential only: the management-minted agent JWT (validated
    /// against the shared `jwt_secret`) or the static `bearer_token`.
    BearerToken,
    /// Either credential is accepted.
    Both,
}

fn default_agent_auth_mode() -> AgentAuthMode {
    AgentAuthMode::Both
}

fn default_agent_jwt_issuer() -> String {
    "reaper-management".to_string()
}

fn default_agent_jwt_audience() -> String {
    "reaper-agent".to_string()
}

/// Inbound authentication for the agent's HTTP API (Plan 01, Phase C).
///
/// The agent refuses to start when bound to a non-loopback address without
/// either inbound auth, agent-terminated mTLS, or the explicit
/// `allow_unauthenticated` opt-out — see
/// [`validate_exposure`](AgentAuthSettings::validate_exposure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuthSettings {
    /// Enable inbound authentication on non-health endpoints (default: false).
    #[serde(default)]
    pub enabled: bool,

    /// Accepted credential kinds when enabled (default: both).
    #[serde(default = "default_agent_auth_mode")]
    pub mode: AgentAuthMode,

    /// Header carrying a client-certificate fingerprint verified by a trusted
    /// reverse proxy (e.g. "x-client-cert-fingerprint"). Only set this when
    /// the proxy strips any client-supplied copy — otherwise it is forgeable.
    #[serde(default)]
    pub mtls_fingerprint_header: Option<String>,

    /// Optional allowlist of accepted certificate fingerprints for the
    /// trusted-proxy header. Empty = any non-empty fingerprint the proxy
    /// verified is accepted.
    #[serde(default)]
    pub mtls_allowed_fingerprints: Vec<String>,

    /// Static bearer token for the simple localhost-sidecar case.
    #[serde(default)]
    pub bearer_token: Option<String>,

    /// Shared secret validating management-minted agent JWTs
    /// (`Authorization: Bearer <jwt>`; same value as the management server's
    /// `REAPER_JWT_SECRET`, so the token an agent received at registration
    /// also authenticates callers holding it).
    #[serde(default)]
    pub jwt_secret: Option<String>,

    /// Expected JWT issuer — must match the management server's
    /// `auth.jwt_issuer` (default "reaper-management").
    #[serde(default = "default_agent_jwt_issuer")]
    pub jwt_issuer: String,

    /// Expected JWT audience — must match the management server's
    /// `auth.jwt_audience` (default "reaper-agent").
    #[serde(default = "default_agent_jwt_audience")]
    pub jwt_audience: String,

    /// Explicit, auditable opt-out: allow serving unauthenticated on a
    /// non-loopback bind. Without this, such a config refuses to start.
    #[serde(default)]
    pub allow_unauthenticated: bool,
}

impl Default for AgentAuthSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_agent_auth_mode(),
            mtls_fingerprint_header: None,
            mtls_allowed_fingerprints: Vec::new(),
            bearer_token: None,
            jwt_secret: None,
            jwt_issuer: default_agent_jwt_issuer(),
            jwt_audience: default_agent_jwt_audience(),
            allow_unauthenticated: false,
        }
    }
}

impl AgentAuthSettings {
    /// Fail-closed exposure check, run at startup: a non-loopback bind with
    /// no inbound auth and no agent-terminated mTLS is refused unless
    /// `allow_unauthenticated` explicitly opts out.
    pub fn validate_exposure(
        &self,
        bind_address: &str,
        tls_requires_client_cert: bool,
    ) -> Result<(), String> {
        if is_loopback_bind(bind_address) {
            return Ok(());
        }
        if self.enabled || tls_requires_client_cert || self.allow_unauthenticated {
            return Ok(());
        }
        Err(format!(
            "refusing to start: bind address '{bind_address}' is reachable from other hosts but \
             inbound authentication is disabled. Enable agent auth (auth.enabled / \
             REAPER_AGENT_AUTH_ENABLED), require client certificates \
             (tls.require_client_cert), bind to 127.0.0.1, or explicitly opt out with \
             auth.allow_unauthenticated / REAPER_AGENT_ALLOW_UNAUTHENTICATED=true"
        ))
    }

    /// True when the configuration accepts mTLS-style credentials.
    pub fn accepts_mtls(&self) -> bool {
        matches!(self.mode, AgentAuthMode::Mtls | AgentAuthMode::Both)
    }

    /// True when the configuration accepts bearer credentials.
    pub fn accepts_bearer(&self) -> bool {
        matches!(self.mode, AgentAuthMode::BearerToken | AgentAuthMode::Both)
    }
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

    // ========================================================================
    // Bundle Signature Verification
    // ========================================================================
    /// Pinned public key (lowercase hex) used to verify every bundle the control
    /// plane serves. When set, downloaded bundles must carry a valid signature
    /// over their bytes or they are rejected (fail closed) — this makes policy
    /// distribution trustworthy independent of the transport.
    #[serde(default)]
    pub bundle_public_key: Option<String>,

    /// Signature algorithm for `bundle_public_key`: `ed25519-sha256` (default)
    /// or `ecdsa-p256-sha256` (FIPS 186 P-256). Must match how the control plane
    /// signs.
    #[serde(default)]
    pub bundle_signature_algorithm: Option<String>,

    /// Optional key id to pin (rotation). When set, a bundle signature whose
    /// `key_id` differs is rejected even if the signature itself is valid.
    #[serde(default)]
    pub bundle_key_id: Option<String>,

    /// Require every applied bundle to be signed and verified. Defaults to true:
    /// if `bundle_public_key` is set, unsigned/invalid bundles are rejected; if
    /// no key is set, the agent refuses to apply *any* management bundle (safe
    /// default). Set to false only for trusted dev/test setups.
    #[serde(default = "default_true")]
    pub require_signed_bundles: bool,

    /// Require the v2 signature envelope (authenticated bundle_id, monotonic
    /// version, validity window) on verified bundles. Defaults to true; set to
    /// false only as a bounded migration window while a pre-v2 control plane
    /// is being upgraded — legacy v1 envelopes carry no anti-replay metadata.
    #[serde(default = "default_true")]
    pub require_envelope_v2: bool,
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
            // Secure by default: require signed bundles. With no key configured
            // this makes managed mode fail closed until signing is set up.
            bundle_public_key: None,
            bundle_signature_algorithm: None,
            bundle_key_id: None,
            require_signed_bundles: true,
            require_envelope_v2: true,
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
        // Loopback by default: exposure must be an explicit decision.
        assert_eq!(settings.bind_address, "127.0.0.1");
        assert_eq!(settings.name, "reaper-agent");
    }

    #[test]
    fn test_loopback_detection() {
        for addr in ["127.0.0.1", "::1", "localhost", "127.0.0.53"] {
            assert!(is_loopback_bind(addr), "{addr} should be loopback");
        }
        for addr in ["0.0.0.0", "::", "10.0.0.5", "192.168.1.2", "example.com"] {
            assert!(!is_loopback_bind(addr), "{addr} must not count as loopback");
        }
    }

    #[test]
    fn test_exposure_validation_fails_closed() {
        let auth = AgentAuthSettings::default();

        // Loopback is always fine, authenticated or not.
        assert!(auth.validate_exposure("127.0.0.1", false).is_ok());

        // Non-loopback + no auth + no client-cert TLS → refused.
        assert!(auth.validate_exposure("0.0.0.0", false).is_err());

        // Any one of the three escape hatches admits the bind.
        assert!(auth.validate_exposure("0.0.0.0", true).is_ok());
        let enabled = AgentAuthSettings {
            enabled: true,
            ..Default::default()
        };
        assert!(enabled.validate_exposure("0.0.0.0", false).is_ok());
        let opted_out = AgentAuthSettings {
            allow_unauthenticated: true,
            ..Default::default()
        };
        assert!(opted_out.validate_exposure("0.0.0.0", false).is_ok());
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
