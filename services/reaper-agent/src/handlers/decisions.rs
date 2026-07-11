//! Decision logging handlers (OPA-style audit logging).
//!
//! This module contains handlers for decision log operations:
//! - `get_decisions` - Query recent decisions with filtering
//! - `get_decision_stats` - Get decision buffer statistics
//! - `export_decisions` - Export decisions as NDJSON for SIEM integration

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Json, Response},
};
use policy_engine::DecisionFilter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::instrument;
use utoipa::ToSchema;

use crate::observability::{
    DECISION_LOG_AUDIT_COMPROMISED, DECISION_LOG_BUFFER_SIZE, DECISION_LOG_DROPPED_ENTRIES,
    DECISION_LOG_ENTRIES, DECISION_LOG_FLUSHES, DECISION_LOG_SAMPLED_OUT,
    DECISION_LOG_WRITER_DROPPED,
};
use crate::state::AgentState;

// ============================================================================
// Decision Query Types
// ============================================================================

/// Query parameters for decision log endpoint.
#[derive(Debug, Deserialize)]
pub struct DecisionQueryParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub principal: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub decision: Option<String>,
    pub policy_id: Option<String>,
}

/// Export request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportRequest {
    /// Export format: "ndjson" (default), "json"
    pub format: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// Get recent decisions from the decision buffer.
///
/// Supports filtering by principal, action, resource, decision, and policy_id.
/// Use limit and offset for pagination.
#[utoipa::path(
    get,
    path = "/api/v1/decisions",
    tag = "decisions",
    responses(
        (status = 200, description = "Recent decisions")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state))]
pub async fn get_decisions(
    State(state): State<Arc<AgentState>>,
    Query(params): Query<DecisionQueryParams>,
) -> Result<Json<Value>, StatusCode> {
    let Some(buffer) = &state.decision_buffer else {
        return Ok(Json(json!({
            "enabled": false,
            "message": "Decision logging is not enabled. Set REAPER_DECISION_LOG_ENABLED=true",
            "decisions": []
        })));
    };

    let limit = params.limit.unwrap_or(100).min(1000);

    // Build filter if any query params are provided
    let decisions = if params.principal.is_some()
        || params.action.is_some()
        || params.resource.is_some()
        || params.decision.is_some()
        || params.policy_id.is_some()
    {
        let mut filter = DecisionFilter::new();
        if let Some(p) = params.principal {
            filter = filter.with_principal(p);
        }
        if let Some(a) = params.action {
            filter = filter.with_action(a);
        }
        if let Some(r) = params.resource {
            filter = filter.with_resource(r);
        }
        if let Some(d) = params.decision {
            filter = filter.with_decision(d);
        }
        if let Some(pid) = params.policy_id {
            filter = filter.with_policy_id(pid);
        }
        buffer.query(filter, limit)
    } else if let Some(offset) = params.offset {
        buffer.get_page(offset, limit)
    } else {
        buffer.get_recent(limit)
    };

    // Update Prometheus metrics
    let stats = buffer.stats();
    DECISION_LOG_ENTRIES.set(stats.total_entries as f64);
    DECISION_LOG_BUFFER_SIZE.set(stats.buffer_size as f64);
    DECISION_LOG_FLUSHES.set(stats.flush_count as f64);

    Ok(Json(json!({
        "enabled": true,
        "count": decisions.len(),
        "decisions": decisions
    })))
}

/// Explain a single decision by `decision_id` — returns the full record
/// including the `input_data` snapshot (the resolved principal/resource
/// attributes the decision branched on) when the explain tier was enabled.
#[utoipa::path(
    get,
    path = "/api/v1/decisions/{decision_id}",
    tag = "decisions",
    params(
        ("decision_id" = String, Path, description = "Decision ID")
    ),
    responses(
        (status = 200, description = "Decision record"),
        (status = 404, description = "Decision not found")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state))]
pub async fn get_decision_by_id(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path(decision_id): axum::extract::Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let Some(buffer) = &state.decision_buffer else {
        return Ok(Json(json!({
            "enabled": false,
            "message": "Decision logging is not enabled. Set REAPER_DECISION_LOG_ENABLED=true"
        })));
    };

    match buffer.find_by_decision_id(&decision_id) {
        Some(entry) => Ok(Json(json!({ "enabled": true, "decision": entry }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Get decision buffer statistics.
#[utoipa::path(
    get,
    path = "/api/v1/decisions/stats",
    tag = "decisions",
    responses(
        (status = 200, description = "Decision buffer statistics")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state))]
pub async fn get_decision_stats(
    State(state): State<Arc<AgentState>>,
) -> Result<Json<Value>, StatusCode> {
    let Some(buffer) = &state.decision_buffer else {
        return Ok(Json(json!({
            "enabled": false,
            "message": "Decision logging is not enabled"
        })));
    };

    let stats = buffer.stats();

    // Update Prometheus metrics
    DECISION_LOG_ENTRIES.set(stats.total_entries as f64);
    DECISION_LOG_BUFFER_SIZE.set(stats.buffer_size as f64);
    DECISION_LOG_FLUSHES.set(stats.flush_count as f64);
    DECISION_LOG_SAMPLED_OUT.set(stats.sampled_out as f64);
    DECISION_LOG_WRITER_DROPPED.set(stats.writer_dropped as f64);
    DECISION_LOG_DROPPED_ENTRIES.set(stats.dropped_entries as f64);
    DECISION_LOG_AUDIT_COMPROMISED.set(if stats.audit_compromised { 1.0 } else { 0.0 });

    Ok(Json(json!({
        "enabled": true,
        "total_entries": stats.total_entries,
        "buffer_size": stats.buffer_size,
        "buffer_capacity": stats.buffer_capacity,
        "dropped_entries": stats.dropped_entries,
        "writer_dropped": stats.writer_dropped,
        "sampled_out": stats.sampled_out,
        "flush_count": stats.flush_count,
        "allow_count": stats.allow_count,
        "deny_count": stats.deny_count,
        "audit_required": buffer.audit_required(),
        "audit_compromised": stats.audit_compromised,
        "config": buffer.config()
    })))
}

/// Export decisions as NDJSON (for SIEM integration).
///
/// Supports two formats:
/// - "ndjson" (default): Newline-delimited JSON
/// - "json": Pretty-printed JSON array
#[utoipa::path(
    post,
    path = "/api/v1/decisions/export",
    tag = "decisions",
    request_body = ExportRequest,
    responses(
        (status = 200, description = "Exported decisions")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state))]
pub async fn export_decisions(
    State(state): State<Arc<AgentState>>,
    Json(request): Json<ExportRequest>,
) -> Result<Response<String>, StatusCode> {
    let Some(buffer) = &state.decision_buffer else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let format = request.format.unwrap_or_else(|| "ndjson".to_string());

    match format.as_str() {
        "ndjson" => {
            let ndjson = buffer.export_ndjson();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/x-ndjson")
                .header(
                    "Content-Disposition",
                    "attachment; filename=\"decisions.ndjson\"",
                )
                .body(ndjson)
                .unwrap())
        }
        "json" => {
            let decisions = buffer.get_recent(10000);
            let json = serde_json::to_string_pretty(&decisions).unwrap_or_default();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header(
                    "Content-Disposition",
                    "attachment; filename=\"decisions.json\"",
                )
                .body(json)
                .unwrap())
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_export_format() {
        let request = ExportRequest { format: None };
        let format = request.format.unwrap_or_else(|| "ndjson".to_string());
        assert_eq!(format, "ndjson");
    }
}
