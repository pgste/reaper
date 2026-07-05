//! JWKS (JSON Web Key Set) validation
//!
//! Provides validation of JWT tokens against external identity providers
//! using JWKS endpoints. Supports Auth0, Okta, Azure AD, and other OIDC providers.

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use thiserror::Error;
use uuid::Uuid;

/// Reject IPs that must never be reachable via a user-configured JWKS URL —
/// loopback, private, link-local (incl. the 169.254.169.254 cloud metadata
/// endpoint), CGNAT, and IPv6 equivalents. This is the core SSRF guard.
fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0
                // 100.64.0.0/10 carrier-grade NAT
                || (o[0] == 100 && (o[1] & 0xC0) == 0x40)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Validate a JWKS URL before fetching: require HTTPS and ensure every resolved
/// address is a public IP. Blocks SSRF to internal services and cloud metadata.
///
/// Note: this resolves the host and checks the addresses; a determined attacker
/// could still attempt DNS rebinding between this check and the actual fetch.
/// That residual risk is much smaller than the unrestricted fetch it replaces.
async fn validate_jwks_url(url: &str) -> Result<(), JwksError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| JwksError::UrlNotAllowed("malformed URL".to_string()))?;

    if parsed.scheme() != "https" {
        return Err(JwksError::UrlNotAllowed("must use https".to_string()));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| JwksError::UrlNotAllowed("missing host".to_string()))?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| JwksError::UrlNotAllowed("host does not resolve".to_string()))?;

    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        if is_disallowed_ip(&addr.ip()) {
            return Err(JwksError::UrlNotAllowed(
                "resolves to a disallowed internal address".to_string(),
            ));
        }
    }
    if !resolved_any {
        return Err(JwksError::UrlNotAllowed(
            "host does not resolve".to_string(),
        ));
    }

    Ok(())
}

/// JWKS configuration for an organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwksConfig {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// JWKS endpoint URL
    pub jwks_url: String,
    /// Expected issuer claim
    pub issuer: String,
    /// Expected audience claim (optional)
    pub audience: Option<String>,
    /// Whether this config is active
    pub is_active: bool,
    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// JWKS response from the endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct JwksResponse {
    pub keys: Vec<Jwk>,
}

/// JSON Web Key
#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(rename = "use")]
    pub key_use: Option<String>,
    pub kid: Option<String>,
    pub alg: Option<String>,
    pub n: Option<String>,
    pub e: Option<String>,
    /// For EC keys
    pub crv: Option<String>,
    pub x: Option<String>,
    pub y: Option<String>,
    /// X.509 certificate chain
    pub x5c: Option<Vec<String>>,
}

impl Jwk {
    /// Convert JWK to jsonwebtoken DecodingKey
    pub fn to_decoding_key(&self) -> Result<DecodingKey, JwksError> {
        match self.kty.as_str() {
            "RSA" => {
                let n = self
                    .n
                    .as_ref()
                    .ok_or_else(|| JwksError::InvalidKey("Missing 'n' for RSA key".to_string()))?;
                let e = self
                    .e
                    .as_ref()
                    .ok_or_else(|| JwksError::InvalidKey("Missing 'e' for RSA key".to_string()))?;
                DecodingKey::from_rsa_components(n, e)
                    .map_err(|e| JwksError::InvalidKey(e.to_string()))
            }
            "EC" => {
                let x = self
                    .x
                    .as_ref()
                    .ok_or_else(|| JwksError::InvalidKey("Missing 'x' for EC key".to_string()))?;
                let y = self
                    .y
                    .as_ref()
                    .ok_or_else(|| JwksError::InvalidKey("Missing 'y' for EC key".to_string()))?;
                DecodingKey::from_ec_components(x, y)
                    .map_err(|e| JwksError::InvalidKey(e.to_string()))
            }
            _ => Err(JwksError::InvalidKey(format!(
                "Unsupported key type: {}",
                self.kty
            ))),
        }
    }

    /// Get the algorithm for this key
    pub fn algorithm(&self) -> Algorithm {
        match self.alg.as_deref() {
            Some("RS256") => Algorithm::RS256,
            Some("RS384") => Algorithm::RS384,
            Some("RS512") => Algorithm::RS512,
            Some("ES256") => Algorithm::ES256,
            Some("ES384") => Algorithm::ES384,
            Some("PS256") => Algorithm::PS256,
            Some("PS384") => Algorithm::PS384,
            Some("PS512") => Algorithm::PS512,
            _ => Algorithm::RS256, // Default to RS256
        }
    }
}

/// Cached JWKS data
struct CachedJwks {
    keys: Vec<Jwk>,
    fetched_at: DateTime<Utc>,
    ttl: Duration,
}

impl CachedJwks {
    fn is_expired(&self) -> bool {
        Utc::now() > self.fetched_at + self.ttl
    }
}

/// JWKS validator for external identity providers
pub struct JwksValidator {
    http_client: reqwest::Client,
    /// Cache keyed by config ID
    cache: DashMap<Uuid, CachedJwks>,
}

impl JwksValidator {
    /// Create a new JWKS validator
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to create HTTP client"),
            cache: DashMap::new(),
        }
    }

    /// Validate a token against a specific JWKS config
    pub async fn validate(
        &self,
        config: &JwksConfig,
        token: &str,
    ) -> Result<JwksClaims, JwksError> {
        if !config.is_active {
            return Err(JwksError::ConfigDisabled(config.id.to_string()));
        }

        // Decode header to get kid
        let header = decode_header(token).map_err(|e| JwksError::TokenDecode(e.to_string()))?;

        let kid = header.kid.as_ref().ok_or_else(|| JwksError::MissingKid)?;

        // Get or fetch JWKS
        let keys = self.get_or_fetch_keys(config).await?;

        // Find the key with matching kid
        let key = keys
            .iter()
            .find(|k| k.kid.as_ref() == Some(kid))
            .ok_or_else(|| JwksError::KeyNotFound(kid.clone()))?;

        // Prepare validation
        let mut validation = Validation::new(key.algorithm());
        validation.set_issuer(&[&config.issuer]);

        // Require an audience. Without it, jsonwebtoken would not bind the token
        // to this relying party, so a token minted for a DIFFERENT service at the
        // same issuer would be accepted. A JWKS config with no audience is a
        // misconfiguration we refuse rather than validate insecurely.
        let audience = config.audience.as_ref().ok_or_else(|| {
            JwksError::TokenValidation(
                "JWKS config has no audience; refusing to validate unbound tokens".to_string(),
            )
        })?;
        validation.set_audience(&[audience]);

        // Decode and validate
        let decoding_key = key.to_decoding_key()?;
        let token_data = decode::<JwksClaims>(token, &decoding_key, &validation)
            .map_err(|e| JwksError::TokenValidation(e.to_string()))?;

        Ok(token_data.claims)
    }

    /// Get keys from cache or fetch from endpoint
    async fn get_or_fetch_keys(&self, config: &JwksConfig) -> Result<Vec<Jwk>, JwksError> {
        // Check cache
        if let Some(cached) = self.cache.get(&config.id) {
            if !cached.is_expired() {
                return Ok(cached.keys.clone());
            }
        }

        // Fetch from endpoint
        let keys = self.fetch_keys(&config.jwks_url).await?;

        // Update cache
        self.cache.insert(
            config.id,
            CachedJwks {
                keys: keys.clone(),
                fetched_at: Utc::now(),
                ttl: Duration::seconds(config.cache_ttl_secs as i64),
            },
        );

        Ok(keys)
    }

    /// Fetch JWKS from endpoint
    async fn fetch_keys(&self, url: &str) -> Result<Vec<Jwk>, JwksError> {
        // SSRF guard: only fetch public https URLs.
        validate_jwks_url(url).await?;

        let response = self
            .http_client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| JwksError::FetchError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(JwksError::FetchError(format!(
                "JWKS endpoint returned status {}",
                response.status()
            )));
        }

        let jwks: JwksResponse = response
            .json()
            .await
            .map_err(|e| JwksError::FetchError(e.to_string()))?;

        Ok(jwks.keys)
    }

    /// Invalidate cache for a config
    pub fn invalidate_cache(&self, config_id: Uuid) {
        self.cache.remove(&config_id);
    }

    /// Clear all cached keys
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

impl Default for JwksValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Claims from a JWKS-validated token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwksClaims {
    /// Subject
    pub sub: String,
    /// Issuer
    pub iss: String,
    /// Audience (can be string or array)
    #[serde(default)]
    pub aud: serde_json::Value,
    /// Expiration
    pub exp: i64,
    /// Issued at
    pub iat: Option<i64>,
    /// Not before
    pub nbf: Option<i64>,
    /// JWT ID
    pub jti: Option<String>,
    /// Email (common in OIDC)
    pub email: Option<String>,
    /// Name (common in OIDC)
    pub name: Option<String>,
    /// Groups/roles (common in enterprise IdPs)
    #[serde(default)]
    pub groups: Vec<String>,
    /// Azure AD specific
    #[serde(default)]
    pub roles: Vec<String>,
    /// Organization ID (custom claim)
    pub org_id: Option<String>,
    /// Any additional claims
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl JwksClaims {
    /// Check if token is expired
    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() > self.exp
    }

    /// Get subject as string
    pub fn subject(&self) -> &str {
        &self.sub
    }

    /// Get email if present
    pub fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    /// Check if user has a specific group
    pub fn has_group(&self, group: &str) -> bool {
        self.groups.contains(&group.to_string())
    }

    /// Check if user has a specific role (Azure AD style)
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(&role.to_string())
    }
}

/// JWKS errors
#[derive(Debug, Error)]
pub enum JwksError {
    #[error("JWKS configuration is disabled: {0}")]
    ConfigDisabled(String),

    #[error("Failed to fetch JWKS: {0}")]
    FetchError(String),

    #[error("Failed to decode token header: {0}")]
    TokenDecode(String),

    #[error("Token validation failed: {0}")]
    TokenValidation(String),

    #[error("Token is missing 'kid' header")]
    MissingKid,

    #[error("Key not found: {0}")]
    KeyNotFound(String),

    #[error("Invalid key: {0}")]
    InvalidKey(String),

    #[error("JWKS URL is not allowed: {0}")]
    UrlNotAllowed(String),

    #[error("Database error: {0}")]
    Database(String),
}

/// Repository for JWKS configurations
pub struct JwksConfigRepository<'a> {
    db: &'a crate::db::Database,
}

impl<'a> JwksConfigRepository<'a> {
    pub fn new(db: &'a crate::db::Database) -> Self {
        Self { db }
    }

    /// Create a new JWKS configuration
    pub async fn create(
        &self,
        org_id: Uuid,
        name: &str,
        jwks_url: &str,
        issuer: &str,
        audience: Option<&str>,
    ) -> Result<JwksConfig, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let sql = r#"
            INSERT INTO jwks_configs (id, org_id, name, jwks_url, issuer, audience, is_active, cache_ttl_secs, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, 1, 3600, $7, $8)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .bind(name)
            .bind(jwks_url)
            .bind(issuer)
            .bind(audience)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| crate::db::DatabaseError::NotFound("Config not found".to_string()))
    }

    /// Get a JWKS configuration by ID
    pub async fn get_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<JwksConfig>, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, jwks_url, issuer, audience, is_active, cache_ttl_secs, created_at, updated_at
            FROM jwks_configs
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_config(&r)).transpose()
    }

    /// List active JWKS configurations for an organization
    pub async fn list_active(
        &self,
        org_id: Uuid,
    ) -> Result<Vec<JwksConfig>, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, jwks_url, issuer, audience, is_active, cache_ttl_secs, created_at, updated_at
            FROM jwks_configs
            WHERE org_id = $1 AND is_active = 1
            ORDER BY name ASC
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_config(r)).collect()
    }

    /// List all JWKS configurations for an organization
    pub async fn list_all(
        &self,
        org_id: Uuid,
    ) -> Result<Vec<JwksConfig>, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, jwks_url, issuer, audience, is_active, cache_ttl_secs, created_at, updated_at
            FROM jwks_configs
            WHERE org_id = $1
            ORDER BY name ASC
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_config(r)).collect()
    }

    /// Delete a JWKS configuration
    pub async fn delete(&self, id: Uuid) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM jwks_configs WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Toggle active status
    pub async fn set_active(
        &self,
        id: Uuid,
        is_active: bool,
    ) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let result =
            sqlx::query("UPDATE jwks_configs SET is_active = $1, updated_at = $2 WHERE id = $3")
                .bind(is_active as i32)
                .bind(now.to_rfc3339())
                .bind(id.to_string())
                .execute(pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Find active JWKS configurations by issuer URL
    ///
    /// This is used for JWKS authentication where we need to find the
    /// appropriate configuration based on the token's issuer claim.
    pub async fn find_by_issuer(
        &self,
        issuer: &str,
    ) -> Result<Vec<JwksConfig>, crate::db::DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, jwks_url, issuer, audience, is_active, cache_ttl_secs, created_at, updated_at
            FROM jwks_configs
            WHERE issuer = $1 AND is_active = 1
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql).bind(issuer).fetch_all(pool).await?;

        rows.iter().map(|r| self.row_to_config(r)).collect()
    }

    fn row_to_config(
        &self,
        row: &sqlx::any::AnyRow,
    ) -> Result<JwksConfig, crate::db::DatabaseError> {
        use sqlx::Row;

        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let is_active: i32 = row.get("is_active");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");

        Ok(JwksConfig {
            id: id
                .parse()
                .map_err(|e| crate::db::DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            org_id: org_id
                .parse()
                .map_err(|e| crate::db::DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            name: row.get("name"),
            jwks_url: row.get("jwks_url"),
            issuer: row.get("issuer"),
            audience: row.get("audience"),
            is_active: is_active != 0,
            cache_ttl_secs: row.get::<i64, _>("cache_ttl_secs") as u64,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

/// Extract the issuer (iss) claim from a JWT token without validating the signature.
///
/// This is used to determine which JWKS configuration to use for validation,
/// since the org_id is determined by the JWKS configuration which is keyed by issuer.
///
/// # Arguments
/// * `token` - The JWT token string (three base64url-encoded parts separated by dots)
///
/// # Returns
/// * `Some(String)` - The issuer claim value if successfully extracted
/// * `None` - If the token is malformed or doesn't contain an issuer claim
pub fn extract_issuer_from_token(token: &str) -> Option<String> {
    // JWT format: header.payload.signature
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    // Decode the payload (second part) using base64url
    let payload = parts[1];

    // Use the base64 crate's URL_SAFE_NO_PAD alphabet
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let payload_str = String::from_utf8(decoded).ok()?;

    // Parse as JSON and extract "iss" claim
    let claims: serde_json::Value = serde_json::from_str(&payload_str).ok()?;
    claims.get("iss").and_then(|v| v.as_str()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_issuer_from_token() {
        // Create a simple JWT with just payload (header.payload.signature format)
        // This is a base64url encoded payload: {"iss": "https://auth.example.com", "sub": "user123"}
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(r#"{"iss":"https://auth.example.com","sub":"user123"}"#);
        let signature = "test_signature";

        let token = format!("{}.{}.{}", header, payload, signature);

        let issuer = extract_issuer_from_token(&token);
        assert_eq!(issuer, Some("https://auth.example.com".to_string()));
    }

    #[test]
    fn test_extract_issuer_missing() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"user123"}"#); // No iss claim
        let signature = "test_signature";

        let token = format!("{}.{}.{}", header, payload, signature);

        let issuer = extract_issuer_from_token(&token);
        assert_eq!(issuer, None);
    }

    #[test]
    fn test_extract_issuer_invalid_token() {
        assert_eq!(extract_issuer_from_token("not.a.valid.jwt.token"), None);
        assert_eq!(extract_issuer_from_token(""), None);
        assert_eq!(extract_issuer_from_token("onlyonepart"), None);
    }

    #[test]
    fn test_jwk_rsa_algorithm() {
        let jwk = Jwk {
            kty: "RSA".to_string(),
            key_use: Some("sig".to_string()),
            kid: Some("test-key".to_string()),
            alg: Some("RS256".to_string()),
            n: None,
            e: None,
            crv: None,
            x: None,
            y: None,
            x5c: None,
        };

        assert_eq!(jwk.algorithm(), Algorithm::RS256);
    }

    #[test]
    fn test_jwk_ec_algorithm() {
        let jwk = Jwk {
            kty: "EC".to_string(),
            key_use: Some("sig".to_string()),
            kid: Some("test-key".to_string()),
            alg: Some("ES256".to_string()),
            n: None,
            e: None,
            crv: Some("P-256".to_string()),
            x: None,
            y: None,
            x5c: None,
        };

        assert_eq!(jwk.algorithm(), Algorithm::ES256);
    }

    #[test]
    fn test_claims_groups() {
        let claims = JwksClaims {
            sub: "user@example.com".to_string(),
            iss: "https://auth.example.com".to_string(),
            aud: serde_json::json!("api"),
            exp: Utc::now().timestamp() + 3600,
            iat: Some(Utc::now().timestamp()),
            nbf: None,
            jti: None,
            email: Some("user@example.com".to_string()),
            name: Some("Test User".to_string()),
            groups: vec!["admins".to_string(), "users".to_string()],
            roles: vec![],
            org_id: None,
            extra: serde_json::json!({}),
        };

        assert!(claims.has_group("admins"));
        assert!(claims.has_group("users"));
        assert!(!claims.has_group("superadmins"));
    }
}
