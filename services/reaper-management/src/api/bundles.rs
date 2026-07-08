//! Bundle API endpoints
//!
//! Provides REST endpoints for managing policy bundles.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::error::{ApiError, ApiResult};
use crate::api::orgs::authorize_org;
use crate::auth::middleware::RequireAuth;
use crate::auth::scopes::Scope;
use crate::domain::bundle::{BundleStatus, CreateBundle, PromotionRequest, UpdateBundle};
use crate::state::AppState;

/// Query parameters for listing bundles
#[derive(Debug, Deserialize)]
pub struct ListBundlesQuery {
    /// Filter by status
    pub status: Option<String>,
}

/// Request to add policies to a bundle
#[derive(Debug, Deserialize)]
pub struct AddPoliciesRequest {
    pub policy_ids: Vec<Uuid>,
}

/// Request to remove policies from a bundle
#[derive(Debug, Deserialize)]
pub struct RemovePoliciesRequest {
    pub policy_ids: Vec<Uuid>,
}

/// Build bundle routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Bundle CRUD
        .route("/orgs/{org}/bundles", get(list_bundles).post(create_bundle))
        .route(
            "/orgs/{org}/bundles/{bundle_id}",
            get(get_bundle).put(update_bundle).delete(delete_bundle),
        )
        // Bundle policies
        .route(
            "/orgs/{org}/bundles/{bundle_id}/policies",
            post(add_policies).delete(remove_policies),
        )
        // Bundle workflow
        .route(
            "/orgs/{org}/bundles/{bundle_id}/compile",
            post(compile_bundle),
        )
        .route("/orgs/{org}/bundles/{bundle_id}/stage", post(stage_bundle))
        .route(
            "/orgs/{org}/bundles/{bundle_id}/promote",
            post(promote_bundle),
        )
        .route(
            "/orgs/{org}/bundles/{bundle_id}/deprecate",
            post(deprecate_bundle),
        )
        // Bundle download
        .route(
            "/orgs/{org}/bundles/{bundle_id}/download",
            get(download_bundle),
        )
        // Get promoted bundle
        .route("/orgs/{org}/bundles/promoted", get(get_promoted_bundle))
        // Bundle diff/preview
        .route("/orgs/{org}/bundles/{bundle_id}/diff", get(get_bundle_diff))
}

/// List bundles for an organization
async fn list_bundles(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<ListBundlesQuery>,
) -> ApiResult<Json<Vec<crate::domain::Bundle>>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let status_filter = query
        .status
        .as_ref()
        .map(|s| s.parse::<BundleStatus>())
        .transpose()
        .map_err(|e| ApiError::BadRequest(format!("Invalid status: {}", e)))?;

    let bundles = state.bundle_service.list(org_id, status_filter).await?;
    Ok(Json(bundles))
}

/// Create a new bundle
async fn create_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(input): Json<CreateBundle>,
) -> ApiResult<(StatusCode, Json<crate::domain::Bundle>)> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    let bundle = state.bundle_service.create(org_id, &input).await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

/// Get a specific bundle
async fn get_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let bundle = state.bundle_service.get_scoped(org_id, bundle_id).await?;
    Ok(Json(bundle))
}

/// Update a bundle
async fn update_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateBundle>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    // Tenant guard: 404 unless the bundle belongs to this org.
    state.bundle_service.get_scoped(org_id, bundle_id).await?;

    // Update bundle metadata through repository
    let bundle = crate::db::repositories::BundleRepository::new(&state.db)
        .update(
            bundle_id,
            input.name.as_deref(),
            input.description.as_deref(),
            None,
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(bundle))
}

/// Delete a bundle
async fn delete_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    state.bundle_service.delete(bundle_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Add policies to a bundle
async fn add_policies(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<AddPoliciesRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state
        .bundle_service
        .add_policies(bundle_id, &input.policy_ids)
        .await?;
    Ok(Json(bundle))
}

/// Remove policies from a bundle
async fn remove_policies(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<RemovePoliciesRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state
        .bundle_service
        .remove_policies(bundle_id, &input.policy_ids)
        .await?;
    Ok(Json(bundle))
}

/// Compile a bundle
async fn compile_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state.bundle_service.compile(bundle_id).await?;
    Ok(Json(bundle))
}

/// Stage a bundle
async fn stage_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state.bundle_service.stage(bundle_id).await?;
    Ok(Json(bundle))
}

/// Promote a bundle to production
async fn promote_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(request): Json<PromotionRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundlePromote])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state.bundle_service.promote(bundle_id, &request).await?;
    Ok(Json(bundle))
}

/// Deprecate a bundle
async fn deprecate_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleWrite])
        .await?
        .id;
    state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let bundle = state.bundle_service.deprecate(bundle_id, None).await?;
    Ok(Json(bundle))
}

/// Download a compiled bundle
async fn download_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Response> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let bundle = state.bundle_service.get_scoped(org_id, bundle_id).await?;
    let download = state.bundle_service.download(bundle_id).await?;

    let filename = format!("{}-{}.rbb", bundle.name, bundle_id);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .header(header::CONTENT_LENGTH, download.data.len());

    // Ship the detached signature so the agent can verify before hot-swap.
    if let Some(sig) = &download.signature {
        match serde_json::to_string(sig) {
            Ok(json) => {
                builder = builder.header(reaper_core::bundle_signing::SIGNATURE_HEADER, json);
            }
            Err(e) => {
                tracing::warn!(bundle_id = %bundle_id, error = %e,
                    "Failed to serialize bundle signature header");
            }
        }
    }

    Ok(builder.body(Body::from(download.data)).unwrap())
}

/// Get the currently promoted bundle
async fn get_promoted_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Option<crate::domain::Bundle>>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let bundle = state.bundle_service.get_promoted(org_id).await?;
    Ok(Json(bundle))
}

// Implement From<BundleError> for ApiError
impl From<crate::bundle::BundleError> for ApiError {
    fn from(err: crate::bundle::BundleError) -> Self {
        use crate::bundle::BundleError;
        match err {
            BundleError::NotFound(msg) => ApiError::NotFound(msg),
            BundleError::InvalidTransition(action, status) => {
                ApiError::BadRequest(format!("Cannot {} bundle in {} state", action, status))
            }
            BundleError::Compilation(e) => {
                ApiError::BadRequest(format!("Compilation error: {}", e))
            }
            BundleError::Storage(e) => ApiError::Internal(format!("Storage error: {}", e)),
            BundleError::Database(e) => ApiError::from(e),
            BundleError::NoPolicies => ApiError::BadRequest("Bundle has no policies".to_string()),
            BundleError::Validation(msg) => ApiError::Validation(msg),
            BundleError::Signing(msg) => ApiError::Internal(format!("Signing error: {}", msg)),
        }
    }
}

// ==================== Bundle Diff Endpoint ====================

/// Query parameters for bundle diff
#[derive(Debug, Deserialize)]
pub struct BundleDiffQuery {
    /// Base bundle ID to compare against (required)
    pub base: Uuid,
}

/// Policy info for diff response
#[derive(Debug, serde::Serialize)]
pub struct PolicyDiffInfo {
    pub id: Uuid,
    pub name: String,
    pub language: String,
    pub version: i32,
}

/// Policy change info for modified policies
#[derive(Debug, serde::Serialize)]
pub struct PolicyChange {
    pub id: Uuid,
    pub name: String,
    pub language: String,
    pub base_version: i32,
    pub new_version: i32,
    /// Content changed between versions
    pub content_changed: bool,
}

/// Bundle diff response
#[derive(Debug, serde::Serialize)]
pub struct BundleDiffResponse {
    /// Base bundle info
    pub base_bundle: BundleSummary,
    /// New bundle info
    pub new_bundle: BundleSummary,
    /// Policies added in new bundle
    pub policies_added: Vec<PolicyDiffInfo>,
    /// Policies removed from base bundle
    pub policies_removed: Vec<PolicyDiffInfo>,
    /// Policies that exist in both but have changed
    pub policies_changed: Vec<PolicyChange>,
    /// Policies unchanged
    pub policies_unchanged: u32,
    /// Summary counts
    pub summary: DiffSummary,
}

#[derive(Debug, serde::Serialize)]
pub struct BundleSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub policy_count: i32,
}

#[derive(Debug, serde::Serialize)]
pub struct DiffSummary {
    pub total_added: u32,
    pub total_removed: u32,
    pub total_changed: u32,
    pub total_unchanged: u32,
}

/// Get diff between two bundles
async fn get_bundle_diff(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Query(query): Query<BundleDiffQuery>,
) -> ApiResult<Json<BundleDiffResponse>> {
    use crate::db::repositories::{BundleRepository, PolicyRepository};
    use std::collections::HashMap;

    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;

    // Get both bundles — org-scoped, so neither side of the diff can address
    // another tenant's bundle by UUID.
    let bundle_repo = BundleRepository::new(&state.db);
    let policy_repo = PolicyRepository::new(&state.db);

    let base_bundle = bundle_repo
        .get_by_id_scoped(org_id, query.base)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("Base bundle not found: {}", query.base)))?;

    let new_bundle = bundle_repo
        .get_by_id_scoped(org_id, bundle_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("New bundle not found: {}", bundle_id)))?;

    // Get policies for both bundles
    let base_policies = bundle_repo
        .get_policies(query.base)
        .await
        .map_err(ApiError::from)?;
    let new_policies = bundle_repo
        .get_policies(bundle_id)
        .await
        .map_err(ApiError::from)?;

    // Build lookup maps by policy_id
    let base_map: HashMap<Uuid, &crate::domain::bundle::BundlePolicy> =
        base_policies.iter().map(|bp| (bp.policy_id, bp)).collect();
    let new_map: HashMap<Uuid, &crate::domain::bundle::BundlePolicy> =
        new_policies.iter().map(|bp| (bp.policy_id, bp)).collect();

    // Calculate diffs
    let mut policies_added = Vec::new();
    let mut policies_removed = Vec::new();
    let mut policies_changed = Vec::new();
    let mut unchanged_count = 0u32;

    // Find added and changed policies
    for (policy_id, new_bp) in &new_map {
        let policy = policy_repo
            .get_by_id(*policy_id)
            .await
            .map_err(ApiError::from)?;

        if let Some(policy) = policy {
            if let Some(base_bp) = base_map.get(policy_id) {
                // Exists in both - check if changed
                if base_bp.policy_version != new_bp.policy_version {
                    policies_changed.push(PolicyChange {
                        id: *policy_id,
                        name: policy.name,
                        language: policy.language.to_string(),
                        base_version: base_bp.policy_version,
                        new_version: new_bp.policy_version,
                        content_changed: true, // Different versions imply content change
                    });
                } else {
                    unchanged_count += 1;
                }
            } else {
                // Added in new bundle
                policies_added.push(PolicyDiffInfo {
                    id: *policy_id,
                    name: policy.name,
                    language: policy.language.to_string(),
                    version: new_bp.policy_version,
                });
            }
        }
    }

    // Find removed policies
    for (policy_id, base_bp) in &base_map {
        if !new_map.contains_key(policy_id) {
            let policy = policy_repo
                .get_by_id(*policy_id)
                .await
                .map_err(ApiError::from)?;

            if let Some(policy) = policy {
                policies_removed.push(PolicyDiffInfo {
                    id: *policy_id,
                    name: policy.name,
                    language: policy.language.to_string(),
                    version: base_bp.policy_version,
                });
            }
        }
    }

    let summary = DiffSummary {
        total_added: policies_added.len() as u32,
        total_removed: policies_removed.len() as u32,
        total_changed: policies_changed.len() as u32,
        total_unchanged: unchanged_count,
    };

    Ok(Json(BundleDiffResponse {
        base_bundle: BundleSummary {
            id: base_bundle.id,
            name: base_bundle.name,
            status: base_bundle.status.to_string(),
            policy_count: base_bundle.policy_count,
        },
        new_bundle: BundleSummary {
            id: new_bundle.id,
            name: new_bundle.name,
            status: new_bundle.status.to_string(),
            policy_count: new_bundle.policy_count,
        },
        policies_added,
        policies_removed,
        policies_changed,
        policies_unchanged: unchanged_count,
        summary,
    }))
}
