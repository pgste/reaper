//! Authentication API endpoints
//!
//! Provides endpoints for API key management and token operations.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{
        api_key::{ApiKeyCreated, ApiKeyRepository, CreateApiKey},
        jwks::{JwksConfig, JwksConfigRepository},
        jwt::JwtManager,
        middleware::RequireAuth,
        mtls::{ClientCertificate, ClientCertificateRepository, RegisterCertificate},
        scopes::Scope,
        ApiKey,
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

/// Build auth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Token operations
        .route("/auth/token/refresh", post(refresh_token))
        // API key management (requires auth)
        .route(
            "/orgs/{org}/api-keys",
            get(list_api_keys).post(create_api_key),
        )
        .route(
            "/orgs/{org}/api-keys/{key_id}",
            get(get_api_key).delete(delete_api_key),
        )
        .route("/orgs/{org}/api-keys/{key_id}/revoke", post(revoke_api_key))
        // JWKS configuration management
        .route(
            "/orgs/{org}/auth/jwks",
            get(list_jwks_configs).post(create_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}",
            get(get_jwks_config).delete(delete_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}/activate",
            post(activate_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}/deactivate",
            post(deactivate_jwks_config),
        )
        // Client certificate management (mTLS)
        .route(
            "/orgs/{org}/auth/certificates",
            get(list_certificates).post(register_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}",
            get(get_certificate).delete(delete_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/revoke",
            post(revoke_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/bind",
            post(bind_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/unbind",
            post(unbind_certificate),
        )
}

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

/// Refresh a JWT token
async fn refresh_token(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RefreshTokenRequest>,
) -> ApiResult<Json<TokenResponse>> {
    let jwt_secret = state
        .config
        .auth
        .jwt_secret
        .as_ref()
        .ok_or_else(|| ApiError::Internal("JWT not configured".to_string()))?;

    let manager = JwtManager::with_secret(
        jwt_secret,
        &state.config.auth.jwt_issuer,
        &state.config.auth.jwt_audience,
        state.config.auth.jwt_expiry_hours,
    );

    let new_token = manager
        .refresh(&request.token)
        .map_err(|e| ApiError::Unauthorized(format!("Invalid token: {}", e)))?;

    let claims = manager
        .validate(&new_token)
        .map_err(|e| ApiError::Internal(format!("Token validation failed: {}", e)))?;

    let expires_at =
        chrono::DateTime::from_timestamp(claims.exp, 0).unwrap_or_else(chrono::Utc::now);

    Ok(Json(TokenResponse {
        token: new_token,
        expires_at,
    }))
}

/// List API keys for an organization
async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListApiKeysResponse>> {
    // Check permissions
    if !user.has_permission(Scope::ApiKeyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing apikey:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access API keys for other organizations".to_string(),
        ));
    }

    let api_key_repo = ApiKeyRepository::new(&state.db);
    let keys = api_key_repo.list_by_org(organization.id).await?;

    let summaries: Vec<ApiKeySummary> = keys.into_iter().map(|k| k.into()).collect();

    Ok(Json(ListApiKeysResponse {
        api_keys: summaries,
    }))
}

/// Create a new API key
async fn create_api_key(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateApiKeyRequest>,
) -> ApiResult<(StatusCode, Json<ApiKeyCreated>)> {
    // Check permissions
    if !user.has_permission(Scope::ApiKeyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing apikey:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot create API keys for other organizations".to_string(),
        ));
    }

    // Validate scopes
    let scopes = if request.scopes.is_empty() {
        Scope::agent_defaults()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        // Validate that all requested scopes are valid
        for scope in &request.scopes {
            if Scope::parse(scope).is_none() {
                return Err(ApiError::BadRequest(format!("Invalid scope: {}", scope)));
            }
        }
        request.scopes
    };

    let api_key_repo = ApiKeyRepository::new(&state.db);
    let input = CreateApiKey {
        name: request.name,
        scopes,
        expires_at: request.expires_at,
        created_by: Some(user.id),
    };

    let created = api_key_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(created)))
}

/// Get an API key by ID
async fn get_api_key(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, key_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ApiKeySummary>> {
    // Check permissions
    if !user.has_permission(Scope::ApiKeyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing apikey:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access API keys for other organizations".to_string(),
        ));
    }

    let api_key_repo = ApiKeyRepository::new(&state.db);
    let key = api_key_repo
        .get_by_id(key_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("API key not found".to_string()))?;

    // Verify key belongs to this org
    if key.org_id != organization.id {
        return Err(ApiError::NotFound("API key not found".to_string()));
    }

    Ok(Json(key.into()))
}

/// Revoke an API key
async fn revoke_api_key(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, key_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Check permissions
    if !user.has_permission(Scope::ApiKeyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing apikey:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot revoke API keys for other organizations".to_string(),
        ));
    }

    let api_key_repo = ApiKeyRepository::new(&state.db);

    // Verify key exists and belongs to this org
    let key = api_key_repo
        .get_by_id(key_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("API key not found".to_string()))?;

    if key.org_id != organization.id {
        return Err(ApiError::NotFound("API key not found".to_string()));
    }

    api_key_repo.revoke(key_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Delete an API key
async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, key_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Check permissions
    if !user.has_permission(Scope::ApiKeyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing apikey:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete API keys for other organizations".to_string(),
        ));
    }

    let api_key_repo = ApiKeyRepository::new(&state.db);

    // Verify key exists and belongs to this org
    let key = api_key_repo
        .get_by_id(key_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("API key not found".to_string()))?;

    if key.org_id != organization.id {
        return Err(ApiError::NotFound("API key not found".to_string()));
    }

    api_key_repo.delete(key_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== JWKS Configuration Endpoints ====================

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

/// List JWKS configurations for an organization
async fn list_jwks_configs(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListJwksConfigsResponse>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access JWKS configs for other organizations".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);
    let configs = jwks_repo.list_all(organization.id).await?;

    let summaries: Vec<JwksConfigSummary> = configs.into_iter().map(|c| c.into()).collect();

    Ok(Json(ListJwksConfigsResponse { configs: summaries }))
}

/// Create a new JWKS configuration
async fn create_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateJwksConfigRequest>,
) -> ApiResult<(StatusCode, Json<JwksConfigSummary>)> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot create JWKS configs for other organizations".to_string(),
        ));
    }

    // Validate URL format
    if !request.jwks_url.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "JWKS URL must use HTTPS".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);
    let config = jwks_repo
        .create(
            organization.id,
            &request.name,
            &request.jwks_url,
            &request.issuer,
            request.audience.as_deref(),
        )
        .await?;

    Ok((StatusCode::CREATED, Json(config.into())))
}

/// Get a JWKS configuration by ID
async fn get_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<JwksConfigSummary>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access JWKS configs for other organizations".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);
    let config = jwks_repo
        .get_by_id(config_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("JWKS configuration not found".to_string()))?;

    // Verify config belongs to this org
    if config.org_id != organization.id {
        return Err(ApiError::NotFound("JWKS configuration not found".to_string()));
    }

    Ok(Json(config.into()))
}

/// Delete a JWKS configuration
async fn delete_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete JWKS configs for other organizations".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);

    // Verify config exists and belongs to this org
    let config = jwks_repo
        .get_by_id(config_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("JWKS configuration not found".to_string()))?;

    if config.org_id != organization.id {
        return Err(ApiError::NotFound("JWKS configuration not found".to_string()));
    }

    // Invalidate cache if we have a validator
    if let Some(validator) = &state.jwks_validator {
        validator.invalidate_cache(config_id);
    }

    jwks_repo.delete(config_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Activate a JWKS configuration
async fn activate_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify JWKS configs for other organizations".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);

    // Verify config exists and belongs to this org
    let config = jwks_repo
        .get_by_id(config_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("JWKS configuration not found".to_string()))?;

    if config.org_id != organization.id {
        return Err(ApiError::NotFound("JWKS configuration not found".to_string()));
    }

    jwks_repo.set_active(config_id, true).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Deactivate a JWKS configuration
async fn deactivate_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify JWKS configs for other organizations".to_string(),
        ));
    }

    let jwks_repo = JwksConfigRepository::new(&state.db);

    // Verify config exists and belongs to this org
    let config = jwks_repo
        .get_by_id(config_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("JWKS configuration not found".to_string()))?;

    if config.org_id != organization.id {
        return Err(ApiError::NotFound("JWKS configuration not found".to_string()));
    }

    // Invalidate cache when deactivating
    if let Some(validator) = &state.jwks_validator {
        validator.invalidate_cache(config_id);
    }

    jwks_repo.set_active(config_id, false).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Client Certificate Endpoints ====================

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

/// List client certificates for an organization
async fn list_certificates(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListCertificatesResponse>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);
    let certs = cert_repo.list_by_org(organization.id).await?;

    let summaries: Vec<CertificateSummary> = certs.into_iter().map(|c| c.into()).collect();

    Ok(Json(ListCertificatesResponse {
        certificates: summaries,
    }))
}

/// Register a new client certificate
async fn register_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<RegisterCertificateRequest>,
) -> ApiResult<(StatusCode, Json<CertificateSummary>)> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot register certificates for other organizations".to_string(),
        ));
    }

    // Validate fingerprint format (should be hex-encoded SHA-256)
    if request.fingerprint.len() != 64 || !request.fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "Fingerprint must be a 64-character hex-encoded SHA-256 hash".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Check if fingerprint already exists
    if cert_repo.get_by_fingerprint(&request.fingerprint).await?.is_some() {
        return Err(ApiError::Conflict(
            "Certificate with this fingerprint already registered".to_string(),
        ));
    }

    let input = RegisterCertificate {
        fingerprint: request.fingerprint,
        subject: request.subject,
        issuer: request.issuer,
        not_before: request.not_before,
        not_after: request.not_after,
        agent_id: request.agent_id,
    };

    let cert = cert_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(cert.into())))
}

/// Get a client certificate by ID
async fn get_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<CertificateSummary>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    // Verify cert belongs to this org
    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    Ok(Json(cert.into()))
}

/// Delete a client certificate
async fn delete_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    cert_repo.delete(cert_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Revoke a client certificate
async fn revoke_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
    Json(request): Json<RevokeCertificateRequest>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot revoke certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    if cert.is_revoked {
        return Err(ApiError::Conflict("Certificate is already revoked".to_string()));
    }

    cert_repo.revoke(cert_id, request.reason.as_deref()).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Bind a certificate to an agent
async fn bind_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
    Json(request): Json<BindCertificateRequest>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    // Verify agent exists and belongs to this org
    let agent_repo = crate::db::repositories::AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(request.agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    cert_repo.bind_to_agent(cert_id, request.agent_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Unbind a certificate from its agent
async fn unbind_certificate(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cert_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing org:admin scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify certificates for other organizations".to_string(),
        ));
    }

    let cert_repo = ClientCertificateRepository::new(&state.db);

    // Verify cert exists and belongs to this org
    let cert = cert_repo
        .get_by_id(cert_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Certificate not found".to_string()))?;

    if cert.org_id != organization.id {
        return Err(ApiError::NotFound("Certificate not found".to_string()));
    }

    cert_repo.unbind_from_agent(cert_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
