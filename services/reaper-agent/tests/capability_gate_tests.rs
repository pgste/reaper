//! F1-s3: pre-eval capability enforcement on the agent's served path.
//!
//! A signed capability presented with an evaluation request must pass
//! signature/window/revocation verification, bind to the request (subject ==
//! principal, actor == actor), and cover (action, resource) with its grants —
//! all BEFORE policy evaluation, all fail-closed. Requests without a
//! capability are untouched unless `auth.require_actor_capability` is set.
//!
//! Denials are served like the other pre-eval guards: `decision: "deny"`
//! with the reason in `matched_rule`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    response::IntoResponse,
};
use policy_engine::{
    cache_config::CacheConfig, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule,
};
use reaper_agent::handlers::{batch_evaluate_policy, evaluate_policy, fast_evaluate_policy};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_agent::types::{BatchEvaluateRequest, BatchRequestItem, EvaluateRequest};
use reaper_core::bundle_signing::SigningKey;
use reaper_core::capability::{issue, Capability, Grant};
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};
use reaper_core::revocation::{RevocationList, SignedRevocationList};

fn signing_key() -> SigningKey {
    SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])))
}

fn now() -> i64 {
    reaper_agent::capability_gate::now_unix()
}

/// Agent state with the capability trust anchor pinned to `key`, one simple
/// allow-all-reads policy loaded, and optional require-mode.
fn state_with(key: Option<&SigningKey>, require: bool) -> Arc<AgentState> {
    let mgmt = match key {
        Some(k) => ManagementSettings {
            enabled: true,
            bundle_public_key: Some(k.public_key_hex()),
            bundle_key_id: Some("k1".to_string()),
            ..Default::default()
        },
        None => ManagementSettings::default(),
    };
    let mut agent_config = ReaperAgentConfig::default();
    agent_config.auth.require_actor_capability = require;

    let engine = PolicyEngine::new();
    engine
        .deploy_policy(EnhancedPolicy::new(
            "cap-test".to_string(),
            String::new(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        ))
        .unwrap();

    Arc::new(AgentState {
        policy_engine: engine,
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
        bundle_verifier: Arc::new(BundleVerifier::from_config(&mgmt)),
    })
}

/// A capability for (alice, agent-1) over the given grants, valid ±300s.
fn cap(key: &SigningKey, grants: Vec<Grant>) -> Capability {
    issue(
        key,
        "k1",
        "alice",
        "agent-1",
        grants,
        now() - 300,
        now() + 300,
    )
    .unwrap()
}

fn req(
    principal: &str,
    actor: Option<&str>,
    action: &str,
    resource: &str,
    capability: Option<Capability>,
) -> EvaluateRequest {
    EvaluateRequest {
        policy_id: None,
        policy_name: Some("cap-test".to_string()),
        principal: principal.to_string(),
        resource: resource.to_string(),
        action: action.to_string(),
        context: None,
        actor: actor.map(str::to_string),
        context_provenance: None,
        capability,
    }
}

async fn decide(state: Arc<AgentState>, r: EvaluateRequest) -> serde_json::Value {
    let resp = evaluate_policy(State(state), Json(r))
        .await
        .expect("handler must serve, not error")
        .into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn valid_capability_allows() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);
    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/doc/1", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "allow", "body: {body}");
}

#[tokio::test]
async fn expired_capability_denies() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let expired = issue(
        &key,
        "k1",
        "alice",
        "agent-1",
        vec![Grant::new("read", "*")],
        now() - 600,
        now() - 300,
    )
    .unwrap();
    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/doc/1", Some(expired)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    let reason = body["matched_rule"].as_str().unwrap();
    assert!(reason.contains("expired"), "reason: {reason}");
}

#[tokio::test]
async fn tampered_capability_denies() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let mut c = cap(&key, vec![Grant::new("read", "/doc/*")]);
    // Widen the grant after signing — the signature must catch it.
    c.grants[0].resource = "*".to_string();
    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/etc", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"].as_str().unwrap().contains("signature"));
}

#[tokio::test]
async fn revoked_capability_denies() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "*")]);

    // Revoke via the same signed list-pull channel bundles use.
    let signed = SignedRevocationList::sign(
        RevocationList {
            issued_at: "2026-01-01T00:00:00Z".into(),
            serial: 1,
            next_update: 0,
            revoked_bundle_hashes: Vec::new(),
            revoked_key_ids: Vec::new(),
            revoked_capability_ids: vec![c.id.clone()],
        },
        &key,
        "k1",
    );
    state.bundle_verifier.apply_revocations(&signed).unwrap();

    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/doc", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"].as_str().unwrap().contains("revoked"));
}

#[tokio::test]
async fn out_of_grant_denies() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);
    // Action outside the grant...
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "write", "/doc/1", Some(c.clone())),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_out_of_grant"));
    // ...and resource outside the grant.
    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/etc/secret", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
}

#[tokio::test]
async fn subject_and_actor_bindings_enforced() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "*")]);

    // Wrong principal: the capability derives from alice, not bob.
    let body = decide(
        state.clone(),
        req("bob", Some("agent-1"), "read", "/doc", Some(c.clone())),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_subject_mismatch"));

    // Wrong actor: minted for agent-1, presented by agent-2 (confused deputy).
    let body = decide(
        state,
        req("alice", Some("agent-2"), "read", "/doc", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_actor_mismatch"));
}

#[tokio::test]
async fn capability_without_trust_anchor_fails_closed() {
    let key = signing_key();
    // Agent has NO pinned key: a presented capability cannot be verified and
    // must deny, never silently pass.
    let state = state_with(None, false);
    let c = cap(&key, vec![Grant::new("read", "*")]);
    let body = decide(
        state,
        req("alice", Some("agent-1"), "read", "/doc", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .contains("no trust anchor"));
}

#[tokio::test]
async fn require_mode_gates_bare_actor_requests() {
    let key = signing_key();
    // require_actor_capability=true: actor without capability is denied...
    let state = state_with(Some(&key), true);
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", None),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_required"));

    // ...while an actor-less request stays served (human traffic unaffected).
    let body = decide(state, req("alice", None, "read", "/doc", None)).await;
    assert_eq!(body["decision"], "allow");
}

#[tokio::test]
async fn plain_requests_unaffected_without_require_mode() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    // Actor named, no capability, require-mode off: policy remains the gate.
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", None),
    )
    .await;
    assert_eq!(body["decision"], "allow");
    let body = decide(state, req("alice", None, "read", "/doc", None)).await;
    assert_eq!(body["decision"], "allow");
}

#[test]
fn gate_binds_actor_from_capability() {
    // Unit-level: a request naming no actor inherits the capability's actor,
    // so `actor.*` policy conditions see the verified identity.
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "*")]);
    let mut actor = None;
    reaper_agent::capability_gate::enforce(
        &state,
        "alice",
        "read",
        "/doc",
        &mut actor,
        Some(&c),
        now(),
    )
    .unwrap();
    assert_eq!(actor.as_deref(), Some("agent-1"));
}

#[tokio::test]
async fn fast_path_enforces_capabilities() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);

    // Out-of-grant via /fast-messages: the agentic dispatch must route it
    // through the gate, not the un-gated SIMD lane.
    let payload = serde_json::json!({
        "policy_name": "cap-test",
        "principal": "alice",
        "actor": "agent-1",
        "action": "write",
        "resource": "/doc/1",
        "capability": c,
    });
    let resp = fast_evaluate_policy(
        State(state.clone()),
        axum::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
    )
    .await
    .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["decision"], "deny", "body: {body}");
    assert!(body["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_out_of_grant"));

    // A plain fast-path request still serves on the SIMD lane.
    let plain = serde_json::json!({
        "policy_name": "cap-test",
        "principal": "alice",
        "action": "read",
        "resource": "/doc/1",
    });
    let resp = fast_evaluate_policy(
        State(state),
        axum::body::Bytes::from(serde_json::to_vec(&plain).unwrap()),
    )
    .await
    .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["decision"], "allow", "body: {body}");
}

#[tokio::test]
async fn batch_enforces_capabilities_per_item() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);

    let item = |id: &str, action: &str, capability: Option<Capability>| BatchRequestItem {
        id: id.to_string(),
        principal: "alice".to_string(),
        resource: "/doc/1".to_string(),
        action: action.to_string(),
        context: None,
        actor: Some("agent-1".to_string()),
        context_provenance: None,
        capability,
    };

    let body = batch_evaluate_policy(
        State(state),
        Json(BatchEvaluateRequest {
            policy_id: None,
            policy_name: Some("cap-test".to_string()),
            requests: vec![
                item("ok", "read", Some(c.clone())),
                item("bad", "write", Some(c)),
            ],
        }),
    )
    .await
    .unwrap();
    let results = body.0["results"].as_array().expect("results array").clone();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["decision"], "allow", "results: {results:?}");
    assert_eq!(results[1]["decision"], "deny");
    assert!(results[1]["matched_rule"]
        .as_str()
        .unwrap()
        .starts_with("capability_out_of_grant"));
}
