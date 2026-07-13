//! Auto-rollback configuration handlers.

use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::{OrganizationRepository, RollbackConfigRepository},
    deployment::DeploymentService,
    domain::agent_deployment::{RollbackConfig, RollbackMode, UpdateRollbackConfig},
    domain::namespace::resolve_namespace,
    state::AppState,
};

use super::types::{CheckRollbackResponse, RollbackConfigResponse, RollbackStatusResponse};

/// Get org-level auto-rollback configuration
#[utoipa::path(
    get,
    path = "/orgs/{org}/auto-rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Org-level auto-rollback configuration")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollback config for other organizations".to_string(),
        ));
    }

    let repo = RollbackConfigRepository::new(&state.db);
    let config = repo
        .get(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, None));

    Ok(Json(config.into()))
}

/// Update org-level auto-rollback configuration
#[utoipa::path(
    post,
    path = "/orgs/{org}/auto-rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Updated auto-rollback configuration"),
        (status = 400, description = "Invalid configuration value")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn update_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<UpdateRollbackConfig>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let organization =
        super::authorize_deploy(&state, &user, &org, "update rollback configuration").await?;

    let repo = RollbackConfigRepository::new(&state.db);

    // Get existing or create new
    let mut config = repo
        .get(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, None));

    // Apply updates
    if let Some(enabled) = request.is_enabled {
        config.is_enabled = enabled;
    }
    if let Some(threshold) = request.error_rate_threshold {
        if !(0.0..=100.0).contains(&threshold) {
            return Err(ApiError::BadRequest(
                "Error rate threshold must be between 0 and 100".to_string(),
            ));
        }
        config.error_rate_threshold = threshold;
    }
    if let Some(window) = request.window_seconds {
        if window == 0 {
            return Err(ApiError::BadRequest(
                "Window seconds must be greater than 0".to_string(),
            ));
        }
        config.window_seconds = window;
    }
    if let Some(min_req) = request.min_requests {
        config.min_requests = min_req;
    }
    if let Some(mode) = &request.mode {
        config.mode = mode.parse::<RollbackMode>().map_err(ApiError::BadRequest)?;
    }
    config.updated_at = chrono::Utc::now();

    repo.upsert(&config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(config.into()))
}

/// Get namespace-level auto-rollback configuration
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{namespace}/auto-rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Namespace-level auto-rollback configuration"),
        (status = 404, description = "Namespace not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_namespace_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollback config for other organizations".to_string(),
        ));
    }

    let namespace_id = resolve_namespace(&state.db, organization.id, &namespace)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    let repo = RollbackConfigRepository::new(&state.db);
    let config = repo
        .get(organization.id, Some(namespace_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, Some(namespace_id)));

    Ok(Json(config.into()))
}

/// Update namespace-level auto-rollback configuration
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{namespace}/auto-rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 200, description = "Updated namespace auto-rollback configuration"),
        (status = 400, description = "Invalid configuration value"),
        (status = 404, description = "Namespace not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn update_namespace_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Json(request): Json<UpdateRollbackConfig>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let organization =
        super::authorize_deploy(&state, &user, &org, "update rollback configuration").await?;

    let namespace_id = resolve_namespace(&state.db, organization.id, &namespace)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    let repo = RollbackConfigRepository::new(&state.db);

    // Get existing or create new
    let mut config = repo
        .get(organization.id, Some(namespace_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, Some(namespace_id)));

    // Apply updates
    if let Some(enabled) = request.is_enabled {
        config.is_enabled = enabled;
    }
    if let Some(threshold) = request.error_rate_threshold {
        if !(0.0..=100.0).contains(&threshold) {
            return Err(ApiError::BadRequest(
                "Error rate threshold must be between 0 and 100".to_string(),
            ));
        }
        config.error_rate_threshold = threshold;
    }
    if let Some(window) = request.window_seconds {
        if window == 0 {
            return Err(ApiError::BadRequest(
                "Window seconds must be greater than 0".to_string(),
            ));
        }
        config.window_seconds = window;
    }
    if let Some(min_req) = request.min_requests {
        config.min_requests = min_req;
    }
    if let Some(mode) = &request.mode {
        config.mode = mode.parse::<RollbackMode>().map_err(ApiError::BadRequest)?;
    }
    config.updated_at = chrono::Utc::now();

    repo.upsert(&config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(config.into()))
}

/// Check if auto-rollback should be triggered for a rollout
#[utoipa::path(
    post,
    path = "/orgs/{org}/rollouts/{rollout_id}/check-rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Auto-rollback trigger evaluation"),
        (status = 404, description = "Rollout not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn check_rollback_trigger(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<CheckRollbackResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot check rollback for other organizations".to_string(),
        ));
    }

    // Get the rollout
    let service = DeploymentService::new(state.db.clone());
    let rollout = service.get_rollout(rollout_id).await.map_err(|e| match e {
        crate::deployment::DeploymentError::RolloutNotFound(_) => {
            ApiError::NotFound("Rollout not found".to_string())
        }
        e => ApiError::Internal(e.to_string()),
    })?;

    // One source of truth for the trigger decision: the same evaluation the
    // autonomous rollout supervisor runs (B2). Response shape unchanged.
    let eval = service
        .evaluate_rollback_trigger(organization.id, &rollout)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CheckRollbackResponse {
        should_rollback: eval.should_rollback,
        current_error_rate: eval.current_error_rate,
        threshold: eval.threshold,
        completed_count: eval.completed_count,
        min_requests: eval.min_requests,
        reason: eval.reason,
    }))
}

/// Get the supervisor's auto-rollback view of a rollout (read-only)
#[utoipa::path(
    get,
    path = "/orgs/{org}/rollouts/{rollout_id}/rollback-status",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Auto-rollback monitoring status for the rollout"),
        (status = 404, description = "Rollout not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_rollback_status(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<RollbackStatusResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot check rollback for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollout = service.get_rollout(rollout_id).await.map_err(|e| match e {
        crate::deployment::DeploymentError::RolloutNotFound(_) => {
            ApiError::NotFound("Rollout not found".to_string())
        }
        e => ApiError::Internal(e.to_string()),
    })?;

    let eval = service
        .evaluate_rollback_trigger(organization.id, &rollout)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RollbackStatusResponse {
        monitoring: eval.enabled,
        mode: eval.mode.to_string(),
        should_rollback: eval.should_rollback,
        current_error_rate: eval.current_error_rate,
        threshold: eval.threshold,
        completed_count: eval.completed_count,
        min_requests: eval.min_requests,
        reason: eval.reason,
    }))
}
