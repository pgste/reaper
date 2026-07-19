//! PERF R2-P2-3 — the request-total SLA histogram must be fed on EVERY
//! served return path, not just success/cache-hit. Early-return denies
//! (`data_stale`, `policy_not_found`, `evaluate_all_disabled`,
//! `no_policies_loaded`, `candidate_cap_exceeded`, fast-path `parse_error`)
//! are served requests; if they skip `reaper_decision_duration_seconds`, a
//! denial storm renders the SLA dashboard silent while the agent answers at
//! line rate.
//!
//! Early-return observations land under the constant policy label
//! `early_deny` (bounded cardinality; the deny *reason* is already counted by
//! `ERRORS_TOTAL`). The Prometheus registry is process-global, so all
//! assertions run inside ONE test function with exact sample-count deltas —
//! no concurrent test in this binary can observe the same label between a
//! snapshot and its assertion.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Json, State},
    response::IntoResponse,
};
use policy_engine::{
    cache_config::CacheConfig, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule,
};
use reaper_agent::handlers::{evaluate_policy, fast_evaluate_policy};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::observability::DECISION_DURATION;
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

fn state_with(
    allow_evaluate_all: bool,
    max_candidate_policies: usize,
    data_sync: DataSyncState,
) -> Arc<AgentState> {
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
        data_sync: Arc::new(data_sync),
        bundle_verifier: Arc::new(BundleVerifier::from_config(&ManagementSettings::default())),
        capability_gate: std::sync::Arc::new(
            reaper_agent::capability_cache::CapabilityGateRuntime::from_auth(
                &reaper_core::config::AgentAuthSettings::default(),
            ),
        ),
    })
}

fn state(allow_evaluate_all: bool, max_candidate_policies: usize) -> Arc<AgentState> {
    state_with(
        allow_evaluate_all,
        max_candidate_policies,
        DataSyncState::from_env(),
    )
}

fn eval_request(policy_id: Option<&str>, resource: &str) -> EvaluateRequest {
    EvaluateRequest {
        policy_id: policy_id.map(str::to_string),
        policy_name: None,
        principal: "alice".to_string(),
        resource: resource.to_string(),
        action: "read".to_string(),
        context: None,
        actor: None,
        context_provenance: None,
        capability: None,
    }
}

/// Drive the standard endpoint and return the decoded JSON body.
async fn standard(state: Arc<AgentState>, req: EvaluateRequest) -> serde_json::Value {
    let resp = evaluate_policy(State(state), Json(req))
        .await
        .expect("handler returned an error status")
        .into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse json body")
}

/// Drive the fast endpoint with a raw body and return the decoded JSON body.
async fn fast(state: Arc<AgentState>, body: &str) -> serde_json::Value {
    let resp = fast_evaluate_policy(State(state), Bytes::from(body.to_string()))
        .await
        .expect("handler returned an error status")
        .into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse json body")
}

fn early_deny_samples() -> u64 {
    DECISION_DURATION
        .with_label_values(&["early_deny"])
        .get_sample_count()
}

#[tokio::test]
async fn every_early_return_deny_feeds_the_sla_histogram() {
    // -------- standard endpoint --------

    // policy_not_found (policy_id is not a UUID and not a known name)
    let before = early_deny_samples();
    let s = state(false, 256);
    let body = standard(s, eval_request(Some("no-such-policy"), "/doc")).await;
    assert_eq!(body["matched_rule"], "policy_not_found");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "policy_not_found deny must observe request-total"
    );

    // evaluate_all_disabled (default: evaluate-all is off)
    let before = early_deny_samples();
    let s = state(false, 256);
    s.policy_engine
        .deploy_policy(simple_allow("p", "/doc"))
        .unwrap();
    let body = standard(s, eval_request(None, "/doc")).await;
    assert_eq!(body["matched_rule"], "evaluate_all_disabled");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "evaluate_all_disabled deny must observe request-total"
    );

    // no_policies_loaded (armed, empty engine)
    let before = early_deny_samples();
    let s = state(true, 256);
    let body = standard(s, eval_request(None, "/doc")).await;
    assert_eq!(body["matched_rule"], "no_policies_loaded");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "no_policies deny must observe request-total"
    );

    // candidate_cap_exceeded (armed, cap 1, two unprunable wildcard policies)
    let before = early_deny_samples();
    let s = state(true, 1);
    s.policy_engine
        .deploy_policy(simple_allow("wild-a", "*"))
        .unwrap();
    s.policy_engine
        .deploy_policy(simple_allow("wild-b", "*"))
        .unwrap();
    let body = standard(s, eval_request(None, "/doc")).await;
    assert_eq!(body["matched_rule"], "candidate_cap_exceeded");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "candidate_cap_exceeded deny must observe request-total"
    );

    // data_stale gate (REAPER_DATA_REQUIRE_SYNC armed, no verified snapshot)
    let before = early_deny_samples();
    let mut data_sync = DataSyncState::from_env();
    data_sync.require_sync = true; // cold-start gate armed, version stays 0
    let s = state_with(false, 256, data_sync);
    s.policy_engine
        .deploy_policy(simple_allow("p", "/doc"))
        .unwrap();
    let body = standard(s, eval_request(Some("p"), "/doc")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "awaiting_initial_data_sync");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "data-sync gate deny must observe request-total"
    );

    // -------- fast endpoint --------

    // parse_error
    let before = early_deny_samples();
    let s = state(false, 256);
    let body = fast(s, "{not json").await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "fast-path parse_error deny must observe request-total"
    );

    // policy_not_found
    let before = early_deny_samples();
    let s = state(false, 256);
    let body = fast(
        s,
        r#"{"policy_id":"no-such-policy","principal":"alice","resource":"/doc","action":"read"}"#,
    )
    .await;
    assert_eq!(body["error"], "policy_not_found");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "fast-path policy_not_found deny must observe request-total"
    );

    // evaluate_all_disabled
    let before = early_deny_samples();
    let s = state(false, 256);
    s.policy_engine
        .deploy_policy(simple_allow("p", "/doc"))
        .unwrap();
    let body = fast(
        s,
        r#"{"principal":"alice","resource":"/doc","action":"read"}"#,
    )
    .await;
    assert_eq!(body["error"], "evaluate_all_disabled");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "fast-path evaluate_all_disabled deny must observe request-total"
    );

    // candidate_cap_exceeded
    let before = early_deny_samples();
    let s = state(true, 1);
    s.policy_engine
        .deploy_policy(simple_allow("wild-c", "*"))
        .unwrap();
    s.policy_engine
        .deploy_policy(simple_allow("wild-d", "*"))
        .unwrap();
    let body = fast(
        s,
        r#"{"principal":"alice","resource":"/doc","action":"read"}"#,
    )
    .await;
    assert_eq!(body["error"], "candidate_cap_exceeded");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "fast-path candidate_cap_exceeded deny must observe request-total"
    );

    // no_policies_loaded
    let before = early_deny_samples();
    let s = state(true, 256);
    let body = fast(
        s,
        r#"{"principal":"alice","resource":"/doc","action":"read"}"#,
    )
    .await;
    assert_eq!(body["error"], "no_policies_loaded");
    assert_eq!(
        early_deny_samples(),
        before + 1,
        "fast-path no_policies deny must observe request-total"
    );

    // -------- sanity: the success path stays on its per-policy label --------
    let before_early = early_deny_samples();
    let before_policy = DECISION_DURATION
        .with_label_values(&["served"])
        .get_sample_count();
    let s = state(false, 256);
    s.policy_engine
        .deploy_policy(simple_allow("served", "/doc"))
        .unwrap();
    let body = standard(s, eval_request(Some("served"), "/doc")).await;
    assert_eq!(body["decision"], "allow");
    assert_eq!(
        early_deny_samples(),
        before_early,
        "a successful decision must NOT observe under early_deny"
    );
    assert_eq!(
        DECISION_DURATION
            .with_label_values(&["served"])
            .get_sample_count(),
        before_policy + 1,
        "a successful decision observes request-total under its policy label"
    );
}
