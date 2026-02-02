//! JWKS configuration management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{
        jwks::JwksConfigRepository,
        middleware::RequireAuth,
        scopes::Scope,
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{CreateJwksConfigRequest, JwksConfigSummary, ListJwksConfigsResponse};

/// List JWKS configurations for an organization
pub async fn list_jwks_configs(
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
pub async fn create_jwks_config(
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
pub async fn get_jwks_config(
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
pub async fn delete_jwks_config(
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
pub async fn activate_jwks_config(
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
pub async fn deactivate_jwks_config(
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
