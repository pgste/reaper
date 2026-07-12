//! Database configuration

use serde::{Deserialize, Serialize};

use super::error::ConfigError;

/// Database configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_type", rename = "type")]
    pub db_type: String,
    #[serde(default = "default_db_url")]
    pub url: String,
    /// Optional read-replica URL (Postgres only): the managed reader endpoint
    /// or the CloudNativePG `-ro` Service. When set, a second pool is opened
    /// for future read-scaling; writes always go to `url`. See
    /// docs/deployment/CONTROL_PLANE_HA_DR.md §6.
    #[serde(default)]
    pub replica_url: Option<String>,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: default_db_type(),
            url: default_db_url(),
            replica_url: None,
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
        } else if (self.db_type == "postgres" || self.db_type == "postgresql")
            && !self.url.starts_with("postgres://")
            && !self.url.starts_with("postgresql://")
        {
            return Err(ConfigError::InvalidDatabaseUrl(
                "PostgreSQL URL must start with 'postgres://' or 'postgresql://'".to_string(),
            ));
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
