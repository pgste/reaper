//! Policy API endpoints
//!
//! Provides CRUD operations for policies within organizations.

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
    api::orgs::resolve_org,
    db::repositories::{OrganizationRepository, PolicyRepository},
    domain::policy::{CreatePolicy, Policy, PolicyLanguage, PolicyVersion, UpdatePolicy},
    state::AppState,
};

/// Build policy routes (nested under orgs)
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/orgs/{org}/policies", get(list_policies).post(create_policy))
        .route(
            "/orgs/{org}/policies/{policy}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        .route(
            "/orgs/{org}/policies/{policy}/versions",
            get(list_versions),
        )
        .route(
            "/orgs/{org}/policies/{policy}/versions/{version}",
            get(get_version),
        )
}

/// Query parameters for listing policies
#[derive(Debug, Deserialize, Default)]
pub struct ListPoliciesQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub team_id: Option<Uuid>,
    pub active_only: Option<bool>,
}

/// Response for listing policies
#[derive(Debug, Serialize)]
pub struct ListPoliciesResponse {
    pub policies: Vec<Policy>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Request to create a policy
#[derive(Debug, Deserialize)]
pub struct CreatePolicyRequest {
    pub name: String,
    pub description: Option<String>,
    pub team_id: Option<Uuid>,
    #[serde(default)]
    pub language: PolicyLanguage,
    pub content: String,
}

/// Request to update a policy
#[derive(Debug, Deserialize)]
pub struct UpdatePolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    /// If provided, creates a new version
    pub content: Option<String>,
}

/// Response for listing policy versions
#[derive(Debug, Serialize)]
pub struct ListVersionsResponse {
    pub versions: Vec<PolicyVersionSummary>,
}

/// Summary of a policy version (without full content)
#[derive(Debug, Serialize)]
pub struct PolicyVersionSummary {
    pub version: i32,
    pub content_hash: String,
    pub source_commit: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// List policies for an organization
async fn list_policies(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Query(query): Query<ListPoliciesQuery>,
) -> ApiResult<Json<ListPoliciesResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let policies = policy_repo
        .list_by_org(organization.id, Some(limit), Some(offset))
        .await?;
    let total = policy_repo.count_by_org(organization.id).await?;

    Ok(Json(ListPoliciesResponse {
        policies,
        total,
        limit,
        offset,
    }))
}

/// Create a new policy
async fn create_policy(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Json(request): Json<CreatePolicyRequest>,
) -> ApiResult<(StatusCode, Json<Policy>)> {
    // Validate policy name
    if request.name.is_empty() {
        return Err(ApiError::BadRequest("Policy name is required".to_string()));
    }

    if request.content.is_empty() {
        return Err(ApiError::BadRequest("Policy content is required".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);

    // Check if name already exists in this org
    if policy_repo
        .get_by_name(organization.id, &request.name)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(format!(
            "Policy with name '{}' already exists in this organization",
            request.name
        )));
    }

    let input = CreatePolicy {
        name: request.name,
        description: request.description,
        team_id: request.team_id,
        source_id: None,
        language: request.language,
        source_path: None,
        content: request.content,
    };

    let policy = policy_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(policy)))
}

/// Get a policy by ID or name
async fn get_policy(
    State(state): State<Arc<AppState>>,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<Json<Policy>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    Ok(Json(policy))
}

/// Update a policy
async fn update_policy(
    State(state): State<Arc<AppState>>,
    Path((org, policy_ref)): Path<(String, String)>,
    Json(request): Json<UpdatePolicyRequest>,
) -> ApiResult<Json<Policy>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let existing = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let input = UpdatePolicy {
        name: request.name,
        description: request.description,
        is_active: request.is_active,
        content: request.content,
    };

    let updated = policy_repo
        .update(existing.id, input)
        .await?
        .ok_or_else(|| ApiError::NotFound("Policy not found after update".to_string()))?;

    Ok(Json(updated))
}

/// Delete a policy
async fn delete_policy(
    State(state): State<Arc<AppState>>,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let existing = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let deleted = policy_repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Policy not found".to_string()))
    }
}

/// List versions of a policy
async fn list_versions(
    State(state): State<Arc<AppState>>,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<Json<ListVersionsResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let versions = policy_repo.get_versions(policy.id).await?;

    let summaries: Vec<PolicyVersionSummary> = versions
        .into_iter()
        .map(|v| PolicyVersionSummary {
            version: v.version,
            content_hash: v.content_hash,
            source_commit: v.source_commit,
            created_at: v.created_at,
        })
        .collect();

    Ok(Json(ListVersionsResponse { versions: summaries }))
}

/// Get a specific version of a policy
async fn get_version(
    State(state): State<Arc<AppState>>,
    Path((org, policy_ref, version)): Path<(String, String, i32)>,
) -> ApiResult<Json<PolicyVersion>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let version = policy_repo
        .get_version(policy.id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Version {} not found", version)))?;

    Ok(Json(version))
}

/// Resolve policy by ID or name
pub async fn resolve_policy(
    repo: &PolicyRepository<'_>,
    org_id: Uuid,
    policy_ref: &str,
) -> ApiResult<Policy> {
    // Try parsing as UUID first
    if let Ok(id) = Uuid::parse_str(policy_ref) {
        if let Some(policy) = repo.get_by_id(id).await? {
            // Verify policy belongs to the org
            if policy.org_id == org_id {
                return Ok(policy);
            }
        }
    }

    // Try by name
    repo.get_by_name(org_id, policy_ref)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Policy '{}' not found", policy_ref)))
}
