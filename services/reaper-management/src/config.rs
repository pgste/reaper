//! Configuration module for Reaper Management Server
//!
//! Supports YAML configuration files with environment variable overrides.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration structure
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub bundles: BundlesConfig,
    #[serde(default)]
    pub events: EventsConfig,
}

impl Config {
    /// Load configuration from file
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from environment variables with defaults
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Config::default();

        // Server overrides
        if let Ok(port) = std::env::var("REAPER_PORT") {
            config.server.port = port.parse().unwrap_or(8081);
        }
        if let Ok(bind) = std::env::var("REAPER_BIND_ADDRESS") {
            config.server.bind_address = bind;
        }

        // Database overrides
        if let Ok(url) = std::env::var("REAPER_DATABASE_URL") {
            config.database.url = url;
        }
        if let Ok(db_type) = std::env::var("REAPER_DATABASE_TYPE") {
            config.database.db_type = db_type;
        }

        // Storage overrides
        if let Ok(storage_type) = std::env::var("REAPER_STORAGE_TYPE") {
            config.storage.storage_type = storage_type;
        }
        if let Ok(path) = std::env::var("REAPER_STORAGE_PATH") {
            config.storage.filesystem.path = PathBuf::from(path);
        }

        // Auth overrides
        if let Ok(secret) = std::env::var("REAPER_JWT_SECRET") {
            config.auth.jwt_secret = Some(secret);
        }

        Ok(config)
    }

    /// Generate a summary of the configuration
    pub fn summary(&self) -> String {
        format!(
            "Server: {}:{}, DB: {} ({}), Storage: {}",
            self.server.bind_address,
            self.server.port,
            self.database.db_type,
            self.database.url,
            self.storage.storage_type
        )
    }
}

/// Server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
        }
    }
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8081
}

/// Database configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_type", rename = "type")]
    pub db_type: String,
    #[serde(default = "default_db_url")]
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: default_db_type(),
            url: default_db_url(),
            max_connections: default_max_connections(),
        }
    }
}

fn default_db_type() -> String {
    "sqlite".to_string()
}

fn default_db_url() -> String {
    "sqlite:///var/lib/reaper/management.db".to_string()
}

fn default_max_connections() -> u32 {
    5
}

/// Storage configuration for compiled bundles
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_type", rename = "type")]
    pub storage_type: String,
    #[serde(default)]
    pub filesystem: FilesystemStorageConfig,
    #[serde(default)]
    pub s3: S3StorageConfig,
    #[serde(default)]
    pub mongodb: MongoDbStorageConfig,
    #[serde(default)]
    pub dynamodb: DynamoDbStorageConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_type: default_storage_type(),
            filesystem: FilesystemStorageConfig::default(),
            s3: S3StorageConfig::default(),
            mongodb: MongoDbStorageConfig::default(),
            dynamodb: DynamoDbStorageConfig::default(),
        }
    }
}

fn default_storage_type() -> String {
    "filesystem".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilesystemStorageConfig {
    #[serde(default = "default_storage_path")]
    pub path: PathBuf,
}

impl Default for FilesystemStorageConfig {
    fn default() -> Self {
        Self {
            path: default_storage_path(),
        }
    }
}

fn default_storage_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/bundles")
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct S3StorageConfig {
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MongoDbStorageConfig {
    pub uri: Option<String>,
    pub database: Option<String>,
    pub collection: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DynamoDbStorageConfig {
    pub table: Option<String>,
    pub region: Option<String>,
}

/// Authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(default = "default_api_key_prefix")]
    pub api_key_prefix: String,
    #[serde(default = "default_jwt_issuer")]
    pub jwt_issuer: String,
    #[serde(default = "default_jwt_audience")]
    pub jwt_audience: String,
    pub jwt_secret: Option<String>,
    pub jwt_secret_file: Option<PathBuf>,
    #[serde(default = "default_jwt_expiry_hours")]
    pub jwt_expiry_hours: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            api_key_prefix: default_api_key_prefix(),
            jwt_issuer: default_jwt_issuer(),
            jwt_audience: default_jwt_audience(),
            jwt_secret: None,
            jwt_secret_file: None,
            jwt_expiry_hours: default_jwt_expiry_hours(),
        }
    }
}

fn default_api_key_prefix() -> String {
    "rpr_".to_string()
}

fn default_jwt_issuer() -> String {
    "reaper-management".to_string()
}

fn default_jwt_audience() -> String {
    "reaper-agent".to_string()
}

fn default_jwt_expiry_hours() -> u64 {
    24
}

/// Policy sources configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub git: GitSourceConfig,
    #[serde(default)]
    pub api: ApiSourceConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitSourceConfig {
    #[serde(default = "default_git_work_dir")]
    pub work_dir: PathBuf,
    #[serde(default = "default_git_poll_interval")]
    pub default_poll_interval_seconds: u64,
}

impl Default for GitSourceConfig {
    fn default() -> Self {
        Self {
            work_dir: default_git_work_dir(),
            default_poll_interval_seconds: default_git_poll_interval(),
        }
    }
}

fn default_git_work_dir() -> PathBuf {
    PathBuf::from("/var/lib/reaper/git")
}

fn default_git_poll_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiSourceConfig {
    #[serde(default = "default_api_poll_interval")]
    pub default_poll_interval_seconds: u64,
    #[serde(default = "default_api_timeout")]
    pub default_timeout_seconds: u64,
}

impl Default for ApiSourceConfig {
    fn default() -> Self {
        Self {
            default_poll_interval_seconds: default_api_poll_interval(),
            default_timeout_seconds: default_api_timeout(),
        }
    }
}

fn default_api_poll_interval() -> u64 {
    300
}

fn default_api_timeout() -> u64 {
    30
}

/// Bundle compilation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundlesConfig {
    #[serde(default)]
    pub auto_compile_on_source_sync: bool,
    #[serde(default = "default_require_staged")]
    pub require_staged_before_promote: bool,
}

impl Default for BundlesConfig {
    fn default() -> Self {
        Self {
            auto_compile_on_source_sync: false,
            require_staged_before_promote: true,
        }
    }
}

fn default_require_staged() -> bool {
    true
}

/// SSE events configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsConfig {
    #[serde(default = "default_sse_keepalive")]
    pub sse_keepalive_seconds: u64,
    #[serde(default = "default_max_sse_connections")]
    pub max_connections_per_org: usize,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            sse_keepalive_seconds: default_sse_keepalive(),
            max_connections_per_org: 1000,
        }
    }
}

fn default_sse_keepalive() -> u64 {
    30
}

fn default_max_sse_connections() -> usize {
    1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.server.port, 8081);
        assert_eq!(config.database.db_type, "sqlite");
        assert_eq!(config.storage.storage_type, "filesystem");
    }

    #[test]
    fn test_config_from_env() {
        std::env::set_var("REAPER_PORT", "9090");
        let config = Config::from_env().unwrap();
        assert_eq!(config.server.port, 9090);
        std::env::remove_var("REAPER_PORT");
    }
}
