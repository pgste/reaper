//! Configuration module for Reaper Management Server
//!
//! Supports YAML configuration files with environment variable overrides.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Configuration validation errors
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Invalid port: {0}. Must be between 1 and 65535")]
    InvalidPort(u16),
    #[error("Invalid bind address: {0}")]
    InvalidBindAddress(String),
    #[error("Invalid database URL: {0}")]
    InvalidDatabaseUrl(String),
    #[error("Unsupported database type: {0}. Supported: sqlite, postgres")]
    UnsupportedDatabaseType(String),
    #[error("Unsupported storage type: {0}. Supported: filesystem, s3, mongodb, dynamodb")]
    UnsupportedStorageType(String),
    #[error("Missing required configuration: {0}")]
    MissingRequired(String),
    #[error("Invalid timeout value: {0}. Must be positive")]
    InvalidTimeout(String),
    #[error("Invalid rate limit: {0}. Must be positive")]
    InvalidRateLimit(String),
    #[error("Path does not exist: {0}")]
    PathNotFound(String),
    #[error("Path is not writable: {0}")]
    PathNotWritable(String),
    #[error("JWT secret too short: minimum 32 characters required")]
    JwtSecretTooShort,
    #[error("S3 storage requires bucket name")]
    S3MissingBucket,
    #[error("MongoDB storage requires URI")]
    MongoDbMissingUri,
    #[error("DynamoDB storage requires table name")]
    DynamoDbMissingTable,
}

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
    pub sync: SyncConfig,
    #[serde(default)]
    pub bundles: BundlesConfig,
    #[serde(default)]
    pub events: EventsConfig,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub oauth: OAuthConfig,
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

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate server config
        self.server.validate()?;

        // Validate database config
        self.database.validate()?;

        // Validate storage config
        self.storage.validate()?;

        // Validate auth config
        self.auth.validate()?;

        // Validate rate limit config
        self.rate_limit.validate()?;

        // Validate sync config
        self.sync.validate()?;

        Ok(())
    }

    /// Validate and prepare directories (create if needed)
    pub fn prepare_directories(&self) -> Result<(), ConfigError> {
        // Prepare storage directory
        if self.storage.storage_type == "filesystem" {
            let path = &self.storage.filesystem.path;
            if !path.exists() {
                std::fs::create_dir_all(path).map_err(|_| {
                    ConfigError::PathNotWritable(path.display().to_string())
                })?;
            }
        }

        // Prepare sync directories
        for path in [
            &self.sync.git_base_path,
            &self.sync.s3_cache_path,
            &self.sync.bundle_storage_path,
        ] {
            if !path.exists() {
                std::fs::create_dir_all(path).map_err(|_| {
                    ConfigError::PathNotWritable(path.display().to_string())
                })?;
            }
        }

        Ok(())
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

impl ServerConfig {
    /// Validate server configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate port range
        if self.port == 0 {
            return Err(ConfigError::InvalidPort(self.port));
        }

        // Validate bind address
        if self.bind_address.parse::<std::net::IpAddr>().is_err() {
            return Err(ConfigError::InvalidBindAddress(self.bind_address.clone()));
        }

        Ok(())
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

impl DatabaseConfig {
    /// Validate database configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate database type
        match self.db_type.as_str() {
            "sqlite" | "postgres" | "postgresql" => {}
            other => return Err(ConfigError::UnsupportedDatabaseType(other.to_string())),
        }

        // Validate URL format
        if self.db_type == "sqlite" {
            if !self.url.starts_with("sqlite:") {
                return Err(ConfigError::InvalidDatabaseUrl(
                    "SQLite URL must start with 'sqlite:'".to_string(),
                ));
            }
        } else if self.db_type == "postgres" || self.db_type == "postgresql" {
            if !self.url.starts_with("postgres://") && !self.url.starts_with("postgresql://") {
                return Err(ConfigError::InvalidDatabaseUrl(
                    "PostgreSQL URL must start with 'postgres://' or 'postgresql://'".to_string(),
                ));
            }
        }

        // Validate max connections
        if self.max_connections == 0 {
            return Err(ConfigError::InvalidRateLimit(
                "max_connections must be positive".to_string(),
            ));
        }

        Ok(())
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

impl StorageConfig {
    /// Validate storage configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        match self.storage_type.as_str() {
            "filesystem" => {
                // Filesystem storage validated later in prepare_directories
                Ok(())
            }
            "s3" => {
                if self.s3.bucket.is_none() {
                    return Err(ConfigError::S3MissingBucket);
                }
                Ok(())
            }
            "mongodb" => {
                if self.mongodb.uri.is_none() {
                    return Err(ConfigError::MongoDbMissingUri);
                }
                Ok(())
            }
            "dynamodb" => {
                if self.dynamodb.table.is_none() {
                    return Err(ConfigError::DynamoDbMissingTable);
                }
                Ok(())
            }
            other => Err(ConfigError::UnsupportedStorageType(other.to_string())),
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

impl AuthConfig {
    /// Validate auth configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate JWT secret if provided
        if let Some(secret) = &self.jwt_secret {
            if secret.len() < 32 {
                return Err(ConfigError::JwtSecretTooShort);
            }
        }

        // Validate JWT secret file if provided
        if let Some(path) = &self.jwt_secret_file {
            if !path.exists() {
                return Err(ConfigError::PathNotFound(path.display().to_string()));
            }
        }

        // Validate expiry hours
        if self.jwt_expiry_hours == 0 {
            return Err(ConfigError::InvalidTimeout(
                "jwt_expiry_hours must be positive".to_string(),
            ));
        }

        Ok(())
    }

    /// Get JWT secret from config or file
    pub fn get_jwt_secret(&self) -> Option<String> {
        if let Some(secret) = &self.jwt_secret {
            return Some(secret.clone());
        }

        if let Some(path) = &self.jwt_secret_file {
            if let Ok(secret) = std::fs::read_to_string(path) {
                return Some(secret.trim().to_string());
            }
        }

        None
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

/// Sync configuration for policy sources
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    /// Base path for Git repositories
    #[serde(default = "default_git_work_dir")]
    pub git_base_path: PathBuf,
    /// Base path for S3 cache
    #[serde(default = "default_s3_cache_path")]
    pub s3_cache_path: PathBuf,
    /// Base path for bundle URL storage
    #[serde(default = "default_bundle_storage_path")]
    pub bundle_storage_path: PathBuf,
    /// Interval to check for due syncs
    #[serde(default = "default_sync_check_interval")]
    pub check_interval_secs: u64,
    /// Maximum concurrent sync operations
    #[serde(default = "default_max_concurrent_syncs")]
    pub max_concurrent: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            git_base_path: default_git_work_dir(),
            s3_cache_path: default_s3_cache_path(),
            bundle_storage_path: default_bundle_storage_path(),
            check_interval_secs: default_sync_check_interval(),
            max_concurrent: default_max_concurrent_syncs(),
        }
    }
}

impl SyncConfig {
    /// Validate sync configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.check_interval_secs == 0 {
            return Err(ConfigError::InvalidTimeout(
                "check_interval_secs must be positive".to_string(),
            ));
        }

        if self.max_concurrent == 0 {
            return Err(ConfigError::InvalidRateLimit(
                "max_concurrent must be positive".to_string(),
            ));
        }

        Ok(())
    }
}

fn default_s3_cache_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/sync/s3")
}

fn default_bundle_storage_path() -> PathBuf {
    PathBuf::from("/var/lib/reaper/sync/bundles")
}

fn default_sync_check_interval() -> u64 {
    60
}

fn default_max_concurrent_syncs() -> usize {
    5
}

/// Policy sources configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub git: GitSourceConfig,
    #[serde(default)]
    pub api: ApiSourceConfig,
    #[serde(default)]
    pub s3: S3SourceConfig,
    #[serde(default)]
    pub bundle_url: BundleUrlSourceConfig,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct S3SourceConfig {
    #[serde(default = "default_s3_poll_interval")]
    pub default_poll_interval_seconds: u64,
    #[serde(default)]
    pub default_region: Option<String>,
}

impl Default for S3SourceConfig {
    fn default() -> Self {
        Self {
            default_poll_interval_seconds: default_s3_poll_interval(),
            default_region: None,
        }
    }
}

fn default_s3_poll_interval() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundleUrlSourceConfig {
    #[serde(default = "default_bundle_download_timeout")]
    pub default_download_timeout_seconds: u64,
    #[serde(default = "default_verify_checksums")]
    pub verify_checksums: bool,
}

impl Default for BundleUrlSourceConfig {
    fn default() -> Self {
        Self {
            default_download_timeout_seconds: default_bundle_download_timeout(),
            verify_checksums: true,
        }
    }
}

fn default_bundle_download_timeout() -> u64 {
    60
}

fn default_verify_checksums() -> bool {
    true
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

/// Rate limiting configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
    /// Requests per second (global)
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: u32,
    /// Burst size (bucket capacity)
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
    /// Signup requests per hour per IP
    #[serde(default = "default_signup_per_hour")]
    pub signup_per_hour: u32,
    /// Login attempts per minute per IP
    #[serde(default = "default_login_per_minute")]
    pub login_per_minute: u32,
    /// API requests per org per minute
    #[serde(default = "default_api_per_org_per_minute")]
    pub api_per_org_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: default_rate_limit_enabled(),
            requests_per_second: default_requests_per_second(),
            burst_size: default_burst_size(),
            signup_per_hour: default_signup_per_hour(),
            login_per_minute: default_login_per_minute(),
            api_per_org_per_minute: default_api_per_org_per_minute(),
        }
    }
}

impl RateLimitConfig {
    /// Validate rate limit configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.enabled {
            if self.requests_per_second == 0 {
                return Err(ConfigError::InvalidRateLimit(
                    "requests_per_second must be positive".to_string(),
                ));
            }

            if self.burst_size == 0 {
                return Err(ConfigError::InvalidRateLimit(
                    "burst_size must be positive".to_string(),
                ));
            }

            // Burst size should be >= requests_per_second for proper token bucket
            if self.burst_size < self.requests_per_second {
                tracing::warn!(
                    "burst_size ({}) is less than requests_per_second ({}), this may cause issues",
                    self.burst_size,
                    self.requests_per_second
                );
            }
        }

        Ok(())
    }
}

fn default_rate_limit_enabled() -> bool {
    true
}

fn default_requests_per_second() -> u32 {
    100
}

fn default_burst_size() -> u32 {
    200
}

fn default_signup_per_hour() -> u32 {
    5
}

fn default_login_per_minute() -> u32 {
    10
}

fn default_api_per_org_per_minute() -> u32 {
    1000
}

/// OAuth configuration for Git providers
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OAuthConfig {
    pub github: Option<GitHubOAuthConfig>,
    pub gitlab: Option<GitLabOAuthConfig>,
    pub bitbucket: Option<BitbucketOAuthConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "default_github_redirect_uri")]
    pub redirect_uri: String,
}

fn default_github_redirect_uri() -> String {
    "http://localhost:8081/auth/github/callback".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitLabOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub base_url: Option<String>, // For self-hosted GitLab
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
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
