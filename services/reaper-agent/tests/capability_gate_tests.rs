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
    state_with_auth(key, |auth| auth.require_actor_capability = require)
}

/// Like [`state_with`] with full control over the auth settings (Phase D:
/// cache flag, TTL, verify rate limit). The gate runtime is built from the
/// SAME auth the state carries — mirroring main.rs.
fn state_with_auth(
    key: Option<&SigningKey>,
    tune: impl FnOnce(&mut reaper_core::config::AgentAuthSettings),
) -> Arc<AgentState> {
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
    tune(&mut agent_config.auth);

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
        policy_cache: None,
        decision_buffer: None,
        agent_id: "test-agent".to_string(),
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: Arc::new(DataSyncState::from_env()),
        bundle_verifier: Arc::new(BundleVerifier::from_config(&mgmt)),
        capability_gate: std::sync::Arc::new(
            reaper_agent::capability_cache::CapabilityGateRuntime::from_auth(&agent_config.auth),
        ),
        agent_config,
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

#[tokio::test]
async fn gate_binds_actor_from_capability() {
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
    .await
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

// ===========================================================================
// Plan 06 Phase D (R3-P2-2/ADR-4): verdict cache, revocation-generation
// invalidation, tamper resistance, hit-path window checks, rate limit.
// ===========================================================================

#[tokio::test]
async fn same_capability_twice_verifies_once() {
    // The DoD unit test: a repeated capability is a cache hit — exactly ONE
    // full ed25519 verification for two requests.
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);
    for _ in 0..2 {
        let body = decide(
            state.clone(),
            req("alice", Some("agent-1"), "read", "/doc/1", Some(c.clone())),
        )
        .await;
        assert_eq!(body["decision"], "allow", "body: {body}");
    }
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        1,
        "second request must be a verdict-cache hit"
    );
    assert_eq!(state.capability_gate.cache.hit_count(), 1);
}

#[tokio::test]
async fn revocation_generation_bump_reverifies_and_revokes() {
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "*")]);

    // Verify + cache under generation 0.
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", Some(c.clone())),
    )
    .await;
    assert_eq!(body["decision"], "allow");
    assert_eq!(state.bundle_verifier.capability_verify_count(), 1);

    // Apply a list revoking an UNRELATED capability: generation bumps, so the
    // DoD contract is a full RE-verification (no stale-generation hit) — which
    // still allows.
    let signed = SignedRevocationList::sign(
        RevocationList {
            issued_at: "2026-01-01T00:00:00Z".into(),
            serial: 1,
            next_update: 0,
            revoked_bundle_hashes: Vec::new(),
            revoked_key_ids: Vec::new(),
            revoked_capability_ids: vec!["some-other-cap".to_string()],
        },
        &key,
        "k1",
    );
    state.bundle_verifier.apply_revocations(&signed).unwrap();
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", Some(c.clone())),
    )
    .await;
    assert_eq!(body["decision"], "allow");
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        2,
        "generation bump must force a full re-verification"
    );

    // Now revoke THIS capability: despite two cached generations, it denies.
    let signed = SignedRevocationList::sign(
        RevocationList {
            issued_at: "2026-01-01T00:00:00Z".into(),
            serial: 2,
            next_update: 0,
            revoked_bundle_hashes: Vec::new(),
            revoked_key_ids: Vec::new(),
            revoked_capability_ids: vec!["some-other-cap".to_string(), c.id.clone()],
        },
        &key,
        "k1",
    );
    state.bundle_verifier.apply_revocations(&signed).unwrap();
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", Some(c)),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(body["matched_rule"].as_str().unwrap().contains("revoked"));
}

#[tokio::test]
async fn tampered_capability_never_hits_the_cache() {
    // FAIL-OPEN DETECTOR for the cache key: after a legitimate capability's
    // verdict is cached, presenting the SAME id+signature with widened grants
    // must MISS (the digest binds every signed claim) and then fail the real
    // signature check. A key of (id, key_id, signature, expiry) alone — the
    // plan's literal wording — would cache-hit here and grant "/etc".
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let c = cap(&key, vec![Grant::new("read", "/doc/*")]);

    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc/1", Some(c.clone())),
    )
    .await;
    assert_eq!(body["decision"], "allow");

    let mut tampered = c;
    tampered.grants[0].resource = "*".to_string();
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/etc", Some(tampered)),
    )
    .await;
    assert_eq!(body["decision"], "deny", "body: {body}");
    assert!(body["matched_rule"].as_str().unwrap().contains("signature"));
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        2,
        "the tampered token must have forced a full (failing) verification"
    );
}

#[tokio::test]
async fn cached_verdict_still_enforces_expiry() {
    // A cache hit proves the content-bound checks, not the clock: a verdict
    // cached while valid must NOT outlive the capability's own window.
    let key = signing_key();
    let state = state_with(Some(&key), false);
    let t0 = now();
    let c = issue(
        &key,
        "k1",
        "alice",
        "agent-1",
        vec![Grant::new("read", "*")],
        t0 - 10,
        t0 + 100,
    )
    .unwrap();

    let mut actor = Some("agent-1".to_string());
    reaper_agent::capability_gate::enforce(
        &state,
        "alice",
        "read",
        "/doc",
        &mut actor,
        Some(&c),
        t0,
    )
    .await
    .expect("valid at t0");
    // Same capability, still inside the cache TTL (default 300s) but PAST the
    // capability's expiry: the hit path's check_validity_at must deny.
    let err = reaper_agent::capability_gate::enforce(
        &state,
        "alice",
        "read",
        "/doc",
        &mut actor,
        Some(&c),
        t0 + 150,
    )
    .await
    .expect_err("expired capability must deny even on a cache hit");
    assert!(err.contains("expired"), "reason: {err}");
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        1,
        "expiry on the hit path must not need a second full verification"
    );
}

#[tokio::test]
async fn garbage_signature_flood_is_rate_limited() {
    let key = signing_key();
    let state = state_with_auth(Some(&key), |auth| {
        auth.capability_verify_limit_per_min = 2;
    });

    // Garbage signatures never cache-hit, so each attempt is a full verify —
    // exactly the budget the limiter bounds.
    let garbage = |i: u8| {
        let mut c = cap(&key, vec![Grant::new("read", "*")]);
        c.signature = format!("{:02x}", i).repeat(64);
        c
    };
    for i in 0..2u8 {
        let body = decide(
            state.clone(),
            req("alice", Some("agent-1"), "read", "/doc", Some(garbage(i))),
        )
        .await;
        assert_eq!(body["decision"], "deny");
        assert!(body["matched_rule"].as_str().unwrap().contains("rejected"));
    }
    // Third full-verify attempt in the same minute: budget exhausted.
    let body = decide(
        state.clone(),
        req("alice", Some("agent-1"), "read", "/doc", Some(garbage(9))),
    )
    .await;
    assert_eq!(body["decision"], "deny");
    assert!(
        body["matched_rule"]
            .as_str()
            .unwrap()
            .starts_with("capability_verify_rate_limited"),
        "body: {body}"
    );
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        2,
        "the rate-limited attempt must never reach ed25519"
    );
}

#[tokio::test]
async fn cache_disabled_is_the_pre_phase_d_inline_path() {
    // Rollback flag: capability_cache_enabled=false → every request is a
    // full inline verification, nothing cached.
    let key = signing_key();
    let state = state_with_auth(Some(&key), |auth| {
        auth.capability_cache_enabled = false;
    });
    let c = cap(&key, vec![Grant::new("read", "*")]);
    for _ in 0..2 {
        let body = decide(
            state.clone(),
            req("alice", Some("agent-1"), "read", "/doc", Some(c.clone())),
        )
        .await;
        assert_eq!(body["decision"], "allow");
    }
    assert_eq!(
        state.bundle_verifier.capability_verify_count(),
        2,
        "cache off: both requests verify in full"
    );
    assert!(state.capability_gate.cache.is_empty());
}
