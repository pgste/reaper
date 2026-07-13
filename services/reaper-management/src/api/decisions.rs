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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    api::error::{ApiError, ApiResult, ProblemDetails},
    api::orgs::resolve_org,
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::OrganizationRepository,
    decisions::{DecisionQuery, DecisionRow, DecisionStats, DecisionStoreError, TimeseriesPoint},
    state::AppState,
};

/// Build decision-query routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_decisions))
        .routes(routes!(decision_stats))
        .routes(routes!(decision_timeseries))
        .routes(routes!(decision_facets))
        .routes(routes!(verify_decisions))
        .routes(routes!(get_decision))
}

#[derive(Debug, Deserialize)]
pub struct StatsParams {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyParams {
    /// Restrict to a single writer-boot chain (per-boot `chain_id`).
    pub chain: Option<String>,
    /// Inclusive lower `seq` bound.
    pub from: Option<u64>,
    /// Inclusive upper `seq` bound.
    pub to: Option<u64>,
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

/// One page of decisions; pass `next_cursor` back as `?cursor=` to resume.
/// `next_cursor` is `null` on the last page.
#[derive(Debug, Serialize, ToSchema)]
struct DecisionListResponse {
    items: Vec<DecisionRow>,
    /// Opaque resume cursor; `null` when this is the last page.
    next_cursor: Option<String>,
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
        (status = 200, description = "One page of decisions with a next_cursor to resume",
         body = DecisionListResponse),
        (status = 400, description = "cursor invalid", body = ProblemDetails),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_decisions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<DecisionQuery>,
) -> ApiResult<Json<DecisionListResponse>> {
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

    Ok(Json(DecisionListResponse {
        items: decisions,
        next_cursor,
    }))
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
        (status = 200, description = "Decision statistics for the organization",
         body = DecisionStats),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn decision_stats(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<StatsParams>,
) -> ApiResult<Json<DecisionStats>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let stats = store
        .stats(&tenant, params.from.as_deref(), params.to.as_deref())
        .await
        .map_err(map_store_error)?;
    Ok(Json(stats))
}

/// Bucketed decision counts for charts.
#[derive(Debug, Serialize, ToSchema)]
struct TimeseriesResponse {
    /// Bucket size in seconds.
    interval_secs: u32,
    points: Vec<TimeseriesPoint>,
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
        (status = 200, description = "Bucketed decision counts for charts",
         body = TimeseriesResponse),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn decision_timeseries(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<TimeseriesParams>,
) -> ApiResult<Json<TimeseriesResponse>> {
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
    Ok(Json(TimeseriesResponse {
        interval_secs: bucket_secs,
        points,
    }))
}

/// Distinct filter values with counts for UI dropdowns.
#[derive(Debug, Serialize, ToSchema)]
struct FacetsResponse {
    /// Facet name → `[value, count]` pairs (dimensions are store-defined,
    /// so the leaf stays dynamic).
    #[schema(value_type = Object)]
    facets: Value,
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
        (status = 200, description = "Distinct filter values with counts",
         body = FacetsResponse),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn decision_facets(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<StatsParams>,
) -> ApiResult<Json<FacetsResponse>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    let facets = store
        .facets(&tenant, params.from.as_deref(), params.to.as_deref())
        .await
        .map_err(map_store_error)?;
    Ok(Json(FacetsResponse { facets }))
}

/// Hash-chain verification report (`policy_engine::decision_log::VerifyReport`
/// shape, flattened) plus an optional explanatory note.
#[derive(Debug, Serialize, ToSchema)]
struct VerifyDecisionsResponse {
    /// The `verify_records` report: chains checked, entries verified,
    /// checkpoint signature results, and any breaks found (engine-defined
    /// shape; see `policy_engine::decision_log::VerifyReport`).
    #[serde(flatten)]
    #[schema(value_type = Object)]
    report: Value,
    /// Present when checkpoint signatures were skipped (no verifying key
    /// configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

/// GET /api/v1/orgs/{org}/decisions/verify — verify the tamper-evident hash
/// chain over the store (round-2 A1). Reconstructs `DecisionLogEntry` /
/// `Checkpoint` rows, groups by per-boot `chain_id`, and runs
/// `decision_log::verify_records`. Checkpoint signatures are checked against the
/// keys in `REAPER_DECISION_LOG_CHECKPOINT_VERIFYING_KEY` (comma-separated
/// `key_id:hex`); with none configured, chains are still verified and
/// `checkpoints_verified` is 0 with an explanatory note.
#[utoipa::path(
    get,
    path = "/orgs/{org}/decisions/verify",
    tag = "decisions",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("chain" = Option<String>, Query, description = "Restrict to one writer-boot chain_id"),
        ("from" = Option<u64>, Query, description = "Inclusive lower seq bound"),
        ("to" = Option<u64>, Query, description = "Inclusive upper seq bound")
    ),
    responses(
        (status = 200, description = "Verification report over the store-backed hash chain",
         body = VerifyDecisionsResponse),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn verify_decisions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(params): Query<VerifyParams>,
) -> ApiResult<Json<VerifyDecisionsResponse>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;

    let tenant_arg = store.tenant_filter().then_some(tenant.as_str());
    let (dec_rows, cp_rows) = store
        .verify_range(tenant_arg, params.chain.as_deref(), params.from, params.to)
        .await
        .map_err(map_store_error)?;

    // Reconstruct the typed records from the raw store rows.
    let mut entries = Vec::with_capacity(dec_rows.len());
    for row in dec_rows {
        let entry = policy_engine::decision_log::entry_from_store_row(row)
            .map_err(|e| ApiError::Internal(format!("decision row reconstruction failed: {e}")))?;
        entries.push(entry);
    }
    let mut checkpoints = Vec::with_capacity(cp_rows.len());
    for row in cp_rows {
        let cp = policy_engine::decision_log::checkpoint_from_store_row(row).map_err(|e| {
            ApiError::Internal(format!("checkpoint row reconstruction failed: {e}"))
        })?;
        checkpoints.push(cp);
    }

    // Load checkpoint verifying keys from config (env). None → chains only.
    let key_spec =
        std::env::var("REAPER_DECISION_LOG_CHECKPOINT_VERIFYING_KEY").unwrap_or_default();
    let verifying_keys = policy_engine::decision_log::parse_verifying_keys(&key_spec)
        .map_err(|e| ApiError::Internal(format!("invalid checkpoint verifying key config: {e}")))?;

    let mut note: Option<String> = None;
    let checkpoints_for_verify: &[policy_engine::decision_log::Checkpoint] =
        if verifying_keys.is_empty() && !checkpoints.is_empty() {
            note = Some(
                "no REAPER_DECISION_LOG_CHECKPOINT_VERIFYING_KEY configured: verified hash chains \
             only; checkpoint signatures were not checked (checkpoints_verified=0)"
                    .to_string(),
            );
            &[]
        } else {
            &checkpoints
        };

    // Store-backed: the queryable ClickHouse projection is not byte-identical to
    // the signed NDJSON, so verify chain LINKAGE over the stored hashes (sound,
    // no false positives). Byte-exact content re-hashing is the CLI `--file`
    // path over the immutable WORM archive.
    let report = policy_engine::decision_log::verify_records(
        entries,
        checkpoints_for_verify,
        &verifying_keys,
        policy_engine::decision_log::VerifyMode::Linkage,
    );

    let report = serde_json::to_value(&report)
        .map_err(|e| ApiError::Internal(format!("serialize verify report: {e}")))?;
    Ok(Json(VerifyDecisionsResponse { report, note }))
}

/// Explain view for one decision.
#[derive(Debug, Serialize, ToSchema)]
struct DecisionDetailResponse {
    decision: DecisionRow,
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
        (status = 200, description = "Explain view for one decision",
         body = DecisionDetailResponse),
        (status = 403, description = "Caller lacks decision-read access on this org", body = ProblemDetails),
        (status = 404, description = "Decision not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_decision(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, decision_id)): Path<(String, String)>,
) -> ApiResult<Json<DecisionDetailResponse>> {
    let tenant = authorize(&state, &user, &org).await?;
    let store = store_or_unavailable(&state)?;
    match store
        .get_by_id(&tenant, &decision_id)
        .await
        .map_err(map_store_error)?
    {
        Some(decision) => Ok(Json(DecisionDetailResponse { decision })),
        None => Err(ApiError::NotFound(format!(
            "Decision '{decision_id}' not found"
        ))),
    }
}
