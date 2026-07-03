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
    /// Header carrying the verified client-certificate fingerprint, set by a
    /// trusted reverse proxy that terminates mTLS (e.g. "x-client-cert-fingerprint").
    ///
    /// `None` (default) disables mTLS client authentication entirely. Only set
    /// this when a trusted proxy performs the TLS client-cert verification AND
    /// strips any client-supplied copy of this header — otherwise a caller could
    /// forge the header and authenticate as any registered agent.
    #[serde(default)]
    pub mtls_fingerprint_header: Option<String>,
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
            mtls_fingerprint_header: None,
        }
    }
}

impl AuthConfig {
    /// Validate auth configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate JWT secret file path if provided
        if let Some(path) = &self.jwt_secret_file {
            if !path.exists() {
                return Err(ConfigError::PathNotFound(path.display().to_string()));
            }
        }

        // A JWT secret is mandatory: it signs session/agent JWTs, keys the OAuth
        // token AEAD, and HMACs the OAuth state. Running without one (previously
        // allowed via `jwt_secret: None`) silently degraded all three to an
        // empty/known key. Require a resolvable secret of at least 32 bytes.
        match self.get_jwt_secret() {
            Some(secret) if secret.len() >= 32 => {}
            Some(_) => return Err(ConfigError::JwtSecretTooShort),
            None => {
                return Err(ConfigError::MissingRequired(
                    "auth.jwt_secret (or auth.jwt_secret_file) must be set to a value of at \
                     least 32 characters"
                        .to_string(),
                ))
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
