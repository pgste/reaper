//! Organization API endpoints
//!
//! Provides CRUD operations for organizations.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    db::repositories::OrganizationRepository,
    domain::organization::{CreateOrganization, Organization, UpdateOrganization},
    state::AppState,
};

use super::teams;
use super::policies;

/// Build organization routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/orgs", get(list_orgs).post(create_org))
        .route(
            "/orgs/{org}",
            get(get_org).put(update_org).delete(delete_org),
        )
        // Nested team routes
        .merge(teams::routes())
        // Nested policy routes
        .merge(policies::routes())
}

/// Query parameters for listing organizations
#[derive(Debug, Deserialize, Default)]
pub struct ListOrgsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Response for listing organizations
#[derive(Debug, Serialize)]
pub struct ListOrgsResponse {
    pub organizations: Vec<Organization>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Request to create an organization
#[derive(Debug, Deserialize)]
pub struct CreateOrgRequest {
    pub name: String,
    pub slug: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// Request to update an organization
#[derive(Debug, Deserialize)]
pub struct UpdateOrgRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<serde_json::Value>,
}

/// List all organizations
async fn list_orgs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListOrgsQuery>,
) -> ApiResult<Json<ListOrgsResponse>> {
    let repo = OrganizationRepository::new(&state.db);

    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let organizations = repo.list(Some(limit), Some(offset)).await?;
    let total = repo.count().await?;

    Ok(Json(ListOrgsResponse {
        organizations,
        total,
        limit,
        offset,
    }))
}

/// Create a new organization
async fn create_org(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateOrgRequest>,
) -> ApiResult<(StatusCode, Json<Organization>)> {
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

    Ok((StatusCode::CREATED, Json(org)))
}

/// Get an organization by ID or slug
async fn get_org(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
) -> ApiResult<Json<Organization>> {
    let repo = OrganizationRepository::new(&state.db);

    let org = resolve_org(&repo, &org).await?;

    Ok(Json(org))
}

/// Update an organization
async fn update_org(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Json(request): Json<UpdateOrgRequest>,
) -> ApiResult<Json<Organization>> {
    let repo = OrganizationRepository::new(&state.db);

    let existing = resolve_org(&repo, &org).await?;

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
async fn delete_org(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
) -> ApiResult<StatusCode> {
    let repo = OrganizationRepository::new(&state.db);

    let existing = resolve_org(&repo, &org).await?;

    let deleted = repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Organization not found".to_string()))
    }
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

    slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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
