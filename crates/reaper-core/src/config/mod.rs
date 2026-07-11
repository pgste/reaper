//! Comprehensive Configuration for Reaper Services
//!
//! Supports YAML/JSON config files and environment variable overrides.
//!
//! # Usage
//! ```rust,ignore
//! use reaper_core::config::ReaperAgentConfig;
//!
//! // Load from file (auto-detects YAML/JSON)
//! let config = ReaperAgentConfig::from_file("/etc/reaper/agent.yaml")?;
//!
//! // Or load with env overrides
//! let config = ReaperAgentConfig::from_file_with_env("/etc/reaper/agent.yaml")?;
//!
//! // Or use defaults
//! let config = ReaperAgentConfig::default();
//! ```

mod error;
mod settings;

// Re-export all types for public API
pub use error::ConfigError;
pub use settings::{
    is_loopback_bind, AgentAuthMode, AgentAuthSettings, AgentSettings, CacheSettings, DataSettings,
    ManagementSettings, ObservabilitySettings, PerformanceSettings, PolicySettings,
    RevocationStaleness, TlsSettings, UdsSettings,
};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Environment variable prefix for all Reaper config
pub const ENV_PREFIX: &str = "REAPER";

// ============================================================================
// Agent Configuration
// ============================================================================

/// Complete agent configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReaperAgentConfig {
    /// Agent identification and network settings
    #[serde(default)]
    pub agent: AgentSettings,

    /// Policy loading and caching settings
    #[serde(default)]
    pub policies: PolicySettings,

    /// Entity data loading and caching settings
    #[serde(default)]
    pub data: DataSettings,

    /// Performance tuning settings
    #[serde(default)]
    pub performance: PerformanceSettings,

    /// Decision cache settings
    #[serde(default)]
    pub cache: CacheSettings,

    /// Observability settings
    #[serde(default)]
    pub observability: ObservabilitySettings,

    /// Management plane settings (optional)
    #[serde(default)]
    pub management: ManagementSettings,

    /// TLS/mTLS settings
    #[serde(default)]
    pub tls: TlsSettings,

    /// Inbound authentication for the agent HTTP API
    #[serde(default)]
    pub auth: AgentAuthSettings,

    /// Unix Domain Socket settings
    #[serde(default)]
    pub uds: UdsSettings,
}

// ============================================================================
// Configuration Loading
// ============================================================================

impl ReaperAgentConfig {
    /// Load configuration from a file (auto-detects YAML/JSON by extension)
    pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::FileRead(path.clone(), e.to_string()))?;

        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("yaml");

        match ext {
            "json" => serde_json::from_str(&content)
                .map_err(|e| ConfigError::Parse(format!("JSON parse error: {}", e))),
            _ => serde_yaml::from_str(&content)
                .map_err(|e| ConfigError::Parse(format!("YAML parse error: {}", e))),
        }
    }

    /// Load configuration from file with environment variable overrides
    pub fn from_file_with_env(path: &PathBuf) -> Result<Self, ConfigError> {
        let mut config = Self::from_file(path)?;
        config.apply_env_overrides();
        Ok(config)
    }

    /// Load from environment variables only (with defaults)
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.apply_env_overrides();
        config
    }

    /// Apply environment variable overrides to the config
    pub fn apply_env_overrides(&mut self) {
        // Agent bind/port: service-specific vars win, then the generic ones,
        // then the combined REAPER_BIND_ADDR form (see resolve_bind).
        let (bind, port) = resolve_bind("REAPER_AGENT", &self.agent.bind_address, self.agent.port);
        self.agent.bind_address = bind;
        self.agent.port = port;
        if let Ok(val) = std::env::var("REAPER_AGENT_NAME") {
            self.agent.name = val;
        }

        // Policy settings
        if let Ok(val) = std::env::var("REAPER_POLICIES_BOOTSTRAP_DIR") {
            self.policies.bootstrap_dir = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("REAPER_POLICIES_CACHE_DIR") {
            self.policies.cache_dir = Some(PathBuf::from(val));
        }

        // Data settings
        if let Ok(val) = std::env::var("REAPER_DATA_BOOTSTRAP_FILE") {
            self.data.bootstrap_file = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("REAPER_DATA_BOOTSTRAP_DIR") {
            self.data.bootstrap_dir = Some(PathBuf::from(val));
        }

        // Cache settings (using existing env vars for compatibility)
        if let Ok(val) = std::env::var("REAPER_CACHE_ENABLED") {
            self.cache.enabled = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_CACHE_CAPACITY") {
            if let Ok(capacity) = val.parse() {
                self.cache.capacity = capacity;
            }
        }
        if let Ok(val) = std::env::var("REAPER_CACHE_TTL_SECS") {
            if let Ok(ttl) = val.parse() {
                self.cache.ttl_seconds = ttl;
            }
        }

        // Performance settings
        if let Ok(val) = std::env::var("REAPER_MAX_BATCH_REQUESTS") {
            if let Ok(max) = val.parse::<usize>() {
                if max > 0 {
                    self.performance.max_batch_requests = max;
                }
            }
        }
        // Plan 08 Phase A: evaluate-all fan-out controls.
        if let Ok(val) = std::env::var("REAPER_ALLOW_EVALUATE_ALL") {
            self.performance.allow_evaluate_all =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_MAX_CANDIDATE_POLICIES") {
            if let Ok(max) = val.parse::<usize>() {
                if max > 0 {
                    self.performance.max_candidate_policies = max;
                }
            }
        }
        if let Ok(val) = std::env::var("REAPER_USE_PRUNING_INDEX") {
            self.performance.use_pruning_index =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }

        // Observability settings
        if let Ok(val) = std::env::var("REAPER_LOG_LEVEL") {
            self.observability.log_level = val;
        }
        if let Ok(val) = std::env::var("REAPER_JSON_LOGGING") {
            self.observability.json_logging =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            self.observability.otel_endpoint = Some(val);
            self.observability.enable_tracing = true;
        }
        if let Ok(val) = std::env::var("REAPER_ENHANCED_METRICS") {
            self.observability.enable_enhanced_metrics =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }

        // Management plane settings
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_ENABLED") {
            self.management.enabled =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_URL") {
            self.management.url = Some(val);
            // Auto-enable management if URL is provided
            if !self.management.enabled {
                self.management.enabled = true;
            }
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_ORG") {
            self.management.org = Some(val);
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_API_KEY") {
            self.management.api_key = Some(val);
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_POLL_INTERVAL") {
            if let Ok(interval) = val.parse() {
                self.management.poll_interval_secs = interval;
            }
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_HEARTBEAT_INTERVAL") {
            if let Ok(interval) = val.parse() {
                self.management.heartbeat_interval_secs = interval;
            }
        }

        // SSE settings
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_SSE_ENABLED") {
            self.management.sse_enabled =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_SSE_RECONNECT_INITIAL") {
            if let Ok(secs) = val.parse() {
                self.management.sse_reconnect_initial_secs = secs;
            }
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_SSE_RECONNECT_MAX") {
            if let Ok(secs) = val.parse() {
                self.management.sse_reconnect_max_secs = secs;
            }
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_POLL_INTERVAL_WITH_SSE") {
            if let Ok(secs) = val.parse() {
                self.management.poll_interval_with_sse_secs = secs;
            }
        }
        // Bundle signature verification
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY") {
            self.management.bundle_public_key = Some(val);
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_BUNDLE_SIGNATURE_ALGORITHM") {
            self.management.bundle_signature_algorithm = Some(val);
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_BUNDLE_KEY_ID") {
            self.management.bundle_key_id = Some(val);
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_REQUIRE_SIGNED_BUNDLES") {
            self.management.require_signed_bundles =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_REQUIRE_ENVELOPE_V2") {
            self.management.require_envelope_v2 =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_MANAGEMENT_REVOCATION_STALENESS") {
            match val.to_lowercase().as_str() {
                "enforce" => self.management.revocation_staleness = RevocationStaleness::Enforce,
                "monitor" => self.management.revocation_staleness = RevocationStaleness::Monitor,
                other => tracing::warn!(
                    "Ignoring invalid REAPER_MANAGEMENT_REVOCATION_STALENESS '{other}' (expected monitor | enforce)"
                ),
            }
        }

        // UDS settings
        if let Ok(val) = std::env::var("REAPER_UDS_ENABLED") {
            self.uds.enabled = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_UDS_PATH") {
            self.uds.socket_path = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("REAPER_UDS_PERMISSIONS") {
            if let Ok(perms) = u32::from_str_radix(&val, 8) {
                self.uds.socket_permissions = perms;
            }
        }
        // Number of thread-per-core shards (0/1 = shared single socket).
        if let Ok(val) = std::env::var("REAPER_UDS_SHARDS") {
            if let Ok(shards) = val.parse::<usize>() {
                self.uds.shards = shards;
            }
        }
        if let Ok(val) = std::env::var("REAPER_UDS_PIN_CORES") {
            self.uds.pin_cores = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }

        // Inbound auth settings
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_ENABLED") {
            self.auth.enabled = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_MODE") {
            match val.to_lowercase().as_str() {
                "mtls" => self.auth.mode = AgentAuthMode::Mtls,
                "bearer_token" | "bearer" => self.auth.mode = AgentAuthMode::BearerToken,
                "both" => self.auth.mode = AgentAuthMode::Both,
                other => tracing::warn!(
                    "Ignoring invalid REAPER_AGENT_AUTH_MODE '{other}' (expected mtls | bearer_token | both)"
                ),
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_MTLS_FINGERPRINT_HEADER") {
            if !val.is_empty() {
                self.auth.mtls_fingerprint_header = Some(val);
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_MTLS_ALLOWED_FINGERPRINTS") {
            self.auth.mtls_allowed_fingerprints = val
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_BEARER_TOKEN") {
            if !val.is_empty() {
                self.auth.bearer_token = Some(val);
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_JWT_SECRET") {
            if !val.is_empty() {
                self.auth.jwt_secret = Some(val);
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_JWT_ISSUER") {
            if !val.is_empty() {
                self.auth.jwt_issuer = val;
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_JWT_AUDIENCE") {
            if !val.is_empty() {
                self.auth.jwt_audience = val;
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_ALLOW_UNAUTHENTICATED") {
            self.auth.allow_unauthenticated =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_AUTH_OPEN_DATA_PLANE") {
            self.auth.open_data_plane =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }

        // TLS settings
        if let Ok(val) = std::env::var("REAPER_TLS_ENABLED") {
            self.tls.enabled = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
        if let Ok(val) = std::env::var("REAPER_TLS_CERT") {
            self.tls.cert_file = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("REAPER_TLS_KEY") {
            self.tls.key_file = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("REAPER_TLS_CA") {
            self.tls.ca_file = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("REAPER_TLS_REQUIRE_CLIENT_CERT") {
            self.tls.require_client_cert =
                matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate port
        if self.agent.port == 0 {
            return Err(ConfigError::Validation("Port cannot be 0".to_string()));
        }

        // Validate bootstrap dirs exist if specified
        if let Some(ref dir) = self.policies.bootstrap_dir {
            if !dir.exists() {
                return Err(ConfigError::Validation(format!(
                    "Policy bootstrap directory does not exist: {:?}",
                    dir
                )));
            }
        }

        if let Some(ref file) = self.data.bootstrap_file {
            if !file.exists() {
                return Err(ConfigError::Validation(format!(
                    "Data bootstrap file does not exist: {:?}",
                    file
                )));
            }
        }

        // Validate management settings if enabled
        if self.management.enabled {
            if self.management.url.is_none() {
                return Err(ConfigError::Validation(
                    "Management URL is required when management is enabled".to_string(),
                ));
            }
            if self.management.org.is_none() {
                return Err(ConfigError::Validation(
                    "Management org is required when management is enabled".to_string(),
                ));
            }
            if self.management.api_key.is_none() {
                return Err(ConfigError::Validation(
                    "Management API key is required when management is enabled".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Get a summary string for logging
    pub fn summary(&self) -> String {
        let mgmt_status = if self.management.enabled {
            format!(
                "connected to {}",
                self.management.url.as_deref().unwrap_or("?")
            )
        } else {
            "standalone".to_string()
        };

        let uds_status = if self.uds.enabled {
            format!("UDS: {}", self.uds.socket_path.display())
        } else {
            "UDS: disabled".to_string()
        };

        format!(
            "Agent: {}:{}, Mode: {}, {}, Cache: {} ({} entries, {}s TTL), Bootstrap: policies={:?}, data={:?}",
            self.agent.bind_address,
            self.agent.port,
            mgmt_status,
            uds_status,
            if self.cache.enabled {
                "enabled"
            } else {
                "disabled"
            },
            self.cache.capacity,
            self.cache.ttl_seconds,
            self.policies.bootstrap_dir,
            self.data.bootstrap_file,
        )
    }
}

// ============================================================================
// Bind/port resolution shared by all Reaper services
// ============================================================================

/// Resolve a service's bind address and port from the environment.
///
/// Every Reaper service (agent, platform, management) accepts the same
/// layered scheme, so one convention works across bare processes, Docker
/// Compose, and Helm:
///
/// 1. `{PREFIX}_BIND_ADDRESS` / `{PREFIX}_PORT` — service-specific, wins
///    (e.g. `REAPER_AGENT_PORT`, `REAPER_PLATFORM_PORT`,
///    `REAPER_MANAGEMENT_PORT`)
/// 2. `REAPER_BIND_ADDRESS` / `REAPER_PORT` — generic, for single-service
///    containers
/// 3. `REAPER_BIND_ADDR` — combined `host:port` form
///
/// Values that fail to parse are ignored (the next layer, or the given
/// default, applies).
pub fn resolve_bind(prefix: &str, default_bind: &str, default_port: u16) -> (String, u16) {
    resolve_bind_with(prefix, default_bind, default_port, |name| {
        std::env::var(name).ok()
    })
}

/// Pure implementation of [`resolve_bind`] over an arbitrary lookup, so the
/// precedence rules are unit-testable without mutating process env vars.
pub fn resolve_bind_with<F: Fn(&str) -> Option<String>>(
    prefix: &str,
    default_bind: &str,
    default_port: u16,
    lookup: F,
) -> (String, u16) {
    let mut bind = default_bind.to_string();
    let mut port = default_port;

    // Layer 3 (lowest): combined REAPER_BIND_ADDR ("0.0.0.0:8080").
    if let Some(addr) = lookup("REAPER_BIND_ADDR") {
        if let Some((host, p)) = addr.rsplit_once(':') {
            if let Ok(p) = p.parse() {
                bind = host.to_string();
                port = p;
            }
        }
    }

    // Layer 2: generic split vars.
    if let Some(val) = lookup("REAPER_BIND_ADDRESS") {
        bind = val;
    }
    if let Some(p) = lookup("REAPER_PORT").and_then(|v| v.parse().ok()) {
        port = p;
    }

    // Layer 1 (highest): service-specific vars.
    if let Some(val) = lookup(&format!("{prefix}_BIND_ADDRESS")) {
        bind = val;
    }
    if let Some(p) = lookup(&format!("{prefix}_PORT")).and_then(|v| v.parse().ok()) {
        port = p;
    }

    (bind, port)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_resolve_bind_defaults_when_nothing_set() {
        let (bind, port) = resolve_bind_with("REAPER_AGENT", "0.0.0.0", 8080, lookup_from(&[]));
        assert_eq!((bind.as_str(), port), ("0.0.0.0", 8080));
    }

    #[test]
    fn test_resolve_bind_combined_form() {
        let (bind, port) = resolve_bind_with(
            "REAPER_AGENT",
            "0.0.0.0",
            8080,
            lookup_from(&[("REAPER_BIND_ADDR", "127.0.0.1:9999")]),
        );
        assert_eq!((bind.as_str(), port), ("127.0.0.1", 9999));
    }

    #[test]
    fn test_resolve_bind_generic_beats_combined() {
        let (bind, port) = resolve_bind_with(
            "REAPER_AGENT",
            "0.0.0.0",
            8080,
            lookup_from(&[
                ("REAPER_BIND_ADDR", "127.0.0.1:9999"),
                ("REAPER_PORT", "7777"),
            ]),
        );
        // Generic port wins over the combined form; its host part still applies.
        assert_eq!((bind.as_str(), port), ("127.0.0.1", 7777));
    }

    #[test]
    fn test_resolve_bind_specific_beats_generic() {
        let (bind, port) = resolve_bind_with(
            "REAPER_AGENT",
            "0.0.0.0",
            8080,
            lookup_from(&[
                ("REAPER_PORT", "7777"),
                ("REAPER_BIND_ADDRESS", "10.0.0.1"),
                ("REAPER_AGENT_PORT", "6666"),
                ("REAPER_AGENT_BIND_ADDRESS", "192.168.0.1"),
            ]),
        );
        assert_eq!((bind.as_str(), port), ("192.168.0.1", 6666));
    }

    #[test]
    fn test_resolve_bind_ignores_unparseable_port() {
        let (bind, port) = resolve_bind_with(
            "REAPER_AGENT",
            "0.0.0.0",
            8080,
            lookup_from(&[
                ("REAPER_PORT", "not-a-port"),
                ("REAPER_BIND_ADDR", "not-an-addr"),
            ]),
        );
        assert_eq!((bind.as_str(), port), ("0.0.0.0", 8080));
    }

    #[test]
    fn test_resolve_bind_other_service_prefix_ignored() {
        // A platform-specific var must not affect the agent.
        let (bind, port) = resolve_bind_with(
            "REAPER_AGENT",
            "0.0.0.0",
            8080,
            lookup_from(&[("REAPER_PLATFORM_PORT", "5555")]),
        );
        assert_eq!((bind.as_str(), port), ("0.0.0.0", 8080));
    }

    #[test]
    fn test_default_config() {
        let config = ReaperAgentConfig::default();
        assert_eq!(config.agent.port, 8080);
        // Loopback by default: exposure is an explicit decision (Plan 01 C1).
        assert_eq!(config.agent.bind_address, "127.0.0.1");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.capacity, 10_000);
    }

    #[test]
    fn test_config_from_yaml() {
        let yaml = r#"
agent:
  name: test-agent
  port: 9090
  bind_address: 127.0.0.1

policies:
  bootstrap_dir: /etc/reaper/policies

cache:
  enabled: true
  capacity: 50000
  ttl_seconds: 30
"#;
        let config: ReaperAgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agent.name, "test-agent");
        assert_eq!(config.agent.port, 9090);
        assert_eq!(config.cache.capacity, 50000);
    }

    #[test]
    fn test_config_from_json() {
        let json = r#"{
            "agent": {
                "name": "json-agent",
                "port": 8888
            },
            "cache": {
                "enabled": false
            }
        }"#;
        let config: ReaperAgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.agent.name, "json-agent");
        assert_eq!(config.agent.port, 8888);
        assert!(!config.cache.enabled);
    }

    #[test]
    fn test_summary() {
        let config = ReaperAgentConfig::default();
        let summary = config.summary();
        assert!(summary.contains("127.0.0.1:8080"));
        assert!(summary.contains("enabled"));
    }

    #[test]
    fn test_validation_port_zero() {
        let mut config = ReaperAgentConfig::default();
        config.agent.port = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_management_missing_url() {
        let mut config = ReaperAgentConfig::default();
        config.management.enabled = true;
        assert!(config.validate().is_err());
    }
}
