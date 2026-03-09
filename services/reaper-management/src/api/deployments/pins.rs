//! Version pin handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError, api::orgs::resolve_org, auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository, deployment::DeploymentService,
    domain::deployment::CreateVersionPin, state::AppState,
};

use super::types::{CreatePinRequest, PinResponse};

/// Create a version pin
pub async fn create_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<CreatePinRequest>,
) -> Result<(StatusCode, Json<PinResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot create pins for other organizations".to_string(),
        ));
    }

    let input = CreateVersionPin {
        bundle_id: request.bundle_id,
        reason: request.reason,
        expires_at: request.expires_at,
    };

    let pinned_by = Some(user.id.as_str());

    let service = DeploymentService::new(state.db.clone());
    let pin = service
        .create_pin(agent_id, &input, pinned_by)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::AgentNotFound(_) => {
                ApiError::NotFound("Agent not found".to_string())
            }
            crate::deployment::DeploymentError::BundleNotFound(_) => {
                ApiError::NotFound("Bundle not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((StatusCode::CREATED, Json(pin.into())))
}

/// Get version pin for an agent
pub async fn get_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Json<Option<PinResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let pin = service
        .get_pin(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(pin.map(Into::into)))
}

/// Delete a version pin
pub async fn delete_pin(
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
            "Cannot delete pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    service.delete_pin(agent_id).await.map_err(|e| match e {
        crate::deployment::DeploymentError::Database(crate::db::DatabaseError::NotFound(_)) => {
            ApiError::NotFound("Pin not found".to_string())
        }
        e => ApiError::Internal(e.to_string()),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// List all version pins
pub async fn list_pins(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<Vec<PinResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let pins = service
        .list_pins(organization.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(pins.into_iter().map(Into::into).collect()))
}
