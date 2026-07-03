//! Health, readiness, and liveness check handlers.
//!
//! These endpoints support Kubernetes health probes and monitoring systems.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Json, Response},
};
use prometheus::{Encoder, TextEncoder};
use reaper_core::VERSION;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::instrument;

use crate::observability::ACTIVE_POLICIES;
use crate::state::AgentState;

/// Health check endpoint (`/health`).
///
/// Returns comprehensive health information including:
/// - Service status and version
/// - Number of policies loaded
/// - Decision statistics
/// - Cache hit/miss rates
#[instrument(skip(state))]
pub async fn health_check(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    let engine_stats = state.policy_engine.get_stats();

    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-agent",
        "version": VERSION,
        "policies_loaded": engine_stats.total_policies,
        "total_evaluations": state.stats.requests_processed.load(Ordering::Relaxed),
        "decisions_allow": state.stats.decisions_allow.load(Ordering::Relaxed),
        "decisions_deny": state.stats.decisions_deny.load(Ordering::Relaxed),
        "cache_hits": state.stats.decision_cache_hits.load(Ordering::Relaxed),
        "cache_misses": state.stats.decision_cache_misses.load(Ordering::Relaxed),
        "capabilities": [
            "policy-evaluation",
            "hot-swapping",
            "sub-microsecond-latency"
        ]
    })))
}

/// Readiness check endpoint (`/ready`).
///
/// Returns 200 OK if the agent is ready to serve traffic.
/// Returns 503 Service Unavailable if no policies are loaded.
///
/// This endpoint is designed for Kubernetes readiness probes.
#[instrument(skip(state))]
pub async fn readiness_check(
    State(state): State<Arc<AgentState>>,
) -> Result<Json<Value>, StatusCode> {
    let engine_stats = state.policy_engine.get_stats();

    if engine_stats.total_policies == 0 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(json!({
        "status": "ready",
        "policies_loaded": engine_stats.total_policies,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

/// Liveness check endpoint (`/live`).
///
/// Simple check - if the service can respond, it's alive.
/// Returns 200 OK.
///
/// This endpoint is designed for Kubernetes liveness probes.
#[instrument]
pub async fn liveness_check() -> StatusCode {
    StatusCode::OK
}

/// Prometheus metrics endpoint (`/metrics`).
///
/// Returns all registered Prometheus metrics in text format.
#[instrument(skip(state))]
pub async fn metrics(State(state): State<Arc<AgentState>>) -> Result<Response, StatusCode> {
    // Update active policies gauge
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    // Encode metrics to Prometheus text format
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();

    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", encoder.format_type())
        .body(buffer.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentStats;
    use policy_engine::{cache_config::CacheConfig, PolicyEngine};
    use reaper_core::config::ReaperAgentConfig;

    fn create_test_state() -> Arc<AgentState> {
        Arc::new(AgentState {
            policy_engine: PolicyEngine::new(),
            data_store: Arc::new(policy_engine::DataStore::new()),
            stats: Arc::new(AgentStats::new(false)),
            decision_cache: None,
            cache_config: CacheConfig::default(),
            agent_config: ReaperAgentConfig::default(),
            policy_cache: None,
            decision_buffer: None,
            agent_id: "test-agent".to_string(),
            decision_metrics: Arc::new(crate::metrics_cache::DecisionMetrics::new()),
        })
    }

    #[tokio::test]
    async fn test_health_check_basic() {
        let state = create_test_state();

        let result = health_check(State(state)).await;
        assert!(result.is_ok());

        let json = result.unwrap();
        let value = json.0;
        assert_eq!(value["status"], "healthy");
        assert_eq!(value["service"], "reaper-agent");
        assert!(value["version"].is_string());
    }

    #[tokio::test]
    async fn test_health_check_with_stats() {
        let state = create_test_state();

        // Record some stats
        state.stats.record_evaluation(1000);
        state.stats.record_allow();
        state.stats.record_allow();
        state.stats.record_deny();
        state.stats.record_decision_cache_hit();

        let result = health_check(State(state)).await;
        assert!(result.is_ok());

        let json = result.unwrap();
        let value = json.0;
        assert_eq!(value["total_evaluations"], 1);
        assert_eq!(value["decisions_allow"], 2);
        assert_eq!(value["decisions_deny"], 1);
        assert_eq!(value["cache_hits"], 1);
    }

    #[tokio::test]
    async fn test_readiness_no_policies() {
        let state = create_test_state();

        let result = readiness_check(State(state)).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_readiness_with_policies() {
        let state = create_test_state();

        // Deploy a policy
        let policy = policy_engine::EnhancedPolicy::new(
            "test-policy".to_string(),
            "Test".to_string(),
            vec![policy_engine::PolicyRule {
                action: policy_engine::PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        state.policy_engine.deploy_policy(policy).unwrap();

        let result = readiness_check(State(state)).await;
        assert!(result.is_ok());

        let json = result.unwrap();
        assert_eq!(json.0["status"], "ready");
        assert_eq!(json.0["policies_loaded"], 1);
    }

    #[tokio::test]
    async fn test_liveness_check() {
        let result = liveness_check().await;
        assert_eq!(result, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let state = create_test_state();

        let result = metrics(State(state)).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Check content type
        let content_type = response
            .headers()
            .get("Content-Type")
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("");
        assert!(content_type.contains("text/plain"));
    }
}
