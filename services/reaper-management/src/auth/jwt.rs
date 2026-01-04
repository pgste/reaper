//! JWT token management
//!
//! Provides JWT generation and validation for authenticated sessions.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AuthConfig;

/// JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (agent ID or user ID)
    pub sub: String,
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// JWT ID
    pub jti: String,
    /// Organization ID
    pub org_id: String,
    /// Scopes
    pub scopes: Vec<String>,
    /// Custom claims
    #[serde(flatten)]
    pub custom: serde_json::Value,
}

/// JWT manager
pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    issuer: String,
    audience: String,
    expiry_hours: u64,
}

impl JwtManager {
    /// Create a new JWT manager from config
    pub fn new(config: &AuthConfig) -> Result<Self, JwtError> {
        let secret = config
            .jwt_secret
            .as_ref()
            .ok_or_else(|| JwtError::Config("JWT secret not configured".to_string()))?;

        Ok(Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            issuer: config.jwt_issuer.clone(),
            audience: config.jwt_audience.clone(),
            expiry_hours: config.jwt_expiry_hours,
        })
    }

    /// Create a new JWT manager with explicit secret
    pub fn with_secret(secret: &str, issuer: &str, audience: &str, expiry_hours: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            issuer: issuer.to_string(),
            audience: audience.to_string(),
            expiry_hours,
        }
    }

    /// Generate a new JWT token
    pub fn generate(
        &self,
        subject: &str,
        org_id: Uuid,
        scopes: Vec<String>,
        custom: Option<serde_json::Value>,
    ) -> Result<String, JwtError> {
        let now = Utc::now();
        let exp = now + Duration::hours(self.expiry_hours as i64);

        let claims = Claims {
            sub: subject.to_string(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
            jti: Uuid::new_v4().to_string(),
            org_id: org_id.to_string(),
            scopes,
            custom: custom.unwrap_or(serde_json::json!({})),
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| JwtError::Encoding(e.to_string()))
    }

    /// Validate and decode a JWT token
    pub fn validate(&self, token: &str) -> Result<Claims, JwtError> {
        let mut validation = Validation::default();
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);

        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| JwtError::Validation(e.to_string()))?;

        Ok(token_data.claims)
    }

    /// Refresh a token (generate new with same claims but new expiry)
    pub fn refresh(&self, token: &str) -> Result<String, JwtError> {
        let claims = self.validate(token)?;

        // Parse org_id
        let org_id = Uuid::parse_str(&claims.org_id)
            .map_err(|e| JwtError::Validation(format!("Invalid org_id in claims: {}", e)))?;

        self.generate(&claims.sub, org_id, claims.scopes, Some(claims.custom))
    }
}

/// JWT errors
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("JWT configuration error: {0}")]
    Config(String),

    #[error("JWT encoding error: {0}")]
    Encoding(String),

    #[error("JWT validation error: {0}")]
    Validation(String),
}

impl Claims {
    /// Get the subject as UUID
    pub fn subject_uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.sub).ok()
    }

    /// Get the org_id as UUID
    pub fn org_uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.org_id).ok()
    }

    /// Check if token has a specific scope
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(&"admin".to_string()) || self.scopes.contains(&scope.to_string())
    }

    /// Check if token is expired
    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() > self.exp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> JwtManager {
        JwtManager::with_secret(
            "test-secret-key-for-testing-only",
            "reaper-management",
            "reaper-agent",
            24,
        )
    }

    #[test]
    fn test_generate_and_validate() {
        let manager = test_manager();
        let org_id = Uuid::new_v4();

        let token = manager
            .generate(
                "agent-123",
                org_id,
                vec!["agent:read".to_string(), "policy:read".to_string()],
                None,
            )
            .unwrap();

        let claims = manager.validate(&token).unwrap();

        assert_eq!(claims.sub, "agent-123");
        assert_eq!(claims.org_id, org_id.to_string());
        assert_eq!(claims.scopes.len(), 2);
        assert!(claims.has_scope("agent:read"));
        assert!(!claims.is_expired());
    }

    #[test]
    fn test_invalid_token() {
        let manager = test_manager();

        let result = manager.validate("invalid-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_refresh() {
        let manager = test_manager();
        let org_id = Uuid::new_v4();

        let token = manager
            .generate("agent-123", org_id, vec!["agent:read".to_string()], None)
            .unwrap();

        // Small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));

        let new_token = manager.refresh(&token).unwrap();

        let old_claims = manager.validate(&token).unwrap();
        let new_claims = manager.validate(&new_token).unwrap();

        // Same subject, different jti and timestamps
        assert_eq!(old_claims.sub, new_claims.sub);
        assert_ne!(old_claims.jti, new_claims.jti);
        assert!(new_claims.exp >= old_claims.exp);
    }

    #[test]
    fn test_admin_scope() {
        let claims = Claims {
            sub: "admin".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: Utc::now().timestamp() + 3600,
            iat: Utc::now().timestamp(),
            jti: Uuid::new_v4().to_string(),
            org_id: Uuid::new_v4().to_string(),
            scopes: vec!["admin".to_string()],
            custom: serde_json::json!({}),
        };

        // Admin should have access to everything
        assert!(claims.has_scope("admin"));
        assert!(claims.has_scope("agent:read"));
        assert!(claims.has_scope("policy:write"));
    }
}
