//! Deployment status handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::{AgentDeploymentRepository, OrganizationRepository},
    state::AppState,
};

use super::types::{AgentDeploymentResponse, DeploymentSummaryResponse};

/// Get per-agent deployment status for a rollout
#[utoipa::path(
    get,
    path = "/orgs/{org}/rollouts/{rollout_id}/deployments",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Per-agent deployment status")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_rollout_deployments(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<AgentDeploymentResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let deployments = repo
        .get_by_rollout(rollout_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(deployments.into_iter().map(Into::into).collect()))
}

/// Get deployment summary for a rollout
#[utoipa::path(
    get,
    path = "/orgs/{org}/rollouts/{rollout_id}/summary",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("rollout_id" = Uuid, Path, description = "Rollout ID")
    ),
    responses(
        (status = 200, description = "Deployment summary")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_deployment_summary(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<DeploymentSummaryResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let summary = repo
        .get_summary(rollout_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(summary.into()))
}

/// Get latest deployment for an agent
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}/deployment",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Latest agent deployment (or null if none)")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_agent_deployment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Json<Option<AgentDeploymentResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let deployment = repo
        .get_latest_for_agent(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(deployment.map(Into::into)))
}

/// Acknowledge a deployment (agent confirms receipt)
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/{agent_id}/deployment/acknowledge",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 204, description = "Deployment acknowledged"),
        (status = 404, description = "No deployment found for agent")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn acknowledge_deployment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot acknowledge deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);

    // Get the latest deployment for this agent
    let deployment = repo
        .get_latest_for_agent(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("No deployment found for agent".to_string()))?;

    repo.acknowledge(deployment.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
