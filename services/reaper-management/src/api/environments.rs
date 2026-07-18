//! Environment API endpoints (Plan 10 Phase A).
//!
//! First-class environments layered over namespaces. Same auth + org-scope
//! pattern as `api/namespaces.rs`: `RequireAuth`, `resolve_org`, and
//! `user.org_id == org.id` (or `Admin`) on every handler.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::{EnvironmentRepository, NamespaceRepository, OrganizationRepository},
    domain::environment::{CreateEnvironment, Environment, UpdateEnvironment},
    state::AppState,
};

/// Build environment routes.
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_environments, create_environment))
        .routes(routes!(
            get_environment,
            update_environment,
            delete_environment
        ))
}

/// List environments for an organization.
#[utoipa::path(
    get,
    path = "/orgs/{org}/environments",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Max environments to return (default 200, max 500)")
    ),
    responses((status = 200, description = "Environments", body = [Environment])),
    security(("bearer_jwt" = []))
)]
async fn list_environments(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(page): Query<crate::api::pagination::LimitQuery>,
) -> ApiResult<Json<Vec<Environment>>> {
    let organization = read_authorized(&state, &user, &org).await?;
    let limit = page.cap()?;
    let envs = EnvironmentRepository::new(&state.db)
        .list_by_org(organization.id, limit)
        .await?;
    Ok(Json(envs))
}

/// Get an environment by id or name.
#[utoipa::path(
    get,
    path = "/orgs/{org}/environments/{env}",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("env" = String, Path, description = "Environment ID or name")
    ),
    responses((status = 200, description = "Environment", body = Environment)),
    security(("bearer_jwt" = []))
)]
async fn get_environment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, env)): Path<(String, String)>,
) -> ApiResult<Json<Environment>> {
    let organization = read_authorized(&state, &user, &org).await?;
    let environment = EnvironmentRepository::new(&state.db)
        .get_by_ref(organization.id, &env)
        .await?
        .ok_or_else(|| ApiError::NotFound("Environment not found".to_string()))?;
    Ok(Json(environment))
}

/// Create an environment.
#[utoipa::path(
    post,
    path = "/orgs/{org}/environments",
    tag = "environments",
    params(("org" = String, Path, description = "Organization ID or slug")),
    request_body = CreateEnvironment,
    responses((status = 201, description = "Environment created", body = Environment)),
    security(("bearer_jwt" = []))
)]
async fn create_environment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateEnvironment>,
) -> ApiResult<(StatusCode, Json<Environment>)> {
    let organization = write_authorized(&state, &user, &org).await?;

    if request.name.trim().is_empty() {
        return Err(ApiError::Validation(
            "Environment name is required".to_string(),
        ));
    }

    // The bound namespace must exist and belong to this org.
    let ns = NamespaceRepository::new(&state.db)
        .get_by_id(request.namespace_id)
        .await?;
    if ns.map(|n| n.org_id) != Some(organization.id) {
        return Err(ApiError::Validation(
            "namespace_id does not reference a namespace in this organization".to_string(),
        ));
    }

    let env_repo = EnvironmentRepository::new(&state.db);

    // Unique env name per org.
    if env_repo
        .get_by_ref(organization.id, &request.name)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "Environment '{}' already exists",
            request.name
        )));
    }
    // One environment per namespace.
    if env_repo
        .namespace_is_bound(request.namespace_id, None)
        .await?
    {
        return Err(ApiError::Conflict(
            "That namespace is already bound to another environment".to_string(),
        ));
    }

    let environment = env_repo.create(organization.id, request).await?;
    Ok((StatusCode::CREATED, Json(environment)))
}

/// Update an environment.
#[utoipa::path(
    put,
    path = "/orgs/{org}/environments/{env}",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("env" = String, Path, description = "Environment ID or name")
    ),
    request_body = UpdateEnvironment,
    responses((status = 200, description = "Environment updated", body = Environment)),
    security(("bearer_jwt" = []))
)]
async fn update_environment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, env)): Path<(String, String)>,
    Json(request): Json<UpdateEnvironment>,
) -> ApiResult<Json<Environment>> {
    let organization = write_authorized(&state, &user, &org).await?;
    let env_repo = EnvironmentRepository::new(&state.db);
    let existing = env_repo
        .get_by_ref(organization.id, &env)
        .await?
        .ok_or_else(|| ApiError::NotFound("Environment not found".to_string()))?;

    // A rename must not collide with another environment.
    if let Some(ref new_name) = request.name {
        if new_name != &existing.name {
            if let Some(other) = env_repo.get_by_ref(organization.id, new_name).await? {
                if other.id != existing.id {
                    return Err(ApiError::Conflict(format!(
                        "Environment '{new_name}' already exists"
                    )));
                }
            }
        }
    }

    let updated = env_repo
        .update(existing.id, request)
        .await?
        .ok_or_else(|| ApiError::NotFound("Environment not found".to_string()))?;
    Ok(Json(updated))
}

/// Delete an environment.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/environments/{env}",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("env" = String, Path, description = "Environment ID or name")
    ),
    responses((status = 204, description = "Environment deleted")),
    security(("bearer_jwt" = []))
)]
async fn delete_environment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, env)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let organization = write_authorized(&state, &user, &org).await?;
    let env_repo = EnvironmentRepository::new(&state.db);
    let existing = env_repo
        .get_by_ref(organization.id, &env)
        .await?
        .ok_or_else(|| ApiError::NotFound("Environment not found".to_string()))?;
    env_repo.delete(existing.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- shared auth helpers (mirror api/namespaces.rs) ------------------------

async fn read_authorized(
    state: &AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    org: &str,
) -> ApiResult<crate::domain::organization::Organization> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }
    let organization = resolve_org(&OrganizationRepository::new(&state.db), org).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access environments for other organizations".to_string(),
        ));
    }
    Ok(organization)
}

async fn write_authorized(
    state: &AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    org: &str,
) -> ApiResult<crate::domain::organization::Organization> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }
    let organization = resolve_org(&OrganizationRepository::new(&state.db), org).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot modify environments for other organizations".to_string(),
        ));
    }
    Ok(organization)
}
