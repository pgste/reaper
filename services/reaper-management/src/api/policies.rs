//! Policy API endpoints
//!
//! Provides CRUD operations for policies within organizations.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::authorize_org,
    api::pagination::{PageQuery, Paginated},
    api::preconditions::{check_precondition, etag},
    auth::middleware::RequireAuth,
    auth::scopes::Scope,
    db::repositories::{PolicyRepository, PolicySourceRepository},
    domain::policy::{CreatePolicy, Policy, PolicyLanguage, PolicyVersion, UpdatePolicy},
    domain::source::{ConflictMode, PolicySource, SourceType},
    state::{AppState, ServerEvent},
    validation::{PolicyValidationResult, ValidationService},
};

/// Resolve whether a policy is backed by a git source and, if so, that
/// source's conflict mode (Plan 09 Step 9). Non-git or source-less policies
/// return `None` and follow the normal direct-write path.
async fn git_backing(
    state: &AppState,
    policy: &Policy,
) -> ApiResult<Option<(PolicySource, ConflictMode)>> {
    let Some(source_id) = policy.source_id else {
        return Ok(None);
    };
    let Some(source) = PolicySourceRepository::new(&state.db)
        .get_by_id(source_id)
        .await?
    else {
        return Ok(None);
    };
    if source.source_type != SourceType::Git {
        return Ok(None);
    }
    let mode = source
        .git_config()
        .map(|c| c.conflict_mode)
        .unwrap_or_default();
    Ok(Some((source, mode)))
}

/// The policy's strong ETag: the current version's `content_hash` (ADR-2), or
/// a version marker when a version row is missing (never the case for
/// API-created policies, which always start at version 1).
async fn policy_etag(repo: &PolicyRepository<'_>, policy_id: Uuid) -> ApiResult<(i32, String)> {
    let (version, hash) = repo.current_version_info(policy_id).await?;
    let tag = match hash {
        Some(h) => h,
        None => format!("v{version}"),
    };
    Ok((version, tag))
}

/// Build policy routes (nested under orgs)
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_policies, create_policy))
        .routes(routes!(validate_policy_content))
        .routes(routes!(get_policy, update_policy, delete_policy))
        .routes(routes!(validate_policy))
        .routes(routes!(list_versions))
        .routes(routes!(get_version))
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    /// If provided, creates a new version
    pub content: Option<String>,
}

/// Response for listing policy versions
#[derive(Debug, Serialize, ToSchema)]
pub struct ListVersionsResponse {
    pub versions: Vec<PolicyVersionSummary>,
}

/// Summary of a policy version (without full content)
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyVersionSummary {
    pub version: i32,
    pub content_hash: String,
    pub source_commit: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// List policies for an organization (keyset-paginated: Plan 07 Phase E).
#[utoipa::path(
    get,
    path = "/orgs/{org}/policies",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of policies with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_policies(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Paginated<Policy>>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;
    let page = query.validate()?;

    let policy_repo = PolicyRepository::new(&state.db);
    let rows = policy_repo
        .list_page_by_org(organization.id, page.limit + 1, page.after.as_ref())
        .await?;

    Ok(Json(Paginated::from_rows(rows, &page, |p| {
        (p.created_at.to_rfc3339(), p.id.to_string())
    })))
}

/// Create a new policy
#[utoipa::path(
    post,
    path = "/orgs/{org}/policies",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 201, description = "Policy created")
    ),
    security(("bearer_jwt" = []))
)]
async fn create_policy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreatePolicyRequest>,
) -> ApiResult<(StatusCode, Json<Policy>)> {
    // Validate policy name
    if request.name.is_empty() {
        return Err(ApiError::BadRequest("Policy name is required".to_string()));
    }

    if request.content.is_empty() {
        return Err(ApiError::BadRequest(
            "Policy content is required".to_string(),
        ));
    }

    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyWrite]).await?;

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
#[utoipa::path(
    get,
    path = "/orgs/{org}/policies/{policy}",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name")
    ),
    responses(
        (status = 200, description = "Policy details; the `ETag` response header \
            carries the current content hash for use as `If-Match` on updates")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_policy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<([(header::HeaderName, String); 1], Json<Policy>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;
    let (_, tag) = policy_etag(&policy_repo, policy.id).await?;

    Ok(([(header::ETAG, etag(&tag))], Json(policy)))
}

/// Update a policy
#[utoipa::path(
    put,
    path = "/orgs/{org}/policies/{policy}",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name")
    ),
    request_body = UpdatePolicyRequest,
    responses(
        (status = 200, description = "Policy updated; the `ETag` response header \
            carries the new content hash"),
        (status = 412, description = "If-Match did not match the current policy \
            (a concurrent writer won) — GET the policy again and retry"),
        (status = 428, description = "If-Match missing while the server enforces \
            preconditions (`server.require_if_match`)")
    ),
    security(("bearer_jwt" = []))
)]
async fn update_policy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<UpdatePolicyRequest>,
) -> ApiResult<Response> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyWrite]).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let existing = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    // Conflict model for git-backed policies (Plan 09 Step 9, ADR-3).
    if let Some((source, mode)) = git_backing(&state, &existing).await? {
        match mode {
            ConflictMode::ReadOnly => {
                return Err(ApiError::Conflict(format!(
                    "policy is managed by git source '{}' (read_only); edit it in git",
                    source.name
                )));
            }
            ConflictMode::CommitBack => {
                // The edit becomes a commit on the source repo; deployed state
                // is reconciled by the normal sync path (one lineage), so we do
                // NOT write the DB here.
                let content = request.content.ok_or_else(|| {
                    ApiError::BadRequest(
                        "commit_back requires policy `content` to commit".to_string(),
                    )
                })?;
                let rel_path = existing.source_path.clone().ok_or_else(|| {
                    ApiError::Internal("git-backed policy is missing its source_path".to_string())
                })?;
                let author_email = format!("{}@reaper", user.id);
                let sha = state
                    .sync_service
                    .git_syncer()
                    .commit_and_push(
                        &source,
                        &rel_path,
                        &content,
                        &user.id,
                        &author_email,
                        &format!("Update {} via Reaper", existing.name),
                    )
                    .await
                    .map_err(|e| ApiError::Internal(format!("commit-back failed: {e}")))?;
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "status": "committed",
                        "commit": sha,
                        "message": "Change committed to git; it will deploy on the next sync.",
                    })),
                )
                    .into_response());
            }
            ConflictMode::LastWriterWins => {
                // Discouraged escape hatch: apply directly and flag drift.
                let _ = state.event_tx.send(ServerEvent::DriftDetected {
                    source_id: source.id,
                    source_name: source.name.clone(),
                    org_id: organization.id,
                    namespace_id: None,
                    added: 0,
                    removed: 0,
                    changed: 1,
                });
            }
        }
    }

    // Optimistic concurrency (Plan 07 Phase C): fast-fail a stale If-Match
    // here; the repository's `AND current_version = $expected` is the atomic
    // arbiter for writers racing past this check.
    let (current_version, current_tag) = policy_etag(&policy_repo, existing.id).await?;
    let guarded = check_precondition(
        &headers,
        &current_tag,
        state.config.server.require_if_match,
        &format!("policy {}", existing.id),
    )?;
    let expected_version = guarded.then_some(current_version);

    let input = UpdatePolicy {
        name: request.name,
        description: request.description,
        is_active: request.is_active,
        content: request.content,
    };

    let updated = policy_repo
        .update(existing.id, input, expected_version)
        .await?
        .ok_or_else(|| ApiError::NotFound("Policy not found after update".to_string()))?;

    let (_, new_tag) = policy_etag(&policy_repo, existing.id).await?;
    Ok(([(header::ETAG, etag(&new_tag))], Json(updated)).into_response())
}

/// Delete a policy
#[utoipa::path(
    delete,
    path = "/orgs/{org}/policies/{policy}",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name")
    ),
    responses(
        (status = 204, description = "Policy deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_policy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyWrite]).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let existing = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    // A read-only git-backed policy can't be deleted through the API — the file
    // must be removed in git (Plan 09 Step 9).
    if let Some((source, ConflictMode::ReadOnly)) = git_backing(&state, &existing).await? {
        return Err(ApiError::Conflict(format!(
            "policy is managed by git source '{}' (read_only); delete it in git",
            source.name
        )));
    }

    let deleted = policy_repo.delete(existing.id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("Policy not found".to_string()))
    }
}

/// List versions of a policy
#[utoipa::path(
    get,
    path = "/orgs/{org}/policies/{policy}/versions",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name")
    ),
    responses(
        (status = 200, description = "List of policy versions", body = ListVersionsResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_versions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<Json<ListVersionsResponse>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;

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

    Ok(Json(ListVersionsResponse {
        versions: summaries,
    }))
}

/// Get a specific version of a policy
#[utoipa::path(
    get,
    path = "/orgs/{org}/policies/{policy}/versions/{version}",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name"),
        ("version" = i32, Path, description = "Policy version number")
    ),
    responses(
        (status = 200, description = "Policy version details")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_version(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref, version)): Path<(String, String, i32)>,
) -> ApiResult<Json<PolicyVersion>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let version = policy_repo
        .get_version(policy.id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Version {} not found", version)))?;

    Ok(Json(version))
}

/// Request to validate policy content (preview before saving)
#[derive(Debug, Deserialize)]
pub struct ValidatePolicyContentRequest {
    #[serde(default)]
    pub language: PolicyLanguage,
    pub content: String,
}

/// Validate an existing policy by ID
#[utoipa::path(
    post,
    path = "/orgs/{org}/policies/{policy}/validate",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("policy" = String, Path, description = "Policy ID or name")
    ),
    responses(
        (status = 200, description = "Policy validation result")
    ),
    security(("bearer_jwt" = []))
)]
async fn validate_policy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, policy_ref)): Path<(String, String)>,
) -> ApiResult<Json<PolicyValidationResult>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;

    let policy_repo = PolicyRepository::new(&state.db);
    let policy = resolve_policy(&policy_repo, organization.id, &policy_ref).await?;

    let validation_service = ValidationService::new(state.db.clone());
    let result = validation_service
        .validate_policy(policy.id, None)
        .await
        .map_err(|e| ApiError::Internal(format!("Validation error: {}", e)))?;

    Ok(Json(result))
}

/// Validate policy content before saving (preview)
#[utoipa::path(
    post,
    path = "/orgs/{org}/policies/validate",
    tag = "policies",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Policy validation result")
    ),
    security(("bearer_jwt" = []))
)]
async fn validate_policy_content(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<ValidatePolicyContentRequest>,
) -> ApiResult<Json<PolicyValidationResult>> {
    let _organization = authorize_org(&state, &user, &org, &[Scope::PolicyRead]).await?;

    if request.content.is_empty() {
        return Err(ApiError::BadRequest(
            "Policy content is required".to_string(),
        ));
    }

    let validation_service = ValidationService::new(state.db.clone());
    let result = validation_service
        .validate_content(
            Uuid::nil(), // Preview has no ID yet
            "preview",
            request.language,
            &request.content,
        )
        .map_err(|e| ApiError::Internal(format!("Validation error: {}", e)))?;

    Ok(Json(result))
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
