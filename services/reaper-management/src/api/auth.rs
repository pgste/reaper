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
        jwt::JwtManager,
        middleware::RequireAuth,
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
