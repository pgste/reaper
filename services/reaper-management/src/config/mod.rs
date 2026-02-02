//! Configuration module for Reaper Management Server
//!
//! Supports YAML configuration files with environment variable overrides.

mod auth;
mod bundles;
mod database;
mod error;
mod events;
mod oauth;
mod rate_limit;
mod server;
mod sources;
mod storage;
mod sync;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export all configuration types
pub use auth::AuthConfig;
pub use bundles::BundlesConfig;
pub use database::DatabaseConfig;
pub use error::ConfigError;
pub use events::EventsConfig;
pub use oauth::{BitbucketOAuthConfig, GitHubOAuthConfig, GitLabOAuthConfig, OAuthConfig};
pub use rate_limit::RateLimitConfig;
pub use server::ServerConfig;
pub use sources::{
    ApiSourceConfig, BundleUrlSourceConfig, GitSourceConfig, S3SourceConfig, SourcesConfig,
};
pub use storage::{
    DynamoDbStorageConfig, FilesystemStorageConfig, MongoDbStorageConfig, S3StorageConfig,
    StorageConfig,
};
pub use sync::SyncConfig;

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
