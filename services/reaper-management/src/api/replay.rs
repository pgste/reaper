//! Counterfactual replay API (Plan 04, step 8).
//!
//! `POST /orgs/{org}/replay` starts an async replay job (202 + job id);
//! `GET /orgs/{org}/replay/{job_id}` polls progress / fetches the diff.
//! Admin-only: replay reads audit data wholesale and may carry a tenant
//! decryption key. Every started job is audited.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::middleware::{AuthenticatedUser, RequireAuth},
    auth::scopes::Scope,
    db::repositories::OrganizationRepository,
    replay::{start_job, ReplayRequest},
    state::AppState,
};

pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(start_replay))
        .routes(routes!(get_replay))
}

/// Admin-only, tenant-scoped (same posture as audit governance).
async fn authorize_admin(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
) -> ApiResult<Uuid> {
    if !user.has_permission(Scope::OrgAdmin) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Replay requires org:admin scope".to_string(),
        ));
    }
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot replay decisions for other organizations".to_string(),
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

/// POST /orgs/{org}/replay — start a counterfactual replay job.
#[utoipa::path(
    post,
    path = "/orgs/{org}/replay",
    tag = "replay",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 202, description = "Replay job started")
    ),
    security(("bearer_jwt" = []))
)]
async fn start_replay(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<ReplayRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let org_id = authorize_admin(&state, &user, &org).await?;

    // Audit BEFORE the run (the analysis request itself is the sensitive act;
    // the key, if any, is never recorded).
    let bundle_id = request.bundle_id;
    let details = json!({
        "bundle_id": bundle_id,
        "from": request.from,
        "to": request.to,
        "namespace": request.namespace,
        "data_version": request.data_version,
        "with_decryption_key": request.decryption_key.is_some(),
    });

    let job_id = start_job(state.clone(), org_id, request)
        .await
        .map_err(map_start_error)?;

    let entry = AuditEntry::builder(actions::AUDIT_REPLAY, actor_type_of(&user), user.id.clone())
        .org_id(org_id)
        .resource(ResourceType::Bundle, bundle_id.to_string())
        .details(details);
    if let Err(e) = entry.log(&state.db).await {
        tracing::error!(error = %e, "failed to write replay audit record");
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id, "state": "running" })),
    ))
}

/// GET /orgs/{org}/replay/{job_id} — poll a job (tenant-scoped).
#[utoipa::path(
    get,
    path = "/orgs/{org}/replay/{job_id}",
    tag = "replay",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("job_id" = Uuid, Path, description = "Replay job ID")
    ),
    responses(
        (status = 200, description = "Replay job status"),
        (status = 404, description = "Replay job not found")
    ),
    security(("bearer_jwt" = []))
)]
async fn get_replay(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, job_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let org_id = authorize_admin(&state, &user, &org).await?;
    let job = state
        .replay_jobs
        .get(&job_id)
        .filter(|j| j.org_id == org_id)
        .map(|j| j.value().clone())
        .ok_or_else(|| ApiError::NotFound(format!("replay job '{job_id}' not found")))?;
    Ok(Json(job.status_json(job_id)))
}

/// Map a start failure onto the API vocabulary: configuration problems are the
/// caller's to fix (404/400/503), never a blanket 500.
fn map_start_error(e: String) -> ApiError {
    if e.contains("decision store not configured") {
        ApiError::ServiceUnavailable(e)
    } else if e.contains("not found") {
        ApiError::NotFound(e)
    } else {
        ApiError::BadRequest(e)
    }
}
