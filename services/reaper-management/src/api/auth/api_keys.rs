//! API key and token management handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{
        api_key::{ApiKeyRepository, CreateApiKey},
        jwt::JwtManager,
        middleware::RequireAuth,
        scopes::Scope,
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{
    ApiKeyCreated, ApiKeySummary, CreateApiKeyRequest, ListApiKeysResponse, RefreshTokenRequest,
    TokenResponse,
};

/// Refresh a JWT token
#[utoipa::path(
    post,
    path = "/auth/token/refresh",
    tag = "auth",
    request_body = RefreshTokenRequest,
    responses(
        (status = 200, description = "New JWT token issued", body = TokenResponse)
    )
)]
pub async fn refresh_token(
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/api-keys",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Max to return (default 200, max 500)")
    ),
    responses(
        (status = 200, description = "List of API keys", body = ListApiKeysResponse)
    ),
    security(("bearer_jwt" = []))
)]
pub async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(page): Query<crate::api::pagination::LimitQuery>,
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

    let limit = page.cap()?;
    let api_key_repo = ApiKeyRepository::new(&state.db);
    let keys = api_key_repo.list_by_org(organization.id, limit).await?;

    let summaries: Vec<ApiKeySummary> = keys.into_iter().map(|k| k.into()).collect();

    Ok(Json(ListApiKeysResponse {
        api_keys: summaries,
    }))
}

/// Create a new API key
#[utoipa::path(
    post,
    path = "/orgs/{org}/api-keys",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn create_api_key(
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

    // Validate scopes. A key must never grant more than its creator holds —
    // otherwise an org admin could mint an `admin` (platform super-admin) key
    // and escalate to cross-tenant access. `has_permission` treats a genuine
    // platform admin as holding everything, so only real platform operators can
    // create admin-scoped keys.
    let scopes = if request.scopes.is_empty() {
        Scope::agent_defaults()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        for scope_str in &request.scopes {
            let scope = Scope::parse(scope_str)
                .ok_or_else(|| ApiError::BadRequest(format!("Invalid scope: {}", scope_str)))?;
            if !user.has_permission(scope) {
                return Err(ApiError::Forbidden(format!(
                    "Cannot grant scope '{}': it exceeds your own permissions",
                    scope_str
                )));
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/api-keys/{key_id}",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("key_id" = Uuid, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "API key details", body = ApiKeySummary)
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_api_key(
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/api-keys/{key_id}/revoke",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("key_id" = Uuid, Path, description = "API key ID")
    ),
    responses(
        (status = 204, description = "API key revoked")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn revoke_api_key(
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
#[utoipa::path(
    delete,
    path = "/orgs/{org}/api-keys/{key_id}",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("key_id" = Uuid, Path, description = "API key ID")
    ),
    responses(
        (status = 204, description = "API key deleted")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn delete_api_key(
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
