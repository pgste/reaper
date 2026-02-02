//! Deployment strategy handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository,
    deployment::DeploymentService,
    domain::deployment::CreateDeploymentStrategy,
    state::AppState,
};

use super::types::{CreateStrategyRequest, StrategiesQuery, StrategyResponse};

/// List deployment strategies
pub async fn list_strategies(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<StrategiesQuery>,
) -> Result<Json<Vec<StrategyResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let strategies = service
        .list_strategies(organization.id, query.namespace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(strategies.into_iter().map(Into::into).collect()))
}

/// Create a deployment strategy
pub async fn create_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateStrategyRequest>,
) -> Result<(StatusCode, Json<StrategyResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot create strategies for other organizations".to_string(),
        ));
    }

    let input = CreateDeploymentStrategy {
        name: request.name,
        namespace_id: request.namespace_id,
        strategy_type: request.strategy_type,
        config: request.config,
        is_default: request.is_default,
    };

    let service = DeploymentService::new(state.db.clone());
    let strategy = service
        .create_strategy(organization.id, &input)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(strategy.into())))
}

/// Get a deployment strategy
pub async fn get_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, strategy_id)): Path<(String, Uuid)>,
) -> Result<Json<StrategyResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let strategy = service
        .get_strategy(strategy_id)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::StrategyNotFound(_) => {
                ApiError::NotFound("Strategy not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    // Verify strategy belongs to this org
    if strategy.org_id != organization.id {
        return Err(ApiError::NotFound("Strategy not found".to_string()));
    }

    Ok(Json(strategy.into()))
}

/// Delete a deployment strategy
pub async fn delete_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, strategy_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot delete strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());

    // Verify strategy belongs to this org
    let strategy = service.get_strategy(strategy_id).await.map_err(|e| {
        match e {
            crate::deployment::DeploymentError::StrategyNotFound(_) => {
                ApiError::NotFound("Strategy not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        }
    })?;

    if strategy.org_id != organization.id {
        return Err(ApiError::NotFound("Strategy not found".to_string()));
    }

    service
        .delete_strategy(strategy_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
