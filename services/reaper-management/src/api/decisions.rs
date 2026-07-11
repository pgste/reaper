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
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::OrganizationRepository,
    decisions::{DecisionQuery, DecisionStoreError},
    state::AppState,
};

/// Build decision-query routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_decisions))
        .routes(routes!(decision_stats))
        .routes(routes!(decision_timeseries))
        .routes(routes!(decision_facets))
        .routes(routes!(get_decision))
}

#[derive(Debug, Deserialize)]
pub struct StatsParams {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimeseriesParams {
    pub from: Option<String>,
    pub to: Option<String>,
    /// Bucket size: "30s", "5m", "1h" (default), "1d", or raw seconds.
    pub interval: Option<String>,
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
/// Keyset-paginated (Plan 07 Phase E): pass back `next_cursor` as `?cursor=` to
/// resume; `offset` remains accepted (deprecated) when no cursor is given.
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("limit" = Option<u64>, Query, description = "Page size (default 100, max 1000)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of decisions with a next_cursor to resume"),
        (status = 400, description = "cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_decisions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<DecisionQuery>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;

    // Decode the opaque cursor and fetch page+1 (has-more sentinel).
    let mut query = query;
    if let Some(cursor) = &query.cursor {
        query.after = Some(crate::api::pagination::decode_cursor(cursor)?);
    }
    let limit = query.limit.unwrap_or(100).min(1000);
    query.limit = Some(limit + 1);

    let mut decisions = store.list(&tenant, &query).await.map_err(map_store_error)?;
    let has_more = decisions.len() as u64 > limit;
    if has_more {
        decisions.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        decisions
            .last()
            .map(|d| crate::api::pagination::encode_cursor(&d.timestamp, &d.decision_id))
    } else {
        None
    };

    Ok(Json(json!({
        "items": decisions,
        "next_cursor": next_cursor,
    })))
}

/// GET /api/v1/orgs/{org}/decisions/stats
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions/stats",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Decision statistics for the organization")
    ),
    security(("bearer_jwt" = []))
)]
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

/// GET /api/v1/orgs/{org}/decisions/timeseries — bucketed counts for charts.
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions/timeseries",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Bucketed decision counts for charts")
    ),
    security(("bearer_jwt" = []))
)]
async fn decision_timeseries(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<TimeseriesParams>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let bucket_secs = crate::decisions::parse_interval_secs(params.interval.as_deref());
    let points = store
        .timeseries(
            &tenant,
            params.from.as_deref(),
            params.to.as_deref(),
            bucket_secs,
        )
        .await
        .map_err(map_store_error)?;
    Ok(Json(json!({
        "interval_secs": bucket_secs,
        "points": points,
    })))
}

/// GET /api/v1/orgs/{org}/decisions/facets — distinct filter values with
/// counts (actions, decisions, policy names, agent ids) for UI dropdowns.
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions/facets",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID")
    ),
    responses(
        (status = 200, description = "Distinct filter values with counts")
    ),
    security(("bearer_jwt" = []))
)]
async fn decision_facets(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<StatsParams>,
) -> ApiResult<Json<Value>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let facets = store
        .facets(&tenant, params.from.as_deref(), params.to.as_deref())
        .await
        .map_err(map_store_error)?;
    Ok(Json(json!({ "facets": facets })))
}

/// GET /api/v1/orgs/{org}/decisions/{decision_id} — explain view for one
/// decision (includes `input_data`, possibly an encryption envelope the
/// tenant's key holder can open).
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions/{decision_id}",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("decision_id" = String, Path, description = "Decision ID")
    ),
    responses(
        (status = 200, description = "Explain view for one decision"),
        (status = 404, description = "Decision not found")
    ),
    security(("bearer_jwt" = []))
)]
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
