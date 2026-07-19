//! JWKS configuration management handlers.

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
    auth::{jwks::JwksConfigRepository, middleware::RequireAuth, scopes::Scope},
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{CreateJwksConfigRequest, JwksConfigSummary, ListJwksConfigsResponse};

/// List JWKS configurations for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/auth/jwks",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Max to return (default 200, max 500)")
    ),
    responses(
        (status = 200, description = "List of JWKS configurations", body = ListJwksConfigsResponse)
    ),
    security(("bearer_jwt" = []))
)]
pub async fn list_jwks_configs(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(page): Query<crate::api::pagination::LimitQuery>,
) -> ApiResult<Json<ListJwksConfigsResponse>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
    let configs = jwks_repo.list_all(organization.id, page.cap()?).await?;

    let summaries: Vec<JwksConfigSummary> = configs.into_iter().map(|c| c.into()).collect();

    Ok(Json(ListJwksConfigsResponse { configs: summaries }))
}

/// Create a new JWKS configuration
#[utoipa::path(
    post,
    path = "/orgs/{org}/auth/jwks",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateJwksConfigRequest,
    responses(
        (status = 201, description = "JWKS configuration created", body = JwksConfigSummary)
    ),
    security(("bearer_jwt" = []))
)]
pub async fn create_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateJwksConfigRequest>,
) -> ApiResult<(StatusCode, Json<JwksConfigSummary>)> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
        return Err(ApiError::BadRequest("JWKS URL must use HTTPS".to_string()));
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/auth/jwks/{config_id}",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("config_id" = Uuid, Path, description = "JWKS configuration ID")
    ),
    responses(
        (status = 200, description = "JWKS configuration details", body = JwksConfigSummary)
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<JwksConfigSummary>> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
        return Err(ApiError::NotFound(
            "JWKS configuration not found".to_string(),
        ));
    }

    Ok(Json(config.into()))
}

/// Delete a JWKS configuration
#[utoipa::path(
    delete,
    path = "/orgs/{org}/auth/jwks/{config_id}",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("config_id" = Uuid, Path, description = "JWKS configuration ID")
    ),
    responses(
        (status = 204, description = "JWKS configuration deleted")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn delete_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
        return Err(ApiError::NotFound(
            "JWKS configuration not found".to_string(),
        ));
    }

    // Invalidate cache if we have a validator
    if let Some(validator) = &state.jwks_validator {
        validator.invalidate_cache(config_id);
    }

    jwks_repo.delete(config_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Activate a JWKS configuration
#[utoipa::path(
    post,
    path = "/orgs/{org}/auth/jwks/{config_id}/activate",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("config_id" = Uuid, Path, description = "JWKS configuration ID")
    ),
    responses(
        (status = 204, description = "JWKS configuration activated")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn activate_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
        return Err(ApiError::NotFound(
            "JWKS configuration not found".to_string(),
        ));
    }

    jwks_repo.set_active(config_id, true).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Deactivate a JWKS configuration
#[utoipa::path(
    post,
    path = "/orgs/{org}/auth/jwks/{config_id}/deactivate",
    tag = "auth",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("config_id" = Uuid, Path, description = "JWKS configuration ID")
    ),
    responses(
        (status = 204, description = "JWKS configuration deactivated")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn deactivate_jwks_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, config_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Require org admin permission
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden("Missing org:admin scope".to_string()));
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
        return Err(ApiError::NotFound(
            "JWKS configuration not found".to_string(),
        ));
    }

    // Invalidate cache when deactivating
    if let Some(validator) = &state.jwks_validator {
        validator.invalidate_cache(config_id);
    }

    jwks_repo.set_active(config_id, false).await?;

    Ok(StatusCode::NO_CONTENT)
}
