//! Health check and metrics handlers.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Json, Response},
};
use prometheus::{Encoder, TextEncoder};
use reaper_core::VERSION;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::instrument;

use crate::metrics::{AGENTS_TOTAL, BUNDLES_TOTAL, POLICIES_TOTAL};
use crate::state::PlatformState;

#[instrument]
pub async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-platform",
        "version": VERSION,
        "capabilities": [
            "policy-management",
            "hot-swapping",
            "agent-orchestration"
        ]
    })))
}

#[instrument(skip(state))]
pub async fn metrics(State(state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    let engine_stats = state.policy_engine.get_stats();
    let deployment_stats = state.deployment_stats.read();

    // Update Prometheus gauges
    POLICIES_TOTAL.set(engine_stats.total_policies as f64);
    BUNDLES_TOTAL.set(state.bundle_storage.read().len() as f64);
    AGENTS_TOTAL.set(state.agents.read().len() as f64);

    Ok(Json(json!({
        "service": "reaper-platform",
        "policies": {
            "total": engine_stats.total_policies,
            "has_default": engine_stats.has_default_policy
        },
        "deployments": {
            "total": deployment_stats.total_deployments,
            "successful": deployment_stats.successful_deployments,
            "failed": deployment_stats.failed_deployments,
            "success_rate": if deployment_stats.total_deployments > 0 {
                (deployment_stats.successful_deployments as f64 / deployment_stats.total_deployments as f64) * 100.0
            } else {
                100.0
            }
        },
        "uptime_seconds": 0, // TODO: Add actual uptime tracking
        "memory_usage_mb": 0, // TODO: Add actual memory tracking
    })))
}

/// Prometheus metrics endpoint (text format for scraping)
pub async fn prometheus_metrics(State(state): State<Arc<PlatformState>>) -> Response {
    // Update gauges before encoding
    let engine_stats = state.policy_engine.get_stats();
    POLICIES_TOTAL.set(engine_stats.total_policies as f64);
    BUNDLES_TOTAL.set(state.bundle_storage.read().len() as f64);
    AGENTS_TOTAL.set(state.agents.read().len() as f64);

    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    let body = String::from_utf8(buffer).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(body.into())
        .unwrap()
}
