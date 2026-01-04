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
        .route("/orgs/{org}/bundles/{bundle_id}/compile", post(compile_bundle))
        .route("/orgs/{org}/bundles/{bundle_id}/stage", post(stage_bundle))
        .route("/orgs/{org}/bundles/{bundle_id}/promote", post(promote_bundle))
        .route("/orgs/{org}/bundles/{bundle_id}/deprecate", post(deprecate_bundle))
        // Bundle download
        .route("/orgs/{org}/bundles/{bundle_id}/download", get(download_bundle))
        // Get promoted bundle
        .route("/orgs/{org}/bundles/promoted", get(get_promoted_bundle))
}

/// List bundles for an organization
async fn list_bundles(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    Query(query): Query<ListBundlesQuery>,
) -> ApiResult<Json<Vec<crate::domain::Bundle>>> {
    let org_id = parse_org_id(&org, &state).await?;
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
    Path(org): Path<String>,
    Json(input): Json<CreateBundle>,
) -> ApiResult<(StatusCode, Json<crate::domain::Bundle>)> {
    let org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.create(org_id, &input).await?;
    Ok((StatusCode::CREATED, Json(bundle)))
}

/// Get a specific bundle
async fn get_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.get(bundle_id).await?;
    Ok(Json(bundle))
}

/// Update a bundle
async fn update_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<UpdateBundle>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;

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
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let _org_id = parse_org_id(&org, &state).await?;
    state.bundle_service.delete(bundle_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Add policies to a bundle
async fn add_policies(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<AddPoliciesRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state
        .bundle_service
        .add_policies(bundle_id, &input.policy_ids)
        .await?;
    Ok(Json(bundle))
}

/// Remove policies from a bundle
async fn remove_policies(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(input): Json<RemovePoliciesRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state
        .bundle_service
        .remove_policies(bundle_id, &input.policy_ids)
        .await?;
    Ok(Json(bundle))
}

/// Compile a bundle
async fn compile_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.compile(bundle_id).await?;
    Ok(Json(bundle))
}

/// Stage a bundle
async fn stage_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.stage(bundle_id).await?;
    Ok(Json(bundle))
}

/// Promote a bundle to production
async fn promote_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(request): Json<PromotionRequest>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.promote(bundle_id, &request).await?;
    Ok(Json(bundle))
}

/// Deprecate a bundle
async fn deprecate_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let _org_id = parse_org_id(&org, &state).await?;
    let bundle = state
        .bundle_service
        .deprecate(bundle_id, None)
        .await?;
    Ok(Json(bundle))
}

/// Download a compiled bundle
async fn download_bundle(
    State(state): State<Arc<AppState>>,
    Path((org, bundle_id)): Path<(String, Uuid)>,
) -> ApiResult<Response> {
    let _org_id = parse_org_id(&org, &state).await?;

    let bundle = state.bundle_service.get(bundle_id).await?;
    let data = state.bundle_service.download(bundle_id).await?;

    let filename = format!("{}-{}.rbb", bundle.name, bundle_id);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .header(header::CONTENT_LENGTH, data.len())
        .body(Body::from(data))
        .unwrap())
}

/// Get the currently promoted bundle
async fn get_promoted_bundle(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
) -> ApiResult<Json<Option<crate::domain::Bundle>>> {
    let org_id = parse_org_id(&org, &state).await?;
    let bundle = state.bundle_service.get_promoted(org_id).await?;
    Ok(Json(bundle))
}

/// Parse organization ID from slug or UUID
async fn parse_org_id(org: &str, state: &AppState) -> ApiResult<Uuid> {
    // Try parsing as UUID first
    if let Ok(id) = org.parse::<Uuid>() {
        return Ok(id);
    }

    // Otherwise, look up by slug
    let org_repo = crate::db::repositories::OrganizationRepository::new(&state.db);
    let organization = org_repo
        .get_by_slug(org)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("Organization not found: {}", org)))?;

    Ok(organization.id)
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
            BundleError::Compilation(e) => ApiError::BadRequest(format!("Compilation error: {}", e)),
            BundleError::Storage(e) => ApiError::Internal(format!("Storage error: {}", e)),
            BundleError::Database(e) => ApiError::from(e),
            BundleError::NoPolicies => ApiError::BadRequest("Bundle has no policies".to_string()),
            BundleError::Validation(msg) => ApiError::Validation(msg),
        }
    }
}
