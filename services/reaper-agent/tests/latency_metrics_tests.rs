//! Plan 08 Phase D — honest latency histograms.
//!
//! Both evaluation endpoints must observe **request-total** latency (handler
//! entry → serialized response) into `reaper_decision_duration_seconds`, and
//! the engine-only slice into the separate `reaper_engine_eval_seconds`
//! series. Before Phase D the standard endpoint fed the engine slice into the
//! total series, so dashboards understated p99; these tests pin the split.

use std::sync::Arc;

use axum::body::Bytes;
use axum::{
    extract::{Json, State},
    response::IntoResponse,
};
use policy_engine::{
    cache_config::CacheConfig, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule,
};
use reaper_agent::handlers::{evaluate_policy, fast_evaluate_policy};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::observability::gather_metrics;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_agent::types::EvaluateRequest;
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};

fn simple_allow(name: &str, resource: &str) -> EnhancedPolicy {
    EnhancedPolicy::new(
        name.to_string(),
        String::new(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: resource.to_string(),
            conditions: vec![],
        }],
    )
}

fn state() -> Arc<AgentState> {
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
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: Arc::new(DataSyncState::from_env()),
        bundle_verifier: Arc::new(BundleVerifier::from_config(&ManagementSettings::default())),
    })
}

/// Extract the sample value for `metric{policy_name="<policy>"}` from the
/// Prometheus text exposition.
fn sample(metrics_text: &str, metric: &str, policy: &str) -> Option<f64> {
    let needle = format!("{metric}{{policy_name=\"{policy}\"}}");
    metrics_text.lines().find_map(|line| {
        line.strip_prefix(needle.as_str())
            .and_then(|rest| rest.trim().parse::<f64>().ok())
    })
}

#[tokio::test]
async fn standard_endpoint_feeds_total_and_engine_series_and_they_differ() {
    // Unique policy name isolates this test's label from the shared global
    // Prometheus registry.
    let policy = "phase-d-metrics-standard";
    let state = state();
    state
        .policy_engine
        .deploy_policy(simple_allow(policy, "/doc"))
        .unwrap();

    let resp = evaluate_policy(
        State(state),
        Json(EvaluateRequest {
            policy_id: None,
            policy_name: Some(policy.to_string()),
            principal: "alice".to_string(),
            resource: "/doc".to_string(),
            action: "read".to_string(),
            context: None,
        }),
    )
    .await
    .expect("handler returned an error status")
    .into_response();
    assert!(resp.status().is_success());

    let text = gather_metrics().expect("gather metrics");

    let total_count = sample(&text, "reaper_decision_duration_seconds_count", policy)
        .expect("request-total series missing for policy");
    let engine_count = sample(&text, "reaper_engine_eval_seconds_count", policy)
        .expect("engine-slice series missing for policy");
    assert_eq!(total_count, 1.0);
    assert_eq!(engine_count, 1.0);

    // The two series measure different things: request-total includes the
    // cache probe, logging, and response serialization on top of the engine
    // slice, so its sum must be strictly larger.
    let total_sum = sample(&text, "reaper_decision_duration_seconds_sum", policy).unwrap();
    let engine_sum = sample(&text, "reaper_engine_eval_seconds_sum", policy).unwrap();
    assert!(
        total_sum > engine_sum,
        "request-total ({total_sum}) must exceed engine slice ({engine_sum})"
    );
}

#[tokio::test]
async fn fast_endpoint_feeds_total_and_engine_series() {
    let policy = "phase-d-metrics-fast";
    let state = state();
    state
        .policy_engine
        .deploy_policy(simple_allow(policy, "/doc"))
        .unwrap();

    let body = serde_json::json!({
        "policy_name": policy,
        "principal": "alice",
        "resource": "/doc",
        "action": "read",
    });
    let resp = fast_evaluate_policy(
        State(state),
        Bytes::from(serde_json::to_vec(&body).unwrap()),
    )
    .await
    .expect("handler returned an error status")
    .into_response();
    assert!(resp.status().is_success());

    let text = gather_metrics().expect("gather metrics");

    let total_count = sample(&text, "reaper_decision_duration_seconds_count", policy)
        .expect("request-total series missing for policy");
    let engine_count = sample(&text, "reaper_engine_eval_seconds_count", policy)
        .expect("engine-slice series missing for policy");
    assert_eq!(total_count, 1.0);
    assert_eq!(engine_count, 1.0);

    let total_sum = sample(&text, "reaper_decision_duration_seconds_sum", policy).unwrap();
    let engine_sum = sample(&text, "reaper_engine_eval_seconds_sum", policy).unwrap();
    assert!(
        total_sum > engine_sum,
        "request-total ({total_sum}) must exceed engine slice ({engine_sum})"
    );
}
