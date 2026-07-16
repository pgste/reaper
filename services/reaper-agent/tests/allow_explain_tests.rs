//! F1-s4: allow-path explainability on the agent's served path.
//!
//! Pins: an ALLOW response names the rule that allowed it (`matched_rule` is
//! the DSL rule name, not "default_deny"); denies name their deny rule; the
//! decision log captures the input-data explain snapshot — including ACTOR
//! attributes — by default for actor-carrying requests, while plain traffic
//! keeps the opt-in denies-only posture.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    response::IntoResponse,
};
use policy_engine::{
    cache_config::CacheConfig, DecisionFilter, DecisionLogConfig, EnhancedPolicy, PolicyEngine,
    PolicyLanguage, SharedDecisionBuffer,
};
use reaper_agent::handlers::evaluate_policy;
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_agent::types::EvaluateRequest;
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};

const POLICY: &str = r#"
policy agent_explain {
    default: deny,
    rule block_rogues {
        deny if actor.trusted == false
    }
    rule trusted_agents_read {
        allow if actor.trusted == true && context.action == "read"
    }
    rule humans_read {
        allow if user.role == "engineer" && context.action == "read"
    }
}
"#;

fn store() -> Arc<policy_engine::DataStore> {
    let s = Arc::new(policy_engine::DataStore::new());
    let data = serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {"role": "engineer"}},
            {"id": "agent-ci", "type": "agent", "attributes": {"trusted": true, "kind": "agent"}},
            {"id": "agent-rogue", "type": "agent", "attributes": {"trusted": false}},
            {"id": "doc-1", "type": "resource", "attributes": {"env": "prod"}}
        ]
    });
    policy_engine::DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .unwrap();
    s
}

/// Agent state with the explain policy deployed and (optionally) a decision
/// buffer using the given config.
fn state_with_buffer(buffer: Option<SharedDecisionBuffer>) -> Arc<AgentState> {
    let s = store();
    let engine = PolicyEngine::new();
    let mut p = EnhancedPolicy::new_with_language(
        "agent_explain".to_string(),
        String::new(),
        PolicyLanguage::ReaperDsl,
        POLICY.to_string(),
    )
    .unwrap();
    p.build_evaluator_with_data(Some(s.clone())).unwrap();
    engine.deploy_policy(p).unwrap();

    Arc::new(AgentState {
        policy_engine: engine,
        data_store: s,
        stats: Arc::new(AgentStats::new(false)),
        decision_cache: None,
        cache_config: CacheConfig::default(),
        agent_config: ReaperAgentConfig::default(),
        policy_cache: None,
        decision_buffer: buffer,
        agent_id: "test-agent".to_string(),
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: Arc::new(DataSyncState::from_env()),
        bundle_verifier: Arc::new(BundleVerifier::from_config(&ManagementSettings::default())),
    })
}

/// Decision logging ON with pristine defaults: explain tier off, denies-only —
/// exactly what an operator gets from REAPER_DECISION_LOG_ENABLED=true alone.
fn default_buffer() -> SharedDecisionBuffer {
    let config = DecisionLogConfig {
        enabled: true,
        // Tests opt into the explicit raw posture (decision logging demands a
        // privacy choice by design).
        privacy_profile: Some(policy_engine::PrivacyProfile::Raw),
        ..Default::default()
    };
    policy_engine::create_shared_buffer(config).unwrap()
}

fn req(principal: &str, actor: Option<&str>, action: &str) -> EvaluateRequest {
    EvaluateRequest {
        policy_id: None,
        policy_name: Some("agent_explain".to_string()),
        principal: principal.to_string(),
        resource: "doc-1".to_string(),
        action: action.to_string(),
        context: None,
        actor: actor.map(str::to_string),
        context_provenance: None,
        capability: None,
    }
}

async fn decide(state: Arc<AgentState>, r: EvaluateRequest) -> serde_json::Value {
    let resp = evaluate_policy(State(state), Json(r))
        .await
        .expect("handler must serve")
        .into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn allow_response_names_the_allowing_rule() {
    let state = state_with_buffer(None);
    let body = decide(state.clone(), req("alice", Some("agent-ci"), "read")).await;
    assert_eq!(body["decision"], "allow", "body: {body}");
    assert_eq!(
        body["matched_rule"], "trusted_agents_read",
        "an allow must name the rule that allowed it"
    );

    // Human path: a different rule name.
    let body = decide(state, req("alice", None, "read")).await;
    assert_eq!(body["decision"], "allow");
    assert_eq!(body["matched_rule"], "humans_read");
}

#[tokio::test]
async fn deny_response_names_the_denying_rule() {
    let state = state_with_buffer(None);
    let body = decide(state.clone(), req("alice", Some("agent-rogue"), "read")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "block_rogues");

    // Default deny (no rule matched) keeps the stable default marker.
    let body = decide(state, req("alice", Some("agent-ci"), "write")).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["matched_rule"], "default_deny");
}

#[tokio::test]
async fn actor_allow_captures_input_data_by_default() {
    let buffer = default_buffer();
    let state = state_with_buffer(Some(buffer.clone()));

    let body = decide(state, req("alice", Some("agent-ci"), "read")).await;
    assert_eq!(body["decision"], "allow");

    let entries = buffer.query(DecisionFilter::new(), 10);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.matched_rule.as_deref(), Some("trusted_agents_read"));

    // Default-on for actor-carrying requests: the explain snapshot exists
    // and includes the ACTOR's attributes — the facts the allow branched on.
    let input = entry
        .input_data
        .as_ref()
        .expect("actor-carrying allow must capture input_data by default");
    assert_eq!(input["actor"]["trusted"], serde_json::json!(true));
    assert!(input["principal"]["role"].is_string());
}

#[tokio::test]
async fn plain_allow_keeps_the_opt_in_posture() {
    let buffer = default_buffer();
    let state = state_with_buffer(Some(buffer.clone()));

    // No actor: pristine defaults capture nothing for allows (explain tier
    // off + denies-only) — pre-F1 behavior is unchanged for human traffic.
    let body = decide(state, req("alice", None, "read")).await;
    assert_eq!(body["decision"], "allow");

    let entries = buffer.query(DecisionFilter::new(), 10);
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].input_data.is_none(),
        "plain allows stay opt-in for input_data"
    );
}

#[tokio::test]
async fn actor_capture_can_be_disabled() {
    let config = DecisionLogConfig {
        enabled: true,
        privacy_profile: Some(policy_engine::PrivacyProfile::Raw),
        input_data_actor_requests: false,
        ..Default::default()
    };
    let buffer = policy_engine::create_shared_buffer(config).unwrap();
    let state = state_with_buffer(Some(buffer.clone()));

    let body = decide(state, req("alice", Some("agent-ci"), "read")).await;
    assert_eq!(body["decision"], "allow");

    let entries = buffer.query(DecisionFilter::new(), 10);
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].input_data.is_none(),
        "operator opt-out respected"
    );
}
