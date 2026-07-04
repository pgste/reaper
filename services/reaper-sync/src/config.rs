//! Sync Client Configuration
//!
//! Configuration for the Reaper Sync Client, which polls a management server
//! and deploys policies to agents.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Configuration errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Root configuration structure
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    pub sync: SyncSettings,
}

/// Main sync settings
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncSettings {
    /// Management server configuration
    pub server: ServerConfig,
    /// Authentication configuration
    pub auth: AuthConfig,
    /// Scope configuration (teams, environments)
    pub scope: ScopeConfig,
    /// Data-plane replication configuration
    #[serde(default)]
    pub datastore: DatastoreSyncConfig,
    /// Sync behavior configuration
    pub behavior: BehaviorConfig,
    /// Agent connection configuration
    pub agent: AgentConfig,
    /// Local cache configuration
    #[serde(default)]
    pub cache: CacheConfig,
    /// Metrics configuration
    #[serde(default)]
    pub metrics: MetricsConfig,
}

/// Management server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// Server URL (e.g., "https://reaper-platform.example.com")
    pub url: String,
    /// API version to use
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

/// Authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    /// Authentication type: "api_token", "mtls", "oauth2", "none"
    #[serde(rename = "type", default = "default_auth_type")]
    pub auth_type: String,
    /// Path to token file (for api_token auth)
    pub token_file: Option<PathBuf>,
    /// API token value (alternative to token_file)
    pub token: Option<String>,
    /// Client certificate file (for mTLS)
    pub cert_file: Option<PathBuf>,
    /// Client key file (for mTLS)
    pub key_file: Option<PathBuf>,
    /// CA certificate file (for mTLS)
    pub ca_file: Option<PathBuf>,
}

/// Scope configuration - what policies to sync
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatastoreSyncConfig {
    /// Enable data-plane replication (fetch published datastore versions
    /// and keep the agent's DataStore current with heartbeat staleness).
    #[serde(default)]
    pub enabled: bool,
    /// Organization slug or id owning the datastore.
    #[serde(default)]
    pub org: String,
    /// Namespace slug the datastore belongs to.
    #[serde(default)]
    pub namespace: String,
}

impl Default for DatastoreSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            org: String::new(),
            namespace: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScopeConfig {
    /// Teams to sync policies for
    #[serde(default)]
    pub teams: Vec<String>,
    /// Environments to sync (e.g., "production", "staging")
    #[serde(default)]
    pub environments: Vec<String>,
    /// Regions to sync
    #[serde(default)]
    pub regions: Vec<String>,
    /// Specific policy IDs to sync (if empty, sync all in scope)
    #[serde(default)]
    pub policy_ids: Vec<String>,
}

/// Sync behavior configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BehaviorConfig {
    /// Sync mode: "active" (continuous polling), "on-demand", "offline"
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Poll interval in seconds (for active mode)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u64,
    /// Batch size for policy fetching
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Maximum retry attempts
    #[serde(default = "default_retry_attempts")]
    pub retry_max_attempts: u32,
    /// Retry backoff in seconds
    #[serde(default = "default_retry_backoff")]
    pub retry_backoff_seconds: u64,
    /// Whether to sync on startup
    #[serde(default = "default_sync_on_start")]
    pub sync_on_start: bool,
}

/// Agent connection configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Agent URL (e.g., "http://localhost:8080")
    pub url: String,
    /// Health check interval in seconds
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_seconds: u64,
    /// Request timeout in seconds
    #[serde(default = "default_agent_timeout")]
    pub timeout_seconds: u64,
}

/// Local cache configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CacheConfig {
    /// Cache directory for offline mode
    pub directory: Option<PathBuf>,
    /// Enable offline mode (use cache when server unavailable)
    #[serde(default = "default_offline_mode")]
    pub enable_offline_mode: bool,
    /// Maximum cache age in hours
    #[serde(default = "default_max_age")]
    pub max_age_hours: u64,
}

/// Metrics configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MetricsConfig {
    /// Enable metrics reporting
    #[serde(default = "default_metrics_enable")]
    pub enable: bool,
    /// Metrics report interval in seconds
    #[serde(default = "default_report_interval")]
    pub report_interval_seconds: u64,
}

// Default value functions
fn default_api_version() -> String {
    "v1".to_string()
}
fn default_timeout() -> u64 {
    30
}
fn default_auth_type() -> String {
    "none".to_string()
}
fn default_mode() -> String {
    "active".to_string()
}
fn default_poll_interval() -> u64 {
    30
}
fn default_batch_size() -> usize {
    100
}
fn default_retry_attempts() -> u32 {
    3
}
fn default_retry_backoff() -> u64 {
    5
}
fn default_sync_on_start() -> bool {
    true
}
fn default_health_check_interval() -> u64 {
    10
}
fn default_agent_timeout() -> u64 {
    10
}
fn default_offline_mode() -> bool {
    true
}
fn default_max_age() -> u64 {
    24
}
fn default_metrics_enable() -> bool {
    false
}
fn default_report_interval() -> u64 {
    60
}

impl SyncConfig {
    /// Load configuration from a YAML file
    pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: SyncConfig = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self, ConfigError> {
        let server_url = std::env::var("REAPER_SERVER_URL")
            .unwrap_or_else(|_| "http://localhost:8081".to_string());
        let agent_url = std::env::var("REAPER_AGENT_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string());
        let teams: Vec<String> = std::env::var("REAPER_TEAMS")
            .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
            .unwrap_or_default();
        let poll_interval: u64 = std::env::var("REAPER_POLL_INTERVAL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let auth_token = std::env::var("REAPER_AUTH_TOKEN").ok();

        let config = SyncConfig {
            sync: SyncSettings {
                server: ServerConfig {
                    url: server_url,
                    api_version: default_api_version(),
                    timeout_seconds: default_timeout(),
                },
                datastore: DatastoreSyncConfig {
                    enabled: std::env::var("REAPER_DATASTORE_SYNC_ENABLED")
                        .map(|v| v == "true" || v == "1")
                        .unwrap_or(false),
                    org: std::env::var("REAPER_DATASTORE_ORG").unwrap_or_default(),
                    namespace: std::env::var("REAPER_DATASTORE_NAMESPACE").unwrap_or_default(),
                },
                auth: AuthConfig {
                    auth_type: if auth_token.is_some() {
                        "api_token".to_string()
                    } else {
                        "none".to_string()
                    },
                    token: auth_token,
                    token_file: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                },
                scope: ScopeConfig {
                    teams,
                    environments: vec![],
                    regions: vec![],
                    policy_ids: vec![],
                },
                behavior: BehaviorConfig {
                    mode: default_mode(),
                    poll_interval_seconds: poll_interval,
                    batch_size: default_batch_size(),
                    retry_max_attempts: default_retry_attempts(),
                    retry_backoff_seconds: default_retry_backoff(),
                    sync_on_start: default_sync_on_start(),
                },
                agent: AgentConfig {
                    url: agent_url,
                    health_check_interval_seconds: default_health_check_interval(),
                    timeout_seconds: default_agent_timeout(),
                },
                cache: CacheConfig::default(),
                metrics: MetricsConfig::default(),
            },
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.sync.server.url.is_empty() {
            return Err(ConfigError::Validation(
                "Server URL is required".to_string(),
            ));
        }
        if self.sync.agent.url.is_empty() {
            return Err(ConfigError::Validation("Agent URL is required".to_string()));
        }
        if self.sync.behavior.poll_interval_seconds == 0 {
            return Err(ConfigError::Validation(
                "Poll interval must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }

    /// Get a summary of the configuration
    pub fn summary(&self) -> String {
        format!(
            "server={}, agent={}, mode={}, poll_interval={}s, teams={:?}",
            self.sync.server.url,
            self.sync.agent.url,
            self.sync.behavior.mode,
            self.sync.behavior.poll_interval_seconds,
            self.sync.scope.teams
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_defaults() {
        // Clear any existing env vars
        std::env::remove_var("REAPER_SERVER_URL");
        std::env::remove_var("REAPER_AGENT_URL");
        std::env::remove_var("REAPER_TEAMS");

        let config = SyncConfig::from_env().unwrap();
        assert_eq!(config.sync.server.url, "http://localhost:8081");
        assert_eq!(config.sync.agent.url, "http://localhost:8080");
        assert_eq!(config.sync.behavior.mode, "active");
    }

    #[test]
    fn test_config_validation_empty_server() {
        let config = SyncConfig {
            sync: SyncSettings {
                datastore: Default::default(),
                server: ServerConfig {
                    url: "".to_string(),
                    api_version: "v1".to_string(),
                    timeout_seconds: 30,
                },
                auth: AuthConfig {
                    auth_type: "none".to_string(),
                    token: None,
                    token_file: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                },
                scope: ScopeConfig {
                    teams: vec![],
                    environments: vec![],
                    regions: vec![],
                    policy_ids: vec![],
                },
                behavior: BehaviorConfig {
                    mode: "active".to_string(),
                    poll_interval_seconds: 30,
                    batch_size: 100,
                    retry_max_attempts: 3,
                    retry_backoff_seconds: 5,
                    sync_on_start: true,
                },
                agent: AgentConfig {
                    url: "http://localhost:8080".to_string(),
                    health_check_interval_seconds: 10,
                    timeout_seconds: 10,
                },
                cache: CacheConfig::default(),
                metrics: MetricsConfig::default(),
            },
        };

        assert!(config.validate().is_err());
    }
}
