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
    extract::{Path, Query, State},
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
        audit_governance::LegalHold, AuditErasureRepository, AuditGovernanceRepository,
        DatastoreRepository, ErasureRecord, NewErasureRecord, OrganizationRepository,
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
        .routes(routes!(list_erasures))
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
        ("org" = String, Path, description = "Organization ID"),
        ("limit" = Option<i64>, Query, description = "Max to return (default 200, max 500)")
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
    Query(page): Query<crate::api::pagination::LimitQuery>,
) -> ApiResult<Json<HoldListResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let holds = AuditGovernanceRepository::new(&state.db)
        .list_holds(org_id, page.cap()?)
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

/// An immutable, append-only surface that subject-erasure does NOT rewrite in
/// place, disclosed on the receipt with the lawful basis for its retention and
/// how the subject's data ultimately leaves it. This is the "documented,
/// receipted exemption" posture (E2 follow-up #2): the WORM archive is
/// un-rewritable *by design* (S3 Object-Lock, COMPLIANCE mode) and published
/// data-bundle versions are checksum-sealed snapshots agents may still be
/// running — so both are retained under a documented audit/technical basis
/// rather than rewritten. See `docs/security/SUBJECT_ERASURE.md`.
#[derive(Debug, Serialize, ToSchema)]
struct ErasureExemption {
    /// Stable identifier of the exempt surface
    /// (`decision_log_worm_archive` | `published_datastore_versions`).
    surface: String,
    /// Why the surface is not rewritten in place.
    reason: String,
    /// How the subject's data ultimately leaves the surface (e.g. ages out with
    /// the retention window, superseded by the next publish).
    disposition: String,
}

/// Immutable surfaces this erasure did not rewrite, given what the live-store
/// steps did. Kept a pure function of the two outcomes so the disclosure is
/// unit-testable without a live store or DB. A WORM-archive exemption is
/// disclosed whenever the decision-log redaction was submitted (the archive
/// still holds the pre-redaction bytes); a published-versions exemption is
/// disclosed whenever the org has authoring DataStores (their immutable
/// published versions are never rewritten, even when the subject had no live
/// entity to cascade-delete).
fn immutable_exemptions(
    decision_log: &DecisionLogEraseResult,
    datastore: &DatastoreEraseResult,
) -> Vec<ErasureExemption> {
    let mut out = Vec::new();
    if matches!(decision_log, DecisionLogEraseResult::Submitted { .. }) {
        out.push(ErasureExemption {
            surface: "decision_log_worm_archive".to_string(),
            reason: "the queryable store is redacted in place, but the S3 Object-Lock \
                     (COMPLIANCE-mode) WORM archive and any NDJSON archive are append-only \
                     and cannot be rewritten by design — they are the tamper-evident audit \
                     anchor a regulator verifies ByteExact against"
                .to_string(),
            disposition: "retained under the audit-retention lawful basis (GDPR Art. 17(3)(b)); \
                          ages out when the Object-Lock retention window expires. Under the \
                          pseudonymize profile the archive already holds only HMAC tokens and \
                          AES-GCM ciphertext, so the plaintext is irrecoverable there"
                .to_string(),
        });
    }
    if let DatastoreEraseResult::Erased {
        datastores_scanned, ..
    } = datastore
    {
        if *datastores_scanned > 0 {
            out.push(ErasureExemption {
                surface: "published_datastore_versions".to_string(),
                reason: "the live entity is hard-deleted (cascade), but published data-bundle \
                         versions (adm_versions) are immutable, checksum-sealed snapshots that \
                         deployed agents may still be running; rewriting one would break its \
                         checksum and the version-immutability contract"
                    .to_string(),
                disposition: "superseded by the next publish; historical versions age out under \
                              data-bundle version retention"
                    .to_string(),
            });
        }
    }
    out
}

/// Proof-of-erasure receipt (also written to the audit trail).
#[derive(Debug, Serialize, ToSchema)]
struct ErasureReceipt {
    subject: String,
    decision_log: DecisionLogEraseResult,
    datastore: DatastoreEraseResult,
    /// Post-erasure verification posture of the decision store. Always
    /// `"linkage"`: redact-in-place preserves the stored hash-chain (completeness
    /// and ordering stay provable via `VerifyMode::Linkage`), but content
    /// re-hashing (`ByteExact`) over the redacted queryable store no longer
    /// matches — ByteExact remains the WORM archive's job. See
    /// `docs/security/SUBJECT_ERASURE.md`.
    verification_posture: String,
    /// Immutable, append-only surfaces NOT rewritten by this erasure, each with
    /// the lawful basis for retention and how the subject's data ultimately
    /// leaves. Empty when no immutable surface was touched.
    exemptions: Vec<ErasureExemption>,
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

            let exemptions = immutable_exemptions(&decision_log, &datastore);
            let receipt = ErasureReceipt {
                subject: subject.clone(),
                decision_log,
                datastore,
                verification_posture: "linkage".to_string(),
                exemptions,
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

            // Persist a queryable erasure-history record (E2 follow-up #3),
            // best-effort like the audit write: the trail is the primary proof,
            // this table is the DSAR convenience. Never fail an already-completed
            // (irreversible) erasure on a history hiccup.
            let (dl_status, holds_honored, matched_pseudonyms) = match &receipt.decision_log {
                DecisionLogEraseResult::Submitted {
                    holds_honored,
                    matched_pseudonyms,
                } => (
                    "submitted",
                    Some(*holds_honored as i64),
                    *matched_pseudonyms,
                ),
                DecisionLogEraseResult::DeferredBlanketHold => {
                    ("deferred_blanket_hold", None, false)
                }
                DecisionLogEraseResult::StoreNotConfigured => ("store_not_configured", None, false),
            };
            let (ds_status, scanned, deleted) = match &receipt.datastore {
                DatastoreEraseResult::Erased {
                    datastores_scanned,
                    entities_deleted,
                } => (
                    "erased",
                    *datastores_scanned as i64,
                    *entities_deleted as i64,
                ),
                DatastoreEraseResult::Skipped => ("skipped", 0, 0),
            };
            if let Err(e) = AuditErasureRepository::new(&state.db)
                .record(NewErasureRecord {
                    org_id,
                    subject: &subject,
                    requested_by: Some(user.id.as_str()),
                    decision_log_status: dl_status,
                    holds_honored,
                    matched_pseudonyms,
                    datastore_status: ds_status,
                    datastores_scanned: scanned,
                    entities_deleted: deleted,
                    verification_posture: &receipt.verification_posture,
                    receipt: &body,
                })
                .await
            {
                tracing::error!(error = %e, "failed to persist subject-erasure receipt");
            }

            Ok((StatusCode::OK, body))
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct ErasureHistoryQuery {
    /// Max records to return, newest first (default 100, hard cap 500).
    limit: Option<i64>,
}

/// A tenant's subject-erasure history.
#[derive(Debug, Serialize, ToSchema)]
struct ErasureHistoryResponse {
    /// Number of records returned (bounded by `limit`).
    count: usize,
    records: Vec<ErasureRecord>,
}

/// GET /orgs/{org}/audit/erasures — the tenant's subject-erasure history
/// (E2 follow-up #3), newest first. Org-admin-gated: reading *who was erased* is
/// a compliance-record read, like legal holds and retention — distinct from the
/// `audit:erase` scope that authorizes performing an erasure.
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/erasures",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("limit" = Option<i64>, Query, description = "Max records, newest first (default 100, max 500)")
    ),
    responses(
        (status = 200, description = "Subject-erasure history", body = ErasureHistoryResponse),
        (status = 403, description = "Caller lacks org:admin on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_erasures(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<ErasureHistoryQuery>,
) -> ApiResult<Json<ErasureHistoryResponse>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let records = AuditErasureRepository::new(&state.db)
        .list_for_org(org_id, query.limit)
        .await?;
    Ok(Json(ErasureHistoryResponse {
        count: records.len(),
        records,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exemptions_disclose_worm_archive_when_decision_log_redacted() {
        let ex = immutable_exemptions(
            &DecisionLogEraseResult::Submitted {
                holds_honored: 0,
                matched_pseudonyms: false,
            },
            &DatastoreEraseResult::Skipped,
        );
        assert_eq!(ex.len(), 1, "{ex:?}");
        assert_eq!(ex[0].surface, "decision_log_worm_archive");
        assert!(ex[0].reason.contains("WORM"));
        assert!(!ex[0].disposition.is_empty());
    }

    #[test]
    fn exemptions_disclose_published_versions_when_datastore_scanned() {
        // Disclosed even with zero live deletes: an old published version may
        // still carry the subject, and those versions are never rewritten.
        let ex = immutable_exemptions(
            &DecisionLogEraseResult::StoreNotConfigured,
            &DatastoreEraseResult::Erased {
                datastores_scanned: 2,
                entities_deleted: 0,
            },
        );
        assert_eq!(ex.len(), 1, "{ex:?}");
        assert_eq!(ex[0].surface, "published_datastore_versions");
    }

    #[test]
    fn exemptions_cover_both_surfaces_when_both_steps_ran() {
        let ex = immutable_exemptions(
            &DecisionLogEraseResult::Submitted {
                holds_honored: 1,
                matched_pseudonyms: true,
            },
            &DatastoreEraseResult::Erased {
                datastores_scanned: 1,
                entities_deleted: 1,
            },
        );
        let surfaces: Vec<&str> = ex.iter().map(|e| e.surface.as_str()).collect();
        assert_eq!(
            surfaces,
            ["decision_log_worm_archive", "published_datastore_versions"]
        );
    }

    #[test]
    fn no_exemptions_when_nothing_immutable_was_touched() {
        // Store unconfigured (no archive) and datastore erasure skipped: there is
        // no immutable surface to disclose.
        let ex = immutable_exemptions(
            &DecisionLogEraseResult::StoreNotConfigured,
            &DatastoreEraseResult::Skipped,
        );
        assert!(ex.is_empty(), "{ex:?}");

        // A deferred blanket hold retained the whole store already; there is no
        // fresh redaction, so no archive divergence to disclose.
        let ex = immutable_exemptions(
            &DecisionLogEraseResult::DeferredBlanketHold,
            &DatastoreEraseResult::Erased {
                datastores_scanned: 0,
                entities_deleted: 0,
            },
        );
        assert!(ex.is_empty(), "{ex:?}");
    }
}
