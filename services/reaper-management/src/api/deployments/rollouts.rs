//! Rollout and rollback handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError, api::orgs::resolve_org, auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository, deployment::DeploymentService,
    domain::deployment::StartRollout, state::AppState,
};

use super::types::{
    CancelRequest, RollbackRequest, RolloutDetailResponse, RolloutOrDryRun, RolloutRequest,
    RolloutResponse, RolloutStartResponse, RolloutsQuery,
};

/// Start a new rollout (or dry-run)
pub async fn start_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(request): Json<RolloutRequest>,
) -> Result<(StatusCode, Json<RolloutOrDryRun>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot start rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());

    // Handle dry-run mode
    if request.dry_run {
        let result = service
            .dry_run_rollout(
                organization.id,
                bundle_id,
                request.strategy_id,
                request.namespace_id,
            )
            .await
            .map_err(|e| match e {
                crate::deployment::DeploymentError::BundleNotFound(_) => {
                    ApiError::NotFound("Bundle not found".to_string())
                }
                crate::deployment::DeploymentError::BundleNotReady(msg) => {
                    ApiError::BadRequest(msg)
                }
                e => ApiError::Internal(e.to_string()),
            })?;

        return Ok((StatusCode::OK, Json(RolloutOrDryRun::DryRun(result.into()))));
    }

    // Actual rollout
    let input = StartRollout {
        bundle_id,
        strategy_id: request.strategy_id,
        namespace_id: request.namespace_id,
    };

    let result = service
        .start_rollout(organization.id, &input, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(_) => {
                ApiError::NotFound("Bundle not found".to_string())
            }
            crate::deployment::DeploymentError::BundleNotReady(msg) => ApiError::BadRequest(msg),
            crate::deployment::DeploymentError::ActiveRolloutExists(_) => {
                ApiError::Conflict("Active rollout already exists for this bundle".to_string())
            }
            crate::deployment::DeploymentError::NoAgentsAvailable => {
                ApiError::BadRequest("No active agents available for deployment".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutOrDryRun::Rollout(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        })),
    ))
}

/// List rollouts
pub async fn list_rollouts(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<RolloutsQuery>,
) -> Result<Json<Vec<RolloutResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollouts = service
        .list_rollouts(organization.id, query.namespace_id, query.limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(rollouts.into_iter().map(Into::into).collect()))
}

/// Get rollout details
pub async fn get_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<RolloutDetailResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let (rollout, waves) =
        service
            .get_rollout_with_waves(rollout_id)
            .await
            .map_err(|e| match e {
                crate::deployment::DeploymentError::RolloutNotFound(_) => {
                    ApiError::NotFound("Rollout not found".to_string())
                }
                e => ApiError::Internal(e.to_string()),
            })?;

    Ok(Json(RolloutDetailResponse {
        rollout: rollout.into(),
        waves: waves.into_iter().map(Into::into).collect(),
    }))
}

/// Approve next wave
pub async fn approve_wave(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<RolloutResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot approve rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollout = service
        .approve_wave(rollout_id, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            crate::deployment::DeploymentError::InvalidState(msg) => ApiError::BadRequest(msg),
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(rollout.into()))
}

/// Cancel a rollout
pub async fn cancel_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
    Json(request): Json<CancelRequest>,
) -> Result<Json<RolloutResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot cancel rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollout = service
        .cancel_rollout(rollout_id, &request.reason, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            crate::deployment::DeploymentError::InvalidState(msg) => ApiError::BadRequest(msg),
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(rollout.into()))
}

/// Rollback a namespace to previous bundle
pub async fn rollback_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Json(request): Json<RollbackRequest>,
) -> Result<(StatusCode, Json<RolloutStartResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot rollback for other organizations".to_string(),
        ));
    }

    // Resolve namespace
    let ns_repo = crate::db::repositories::NamespaceRepository::new(&state.db);
    let ns = ns_repo
        .get_by_slug(organization.id, &namespace)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    let service = DeploymentService::new(state.db.clone());
    let result = service
        .rollback(
            organization.id,
            Some(ns.id),
            request.target_bundle_id,
            &request.reason,
            &state,
        )
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(msg) => ApiError::NotFound(msg),
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        }),
    ))
}

/// Rollback entire org to previous bundle
pub async fn rollback_org(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<RollbackRequest>,
) -> Result<(StatusCode, Json<RolloutStartResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot rollback for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let result = service
        .rollback(
            organization.id,
            None,
            request.target_bundle_id,
            &request.reason,
            &state,
        )
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(msg) => ApiError::NotFound(msg),
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        }),
    ))
}
