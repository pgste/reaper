//! Authentication configuration

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::ConfigError;

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
