//! Team API endpoints
//!
//! Provides CRUD operations for teams within organizations.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::authorize_org,
    api::pagination::{PageQuery, Paginated},
    auth::middleware::RequireAuth,
    auth::scopes::Scope,
    db::repositories::TeamRepository,
    domain::team::{CreateTeam, Team, UpdateTeam},
    state::AppState,
};

/// Build team routes (nested under orgs)
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_teams, create_team))
        .routes(routes!(get_team, update_team, delete_team))
}

/// Request to create a team
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTeamRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

/// Request to update a team
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTeamRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// List teams for an organization (keyset-paginated: Plan 07 Phase E).
#[utoipa::path(
    get,
    path = "/orgs/{org}/teams",
    tag = "teams",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of teams with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_teams(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Paginated<Team>>> {
    // Any authenticated member of the org may read teams.
    let organization = authorize_org(&state, &user, &org, &[]).await?;
    let page = query.validate()?;

    let team_repo = TeamRepository::new(&state.db);
    let teams = team_repo
        .list_page_by_org(organization.id, page.limit + 1, page.after.as_ref())
        .await?;

    Ok(Json(Paginated::from_rows(teams, &page, |t| {
        (t.created_at.to_rfc3339(), t.id.to_string())
    })))
}

/// Create a new team
#[utoipa::path(
    post,
    path = "/orgs/{org}/teams",
    tag = "teams",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateTeamRequest,
    responses(
        (status = 201, description = "Team created")
    ),
    security(("bearer_jwt" = []))
)]
async fn create_team(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateTeamRequest>,
) -> ApiResult<(StatusCode, Json<Team>)> {
    // Validate slug format
    if !is_valid_slug(&request.slug) {
        return Err(ApiError::BadRequest(
            "Slug must be lowercase alphanumeric with hyphens".to_string(),
        ));
    }

    let organization =
        authorize_org(&state, &user, &org, &[Scope::OrgWrite, Scope::OrgAdmin]).await?;

    let team_repo = TeamRepository::new(&state.db);

    // Check if slug already exists in this org
    if team_repo
        .get_by_slug(organization.id, &request.slug)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "Team with slug '{}' already exists in this organization",
            request.slug
        )));
    }

    let input = CreateTeam {
        name: request.name,
        slug: request.slug,
        description: request.description,
    };

    let team = team_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(team)))
}

/// Get a team by ID or slug
#[utoipa::path(
    get,
    path = "/orgs/{org}/teams/{team}",
    tag = "teams",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("team" = String, Path, description = "Team ID or slug")
    ),
    responses(
        (status = 200, description = "Team details")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_team(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, team_ref)): Path<(String, String)>,
) -> ApiResult<Json<Team>> {
    let organization = authorize_org(&state, &user, &org, &[]).await?;

    let team_repo = TeamRepository::new(&state.db);
    let team = resolve_team(&team_repo, organization.id, &team_ref).await?;

    Ok(Json(team))
}

/// Update a team
#[utoipa::path(
    put,
    path = "/orgs/{org}/teams/{team}",
    tag = "teams",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("team" = String, Path, description = "Team ID or slug")
    ),
    request_body = UpdateTeamRequest,
    responses(
        (status = 200, description = "Team updated")
    ),
    security(("bearer_jwt" = []))
)]
async fn update_team(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, team_ref)): Path<(String, String)>,
    Json(request): Json<UpdateTeamRequest>,
) -> ApiResult<Json<Team>> {
    let organization =
        authorize_org(&state, &user, &org, &[Scope::OrgWrite, Scope::OrgAdmin]).await?;

    let team_repo = TeamRepository::new(&state.db);
    let existing = resolve_team(&team_repo, organization.id, &team_ref).await?;

    let input = UpdateTeam {
        name: request.name,
        description: request.description,
    };

    let updated = team_repo
        .update(existing.id, input)
        .await?
        .ok_or_else(|| ApiError::NotFound("Team not found after update".to_string()))?;

    Ok(Json(updated))
}

/// Delete a team
#[utoipa::path(
    delete,
    path = "/orgs/{org}/teams/{team}",
    tag = "teams",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("team" = String, Path, description = "Team ID or slug")
    ),
    responses(
        (status = 204, description = "Team deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_team(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, team_ref)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let organization =
        authorize_org(&state, &user, &org, &[Scope::OrgWrite, Scope::OrgAdmin]).await?;

    let team_repo = TeamRepository::new(&state.db);
    let existing = resolve_team(&team_repo, organization.id, &team_ref).await?;

    let deleted = team_repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Team not found".to_string()))
    }
}

/// Resolve team by ID or slug
pub async fn resolve_team(
    repo: &TeamRepository<'_>,
    org_id: Uuid,
    team_ref: &str,
) -> ApiResult<Team> {
    // Try parsing as UUID first
    if let Ok(id) = Uuid::parse_str(team_ref) {
        if let Some(team) = repo.get_by_id(id).await? {
            // Verify team belongs to the org
            if team.org_id == org_id {
                return Ok(team);
            }
        }
    }

    // Try by slug
    repo.get_by_slug(org_id, team_ref)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Team '{}' not found", team_ref)))
}

/// Validate slug format
fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 63 {
        return false;
    }

    slug.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}
