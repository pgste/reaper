//! Push-path signature enforcement (Plan 02 Phase A).
//!
//! The HTTP deploy/load endpoints must enforce the same fail-closed signature
//! policy as the managed pull path: with a pinned verification key, an
//! unsigned / expired / legacy-enveloped bundle is rejected **before** any
//! byte is parsed, and no policy swap occurs. Standalone agents (no
//! management plane, no key) keep accepting local pushes — that surface is
//! protected by inbound auth + the loopback bind, not signatures.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use policy_engine::{cache_config::CacheConfig, PolicyEngine};
use reaper_agent::handlers::{deploy_bundle, load_bundles_atomic};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_core::bundle_signing::{
    sign_bundle, sign_bundle_v2, unix_now, BundleSignature, EnvelopeClaims, SigningKey,
};
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};
use tower::ServiceExt;

fn signing_key() -> SigningKey {
    SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])))
}

/// Managed agent with a pinned verification key (strict defaults).
fn managed_settings(key: &SigningKey) -> ManagementSettings {
    ManagementSettings {
        enabled: true,
        bundle_public_key: Some(key.public_key_hex()),
        bundle_key_id: Some("k1".to_string()),
        ..Default::default()
    }
}

fn make_app(mgmt: ManagementSettings) -> (Router, Arc<AgentState>) {
    let state = Arc::new(AgentState {
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
        bundle_verifier: Arc::new(BundleVerifier::from_config(&mgmt)),
    });
    let app = Router::new()
        .route("/api/v1/bundles/deploy", post(deploy_bundle))
        .route("/api/v1/bundles/load", post(load_bundles_atomic))
        .with_state(state.clone());
    (app, state)
}

/// A real compiled .rbb bundle.
fn bundle_bytes() -> Vec<u8> {
    let policy: policy_engine::reap::ReaperPolicy = r#"
        policy push_sig_test {
            default: deny,
            rule readers {
                allow if context.action == "read"
            }
        }
    "#
    .parse()
    .unwrap();
    policy.compile_to_bundle().unwrap()
}

fn v2_signature(key: &SigningKey, bytes: &[u8]) -> BundleSignature {
    let now = unix_now();
    sign_bundle_v2(
        bytes,
        key,
        "k1",
        &EnvelopeClaims {
            bundle_id: "11111111-2222-4333-8444-555555555555".to_string(),
            version: 1,
            not_before: now - 60,
            expires_at: now + 3600,
        },
    )
}

async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> StatusCode {
    let request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn unsigned_push_is_rejected_and_nothing_swaps() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": bytes, "version": "1"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state.policy_engine.get_stats().total_policies,
        0,
        "a rejected bundle must not have been applied"
    );

    // The atomic full-replace path is gated identically.
    let status = post_json(
        &app,
        "/api/v1/bundles/load",
        serde_json::json!({"bundles": [bytes]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.policy_engine.get_stats().total_policies, 0);
}

#[tokio::test]
async fn signed_v2_push_is_applied() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();
    let sig = v2_signature(&key, &bytes);

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": bytes, "version": "1", "signature": sig}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.policy_engine.get_stats().total_policies, 1);
}

#[tokio::test]
async fn signed_atomic_load_is_applied() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();
    let sig = v2_signature(&key, &bytes);

    let status = post_json(
        &app,
        "/api/v1/bundles/load",
        serde_json::json!({"bundles": [bytes], "signatures": [sig]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.policy_engine.get_stats().total_policies, 1);

    // Mismatched signatures length is a 400, not a partial apply.
    let status = post_json(
        &app,
        "/api/v1/bundles/load",
        serde_json::json!({"bundles": [bundle_bytes(), bundle_bytes()], "signatures": [v2_signature(&key, b"x")]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn expired_envelope_is_rejected() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();

    let now = unix_now();
    let expired = sign_bundle_v2(
        &bytes,
        &key,
        "k1",
        &EnvelopeClaims {
            bundle_id: "11111111-2222-4333-8444-555555555555".to_string(),
            version: 1,
            not_before: now - 7200,
            expires_at: now - 3600, // already past
        },
    );

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": bytes, "version": "1", "signature": expired}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.policy_engine.get_stats().total_policies, 0);
}

#[tokio::test]
async fn legacy_v1_envelope_is_rejected_under_strict_default() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();
    let v1 = sign_bundle(&bytes, &key, "k1"); // valid, but legacy schema

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": bytes, "version": "1", "signature": v1}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.policy_engine.get_stats().total_policies, 0);
}

#[tokio::test]
async fn tampered_bundle_bytes_are_rejected() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();
    let sig = v2_signature(&key, &bytes);

    // Flip one byte after signing.
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": tampered, "version": "1", "signature": sig}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.policy_engine.get_stats().total_policies, 0);
}

#[tokio::test]
async fn standalone_agent_still_accepts_local_pushes() {
    // No management plane, no pinned key: the OPA-sidecar-style workflow —
    // inbound auth + loopback bind protect this surface, not signatures.
    let (app, state) = make_app(ManagementSettings::default());
    let bytes = bundle_bytes();

    let status = post_json(
        &app,
        "/api/v1/bundles/deploy",
        serde_json::json!({"bundle": bytes, "version": "1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.policy_engine.get_stats().total_policies, 1);
}
