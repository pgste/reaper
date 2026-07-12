//! Plan 08 Phase A, ADR-2 — evaluate-all fan-out controls on the served path.
//!
//! A policy-less request (no `policy_id`/`policy_name`) is a DoS amplifier, so:
//!   * it fails **closed** by default (`allow_evaluate_all=false`) with
//!     `evaluate_all_disabled`;
//!   * even when armed, it is rejected with `candidate_cap_exceeded` when the
//!     pruning index yields more candidates than `max_candidate_policies`,
//!     instead of fanning out to an N-eval;
//!   * a within-cap armed request is served normally via the pruned candidate set.

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    response::IntoResponse,
};
use policy_engine::{
    cache_config::CacheConfig, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule,
};
use reaper_agent::handlers::evaluate_policy;
use reaper_agent::management::verify::BundleVerifier;
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

/// Build agent state with the evaluate-all knobs set.
fn state(allow_evaluate_all: bool, max_candidate_policies: usize) -> Arc<AgentState> {
    let mut agent_config = ReaperAgentConfig::default();
    agent_config.performance.allow_evaluate_all = allow_evaluate_all;
    agent_config.performance.max_candidate_policies = max_candidate_policies;

    Arc::new(AgentState {
        policy_engine: PolicyEngine::new(),
        data_store: Arc::new(policy_engine::DataStore::new()),
        stats: Arc::new(AgentStats::new(false)),
        decision_cache: None,
        cache_config: CacheConfig::default(),
        agent_config,
        policy_cache: None,
        decision_buffer: None,
        agent_id: "test-agent".to_string(),
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: Arc::new(DataSyncState::from_env()),
        bundle_verifier: Arc::new(BundleVerifier::from_config(&ManagementSettings::default())),
    })
}

fn evaluate_all_request(resource: &str) -> EvaluateRequest {
    EvaluateRequest {
        policy_id: None,
        policy_name: None,
        principal: "alice".to_string(),
        resource: resource.to_string(),
        action: "read".to_string(),
        context: None,
    }
}

async fn body_json(state: Arc<AgentState>, req: EvaluateRequest) -> serde_json::Value {
    let resp = evaluate_policy(State(state), Json(req))
        .await
        .expect("handler returned an error status")
        .into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse json body")
}

#[tokio::test]
async fn evaluate_all_disabled_by_default() {
    let state = state(false, 256);
    // Even with policies loaded, a policy-less request is denied closed.
    state
        .policy_engine
        .deploy_policy(simple_allow("p", "/doc"))
        .unwrap();

    let body = body_json(state, evaluate_all_request("/doc")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "evaluate_all_disabled");
}

#[tokio::test]
async fn evaluate_all_over_cap_is_rejected() {
    // Armed, but cap of 2 with 3 wildcard (unprunable) policies => every
    // request has 3 candidates > cap => candidate_cap_exceeded, not a 3-eval.
    let state = state(true, 2);
    for i in 0..3 {
        let mut p = simple_allow(&format!("wild{i}"), "*");
        p.rules[0].action = PolicyAction::Allow;
        state.policy_engine.deploy_policy(p).unwrap();
    }

    let body = body_json(state, evaluate_all_request("/anything")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "candidate_cap_exceeded");
}

#[tokio::test]
async fn evaluate_all_within_cap_is_served_via_pruning() {
    // Armed, generous cap. 500 unrelated policies + one matching /doc. The
    // pruned candidate set for /doc is 1, well under the cap, and the matching
    // allow decides — proving we did NOT fan out to all 501.
    let state = state(true, 256);
    for i in 0..500 {
        state
            .policy_engine
            .deploy_policy(simple_allow(&format!("noise{i}"), &format!("/noise/{i}")))
            .unwrap();
    }
    state
        .policy_engine
        .deploy_policy(simple_allow("doc-allow", "/doc"))
        .unwrap();

    let body = body_json(state, evaluate_all_request("/doc")).await;
    assert_eq!(body["decision"], "allow");
}

#[tokio::test]
async fn evaluate_all_armed_but_no_policies_denies_no_policies_loaded() {
    let state = state(true, 256);
    let body = body_json(state, evaluate_all_request("/doc")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "no_policies_loaded");
}
