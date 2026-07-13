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
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult, ProblemDetails},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::middleware::{AuthenticatedUser, RequireAuth},
    auth::scopes::Scope,
    db::repositories::{
        audit_governance::LegalHold, AuditGovernanceRepository, OrganizationRepository,
    },
    decisions::purge::{default_retention_days, run_org_purge, PurgeError},
    decisions::{HoldFilter, PurgeOutcome},
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
