//! Version pin handlers.

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
    api::pagination::{PageQuery, Paginated},
    auth::middleware::RequireAuth,
    db::repositories::{AgentRepository, OrganizationRepository},
    deployment::DeploymentService,
    domain::deployment::CreateVersionPin,
    state::AppState,
};

/// Resource-org recheck for by-id pin mutations (round-3 SEC P1-b).
///
/// A pin is addressed by `agent_id` (a global UUID); `authorize_deploy` only
/// bound the caller to the *path* org. Without this an operator in org A could
/// pin/unpin an agent in org B by id. Returns `404` (not `403`) so a foreign
/// agent id is not an existence oracle. Recognised by the tenant-authz fitness
/// function via `.org_id != org_id`.
async fn ensure_agent_in_org(
    state: &AppState,
    agent_id: Uuid,
    org_id: Uuid,
) -> Result<(), ApiError> {
    let agent = AgentRepository::new(&state.db)
        .get_by_id(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;
    if agent.org_id != org_id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }
    Ok(())
}

use super::types::{CreatePinRequest, PinResponse};

/// Create a version pin
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/{agent_id}/pin",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 201, description = "Version pin created"),
        (status = 404, description = "Agent or bundle not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn create_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<CreatePinRequest>,
) -> Result<(StatusCode, Json<PinResponse>), ApiError> {
    let organization = super::authorize_deploy(&state, &user, &org, "create version pins").await?;
    ensure_agent_in_org(&state, agent_id, organization.id).await?;

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
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}/pin",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Version pin (or null if none)")
    ),
    security(("bearer_jwt" = []))
)]
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
    ensure_agent_in_org(&state, agent_id, organization.id).await?;

    let service = DeploymentService::new(state.db.clone());
    let pin = service
        .get_pin(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(pin.map(Into::into)))
}

/// Delete a version pin
#[utoipa::path(
    delete,
    path = "/orgs/{org}/agents/{agent_id}/pin",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 204, description = "Pin deleted"),
        (status = 404, description = "Pin not found")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn delete_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let organization = super::authorize_deploy(&state, &user, &org, "delete version pins").await?;
    ensure_agent_in_org(&state, agent_id, organization.id).await?;

    let service = DeploymentService::new(state.db.clone());
    service.delete_pin(agent_id).await.map_err(|e| match e {
        crate::deployment::DeploymentError::Database(crate::db::DatabaseError::NotFound(_)) => {
            ApiError::NotFound("Pin not found".to_string())
        }
        e => ApiError::Internal(e.to_string()),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// List version pins for an organization (keyset-paginated: round-3 Plan 06
/// §4.2, R3-02). Pins are fleet-cardinality, so an unbounded list returned the
/// whole set in one array at scale.
#[utoipa::path(
    get,
    path = "/orgs/{org}/pins",
    tag = "deployments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of version pins with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn list_pins(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Paginated<PinResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access pins for other organizations".to_string(),
        ));
    }

    let page = query.validate()?;

    let service = DeploymentService::new(state.db.clone());
    let pins = service
        .list_pins_page(organization.id, page.limit + 1, page.after.as_ref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let items: Vec<PinResponse> = pins.into_iter().map(Into::into).collect();
    Ok(Json(Paginated::from_rows(items, &page, |p| {
        (p.created_at.to_rfc3339(), p.agent_id.to_string())
    })))
}
