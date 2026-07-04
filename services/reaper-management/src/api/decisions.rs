//! Decision-log query API (control plane).
//!
//! Tenant-scoped queries over the central ClickHouse decision store — the
//! full history across every agent in the org, unlike the per-agent in-memory
//! ring exposed on the agents themselves. Backed by
//! [`crate::decisions::DecisionStore`]; disabled (503 + setup guidance) until
//! `REAPER_CLICKHOUSE_URL` is configured.

use axum::{
    extract::{Path, Query, State},
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::OrganizationRepository,
    decisions::{DecisionQuery, DecisionStoreError},
    state::AppState,
};

/// Build decision-query routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/orgs/{org}/decisions", get(list_decisions))
        .route("/orgs/{org}/decisions/stats", get(decision_stats))
        .route("/orgs/{org}/decisions/{decision_id}", get(get_decision))
}

#[derive(Debug, Deserialize)]
pub struct StatsParams {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Authorize the caller for decision reads on `org` and return the tenant id
/// used in ClickHouse (the org UUID string, matching what Vector injects).
async fn authorize(
    state: &AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    org_ref: &str,
) -> ApiResult<String> {
    // Decision logs are audit data: allow agent readers and org admins.
    if !user.has_permission(Scope::AgentRead)
        && !user.has_permission(Scope::OrgAdmin)
        && !user.has_permission(Scope::Admin)
    {
        return Err(ApiError::Forbidden(
            "Missing agent:read or org:admin scope".to_string(),
        ));
    }
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access decisions for other organizations".to_string(),
        ));
    }
    Ok(organization.id.to_string())
}

fn map_store_error(e: DecisionStoreError) -> ApiError {
    match e {
        DecisionStoreError::NotConfigured => ApiError::ServiceUnavailable(e.to_string()),
        DecisionStoreError::Http(msg) => {
            ApiError::ServiceUnavailable(format!("decision store unreachable: {msg}"))
        }
        DecisionStoreError::Query(msg) | DecisionStoreError::Parse(msg) => {
            ApiError::Internal(format!("decision store error: {msg}"))
        }
    }
}

fn store_or_unavailable(state: &AppState) -> ApiResult<&crate::decisions::DecisionStore> {
    state
        .decision_store
        .as_deref()
        .ok_or_else(|| map_store_error(DecisionStoreError::NotConfigured))
}

/// GET /api/v1/orgs/{org}/decisions — full-history, cross-agent, tenant-scoped.
async fn list_decisions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<DecisionQuery>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let decisions = store.list(&tenant, &query).await.map_err(map_store_error)?;
    Ok(Json(json!({
        "count": decisions.len(),
        "decisions": decisions,
    })))
}

/// GET /api/v1/orgs/{org}/decisions/stats
async fn decision_stats(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<StatsParams>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let stats = store
        .stats(&tenant, params.from.as_deref(), params.to.as_deref())
        .await
        .map_err(map_store_error)?;
    Ok(Json(serde_json::to_value(stats).unwrap_or_default()))
}

/// GET /api/v1/orgs/{org}/decisions/{decision_id} — explain view for one
/// decision (includes `input_data`, possibly an encryption envelope the
/// tenant's key holder can open).
async fn get_decision(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, decision_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    match store
        .get_by_id(&tenant, &decision_id)
        .await
        .map_err(map_store_error)?
    {
        Some(decision) => Ok(Json(json!({ "decision": decision }))),
        None => Err(ApiError::NotFound(format!(
            "Decision '{decision_id}' not found"
        ))),
    }
}
