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
    make_app_with(Arc::new(BundleVerifier::from_config(&mgmt)))
}

fn make_app_with(verifier: Arc<BundleVerifier>) -> (Router, Arc<AgentState>) {
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
        bundle_verifier: verifier,
        capability_gate: std::sync::Arc::new(
            reaper_agent::capability_cache::CapabilityGateRuntime::from_auth(
                &reaper_core::config::AgentAuthSettings::default(),
            ),
        ),
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
    v2_signature_versioned(key, bytes, 1)
}

fn v2_signature_versioned(key: &SigningKey, bytes: &[u8], version: u64) -> BundleSignature {
    let now = unix_now();
    sign_bundle_v2(
        bytes,
        key,
        "k1",
        &EnvelopeClaims {
            bundle_id: "11111111-2222-4333-8444-555555555555".to_string(),
            version,
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

#[tokio::test]
async fn replayed_older_version_is_rejected_and_persists_across_restart() {
    use reaper_agent::management::verify::BundleVerifier;
    use tempfile::TempDir;

    let key = signing_key();
    let dir = TempDir::new().unwrap();
    let floor_path = dir.path().join("anti_rollback.json");
    let bytes = bundle_bytes();

    // Agent #1: apply v5 (valid signature), then attempt a genuinely-signed v4.
    {
        let verifier = Arc::new(BundleVerifier::from_config_persistent(
            &managed_settings(&key),
            floor_path.clone(),
        ));
        let (app, state) = make_app_with(verifier);

        let v5 = v2_signature_versioned(&key, &bytes, 5);
        let status = post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "5", "signature": v5}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state.policy_engine.get_stats().total_policies, 1);

        // Replay an older, still-validly-signed v4 → rejected.
        let v4 = v2_signature_versioned(&key, &bytes, 4);
        let status = post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "4", "signature": v4}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // (Idempotent re-apply of the current version is covered by the
        // anti_rollback unit tests; the engine's own same-version dedup makes
        // a re-push here a separate concern.)
    }

    // Agent #2: fresh process, SAME floor file. The v4 downgrade is still
    // refused because the floor (5) survived the restart.
    {
        let verifier = Arc::new(BundleVerifier::from_config_persistent(
            &managed_settings(&key),
            floor_path,
        ));
        let (app, state) = make_app_with(verifier);
        let v4 = v2_signature_versioned(&key, &bytes, 4);
        let status = post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "4", "signature": v4}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "persisted floor must reject a downgrade after restart"
        );
        assert_eq!(state.policy_engine.get_stats().total_policies, 0);
    }
}

#[tokio::test]
async fn force_overrides_anti_rollback_but_not_signature() {
    let key = signing_key();
    let (app, state) = make_app(managed_settings(&key));
    let bytes = bundle_bytes();

    // Apply v10.
    let v10 = v2_signature_versioned(&key, &bytes, 10);
    assert_eq!(
        post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "10", "signature": v10}),
        )
        .await,
        StatusCode::OK
    );

    // A normal older v3 is refused...
    let v3 = v2_signature_versioned(&key, &bytes, 3);
    assert_eq!(
        post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "3", "signature": v3.clone()}),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // ...but force applies it (still a valid signature).
    assert_eq!(
        post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "3", "force": true, "signature": v3}),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(state.policy_engine.get_stats().total_policies, 1);

    // force does NOT bypass the signature: a force push of tampered bytes fails.
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let sig = v2_signature_versioned(&key, &bytes, 11);
    assert_eq!(
        post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": tampered, "version": "11", "force": true, "signature": sig}),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn revoked_bundle_hash_and_key_are_rejected_at_load() {
    use reaper_agent::management::verify::BundleVerifier;
    use reaper_core::revocation::{bundle_hash_hex, RevocationList, SignedRevocationList};

    let key = signing_key();

    // Case 1: revoke by bundle hash.
    {
        let verifier = Arc::new(BundleVerifier::from_config(&managed_settings(&key)));
        let bytes = bundle_bytes();
        let signed_list = SignedRevocationList::sign(
            RevocationList {
                issued_at: "2026-01-01T00:00:00Z".into(),
                serial: 1,
                next_update: 0,
                revoked_bundle_hashes: vec![bundle_hash_hex(&bytes)],
                revoked_key_ids: vec![],
                revoked_capability_ids: Vec::new(),
            },
            &key,
            "k1",
        );
        verifier.apply_revocations(&signed_list).unwrap();

        let (app, state) = make_app_with(verifier);
        let sig = v2_signature(&key, &bytes);
        let status = post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "1", "signature": sig}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "revoked hash rejected"
        );
        assert_eq!(state.policy_engine.get_stats().total_policies, 0);
    }

    // Case 2: revoke by signing key id — a correctly-signed, current bundle by
    // the distrusted key is refused; force does not override revocation.
    {
        let verifier = Arc::new(BundleVerifier::from_config(&managed_settings(&key)));
        let bytes = bundle_bytes();
        let signed_list = SignedRevocationList::sign(
            RevocationList {
                issued_at: "2026-01-01T00:00:00Z".into(),
                serial: 1,
                next_update: 0,
                revoked_bundle_hashes: vec![],
                revoked_key_ids: vec!["k1".into()],
                revoked_capability_ids: Vec::new(),
            },
            &key,
            "k1",
        );
        verifier.apply_revocations(&signed_list).unwrap();

        let (app, state) = make_app_with(verifier);
        let sig = v2_signature_versioned(&key, &bytes, 9);
        let status = post_json(
            &app,
            "/api/v1/bundles/deploy",
            serde_json::json!({"bundle": bytes, "version": "9", "force": true, "signature": sig}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "revoked key rejected even with force"
        );
        assert_eq!(state.policy_engine.get_stats().total_policies, 0);
    }
}
