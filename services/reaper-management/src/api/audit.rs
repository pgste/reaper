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
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::middleware::{AuthenticatedUser, RequireAuth},
    auth::scopes::Scope,
    db::repositories::{AuditGovernanceRepository, OrganizationRepository},
    decisions::purge::{default_retention_days, run_org_purge, PurgeError},
    decisions::HoldFilter,
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

/// GET /orgs/{org}/audit/retention — effective window (explicit or default).
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/retention",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Effective retention window")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_retention(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Value>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let repo = AuditGovernanceRepository::new(&state.db);
    match repo.get_retention(org_id).await? {
        Some(r) => Ok(Json(json!({
            "days": r.days,
            "source": "explicit",
            "updated_by": r.updated_by,
            "updated_at": r.updated_at,
        }))),
        None => Ok(Json(json!({
            "days": default_retention_days(),
            "source": "default",
        }))),
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
        (status = 200, description = "Retention window updated")
    ),
    security(("bearer_jwt" = []))
)]
async fn set_retention(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<SetRetentionRequest>,
) -> ApiResult<Json<Value>> {
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

    Ok(Json(json!({
        "days": setting.days,
        "source": "explicit",
        "updated_by": setting.updated_by,
        "updated_at": setting.updated_at,
    })))
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
        (status = 201, description = "Legal hold created")
    ),
    security(("bearer_jwt" = []))
)]
async fn create_hold(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<CreateHoldRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
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

    let body = serde_json::to_value(&hold)
        .map_err(|e| ApiError::Internal(format!("serialize hold: {e}")))?;
    Ok((StatusCode::CREATED, Json(body)))
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
        (status = 200, description = "Legal holds (active and released)")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_holds(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Value>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let holds = AuditGovernanceRepository::new(&state.db)
        .list_holds(org_id)
        .await?;
    let active = holds.iter().filter(|h| h.is_active()).count();
    Ok(Json(json!({
        "count": holds.len(),
        "active": active,
        "holds": holds,
    })))
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
        (status = 200, description = "Legal hold detail"),
        (status = 404, description = "Legal hold not found")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_hold(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, hold_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    match AuditGovernanceRepository::new(&state.db)
        .get_hold(org_id, hold_id)
        .await?
    {
        Some(hold) => {
            Ok(Json(serde_json::to_value(&hold).map_err(|e| {
                ApiError::Internal(format!("serialize hold: {e}"))
            })?))
        }
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
        (status = 404, description = "Legal hold not found or already released")
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
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/purge",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Retention purge executed")
    ),
    security(("bearer_jwt" = []))
)]
async fn trigger_purge(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<Value>> {
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

    Ok(Json(json!({ "days": days, "result": outcome })))
}
