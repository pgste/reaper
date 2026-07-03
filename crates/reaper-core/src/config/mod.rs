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
    AgentSettings, CacheSettings, DataSettings, ManagementSettings, ObservabilitySettings,
    PerformanceSettings, PolicySettings, TlsSettings, UdsSettings,
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
