//! Organization API endpoints
//!
//! Provides CRUD operations for organizations.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::idempotency,
    api::pagination::{PageQuery, Paginated},
    auth::middleware::{AuthenticatedUser, OptionalAuth, RequireAuth},
    auth::scopes::Scope,
    auth::users::{OrgRole, UserOrg, UserOrgRepository, UserRepository},
    db::repositories::OrganizationRepository,
    domain::organization::{CreateOrganization, Organization, UpdateOrganization},
    state::AppState,
};

use super::policies;
use super::teams;

/// Build organization routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_orgs, create_org))
        .routes(routes!(get_org, update_org, delete_org))
        // Nested team routes
        .merge(teams::routes())
        // Nested policy routes
        .merge(policies::routes())
}

/// Request to create an organization
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateOrgRequest {
    pub name: String,
    pub slug: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// Request to update an organization
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateOrgRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<serde_json::Value>,
}

/// List organizations (keyset-paginated: Plan 07 Phase E). Platform admins
/// see every org; everyone else sees only the org they belong to (org
/// enumeration is a tenant-isolation leak).
#[utoipa::path(
    get,
    path = "/orgs",
    tag = "orgs",
    params(
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of organizations with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_orgs(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Paginated<Organization>>> {
    let repo = OrganizationRepository::new(&state.db);
    let page = query.validate()?;

    if user.has_permission(Scope::Admin) {
        let rows = repo.list_page(page.limit + 1, page.after.as_ref()).await?;
        Ok(Json(Paginated::from_rows(rows, &page, |o| {
            (o.created_at.to_rfc3339(), o.id.to_string())
        })))
    } else {
        let own = repo.get_by_id(user.org_id).await?;
        let organizations: Vec<Organization> = own.into_iter().collect();
        Ok(Json(Paginated {
            items: organizations,
            next_cursor: None,
        }))
    }
}

/// Create a new organization.
///
/// Anonymous callers are rejected by the auth gateway in production; the
/// handler itself stays extractor-light so gateway-less deployments (and the
/// direct-router test harness) keep working. When the creator is a real user
/// (session principal), they are recorded as the org's Owner so the org is
/// immediately manageable by them — orgs created by platform automation
/// (API keys) have no user to bind.
#[utoipa::path(
    post,
    path = "/orgs",
    tag = "orgs",
    request_body = CreateOrgRequest,
    params(
        ("Idempotency-Key" = Option<String>, Header,
         description = "Optional retry-safety key: a replay within the retention \
                        window returns the original response without creating a \
                        second organization (Plan 07 Phase D)")
    ),
    responses(
        (status = 201, description = "Organization created"),
        (status = 409, description = "Original request for this Idempotency-Key still in flight"),
        (status = 422, description = "Idempotency-Key was already used for a different request")
    ),
    security(("bearer_jwt" = []))
)]
async fn create_org(
    State(state): State<Arc<AppState>>,
    OptionalAuth(caller): OptionalAuth,
    headers: HeaderMap,
    Json(request): Json<CreateOrgRequest>,
) -> ApiResult<Response> {
    // Propagation-triggering POST: retries must not double-create (Phase D).
    let fingerprint = idempotency::fingerprint(&[
        "orgs.create",
        &request.name,
        &request.slug,
        request.display_name.as_deref().unwrap_or(""),
        request.description.as_deref().unwrap_or(""),
        &request.settings.to_string(),
    ]);
    let db = state.db.clone();
    idempotency::run(&db, &headers, "orgs.create", "-", &fingerprint, || {
        create_org_inner(state, caller, request)
    })
    .await
}

/// The actual org-create side effect; runs at most once per idempotency key.
async fn create_org_inner(
    state: Arc<AppState>,
    caller: Option<AuthenticatedUser>,
    request: CreateOrgRequest,
) -> ApiResult<(StatusCode, serde_json::Value)> {
    // Validate slug format
    if !is_valid_slug(&request.slug) {
        return Err(ApiError::BadRequest(
            "Slug must be lowercase alphanumeric with hyphens".to_string(),
        ));
    }

    let repo = OrganizationRepository::new(&state.db);

    // Check if slug already exists
    if repo.get_by_slug(&request.slug).await?.is_some() {
        return Err(ApiError::Conflict(format!(
            "Organization with slug '{}' already exists",
            request.slug
        )));
    }

    let input = CreateOrganization {
        name: request.name,
        slug: request.slug,
        display_name: request.display_name,
        description: request.description,
        settings: if request.settings.is_null() {
            serde_json::json!({})
        } else {
            request.settings
        },
    };

    let org = repo.create(input).await?;

    // Owner-on-create: if the caller is a real user (their principal id is a
    // row in `users` — API-key ids never are), grant them Owner membership.
    if let Some(caller) = caller {
        if let Ok(user_id) = Uuid::parse_str(&caller.id) {
            let user_repo = UserRepository::new(&state.db);
            if let Ok(Some(_)) = user_repo.find_by_id(user_id).await {
                let membership = UserOrg {
                    id: Uuid::new_v4(),
                    user_id,
                    org_id: org.id,
                    role: OrgRole::Owner,
                    invited_by: None,
                    joined_at: chrono::Utc::now(),
                };
                if let Err(e) = UserOrgRepository::new(&state.db)
                    .add_membership(&membership)
                    .await
                {
                    // The org exists; failing the request now would leave the
                    // caller unsure whether creation happened. Surface loudly.
                    tracing::error!(
                        org_id = %org.id, user_id = %user_id, error = %e,
                        "org created but owner membership could not be recorded"
                    );
                }
            }
        }
    }

    let body = serde_json::to_value(&org)
        .map_err(|e| ApiError::Internal(format!("serialize organization: {e}")))?;
    Ok((StatusCode::CREATED, body))
}

/// Get an organization by ID or slug (any authenticated member of the org)
#[utoipa::path(
    get,
    path = "/orgs/{org}",
    tag = "orgs",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Organization details")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_org(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Organization>> {
    let org = authorize_org(&state, &user, &org, &[]).await?;

    Ok(Json(org))
}

/// Update an organization
#[utoipa::path(
    put,
    path = "/orgs/{org}",
    tag = "orgs",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = UpdateOrgRequest,
    responses(
        (status = 200, description = "Organization updated")
    ),
    security(("bearer_jwt" = []))
)]
async fn update_org(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<UpdateOrgRequest>,
) -> ApiResult<Json<Organization>> {
    let existing = authorize_org(&state, &user, &org, &[Scope::OrgWrite, Scope::OrgAdmin]).await?;

    let repo = OrganizationRepository::new(&state.db);

    let input = UpdateOrganization {
        display_name: request.display_name,
        description: request.description,
        settings: request.settings,
    };

    let updated = repo
        .update(existing.id, input)
        .await?
        .ok_or_else(|| ApiError::NotFound("Organization not found after update".to_string()))?;

    Ok(Json(updated))
}

/// Delete an organization
#[utoipa::path(
    delete,
    path = "/orgs/{org}",
    tag = "orgs",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 204, description = "Organization deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_org(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<StatusCode> {
    let existing = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;

    let repo = OrganizationRepository::new(&state.db);

    let deleted = repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Organization not found".to_string()))
    }
}

/// Authorize `user` against the organization referenced by `org_ref`.
///
/// Enforces, in order:
/// 1. the user holds at least one of `required` scopes (an empty slice means
///    any authenticated principal qualifies) — `403` otherwise;
/// 2. the org exists — `404` otherwise;
/// 3. the user belongs to that org — `403` otherwise. The platform `admin`
///    scope is the only cross-org escape hatch; org roles never confer it.
///
/// Returns the resolved organization so handlers use its real `id` instead of
/// trusting the path parameter.
pub async fn authorize_org(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
    required: &[Scope],
) -> ApiResult<Organization> {
    if !required.is_empty() && !user.has_any_permission(required) {
        let names: Vec<&str> = required.iter().map(|s| s.as_str()).collect();
        return Err(ApiError::Forbidden(format!(
            "Missing required scope (need one of: {})",
            names.join(", ")
        )));
    }

    let repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&repo, org_ref).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access resources of another organization".to_string(),
        ));
    }

    Ok(organization)
}

/// Resolve organization by ID or slug
pub async fn resolve_org(
    repo: &OrganizationRepository<'_>,
    org_ref: &str,
) -> ApiResult<Organization> {
    // Try parsing as UUID first
    if let Ok(id) = Uuid::parse_str(org_ref) {
        if let Some(org) = repo.get_by_id(id).await? {
            return Ok(org);
        }
    }

    // Try by slug
    repo.get_by_slug(org_ref)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Organization '{}' not found", org_ref)))
}

/// Validate slug format (lowercase alphanumeric with hyphens)
fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 63 {
        return false;
    }

    slug.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_slugs() {
        assert!(is_valid_slug("my-org"));
        assert!(is_valid_slug("org123"));
        assert!(is_valid_slug("a"));
        assert!(is_valid_slug("my-cool-org-2024"));
    }

    #[test]
    fn test_invalid_slugs() {
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("My-Org")); // uppercase
        assert!(!is_valid_slug("-org")); // starts with hyphen
        assert!(!is_valid_slug("org-")); // ends with hyphen
        assert!(!is_valid_slug("my_org")); // underscore
        assert!(!is_valid_slug("my org")); // space
    }
}
