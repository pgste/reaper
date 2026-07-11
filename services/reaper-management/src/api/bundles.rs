//! Bundle API endpoints
//!
//! Provides REST endpoints for managing policy bundles.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::api::error::{ApiError, ApiResult};
use crate::api::orgs::authorize_org;
use crate::audit::{ActorType, AuditEntry, ResourceType};
use crate::auth::middleware::{AuthenticatedUser, RequireAuth};
use crate::auth::scopes::Scope;
use crate::db::repositories::PromotionChangeRepository;
use crate::domain::bundle::{BundleStatus, CreateBundle, PromotionRequest, UpdateBundle};
use crate::domain::promotion::{ChangeKind, ChangeStatus, PromotionChangeRequest};
use crate::state::AppState;

/// Map an authenticated principal to the audit actor type.
fn actor_type_of(user: &AuthenticatedUser) -> ActorType {
    match user.auth_method {
        crate::auth::middleware::AuthMethod::ApiKey { .. } => ActorType::ApiKey,
        crate::auth::middleware::AuthMethod::Mtls { .. } => ActorType::Agent,
        crate::auth::middleware::AuthMethod::Jwt { .. } => ActorType::User,
    }
}

/// Best-effort audit write — a promotion decision must be recorded, but a
/// logging hiccup should not fail the operation the user already performed.
async fn audit(
    state: &AppState,
    user: &AuthenticatedUser,
    org_id: Uuid,
    action: &str,
    cr: &PromotionChangeRequest,
) {
    let entry = AuditEntry::builder(action, actor_type_of(user), user.id.clone())
        .org_id(org_id)
        .resource(ResourceType::Bundle, cr.bundle_id.to_string())
        .details(serde_json::json!({
            "change_request_id": cr.id,
            "bundle_id": cr.bundle_id,
            "bundle_version": cr.bundle_version,
            "kind": cr.kind.as_str(),
            "requester_id": cr.requester_id,
            "approver_id": cr.approver_id,
        }));
    if let Err(e) = entry.log(&state.db).await {
        tracing::error!(error = %e, action, "failed to write promotion audit record");
    }
}

/// Query parameters for listing bundles
#[derive(Debug, Deserialize)]
pub struct ListBundlesQuery {
    /// Filter by status
    pub status: Option<String>,
}

/// Request to add policies to a bundle
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddPoliciesRequest {
    pub policy_ids: Vec<Uuid>,
}

/// Request to remove policies from a bundle
#[derive(Debug, Deserialize, ToSchema)]
pub struct RemovePoliciesRequest {
    pub policy_ids: Vec<Uuid>,
}

/// Build bundle routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Bundle CRUD
        .routes(routes!(list_bundles, create_bundle))
        .routes(routes!(get_bundle, update_bundle, delete_bundle))
        // Bundle policies
        .routes(routes!(add_policies, remove_policies))
        // Bundle workflow
        .routes(routes!(compile_bundle))
        .routes(routes!(stage_bundle))
        // Governed promotion (two-person control): promote/rollback OPEN a
        // pending change request; a second distinct principal approves to
        // execute. See Plan 02 step 5.
        .routes(routes!(promote_bundle))
        .routes(routes!(rollback_bundle))
        .routes(routes!(list_change_requests))
        .routes(routes!(get_change_request))
        .routes(routes!(approve_change_request))
        .routes(routes!(reject_change_request))
        .routes(routes!(deprecate_bundle))
        // Bundle download
        .routes(routes!(download_bundle))
        // Get promoted bundle
        .routes(routes!(get_promoted_bundle))
        // Bundle diff/preview
        .routes(routes!(get_bundle_diff))
}

/// List bundles for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/bundles",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Bundles for the organization")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 201, description = "Bundle created")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/bundles/{bundle_id}",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Bundle details")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    put,
    path = "/orgs/{org}/bundles/{bundle_id}",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Bundle updated")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    delete,
    path = "/orgs/{org}/bundles/{bundle_id}",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 204, description = "Bundle deleted")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/policies",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    request_body = AddPoliciesRequest,
    responses(
        (status = 200, description = "Policies added")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    delete,
    path = "/orgs/{org}/bundles/{bundle_id}/policies",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    request_body = RemovePoliciesRequest,
    responses(
        (status = 200, description = "Policies removed")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/compile",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Bundle compiled")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/stage",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Bundle staged")
    ),
    security(("bearer_jwt" = []))
)]
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

/// Body for opening a promote/rollback change request. Notes are optional and
/// carried through to the eventual promotion for the audit trail.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct OpenChangeRequest {
    #[serde(default)]
    pub notes: Option<String>,
}

/// Is a bundle in a state where this change kind may execute?
fn transition_allowed(kind: ChangeKind, status: BundleStatus) -> bool {
    match kind {
        ChangeKind::Promote => status == BundleStatus::Staged,
        // Rollback restores a previously-live bundle (now Deprecated), or a
        // Staged one that was never promoted.
        ChangeKind::Rollback => {
            matches!(status, BundleStatus::Deprecated | BundleStatus::Staged)
        }
    }
}

/// Open a **promote** change request (or, under single-control, promote now).
///
/// Behaviour depends on `bundles.promotion_approval`:
/// - **single-control (default):** the caller with `bundle:promote` promotes
///   immediately; a change record is still written (200, the promoted bundle).
/// - **dual-control:** records a *pending* change request that a second,
///   distinct principal must approve before the bundle goes live (201, the
///   change request).
///
/// Either way the org is resolved tenant-safe and the bundle must belong to it
/// (404 otherwise), and it must be in a promotable state (400 otherwise).
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/promote",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    request_body = OpenChangeRequest,
    responses(
        (status = 200, description = "Bundle promoted (single-control)"),
        (status = 201, description = "Change request opened (dual-control)")
    ),
    security(("bearer_jwt" = []))
)]
async fn promote_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    body: Option<Json<OpenChangeRequest>>,
) -> ApiResult<Response> {
    open_change_request(state, user, org, bundle_id, body, ChangeKind::Promote).await
}

/// Open a **rollback** change request (restore a previously-good bundle), or
/// roll back now under single-control. Same authorization and governance as
/// promote; only the recorded kind and the execution path differ.
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/rollback",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    request_body = OpenChangeRequest,
    responses(
        (status = 200, description = "Bundle rolled back (single-control)"),
        (status = 201, description = "Change request opened (dual-control)")
    ),
    security(("bearer_jwt" = []))
)]
async fn rollback_bundle(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    body: Option<Json<OpenChangeRequest>>,
) -> ApiResult<Response> {
    open_change_request(state, user, org, bundle_id, body, ChangeKind::Rollback).await
}

/// Shared entry point for promote/rollback: record the change, then either
/// return it pending (dual-control) or execute it immediately (single-control).
async fn open_change_request(
    state: Arc<AppState>,
    user: AuthenticatedUser,
    org: String,
    bundle_id: Uuid,
    body: Option<Json<OpenChangeRequest>>,
    kind: ChangeKind,
) -> ApiResult<Response> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundlePromote])
        .await?
        .id;
    // Tenant guard + pin the exact artifact (checksum) at request time so the
    // change record names *which* bundle content was approved, not just an id.
    let bundle = state.bundle_service.get_scoped(org_id, bundle_id).await?;

    // Reject an impossible change before recording it, so we don't leave a
    // dangling pending request for a bundle that can't move.
    if !transition_allowed(kind, bundle.status) {
        return Err(ApiError::BadRequest(format!(
            "bundle {} cannot be {}d from {} state",
            bundle_id,
            kind.as_str(),
            bundle.status
        )));
    }

    let notes = body.and_then(|Json(b)| b.notes);
    let repo = PromotionChangeRepository::new(&state.db);
    let cr = repo
        .create(
            org_id,
            bundle_id,
            bundle.checksum.as_deref(),
            kind,
            &user.id,
            notes.as_deref(),
        )
        .await?;
    audit(&state, &user, org_id, "bundle.change_request.open", &cr).await;

    if state.config.bundles.promotion_approval.is_dual_control() {
        // Await a second, distinct principal.
        return Ok((StatusCode::CREATED, Json(cr)).into_response());
    }

    // Single-control: the requester is also the executor. Execute now and still
    // record the (self-approved) decision, so the change ledger is uniform.
    let promoted = execute_promotion(&state, org_id, &cr, &user.id).await?;
    let executed = repo.get_scoped(org_id, cr.id).await?.unwrap_or(cr);
    audit(&state, &user, org_id, "bundle.promote", &executed).await;
    Ok((StatusCode::OK, Json(promoted)).into_response())
}

/// Atomically claim a pending change request and apply it (promote or
/// rollback). Returns `Conflict` if it was already decided (lost race), or
/// `BadRequest` if the bundle drifted out of an executable state since the
/// request was opened. Shared by single-control execution and dual-control
/// approval.
async fn execute_promotion(
    state: &AppState,
    org_id: Uuid,
    cr: &PromotionChangeRequest,
    approver_id: &str,
) -> ApiResult<crate::domain::Bundle> {
    // Re-check eligibility against the current bundle state — under dual-control
    // the bundle may have moved between open and approve.
    let bundle = state
        .bundle_service
        .get_scoped(org_id, cr.bundle_id)
        .await?;
    if !transition_allowed(cr.kind, bundle.status) {
        return Err(ApiError::BadRequest(format!(
            "bundle {} cannot be {}d from {} state",
            cr.bundle_id,
            cr.kind.as_str(),
            bundle.status
        )));
    }

    // Claim it (pending→executed). rows == 0 means someone else decided it
    // between our read and here — a conflict, not a promotion.
    let repo = PromotionChangeRepository::new(&state.db);
    let claimed = repo.mark_executed(org_id, cr.id, approver_id).await?;
    if claimed == 0 {
        return Err(ApiError::Conflict(
            "change request was already decided".to_string(),
        ));
    }

    let request = PromotionRequest {
        notes: cr.notes.clone(),
        target_agents: None,
        notify_only: false,
    };
    let promoted = match cr.kind {
        ChangeKind::Promote => state.bundle_service.promote(cr.bundle_id, &request).await,
        ChangeKind::Rollback => state.bundle_service.rollback(cr.bundle_id, &request).await,
    }?;
    Ok(promoted)
}

/// List an org's promotion change requests (newest first).
#[utoipa::path(
    get,
    path = "/orgs/{org}/change-requests",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Promotion change requests")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_change_requests(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Vec<PromotionChangeRequest>>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let crs = PromotionChangeRepository::new(&state.db)
        .list(org_id)
        .await?;
    Ok(Json(crs))
}

/// Get a single change request (tenant-safe: another org's id is a 404).
#[utoipa::path(
    get,
    path = "/orgs/{org}/change-requests/{cr_id}",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("cr_id" = Uuid, Path, description = "Change request ID")
    ),
    responses(
        (status = 200, description = "Change request details")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_change_request(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cr_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<PromotionChangeRequest>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleRead])
        .await?
        .id;
    let cr = PromotionChangeRepository::new(&state.db)
        .get_scoped(org_id, cr_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("change request not found: {}", cr_id)))?;
    Ok(Json(cr))
}

/// Approve and execute a pending change request.
///
/// The heart of the two-person control:
/// - the approver needs the dedicated `bundle:approve` scope — separate from
///   `bundle:promote` so approval authority can be granted independently of the
///   authority to request a promotion (an IdP group / role for a change-approval
///   board, or a service account, holds `bundle:approve` *without*
///   `bundle:promote`);
/// - unless `bundles.allow_self_approval` is set, the approver must be a
///   **distinct** principal from the requester (self-approval is a 403);
/// - the request must still be pending (409 otherwise);
/// - the pending→executed transition is a single atomic UPDATE, so two
///   concurrent approvals can't both promote — the loser sees a 409;
/// - only after the request is claimed is the bundle actually promoted (or
///   rolled back), and the whole decision is written to the audit log.
#[utoipa::path(
    post,
    path = "/orgs/{org}/change-requests/{cr_id}/approve",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("cr_id" = Uuid, Path, description = "Change request ID")
    ),
    responses(
        (status = 200, description = "Change request approved and executed")
    ),
    security(("bearer_jwt" = []))
)]
async fn approve_change_request(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cr_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<crate::domain::Bundle>> {
    let org_id = authorize_org(&state, &user, &org, &[Scope::BundleApprove])
        .await?
        .id;
    let repo = PromotionChangeRepository::new(&state.db);
    let cr = repo
        .get_scoped(org_id, cr_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("change request not found: {}", cr_id)))?;

    if cr.status != ChangeStatus::Pending {
        return Err(ApiError::Conflict(format!(
            "change request is {}, not pending",
            cr.status.as_str()
        )));
    }
    // Four-eyes: the approver must not be the requester. Enforced on the stable
    // principal id — which is the corporate subject for SSO/JWT callers and the
    // key id for service accounts, i.e. the identity we audit. Orgs running a
    // fully-automated pipeline can opt out with `allow_self_approval`.
    if !state.config.bundles.allow_self_approval && cr.requester_id == user.id {
        return Err(ApiError::Forbidden(
            "a change request must be approved by a different principal than the requester"
                .to_string(),
        ));
    }

    let promoted = execute_promotion(&state, org_id, &cr, &user.id).await?;

    // Re-read so the audit record carries the executed status + approver.
    let executed = repo.get_scoped(org_id, cr_id).await?.unwrap_or(cr);
    audit(
        &state,
        &user,
        org_id,
        "bundle.change_request.approve",
        &executed,
    )
    .await;

    Ok(Json(promoted))
}

/// Reject a pending change request. Accepted from either an approver
/// (`bundle:approve`) declining it, or the requester (`bundle:promote`)
/// withdrawing their own — rejection is non-destructive, so either authority
/// suffices.
#[utoipa::path(
    post,
    path = "/orgs/{org}/change-requests/{cr_id}/reject",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("cr_id" = Uuid, Path, description = "Change request ID")
    ),
    responses(
        (status = 200, description = "Change request rejected")
    ),
    security(("bearer_jwt" = []))
)]
async fn reject_change_request(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, cr_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<PromotionChangeRequest>> {
    let org_id = authorize_org(
        &state,
        &user,
        &org,
        &[Scope::BundleApprove, Scope::BundlePromote],
    )
    .await?
    .id;
    let repo = PromotionChangeRepository::new(&state.db);
    let cr = repo
        .get_scoped(org_id, cr_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("change request not found: {}", cr_id)))?;

    if cr.status != ChangeStatus::Pending {
        return Err(ApiError::Conflict(format!(
            "change request is {}, not pending",
            cr.status.as_str()
        )));
    }

    let rejected = repo.mark_rejected(org_id, cr_id, &user.id).await?;
    if rejected == 0 {
        return Err(ApiError::Conflict(
            "change request was already decided".to_string(),
        ));
    }

    let updated = repo
        .get_scoped(org_id, cr_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("change request not found: {}", cr_id)))?;
    audit(
        &state,
        &user,
        org_id,
        "bundle.change_request.reject",
        &updated,
    )
    .await;
    Ok(Json(updated))
}

/// Deprecate a bundle
#[utoipa::path(
    post,
    path = "/orgs/{org}/bundles/{bundle_id}/deprecate",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Bundle deprecated")
    ),
    security(("bearer_jwt" = []))
)]
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
#[utoipa::path(
    get,
    path = "/orgs/{org}/bundles/{bundle_id}/download",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Compiled bundle artifact", content_type = "application/octet-stream")
    ),
    security(("bearer_jwt" = []))
)]
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

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            bundle_content_disposition(&bundle.name, &bundle_id),
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

    // A malformed header earlier in the chain poisons the builder; surface a
    // clean 500 instead of `.unwrap()` panicking the process (Plan 05, Step 4).
    builder
        .body(Body::from(download.data))
        .map_err(|e| ApiError::Internal(format!("failed to build bundle download response: {e}")))
}

/// Build a safe `Content-Disposition` value for a bundle download.
///
/// `bundle.name` is user-controlled and flows into a response header. A raw
/// name containing CR/LF, quotes, or other bytes invalid in a header value
/// would either inject a header (response splitting) or make `HeaderValue`
/// construction fail and poison the response builder. Per RFC 6266 we emit a
/// sanitized ASCII `filename` fallback (only `[A-Za-z0-9._-]`, everything else
/// collapsed to `_`) plus an RFC 5987 `filename*` that percent-encodes the real
/// name, so Unicode survives without ever placing a raw control byte in the
/// header. The result is always a valid header value.
fn bundle_content_disposition(name: &str, bundle_id: &Uuid) -> String {
    let raw = format!("{name}-{bundle_id}.rbb");
    let ascii_fallback: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded = urlencoding::encode(&raw);
    format!("attachment; filename=\"{ascii_fallback}\"; filename*=UTF-8''{encoded}")
}

/// Get the currently promoted bundle
#[utoipa::path(
    get,
    path = "/orgs/{org}/bundles/promoted",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Currently promoted bundle (if any)")
    ),
    security(("bearer_jwt" = []))
)]
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
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PolicyDiffInfo {
    pub id: Uuid,
    pub name: String,
    pub language: String,
    pub version: i32,
}

/// Policy change info for modified policies
#[derive(Debug, serde::Serialize, ToSchema)]
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
#[derive(Debug, serde::Serialize, ToSchema)]
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

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct BundleSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub policy_count: i32,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct DiffSummary {
    pub total_added: u32,
    pub total_removed: u32,
    pub total_changed: u32,
    pub total_unchanged: u32,
}

/// Get diff between two bundles
#[utoipa::path(
    get,
    path = "/orgs/{org}/bundles/{bundle_id}/diff",
    tag = "bundles",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("bundle_id" = Uuid, Path, description = "Bundle ID")
    ),
    responses(
        (status = 200, description = "Diff between two bundles", body = BundleDiffResponse)
    ),
    security(("bearer_jwt" = []))
)]
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

#[cfg(test)]
mod tests {
    use super::bundle_content_disposition;
    use axum::http::HeaderValue;
    use uuid::Uuid;

    #[test]
    fn content_disposition_neutralizes_header_injection() {
        let id = Uuid::nil();
        // A name that tries to split the response (CRLF) and break out of the
        // quoted filename token (embedded `"`) to smuggle another directive.
        let evil = "evil\r\nSet-Cookie: pwned=1\"; attachment";
        let value = bundle_content_disposition(evil, &id);

        // Response-splitting is dead: no raw CR/LF survives, so the value is a
        // single legal header value (`HeaderValue` rejects CR/LF outright).
        let hv = HeaderValue::from_str(&value).expect("sanitized value is a valid header");
        assert!(
            !hv.as_bytes().contains(&b'\r') && !hv.as_bytes().contains(&b'\n'),
            "no CR/LF"
        );

        // Quote-injection is dead: the embedded `"` was collapsed to `_`, so the
        // only quotes are the two we add around the ASCII fallback — the
        // attacker cannot close the token early and append a directive.
        assert_eq!(
            value.matches('"').count(),
            2,
            "no stray quote escapes token"
        );
    }

    #[test]
    fn content_disposition_preserves_unicode_via_rfc5987() {
        let id = Uuid::nil();
        let value = bundle_content_disposition("policy-café-😀", &id);

        // Valid header value.
        HeaderValue::from_str(&value).expect("valid header value");
        // ASCII fallback present; non-ASCII collapsed to `_`.
        assert!(value.contains("filename=\""));
        // RFC 5987 form preserves the real (percent-encoded) name.
        assert!(value.contains("filename*=UTF-8''"));
        let expected =
            urlencoding::encode(&format!("policy-café-😀-{}.rbb", Uuid::nil())).into_owned();
        assert!(value.contains(&expected), "percent-encoded name present");
    }

    #[test]
    fn content_disposition_plain_name_roundtrips() {
        let id = Uuid::nil();
        let value = bundle_content_disposition("my-bundle", &id);
        assert!(value.contains(&format!("filename=\"my-bundle-{}.rbb\"", Uuid::nil())));
        HeaderValue::from_str(&value).expect("valid header value");
    }
}
