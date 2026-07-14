//! Audit governance API (Plan 04, step 6): tenant retention windows and
//! legal holds over the central decision store, plus a manual purge trigger.
//!
//! Governance state lives in the management DB (transactional, audited);
//! the purge it governs executes against ClickHouse via
//! [`crate::decisions::DecisionStore::purge_expired`]. All routes are
//! tenant-scoped and admin-only: holds reveal litigation posture, and
//! retention changes alter what evidence survives — this is compliance
//! surface, not operational decision data.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult, ProblemDetails},
    api::idempotency,
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::middleware::{AuthenticatedUser, RequireAuth},
    auth::scopes::Scope,
    db::repositories::{
        audit_governance::LegalHold, AuditGovernanceRepository, DatastoreRepository,
        OrganizationRepository,
    },
    decisions::purge::{default_retention_days, run_org_purge, PurgeError},
    decisions::{EraseOutcome, HoldFilter, PurgeOutcome, SubjectPseudonyms},
    state::AppState,
};

/// Retention windows must be positive and bounded (10 years) so a typo can't
/// silently configure a near-infinite or instant-delete window.
const MAX_RETENTION_DAYS: i64 = 3650;

/// Build audit-governance routes.
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(get_retention, set_retention))
        .routes(routes!(list_holds, create_hold))
        .routes(routes!(get_hold, release_hold))
        .routes(routes!(trigger_purge))
        .routes(routes!(erase_subject))
}

/// Authorize audit-governance access on `org` and return the org id.
/// Admin-only (org or platform): retention and holds are compliance controls.
async fn authorize_admin(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
) -> ApiResult<Uuid> {
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Audit governance requires org:admin scope".to_string(),
        ));
    }
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot manage audit governance for other organizations".to_string(),
        ));
    }
    Ok(organization.id)
}

/// Authorize a subject-erasure on `org` and return the org id. Requires the
/// dedicated `audit:erase` scope (separation of duties — erasure is irreversible
/// and destroys evidence, so it is not conferred by `org:admin`); the global
/// `admin` scope still covers it and the platform-operator cross-org escape.
async fn authorize_erase(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
) -> ApiResult<Uuid> {
    if !user.has_permission(Scope::AuditErase) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Subject erasure requires the audit:erase scope".to_string(),
        ));
    }
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot erase subjects for other organizations".to_string(),
        ));
    }
    Ok(organization.id)
}

fn actor_type_of(user: &AuthenticatedUser) -> ActorType {
    match user.auth_method {
        crate::auth::middleware::AuthMethod::ApiKey { .. } => ActorType::ApiKey,
        crate::auth::middleware::AuthMethod::Mtls { .. } => ActorType::Agent,
        crate::auth::middleware::AuthMethod::Jwt { .. } => ActorType::User,
    }
}

/// Write a governance audit record; failure is logged, never blocks the API
/// (the governance change itself already committed).
async fn write_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    org_id: Uuid,
    action: &str,
    resource: (ResourceType, String),
    details: Value,
) {
    let entry = AuditEntry::builder(action, actor_type_of(user), user.id.clone())
        .org_id(org_id)
        .resource(resource.0, resource.1)
        .details(details);
    if let Err(e) = entry.log(&state.db).await {
        tracing::error!(error = %e, action, "failed to write audit-governance record");
    }
}

// ---- Retention ----

/// The effective audit retention window for a tenant.
#[derive(Debug, Serialize, ToSchema)]
struct RetentionResponse {
    /// Retention window in days.
    days: i64,
    /// `explicit` (tenant-configured) or `default` (deployment default).
    source: String,
    /// Who set the explicit window (present only when `source` is
    /// `explicit`; `null` when unattributed).
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_by: Option<Option<String>>,
    /// When the explicit window was set (present only when `source` is
    /// `explicit`).
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /orgs/{org}/audit/retention — effective window (explicit or default).
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/retention",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Effective retention window", body = RetentionResponse),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_retention(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<RetentionResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let repo = AuditGovernanceRepository::new(&state.db);
    match repo.get_retention(org_id).await? {
        Some(r) => Ok(Json(RetentionResponse {
            days: r.days,
            source: "explicit".to_string(),
            updated_by: Some(r.updated_by),
            updated_at: Some(r.updated_at),
        })),
        None => Ok(Json(RetentionResponse {
            days: default_retention_days(),
            source: "default".to_string(),
            updated_by: None,
            updated_at: None,
        })),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
struct SetRetentionRequest {
    days: i64,
}

/// PUT /orgs/{org}/audit/retention {days} — set the tenant window. Audited.
#[utoipa::path(
    put,
    path = "/orgs/{org}/audit/retention",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    request_body = SetRetentionRequest,
    responses(
        (status = 200, description = "Retention window updated", body = RetentionResponse),
        (status = 400, description = "days out of range", body = ProblemDetails),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn set_retention(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<SetRetentionRequest>,
) -> ApiResult<Json<RetentionResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    if req.days < 1 || req.days > MAX_RETENTION_DAYS {
        return Err(ApiError::BadRequest(format!(
            "days must be between 1 and {MAX_RETENTION_DAYS}"
        )));
    }
    let repo = AuditGovernanceRepository::new(&state.db);
    let previous = repo.get_retention(org_id).await?.map(|r| r.days);
    let setting = repo
        .set_retention(org_id, req.days, Some(user.id.as_str()))
        .await?;

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_RETENTION_UPDATE,
        (ResourceType::Org, org_id.to_string()),
        json!({ "days": setting.days, "previous_days": previous }),
    )
    .await;

    Ok(Json(RetentionResponse {
        days: setting.days,
        source: "explicit".to_string(),
        updated_by: Some(setting.updated_by),
        updated_at: Some(setting.updated_at),
    }))
}

// ---- Legal holds ----

#[derive(Debug, Deserialize)]
struct CreateHoldRequest {
    reason: String,
    /// Omitted or `{}` = blanket hold: protects every decision the org has
    /// and suspends its retention purge entirely while active.
    #[serde(default)]
    filter: HoldFilter,
}

/// POST /orgs/{org}/audit/legal-holds — place a hold. Audited.
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/legal-holds",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 201, description = "Legal hold created", body = LegalHold),
        (status = 400, description = "Missing hold reason", body = ProblemDetails),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn create_hold(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<CreateHoldRequest>,
) -> ApiResult<(StatusCode, Json<LegalHold>)> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest(
            "a legal hold requires a non-empty reason (it is a compliance record)".to_string(),
        ));
    }
    let repo = AuditGovernanceRepository::new(&state.db);
    let hold = repo
        .create_hold(org_id, &req.filter, reason, Some(user.id.as_str()))
        .await?;

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_LEGAL_HOLD_CREATE,
        (ResourceType::LegalHold, hold.id.to_string()),
        json!({
            "reason": hold.reason,
            "filter": hold.filter,
            "blanket": hold.filter.is_blanket(),
        }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(hold)))
}

/// Every legal hold the org has placed, with active/total counts.
#[derive(Debug, Serialize, ToSchema)]
struct HoldListResponse {
    /// Total holds (active and released).
    count: usize,
    /// Holds still active (not yet released).
    active: usize,
    holds: Vec<LegalHold>,
}

/// GET /orgs/{org}/audit/legal-holds — active and released (the compliance
/// record includes released holds).
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/legal-holds",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Legal holds (active and released)", body = HoldListResponse),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_holds(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<HoldListResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let holds = AuditGovernanceRepository::new(&state.db)
        .list_holds(org_id)
        .await?;
    let active = holds.iter().filter(|h| h.is_active()).count();
    Ok(Json(HoldListResponse {
        count: holds.len(),
        active,
        holds,
    }))
}

/// GET /orgs/{org}/audit/legal-holds/{hold_id}
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/legal-holds/{hold_id}",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("hold_id" = Uuid, Path, description = "Legal hold ID")
    ),
    responses(
        (status = 200, description = "Legal hold detail", body = LegalHold),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Legal hold not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_hold(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, hold_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<LegalHold>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    match AuditGovernanceRepository::new(&state.db)
        .get_hold(org_id, hold_id)
        .await?
    {
        Some(hold) => Ok(Json(hold)),
        None => Err(ApiError::NotFound(format!(
            "Legal hold '{hold_id}' not found"
        ))),
    }
}

/// DELETE /orgs/{org}/audit/legal-holds/{hold_id} — release (never deletes
/// the record; the hold's lifecycle stays auditable). Audited.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/audit/legal-holds/{hold_id}",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("hold_id" = Uuid, Path, description = "Legal hold ID")
    ),
    responses(
        (status = 204, description = "Legal hold released"),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Legal hold not found or already released", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn release_hold(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, hold_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let released = AuditGovernanceRepository::new(&state.db)
        .release_hold(org_id, hold_id, Some(user.id.as_str()))
        .await?;
    if !released {
        return Err(ApiError::NotFound(format!(
            "Legal hold '{hold_id}' not found or already released"
        )));
    }

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_LEGAL_HOLD_RELEASE,
        (ResourceType::LegalHold, hold_id.to_string()),
        json!({}),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

// ---- Manual purge ----

/// POST /orgs/{org}/audit/purge — run the org's retention purge now (the
/// background sweeper runs the same path on an interval). Audited.
/// Outcome of a manually triggered retention purge.
#[derive(Debug, Serialize, ToSchema)]
struct PurgeResponse {
    /// The retention window the purge enforced.
    days: i64,
    result: PurgeOutcome,
}

/// Run the org's retention purge now (same path the background sweeper runs).
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/purge",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Retention purge executed", body = PurgeResponse),
        (status = 400, description = "Retention disabled for this org", body = ProblemDetails),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn trigger_purge(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<PurgeResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let store = state.decision_store.as_deref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "decision store not configured: set REAPER_CLICKHOUSE_URL (see deploy/decision-logs/)"
                .to_string(),
        )
    })?;

    let repo = AuditGovernanceRepository::new(&state.db);
    let days = match repo.get_retention(org_id).await? {
        Some(r) => r.days,
        None => default_retention_days(),
    };

    let outcome = run_org_purge(&state.db, store, org_id, days)
        .await
        .map_err(|e| match e {
            PurgeError::RetentionDisabled => ApiError::BadRequest(
                "retention is disabled for this org (no explicit window and \
                 REAPER_AUDIT_DEFAULT_RETENTION_DAYS=0)"
                    .to_string(),
            ),
            PurgeError::Db(e) => ApiError::from(e),
            PurgeError::Store(e) => ApiError::ServiceUnavailable(format!("decision store: {e}")),
        })?;

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_PURGE,
        (ResourceType::Org, org_id.to_string()),
        json!({ "days": days, "outcome": outcome }),
    )
    .await;

    Ok(Json(PurgeResponse {
        days,
        result: outcome,
    }))
}

// ---- Subject erasure (E2, GDPR Art. 17) ----

#[derive(Debug, Deserialize, ToSchema)]
struct EraseSubjectRequest {
    /// The data subject's identifier — matched against decision-log
    /// `principal`/`resource` and DataStore `entity_id`.
    subject: String,
    /// Also erase the subject's entity (and its tuples/bindings) from the org's
    /// authoring DataStores. Default `true`; set `false` to erase only the
    /// decision-log trail.
    #[serde(default)]
    erase_datastore: Option<bool>,
    /// The tenant's decision-log pseudonymisation salt
    /// (`REAPER_DECISION_LOG_HASH_SALT`). Supply it when the tenant runs the
    /// `pseudonymize` privacy profile: the decision-log `principal`/`resource`
    /// columns then hold `sha256:<hmac>` tokens, and without the salt the
    /// control plane cannot match — and would silently miss — those rows. The
    /// salt is used only to derive the match tokens for this request; it is
    /// never persisted, echoed in the receipt, or written to the audit trail.
    #[serde(default)]
    pseudonym_salt: Option<String>,
}

/// What happened to the decision-log trail for the subject.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "status")]
enum DecisionLogEraseResult {
    /// Redact-in-place UPDATE submitted; `holds_honored` active holds were
    /// excluded from the redaction. `matched_pseudonyms` is true when a tenant
    /// salt was supplied and the redaction also targeted the `sha256:<hmac>`
    /// principal/resource columns of a `pseudonymize`-profile tenant.
    Submitted {
        holds_honored: usize,
        matched_pseudonyms: bool,
    },
    /// An active blanket legal hold preserves the whole tenant — a lawful basis
    /// to retain, so the decision-log redaction was deferred, not applied.
    DeferredBlanketHold,
    /// No decision store is configured (`REAPER_CLICKHOUSE_URL` unset), so there
    /// is no decision-log trail to erase.
    StoreNotConfigured,
}

/// What happened to the subject's authoring-DataStore records.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "status")]
enum DatastoreEraseResult {
    /// Hard-deleted the subject's entity (cascading its tuples + bindings) from
    /// `entities_deleted` of the `datastores_scanned` DataStores it appeared in.
    Erased {
        datastores_scanned: usize,
        entities_deleted: usize,
    },
    /// DataStore erasure was not requested (`erase_datastore = false`).
    Skipped,
}

/// Proof-of-erasure receipt (also written to the audit trail).
#[derive(Debug, Serialize, ToSchema)]
struct ErasureReceipt {
    subject: String,
    decision_log: DecisionLogEraseResult,
    datastore: DatastoreEraseResult,
    /// The principal who requested the erasure.
    requested_by: String,
}

/// POST /orgs/{org}/audit/erasure — erase a data subject (GDPR Art. 17).
///
/// Redacts the subject in place across the decision-log store (preserving the
/// tamper-evident chain) and hard-deletes their entity from the org's authoring
/// DataStores. Idempotent via `Idempotency-Key`; the receipt is also recorded on
/// the audit trail as durable proof of erasure.
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/erasure",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    request_body = EraseSubjectRequest,
    responses(
        (status = 200, description = "Erasure applied (see receipt)", body = ErasureReceipt),
        (status = 400, description = "Missing subject identifier", body = ProblemDetails),
        (status = 403, description = "Caller lacks audit:erase on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 409, description = "An erasure with this Idempotency-Key is in flight", body = ProblemDetails),
        (status = 503, description = "Decision store configured but unreachable", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn erase_subject(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(req): Json<EraseSubjectRequest>,
) -> ApiResult<Response> {
    let org_id = authorize_erase(&state, &user, &org).await?;
    let subject = req.subject.trim().to_string();
    if subject.is_empty() {
        return Err(ApiError::BadRequest(
            "erasure requires a non-empty subject identifier".to_string(),
        ));
    }
    let erase_datastore = req.erase_datastore.unwrap_or(true);
    let org_str = org_id.to_string();

    // Derive the subject's pseudonymised match tokens from the tenant salt (if
    // supplied). This lets one erasure reach a `pseudonymize`-profile tenant,
    // whose decision-log columns store `sha256:<hmac>` rather than plaintext.
    let pseudonyms = req
        .pseudonym_salt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|salt| SubjectPseudonyms {
            principal: policy_engine::pseudonymize(salt.as_bytes(), &subject),
            resource: policy_engine::pseudonymize_domain(salt.as_bytes(), "resource", &subject),
        });

    // Fingerprint the request identity so a retried Idempotency-Key that
    // materially changes the operation (adding pseudonym matching, toggling the
    // datastore erasure) is rejected as a different request rather than replaying
    // a stale, narrower result. The salt itself is a secret and never enters the
    // fingerprint — only the *fact* that pseudonym matching was requested.
    let fp = idempotency::fingerprint(&[
        actions::AUDIT_SUBJECT_ERASURE,
        &org_str,
        &subject,
        if erase_datastore { "ds" } else { "nods" },
        if pseudonyms.is_some() {
            "pseudo"
        } else {
            "plain"
        },
    ]);

    idempotency::run(
        &state.db,
        &headers,
        actions::AUDIT_SUBJECT_ERASURE,
        &org_str,
        &fp,
        || async {
            // 1. Decision-log redaction (only if a store is configured).
            let decision_log = match state.decision_store.as_deref() {
                Some(store) => {
                    let holds: Vec<HoldFilter> = AuditGovernanceRepository::new(&state.db)
                        .active_holds(org_id)
                        .await?
                        .into_iter()
                        .map(|h| h.filter)
                        .collect();
                    let outcome = store
                        .erase_subject(&org_str, &subject, pseudonyms.as_ref(), &holds)
                        .await
                        .map_err(|e| {
                            ApiError::ServiceUnavailable(format!("decision store: {e}"))
                        })?;
                    match outcome {
                        EraseOutcome::Submitted { holds_honored } => {
                            DecisionLogEraseResult::Submitted {
                                holds_honored,
                                matched_pseudonyms: pseudonyms.is_some(),
                            }
                        }
                        EraseOutcome::DeferredBlanketHold => {
                            DecisionLogEraseResult::DeferredBlanketHold
                        }
                    }
                }
                None => DecisionLogEraseResult::StoreNotConfigured,
            };

            // 2. Authoring-DataStore erasure across every namespace the org owns.
            let datastore = if erase_datastore {
                let repo = DatastoreRepository::new(&state.db);
                let ids = repo.datastore_ids_for_org(org_id).await?;
                let mut entities_deleted = 0usize;
                for ds in &ids {
                    let (deleted, _affected) = repo.delete_entity_cascade(*ds, &subject).await?;
                    if deleted {
                        entities_deleted += 1;
                    }
                }
                DatastoreEraseResult::Erased {
                    datastores_scanned: ids.len(),
                    entities_deleted,
                }
            } else {
                DatastoreEraseResult::Skipped
            };

            let receipt = ErasureReceipt {
                subject: subject.clone(),
                decision_log,
                datastore,
                requested_by: user.id.clone(),
            };
            let body = serde_json::to_value(&receipt).unwrap_or(Value::Null);

            // The audit entry is the durable proof of erasure.
            write_audit(
                &state,
                &user,
                org_id,
                actions::AUDIT_SUBJECT_ERASURE,
                (ResourceType::Org, org_str.clone()),
                body.clone(),
            )
            .await;

            Ok((StatusCode::OK, body))
        },
    )
    .await
}
