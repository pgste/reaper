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

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Environment variable prefix for all Reaper config
pub const ENV_PREFIX: &str = "REAPER";

// ============================================================================
// Agent Configuration
// ============================================================================

/// Complete agent configuration
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
}

/// Agent network and identification settings
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

/// Policy loading settings
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

/// Entity data settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataSettings {
    /// File to load bootstrap entity data from
    pub bootstrap_file: Option<PathBuf>,

    /// Directory containing entity data files
    pub bootstrap_dir: Option<PathBuf>,

    /// Directory to cache synced entity data
    pub cache_dir: Option<PathBuf>,
}

/// Performance tuning settings
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

/// Decision cache settings
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

/// Observability settings
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

/// Management plane settings
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
}

/// TLS/mTLS settings for secure connections
///
/// Enables HTTPS with optional mutual TLS authentication.
/// When `require_client_cert` is true, clients must present valid certificates.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
// Default Values
// ============================================================================

fn default_agent_name() -> String {
    "reaper-agent".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_target_latency() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_cache_capacity() -> usize {
    10_000
}

fn default_cache_ttl() -> u64 {
    10
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_policy_extensions() -> Vec<String> {
    vec![
        "reap".to_string(),
        "yaml".to_string(),
        "yml".to_string(),
        "json".to_string(),
    ]
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

// ============================================================================
// Default Implementations
// ============================================================================

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

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            target_latency_microseconds: default_target_latency(),
            worker_threads: 0,
            enable_simd: true,
        }
    }
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
        }
    }
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_file: None,
            key_file: None,
            ca_file: None,
            require_client_cert: false,
        }
    }
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
        // Agent settings
        if let Ok(val) = std::env::var("REAPER_AGENT_PORT") {
            if let Ok(port) = val.parse() {
                self.agent.port = port;
            }
        }
        if let Ok(val) = std::env::var("REAPER_AGENT_BIND_ADDRESS") {
            self.agent.bind_address = val;
        }
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

        format!(
            "Agent: {}:{}, Mode: {}, Cache: {} ({} entries, {}s TTL), Bootstrap: policies={:?}, data={:?}",
            self.agent.bind_address,
            self.agent.port,
            mgmt_status,
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
// Error Types
// ============================================================================

/// Configuration errors
#[derive(Debug, Clone)]
pub enum ConfigError {
    FileRead(PathBuf, String),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::FileRead(path, err) => {
                write!(f, "Failed to read config file {:?}: {}", path, err)
            }
            ConfigError::Parse(err) => write!(f, "Failed to parse config: {}", err),
            ConfigError::Validation(err) => write!(f, "Config validation failed: {}", err),
        }
    }
}

impl std::error::Error for ConfigError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReaperAgentConfig::default();
        assert_eq!(config.agent.port, 8080);
        assert_eq!(config.agent.bind_address, "0.0.0.0");
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
        assert!(summary.contains("0.0.0.0:8080"));
        assert!(summary.contains("enabled"));
    }
}
