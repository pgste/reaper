//! Change-request (env→env promotion) API endpoints (Plan 10 Phase B).
//!
//! - `POST /orgs/{org}/environments/{env}/promote` creates a **pending**
//!   change request (bundle + source data version pinned) — it NEVER applies
//!   inline, whatever the approval policy, so promotion is always an explicit
//!   two-step act and a single call cannot accidentally deploy to prod.
//! - `POST /orgs/{org}/promotions/{id}/approve|reject` records an approver
//!   decision; once the target env's approval policy is satisfied the request
//!   applies via the **existing** rollout machinery. Under the default
//!   zero-approver policy the requester's own approve suffices (self-service
//!   confirmation); stricter envs demand N distinct approvers.
//! - `GET /orgs/{org}/promotions[/{id}]` is the auditable change-record trail.
//!
//! (The `/promotions` path is distinct from Plan 02's `/change-requests`, which
//! governs bundle-status promotion — a separate mechanism.)
//!
//! Same auth + org-scope pattern as the rest of the control plane.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    api::pagination::{PageQuery, Paginated},
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::{
        ChangeRequestRepository, DatastoreRepository, EnvironmentRepository, OrganizationRepository,
    },
    deployment::service::DeploymentService,
    domain::change_request::{
        ApprovalDecision, ChangeApproval, ChangeRequest, ChangeRequestStatus, CreateChangeRequest,
    },
    domain::deployment::StartRollout,
    domain::environment::{ApprovalOutcome, Environment, ExternalChangeRecordMode, WindowDecision},
    integrations::{ChangeRecordCheck, ServiceNowClient},
    state::AppState,
};

pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(promote))
        .routes(routes!(list_promotions))
        .routes(routes!(get_promotion))
        .routes(routes!(approve_promotion))
        .routes(routes!(reject_promotion))
}

/// Promotion request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PromoteRequest {
    /// Bundle to promote.
    pub bundle_id: Uuid,
    /// The source environment (id or name) the bundle is promoted FROM.
    pub from_env: String,
    /// Optional rollout strategy on apply (else the namespace/org default).
    #[serde(default)]
    pub strategy_id: Option<Uuid>,
    /// External ITSM change-record reference (e.g. a ServiceNow CHG number).
    /// Required — and, in `validated` mode, checked live against the
    /// configured ServiceNow instance — when the target environment's
    /// approval policy sets `external_change_record`; stored opaquely
    /// otherwise.
    #[serde(default)]
    pub change_ref: Option<String>,
}

/// A change request together with its recorded approvals.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChangeRequestDetail {
    #[serde(flatten)]
    pub request: ChangeRequest,
    pub approvals: Vec<ChangeApproval>,
}

/// Decision body for approve/reject.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DecisionRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Promote a bundle into `{env}` (the target) from `from_env`.
#[utoipa::path(
    post,
    path = "/orgs/{org}/environments/{env}/promote",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("env" = String, Path, description = "Target environment ID or name")
    ),
    request_body = PromoteRequest,
    responses(
        (status = 201, description = "Pending change request created", body = ChangeRequestDetail),
        (status = 400, description = "Downward/same-tier promotion"),
        (status = 409, description = "Blocked by a freeze window")
    ),
    security(("bearer_jwt" = []))
)]
async fn promote(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, env)): Path<(String, String)>,
    Json(request): Json<PromoteRequest>,
) -> ApiResult<(StatusCode, Json<ChangeRequestDetail>)> {
    let organization = authorize(&state, &user, &org, Scope::PolicyWrite).await?;
    let env_repo = EnvironmentRepository::new(&state.db);

    let to_env = env_repo
        .get_by_ref(organization.id, &env)
        .await?
        .ok_or_else(|| ApiError::NotFound("Target environment not found".to_string()))?;
    let from_env = env_repo
        .get_by_ref(organization.id, &request.from_env)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source environment not found".to_string()))?;

    // Upward-only (dev < staging < prod).
    if !from_env.can_promote_to(&to_env) {
        return Err(ApiError::BadRequest(format!(
            "cannot promote from '{}' (tier {}) to '{}' (tier {}); promotion must be to a higher tier",
            from_env.name, from_env.tier_order, to_env.name, to_env.tier_order
        )));
    }

    // Change window on the TARGET environment.
    if let WindowDecision::InFreeze { reason } = to_env.change_windows.is_change_allowed(now()) {
        return Err(ApiError::Conflict(format!(
            "environment '{}' is in a freeze window{}",
            to_env.name,
            reason.map(|r| format!(": {r}")).unwrap_or_default()
        )));
    }

    // Bundle must exist in this org.
    let bundle = crate::db::repositories::BundleRepository::new(&state.db)
        .get_by_id(request.bundle_id)
        .await?;
    if bundle.map(|b| b.org_id) != Some(organization.id) {
        return Err(ApiError::NotFound("Bundle not found".to_string()));
    }

    // External change record (e.g. ServiceNow), per the target env's policy:
    // required in `reference`/`validated` mode, live-checked in `validated`.
    let change_ref = request
        .change_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    check_external_change_record(&state, &to_env, change_ref.as_deref()).await?;

    // Pin the source environment's current data-plane version so policy + data
    // promote together (applied to the target in Phase C).
    let data_version = DatastoreRepository::new(&state.db)
        .get(organization.id, from_env.namespace_id)
        .await?
        .map(|d| d.current_version);

    let cr_repo = ChangeRequestRepository::new(&state.db);
    let cr = cr_repo
        .create(
            organization.id,
            CreateChangeRequest {
                from_env_id: from_env.id,
                to_env_id: to_env.id,
                bundle_id: request.bundle_id,
                data_version,
                strategy_id: request.strategy_id,
                requested_by: user.id.clone(),
                external_change_ref: change_ref,
            },
        )
        .await?;

    audit(
        &state,
        &user,
        actions::CHANGE_REQUEST_CREATE,
        cr.id,
        &organization,
    )
    .await;
    audit(
        &state,
        &user,
        actions::ENV_PROMOTE,
        to_env.id,
        &organization,
    )
    .await;

    // Deliberately NO inline apply, whatever the approval policy: a promotion
    // is always a two-step act (create the change record, then explicitly
    // approve it), so a single mistyped call can never land in prod. Under
    // the default zero-approver policy the requester's own approve applies it
    // — self-service, but never accidental.

    let approvals = cr_repo.list_approvals(cr.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(ChangeRequestDetail {
            request: cr,
            approvals,
        }),
    ))
}

/// Approve a change request.
#[utoipa::path(
    post,
    path = "/orgs/{org}/promotions/{id}/approve",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("id" = Uuid, Path, description = "Change request ID")
    ),
    request_body = DecisionRequest,
    responses((status = 200, description = "Approval recorded", body = ChangeRequestDetail)),
    security(("bearer_jwt" = []))
)]
async fn approve_promotion(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
    Json(body): Json<DecisionRequest>,
) -> ApiResult<Json<ChangeRequestDetail>> {
    let organization = authorize(&state, &user, &org, Scope::PolicyWrite).await?;
    let cr_repo = ChangeRequestRepository::new(&state.db);
    let cr = load_scoped(&cr_repo, organization.id, id).await?;

    if cr.status != ChangeRequestStatus::Pending {
        return Err(ApiError::Conflict(format!(
            "change request is {}, not pending",
            cr.status.as_str()
        )));
    }

    let to_env = EnvironmentRepository::new(&state.db)
        .get_by_id(cr.to_env_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Target environment not found".to_string()))?;

    // The approver must hold every scope the target env's policy requires.
    for scope in &to_env.approval_policy.required_scopes {
        if !user.has_permission(*scope) {
            return Err(ApiError::Forbidden(format!(
                "approval requires the '{}' scope",
                scope.as_str()
            )));
        }
    }

    cr_repo
        .record_decision(
            cr.id,
            &user.id,
            ApprovalDecision::Approve,
            body.reason.as_deref(),
        )
        .await?;
    audit(
        &state,
        &user,
        actions::CHANGE_REQUEST_APPROVE,
        cr.id,
        &organization,
    )
    .await;

    let cr = maybe_apply(&state, &organization.id, cr, &to_env).await?;
    let approvals = cr_repo.list_approvals(cr.id).await?;
    Ok(Json(ChangeRequestDetail {
        request: cr,
        approvals,
    }))
}

/// Reject a change request.
#[utoipa::path(
    post,
    path = "/orgs/{org}/promotions/{id}/reject",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("id" = Uuid, Path, description = "Change request ID")
    ),
    request_body = DecisionRequest,
    responses((status = 200, description = "Rejected", body = ChangeRequestDetail)),
    security(("bearer_jwt" = []))
)]
async fn reject_promotion(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
    Json(body): Json<DecisionRequest>,
) -> ApiResult<Json<ChangeRequestDetail>> {
    let organization = authorize(&state, &user, &org, Scope::PolicyWrite).await?;
    let cr_repo = ChangeRequestRepository::new(&state.db);
    let cr = load_scoped(&cr_repo, organization.id, id).await?;

    if cr.status != ChangeRequestStatus::Pending {
        return Err(ApiError::Conflict(format!(
            "change request is {}, not pending",
            cr.status.as_str()
        )));
    }

    cr_repo
        .record_decision(
            cr.id,
            &user.id,
            ApprovalDecision::Reject,
            body.reason.as_deref(),
        )
        .await?;
    cr_repo
        .set_status(
            cr.id,
            ChangeRequestStatus::Rejected,
            None,
            body.reason.as_deref(),
        )
        .await?;
    audit(
        &state,
        &user,
        actions::CHANGE_REQUEST_REJECT,
        cr.id,
        &organization,
    )
    .await;

    let cr = load_scoped(&cr_repo, organization.id, id).await?;
    let approvals = cr_repo.list_approvals(cr.id).await?;
    Ok(Json(ChangeRequestDetail {
        request: cr,
        approvals,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// List change requests (the auditable change-record trail), keyset-paginated
/// (Plan 07 pattern).
#[utoipa::path(
    get,
    path = "/orgs/{org}/promotions",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses((status = 200, description = "One page of change requests with a next_cursor to resume")),
    security(("bearer_jwt" = []))
)]
async fn list_promotions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Paginated<ChangeRequest>>> {
    let organization = authorize(&state, &user, &org, Scope::PolicyRead).await?;
    let status = query.status.as_deref().map(ChangeRequestStatus::parse);
    let page = PageQuery {
        limit: query.limit,
        cursor: query.cursor,
    }
    .validate()?;

    let rows = ChangeRequestRepository::new(&state.db)
        .list_page_by_org(organization.id, status, page.limit + 1, page.after.as_ref())
        .await?;

    Ok(Json(Paginated::from_rows(rows, &page, |cr| {
        (cr.created_at.to_rfc3339(), cr.id.to_string())
    })))
}

/// Get a change request with its approvals.
#[utoipa::path(
    get,
    path = "/orgs/{org}/promotions/{id}",
    tag = "environments",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("id" = Uuid, Path, description = "Change request ID")
    ),
    responses((status = 200, description = "Change request detail", body = ChangeRequestDetail)),
    security(("bearer_jwt" = []))
)]
async fn get_promotion(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ChangeRequestDetail>> {
    let organization = authorize(&state, &user, &org, Scope::PolicyRead).await?;
    let cr_repo = ChangeRequestRepository::new(&state.db);
    let cr = load_scoped(&cr_repo, organization.id, id).await?;
    let approvals = cr_repo.list_approvals(cr.id).await?;
    Ok(Json(ChangeRequestDetail {
        request: cr,
        approvals,
    }))
}

// --- shared helpers --------------------------------------------------------

/// If the target env's approval policy is satisfied by the recorded approvals,
/// start the rollout and mark the change request `applied`. Otherwise leave it
/// pending. Idempotent — a request already applied/decided is returned as-is.
async fn maybe_apply(
    state: &AppState,
    org_id: &Uuid,
    cr: ChangeRequest,
    to_env: &Environment,
) -> ApiResult<ChangeRequest> {
    if cr.status != ChangeRequestStatus::Pending {
        return Ok(cr);
    }
    let cr_repo = ChangeRequestRepository::new(&state.db);

    // Count distinct approvers who voted "approve".
    let approver_ids: Vec<Uuid> = cr_repo
        .list_approvals(cr.id)
        .await?
        .into_iter()
        .filter(|a| a.decision == ApprovalDecision::Approve)
        .filter_map(|a| Uuid::parse_str(&a.approver_id).ok())
        .collect();
    let requester = Uuid::parse_str(&cr.requested_by).unwrap_or_default();

    if matches!(
        to_env.approval_policy.evaluate(requester, &approver_ids),
        ApprovalOutcome::Pending { .. }
    ) {
        // Not yet enough distinct approvers — leave the request pending.
        return Ok(cr);
    }

    // Re-check the freeze window at APPLY time, not just at request time — a
    // request opened before a freeze must not slip through by being approved
    // mid-freeze. The approval itself is recorded; the request stays pending
    // and a re-approve after the freeze applies it.
    if let WindowDecision::InFreeze { reason } = to_env.change_windows.is_change_allowed(now()) {
        return Err(ApiError::Conflict(format!(
            "environment '{}' is in a freeze window{}; the change request remains pending — \
             approve again once the freeze lifts",
            to_env.name,
            reason.map(|r| format!(": {r}")).unwrap_or_default()
        )));
    }

    // Re-check the external change record at APPLY time in `validated` mode —
    // the record could have been rejected (or the policy tightened) between
    // request and approval. Fails closed: the request stays pending.
    check_external_change_record(state, to_env, cr.external_change_ref.as_deref()).await?;

    // Satisfied. FIRST move the pinned data version into the target env's
    // data plane (Plan 10 Step 7) — policy and data promote together, and a
    // promotion that cannot resolve its data version fails CLOSED here (the
    // request stays pending; a later approve retries) rather than deploying
    // policy against the target's stale data.
    apply_pinned_data_version(state, org_id, &cr, to_env).await?;

    // Then run the existing rollout machinery into the target namespace.
    let service = DeploymentService::new(state.db.clone());
    let input = StartRollout {
        bundle_id: cr.bundle_id,
        strategy_id: cr.strategy_id,
        namespace_id: Some(to_env.namespace_id),
        triggered_by: None,
    };
    let result = service
        .start_rollout(*org_id, &input, state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(_) => {
                ApiError::NotFound("Bundle not found".to_string())
            }
            crate::deployment::DeploymentError::BundleNotReady(msg) => ApiError::BadRequest(msg),
            crate::deployment::DeploymentError::ActiveRolloutExists(_) => {
                ApiError::Conflict("Active rollout already exists for this bundle".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    cr_repo
        .set_status(
            cr.id,
            ChangeRequestStatus::Applied,
            Some(result.rollout.id),
            None,
        )
        .await?;

    // Return the refreshed request.
    cr_repo
        .get(cr.id)
        .await?
        .ok_or_else(|| ApiError::Internal("change request vanished after apply".to_string()))
}

/// Move the change request's pinned data version from the source env's
/// datastore into the target env's (Plan 10 Step 7). No-op when the request
/// pinned no data version (the source env had no datastore — a policy-only
/// promotion). Fails closed when a pinned version can no longer be resolved
/// or the target has no datastore to receive it.
async fn apply_pinned_data_version(
    state: &AppState,
    org_id: &Uuid,
    cr: &ChangeRequest,
    to_env: &Environment,
) -> ApiResult<()> {
    let Some(pinned_version) = cr.data_version else {
        return Ok(());
    };
    // A datastore that existed but had never published pins version 0 —
    // nothing to move yet.
    if pinned_version == 0 {
        return Ok(());
    }

    let env_repo = EnvironmentRepository::new(&state.db);
    let from_env = env_repo
        .get_by_id(cr.from_env_id)
        .await?
        .ok_or_else(|| ApiError::Internal("Source environment vanished".to_string()))?;

    let ds_repo = DatastoreRepository::new(&state.db);
    let source_store = ds_repo
        .get(*org_id, from_env.namespace_id)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(
                "promotion pinned a data version but the source environment's datastore \
                 no longer exists (fail closed)"
                    .to_string(),
            )
        })?;
    let (meta, document) = ds_repo
        .get_version_document(source_store.id, pinned_version)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "promotion pinned data version {pinned_version} but it is no longer \
                 available in the source environment (fail closed)"
            ))
        })?;
    let target_store = ds_repo
        .get(*org_id, to_env.namespace_id)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "target environment '{}' has no datastore provisioned to receive the \
                 promoted data version (fail closed)",
                to_env.name
            ))
        })?;

    let imported = ds_repo
        .import_version(
            &target_store,
            &document,
            (meta.entity_count, meta.tuple_count, meta.binding_count),
            &format!("promotion:{}", cr.id),
        )
        .await?;

    // Wake the target namespace's fleet exactly like a normal publish.
    let _ = state
        .event_tx
        .send(crate::state::ServerEvent::DatastorePublished {
            datastore_id: target_store.id,
            org_id: *org_id,
            namespace_id: Some(to_env.namespace_id),
            version: imported.version,
            checksum: imported.checksum.clone(),
        });
    crate::events_pg::notify_datastore_published(
        state,
        target_store.id,
        *org_id,
        Some(to_env.namespace_id),
        imported.version,
        &imported.checksum,
    )
    .await;

    Ok(())
}

/// Enforce the target environment's `external_change_record` policy against
/// the supplied ITSM reference (Plan 10 follow-up). Called at promotion
/// request time AND again at apply time, so a record that gets rejected in
/// ServiceNow mid-flight cannot still deploy.
///
/// - `off`: a supplied reference only has to be well-formed (it is stored
///   opaquely as part of the change record).
/// - `reference`: a well-formed reference is mandatory but not verified.
/// - `validated`: the reference must exist in the configured ServiceNow
///   instance with an accepted `approval` value. Missing configuration or an
///   unreachable instance fails CLOSED with 409.
async fn check_external_change_record(
    state: &AppState,
    to_env: &Environment,
    reference: Option<&str>,
) -> ApiResult<()> {
    let mode = to_env.approval_policy.external_change_record;

    let Some(reference) = reference else {
        return match mode {
            ExternalChangeRecordMode::Off => Ok(()),
            _ => Err(ApiError::BadRequest(format!(
                "environment '{}' requires an external change-record reference \
                 (`change_ref`, e.g. a ServiceNow CHG number) on promotion",
                to_env.name
            ))),
        };
    };

    if !ServiceNowClient::is_valid_reference(reference) {
        return Err(ApiError::BadRequest(format!(
            "'{reference}' is not a valid change-record reference \
             (letters, digits, '-' or '_', at most 64 characters)"
        )));
    }
    if mode != ExternalChangeRecordMode::Validated {
        return Ok(());
    }

    let Some(snow) = state.config.integrations.servicenow.clone() else {
        return Err(ApiError::Conflict(format!(
            "environment '{}' requires validated external change records but no \
             ServiceNow instance is configured (fail closed)",
            to_env.name
        )));
    };
    match ServiceNowClient::new(snow)
        .validate_change_record(reference)
        .await
    {
        Ok(ChangeRecordCheck::Valid) => Ok(()),
        Ok(ChangeRecordCheck::NotFound) => Err(ApiError::BadRequest(format!(
            "change record '{reference}' was not found in ServiceNow"
        ))),
        Ok(ChangeRecordCheck::NotApproved(approval)) => Err(ApiError::Conflict(format!(
            "change record '{reference}' is not approved in ServiceNow \
             (approval: '{approval}')"
        ))),
        Err(e) => Err(ApiError::Conflict(format!(
            "could not validate change record '{reference}' against ServiceNow \
             (fail closed): {e}"
        ))),
    }
}

async fn load_scoped(
    cr_repo: &ChangeRequestRepository<'_>,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<ChangeRequest> {
    let cr = cr_repo
        .get(id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Change request not found".to_string()))?;
    if cr.org_id != org_id {
        return Err(ApiError::NotFound("Change request not found".to_string()));
    }
    Ok(cr)
}

async fn authorize(
    state: &AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    org: &str,
    scope: Scope,
) -> ApiResult<crate::domain::organization::Organization> {
    if !user.has_permission(scope) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(format!(
            "Missing {} scope",
            scope.as_str()
        )));
    }
    let organization = resolve_org(&OrganizationRepository::new(&state.db), org).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access change requests for other organizations".to_string(),
        ));
    }
    Ok(organization)
}

async fn audit(
    state: &AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    action: &str,
    resource_id: Uuid,
    org: &crate::domain::organization::Organization,
) {
    let resource = if action == actions::ENV_PROMOTE {
        ResourceType::Environment
    } else {
        ResourceType::ChangeRequest
    };
    AuditEntry::builder(action, ActorType::User, user.id.clone())
        .org_id(org.id)
        .resource(resource, resource_id.to_string())
        .log(&state.db)
        .await
        .ok();
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}
