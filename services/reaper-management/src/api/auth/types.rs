//! Request and response types for authentication API endpoints.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{
    jwks::JwksConfig,
    mtls::ClientCertificate,
    ApiKey,
};

// ==================== API Key Types ====================

/// Response for listing API keys
#[derive(Debug, Serialize)]
pub struct ListApiKeysResponse {
    pub api_keys: Vec<ApiKeySummary>,
}

/// Summary of an API key (without sensitive data)
#[derive(Debug, Serialize)]
pub struct ApiKeySummary {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_revoked: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ApiKey> for ApiKeySummary {
    fn from(key: ApiKey) -> Self {
        Self {
            id: key.id,
            name: key.name,
            key_prefix: key.key_prefix,
            scopes: key.scopes,
            expires_at: key.expires_at,
            last_used_at: key.last_used_at,
            is_revoked: key.is_revoked,
            created_at: key.created_at,
        }
    }
}

/// Request to create an API key
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ==================== Token Types ====================

/// Request to refresh a token
#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    pub token: String,
}

/// Response with new token
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

// ==================== JWKS Types ====================

/// Response for listing JWKS configurations
#[derive(Debug, Serialize)]
pub struct ListJwksConfigsResponse {
    pub configs: Vec<JwksConfigSummary>,
}

/// Summary of a JWKS configuration
#[derive(Debug, Serialize)]
pub struct JwksConfigSummary {
    pub id: Uuid,
    pub name: String,
    pub jwks_url: String,
    pub issuer: String,
    pub audience: Option<String>,
    pub is_active: bool,
    pub cache_ttl_secs: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<JwksConfig> for JwksConfigSummary {
    fn from(config: JwksConfig) -> Self {
        Self {
            id: config.id,
            name: config.name,
            jwks_url: config.jwks_url,
            issuer: config.issuer,
            audience: config.audience,
            is_active: config.is_active,
            cache_ttl_secs: config.cache_ttl_secs,
            created_at: config.created_at,
            updated_at: config.updated_at,
        }
    }
}

/// Request to create a JWKS configuration
#[derive(Debug, Deserialize)]
pub struct CreateJwksConfigRequest {
    /// Display name for this configuration
    pub name: String,
    /// JWKS endpoint URL (e.g., https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys)
    pub jwks_url: String,
    /// Expected issuer claim in tokens
    pub issuer: String,
    /// Expected audience claim (optional)
    pub audience: Option<String>,
}

// ==================== Client Certificate Types ====================

/// Response for listing client certificates
#[derive(Debug, Serialize)]
pub struct ListCertificatesResponse {
    pub certificates: Vec<CertificateSummary>,
}

/// Summary of a client certificate
#[derive(Debug, Serialize)]
pub struct CertificateSummary {
    pub id: Uuid,
    pub fingerprint: String,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub agent_id: Option<Uuid>,
    pub not_before: Option<chrono::DateTime<chrono::Utc>>,
    pub not_after: Option<chrono::DateTime<chrono::Utc>>,
    pub is_revoked: bool,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revocation_reason: Option<String>,
    pub is_valid: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ClientCertificate> for CertificateSummary {
    fn from(cert: ClientCertificate) -> Self {
        let is_valid = cert.is_valid();
        Self {
            id: cert.id,
            fingerprint: cert.fingerprint,
            subject: cert.subject,
            issuer: cert.issuer,
            agent_id: cert.agent_id,
            not_before: cert.not_before,
            not_after: cert.not_after,
            is_revoked: cert.is_revoked,
            revoked_at: cert.revoked_at,
            revocation_reason: cert.revocation_reason,
            is_valid,
            created_at: cert.created_at,
        }
    }
}

/// Request to register a client certificate
#[derive(Debug, Deserialize)]
pub struct RegisterCertificateRequest {
    /// SHA-256 fingerprint of the certificate (hex encoded)
    pub fingerprint: String,
    /// Subject DN (Distinguished Name)
    pub subject: Option<String>,
    /// Issuer DN
    pub issuer: Option<String>,
    /// Certificate validity start
    pub not_before: Option<chrono::DateTime<chrono::Utc>>,
    /// Certificate validity end
    pub not_after: Option<chrono::DateTime<chrono::Utc>>,
    /// Agent to bind this certificate to (optional)
    pub agent_id: Option<Uuid>,
}

/// Request to revoke a certificate
#[derive(Debug, Deserialize)]
pub struct RevokeCertificateRequest {
    pub reason: Option<String>,
}

/// Request to bind a certificate to an agent
#[derive(Debug, Deserialize)]
pub struct BindCertificateRequest {
    pub agent_id: Uuid,
}

// Re-export ApiKeyCreated for convenience
pub use crate::auth::api_key::ApiKeyCreated;
