//! Rollout and rollback handlers.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError, api::idempotency, api::orgs::resolve_org, auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository, deployment::DeploymentService,
    domain::deployment::StartRollout, state::AppState,
};

use super::types::{
    CancelRequest, RollbackRequest, RolloutDetailResponse, RolloutOrDryRun, RolloutRequest,
    RolloutResponse, RolloutStartResponse, RolloutsQuery,
};

/// Start a new rollout (or dry-run)
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/rollout",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID"),
        ("Idempotency-Key" = Option<String>, Header,
         description = "Optional retry-safety key: a replay within the retention \
                        window returns the original response without starting a \
                        second rollout (Plan 07 Phase D)")
    ),
    responses(
        (status = 201, description = "Rollout started"),
        (status = 200, description = "Dry-run result"),
        (status = 409, description = "Original request for this Idempotency-Key still in flight"),
        (status = 422, description = "Idempotency-Key was already used for a different request")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn start_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RolloutRequest>,
) -> Result<Response, ApiError> {
    // Propagation-triggering POST: a retried request must not start a second
    // rollout (Plan 07 Phase D).
    let fingerprint = idempotency::fingerprint(&[
        "deployments.rollout",
        &org,
        &bundle_id.to_string(),
        &request
            .strategy_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        &request
            .namespace_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        if request.dry_run { "dry" } else { "live" },
    ]);
    let scope_id = org.clone();
    let db = state.db.clone();
    idempotency::run(
        &db,
        &headers,
        "deployments.rollout",
        &scope_id,
        &fingerprint,
        || start_rollout_inner(state, user, org, bundle_id, request),
    )
    .await
}

/// The actual rollout side effect; runs at most once per idempotency key.
async fn start_rollout_inner(
    state: Arc<AppState>,
    user: crate::auth::middleware::AuthenticatedUser,
    org: String,
    bundle_id: Uuid,
    request: RolloutRequest,
) -> Result<(StatusCode, serde_json::Value), ApiError> {
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

        let body = serde_json::to_value(RolloutOrDryRun::DryRun(result.into()))
            .map_err(|e| ApiError::Internal(format!("serialize dry-run: {e}")))?;
        return Ok((StatusCode::OK, body));
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

    let body = serde_json::to_value(RolloutOrDryRun::Rollout(RolloutStartResponse {
        rollout: result.rollout.into(),
        waves: result.waves.into_iter().map(Into::into).collect(),
        target_agent_count: result.target_agents.len(),
    }))
    .map_err(|e| ApiError::Internal(format!("serialize rollout: {e}")))?;
    Ok((StatusCode::CREATED, body))
}

/// List rollouts
#[utoipa::path(
    get,
    path = "/orgs/{org}/rollouts",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "List of rollouts")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/rollouts/{rollout_id}",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Rollout details"),
        (status = 404, description = "Rollout not found")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/rollouts/{rollout_id}/approve",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Wave approved"),
        (status = 404, description = "Rollout not found")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/rollouts/{rollout_id}/cancel",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Rollout cancelled"),
        (status = 404, description = "Rollout not found")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces/{namespace}/rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses(
        (status = 201, description = "Rollback rollout started"),
        (status = 404, description = "Namespace or bundle not found")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/rollback",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 201, description = "Rollback rollout started"),
        (status = 404, description = "Bundle not found")
    ),
    security(("bearer_jwt" = []))
)]
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
