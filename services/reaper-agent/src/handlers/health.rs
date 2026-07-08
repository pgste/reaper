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
/// Status code carries the routing decision (200 = ready, 503 = not
/// ready — Kubernetes and load balancers read only this), and the body
/// ALWAYS carries the machine-readable WHY, so SDK health indicators
/// (Spring Actuator, .NET IHealthCheck, …) can surface an actionable
/// reason instead of a bare DOWN:
///
/// ```json
/// {
///   "status": "ready" | "not_ready",
///   "reason": null | "no_policies_loaded" | "awaiting_initial_data_sync"
///           | "data_staleness_exceeded",
///   "policies_loaded": 3,
///   "data_version": 1,
///   "data_applied_seq": 42,
///   "data_staleness_secs": 7,
///   "data_stale": false,
///   "data_staleness_mode": "monitor" | "flag" | "enforce",
///   "data_require_sync": false,
///   "timestamp": "…"
/// }
/// ```
///
/// `data_stale: true` with 200 (flag/monitor mode) is the "degraded but
/// serving" state — map it to a degraded health status client-side.
#[instrument(skip(state))]
pub async fn readiness_check(State(state): State<Arc<AgentState>>) -> (StatusCode, Json<Value>) {
    let engine_stats = state.policy_engine.get_stats();
    let (data_version, _) = state.data_sync.provenance();
    let stale = state.data_sync.is_stale();

    // First blocking reason wins, ordered cold-start → steady-state:
    // no policies yet, then the data cold-start gate, then enforce-mode
    // staleness (fail closed at the traffic layer too).
    let reason = if engine_stats.total_policies == 0 {
        Some("no_policies_loaded")
    } else if state.data_sync.awaiting_initial_sync() {
        tracing::debug!("readiness gated: awaiting initial data sync");
        Some("awaiting_initial_data_sync")
    } else if stale && state.data_sync.mode == crate::state::StalenessMode::Enforce {
        Some("data_staleness_exceeded")
    } else {
        None
    };

    let body = Json(json!({
        "status": if reason.is_none() { "ready" } else { "not_ready" },
        "reason": reason,
        "policies_loaded": engine_stats.total_policies,
        "data_version": data_version,
        "data_applied_seq": state.data_sync.applied_seq.load(std::sync::atomic::Ordering::Acquire),
        "data_staleness_secs": state.data_sync.staleness_secs(),
        "data_stale": stale,
        "data_staleness_mode": match state.data_sync.mode {
            crate::state::StalenessMode::Monitor => "monitor",
            crate::state::StalenessMode::Flag => "flag",
            crate::state::StalenessMode::Enforce => "enforce",
        },
        "data_require_sync": state.data_sync.require_sync,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }));
    let code = if reason.is_none() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, body)
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
        create_test_state_with_sync(crate::state::DataSyncState::from_env())
    }

    fn create_test_state_with_sync(data_sync: crate::state::DataSyncState) -> Arc<AgentState> {
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
            data_sync: std::sync::Arc::new(data_sync),
            bundle_verifier: Arc::new(crate::management::verify::BundleVerifier::from_config(
                &reaper_core::config::ManagementSettings::default(),
            )),
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

        // 503 for routers AND a machine-readable reason for SDK health
        // indicators — never a bare status code.
        let (code, body) = readiness_check(State(state)).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "not_ready");
        assert_eq!(body.0["reason"], "no_policies_loaded");
        assert_eq!(body.0["policies_loaded"], 0);
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

        let (code, body) = readiness_check(State(state)).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.0["status"], "ready");
        assert_eq!(body.0["reason"], serde_json::Value::Null);
        assert_eq!(body.0["policies_loaded"], 1);
        assert_eq!(body.0["data_stale"], false);
        assert!(body.0["data_staleness_mode"].is_string());
    }

    #[tokio::test]
    async fn test_readiness_reports_cold_start_gate_reason() {
        let data_sync = crate::state::DataSyncState {
            version: std::sync::atomic::AtomicI64::new(0),
            checksum: parking_lot::RwLock::new(String::new()),
            last_synced_epoch: std::sync::atomic::AtomicU64::new(0),
            applied_seq: std::sync::atomic::AtomicI64::new(0),
            max_staleness_secs: 0,
            mode: crate::state::StalenessMode::Monitor,
            require_sync: true,
        };
        let state = create_test_state_with_sync(data_sync);

        // Policies loaded, so the ONLY blocker is the data gate — the
        // body must name it ("starting up", not a mystery 503).
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

        let (code, body) = readiness_check(State(state.clone())).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "not_ready");
        assert_eq!(body.0["reason"], "awaiting_initial_data_sync");
        assert_eq!(body.0["data_require_sync"], true);

        // First verified snapshot opens the gate.
        state.data_sync.record_sync(1, "sha256:abc".into());
        let (code, body) = readiness_check(State(state)).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.0["status"], "ready");
        assert_eq!(body.0["reason"], serde_json::Value::Null);
        assert_eq!(body.0["data_version"], 1);
    }

    #[tokio::test]
    async fn test_readiness_reports_staleness_reason() {
        let data_sync = crate::state::DataSyncState {
            version: std::sync::atomic::AtomicI64::new(0),
            checksum: parking_lot::RwLock::new(String::new()),
            last_synced_epoch: std::sync::atomic::AtomicU64::new(0),
            applied_seq: std::sync::atomic::AtomicI64::new(0),
            max_staleness_secs: 10,
            mode: crate::state::StalenessMode::Enforce,
            require_sync: false,
        };
        let state = create_test_state_with_sync(data_sync);
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

        // Synced at epoch 1 (1970) with a 10s budget: stale, enforce mode.
        state.data_sync.record_sync(1, "sha256:abc".into());
        state
            .data_sync
            .last_synced_epoch
            .store(1, std::sync::atomic::Ordering::Release);

        let (code, body) = readiness_check(State(state)).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "not_ready");
        assert_eq!(body.0["reason"], "data_staleness_exceeded");
        assert_eq!(body.0["data_stale"], true);
        assert_eq!(body.0["data_staleness_mode"], "enforce");
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
