//! R4-01 step 6b — the agent as a Kubernetes admission webhook target.
//!
//! Round-trips real `AdmissionReview` (admission.k8s.io/v1) fixtures against
//! the k8s library policy (`policy-library/kubernetes/admission-control`)
//! through the served handlers:
//!
//! * `POST /api/v1/admission/{policy}`: uid echoed, well-formed v1 response
//!   envelope, `allowed` correct for violating/clean pods, every violation
//!   message joined into `status.message`.
//! * Fail-closed posture: a parseable review against a missing policy is
//!   DENIED (allowed=false + reason), not a 5xx; a body without `request.uid`
//!   and an unknown apiVersion are 400 (no well-formed response exists).
//! * `POST /api/v1/check` serves the same driver from the policy's CACHED
//!   preferred evaluator (compiled — not a per-call AST parse), and agrees
//!   with the admission verdicts.

use std::sync::Arc;

use axum::extract::{Json, Path, State};
use policy_engine::cache_config::CacheConfig;
use policy_engine::{EnhancedPolicy, PolicyEngine, PolicyLanguage};
use reaper_agent::handlers::{admission_review, check_document};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};
use serde_json::{json, Value};

const POLICY_NAME: &str = "k8s_admission";

fn k8s_policy_content() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../policy-library/kubernetes/admission-control/policy.reap"
    );
    std::fs::read_to_string(path).expect("read k8s library policy")
}

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/admission/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
        .expect("parse fixture")
}

fn state_with_k8s_policy() -> Arc<AgentState> {
    let store = Arc::new(policy_engine::DataStore::new());
    let engine = PolicyEngine::new();
    let mut policy = EnhancedPolicy::new_with_language(
        POLICY_NAME.to_string(),
        String::new(),
        PolicyLanguage::ReaperDsl,
        k8s_policy_content(),
    )
    .expect("parse k8s library policy");
    policy
        .build_evaluator_with_data(Some(store.clone()))
        .expect("build evaluator");
    engine.deploy_policy(policy).expect("deploy");

    Arc::new(AgentState {
        policy_engine: engine,
        data_store: store,
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
        capability_gate: Arc::new(
            reaper_agent::capability_cache::CapabilityGateRuntime::from_auth(
                &reaper_core::config::AgentAuthSettings::default(),
            ),
        ),
    })
}

async fn admission(state: Arc<AgentState>, policy: &str, review: Value) -> Value {
    let Json(body) = admission_review(State(state), Path(policy.to_string()), Json(review))
        .await
        .expect("admission handler returned non-200");
    body
}

#[tokio::test]
async fn violating_pod_is_denied_with_all_messages() {
    let state = state_with_k8s_policy();
    let review = fixture("pod-violating.json");
    let body = admission(state, POLICY_NAME, review).await;

    assert_eq!(body["apiVersion"], "admission.k8s.io/v1");
    assert_eq!(body["kind"], "AdmissionReview");
    let response = &body["response"];
    assert_eq!(response["uid"], "705ab4f5-6393-11e8-b7cc-42010a800002");
    assert_eq!(response["allowed"], false);
    assert_eq!(response["status"]["code"], 403);

    // All five library rules fire on this pod; their rendered messages all
    // travel in status.message (the field kubectl surfaces to the user).
    let message = response["status"]["message"].as_str().expect("message");
    for expected in [
        "image uses :latest tag: registry.corp.internal/web:latest",
        "image from unapproved registry: docker.io/random/sidecar:1.2",
        "privileged container: web",
        "pod is missing required label: owner",
        "container without resource limits: sidecar",
    ] {
        assert!(
            message.contains(expected),
            "status.message missing '{expected}': {message}"
        );
    }
}

#[tokio::test]
async fn clean_pod_is_allowed_with_no_status() {
    let state = state_with_k8s_policy();
    let review = fixture("pod-clean.json");
    let body = admission(state, POLICY_NAME, review).await;

    let response = &body["response"];
    assert_eq!(response["uid"], "3f6bd0f6-40ba-4a52-9a0d-3c8ac3af0f4e");
    assert_eq!(response["allowed"], true);
    assert!(
        response.get("status").is_none(),
        "allowed responses carry no status: {response}"
    );
}

#[tokio::test]
async fn missing_policy_fails_closed_as_denial() {
    let state = state_with_k8s_policy();
    let review = fixture("pod-clean.json");
    let body = admission(state, "no-such-policy", review).await;

    let response = &body["response"];
    assert_eq!(response["allowed"], false, "fail closed, not fail open");
    assert_eq!(response["status"]["code"], 500);
    let message = response["status"]["message"].as_str().expect("message");
    assert!(
        message.contains("not deployed"),
        "operator-debuggable reason expected: {message}"
    );
    // Still a well-formed response the API server can act on.
    assert_eq!(response["uid"], "3f6bd0f6-40ba-4a52-9a0d-3c8ac3af0f4e");
}

#[tokio::test]
async fn body_without_uid_is_a_400() {
    let state = state_with_k8s_policy();
    let err = admission_review(
        State(state),
        Path(POLICY_NAME.to_string()),
        Json(json!({"kind": "Pod", "spec": {}})),
    )
    .await
    .expect_err("not an AdmissionReview");
    assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    assert!(err.1.contains("request.uid"), "{}", err.1);
}

#[tokio::test]
async fn unknown_api_version_is_a_400() {
    let state = state_with_k8s_policy();
    let mut review = fixture("pod-clean.json");
    review["apiVersion"] = json!("admission.k8s.io/v1beta1");
    let err = admission_review(State(state), Path(POLICY_NAME.to_string()), Json(review))
        .await
        .expect_err("v1beta1 is not served");
    assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    assert!(err.1.contains("v1beta1"), "{}", err.1);
}

#[tokio::test]
async fn check_endpoint_serves_cached_compiled_evaluator_and_agrees() {
    let state = state_with_k8s_policy();

    // The k8s library policy compiles whole since R4-01 B.3 — the served
    // evaluator must be the cached compiled one, not a per-call AST parse.
    let review = fixture("pod-violating.json");
    let Json(check) = check_document(
        State(state.clone()),
        Json(
            serde_json::from_value(json!({
                "policy_name": POLICY_NAME,
                "input": review,
            }))
            .expect("check request"),
        ),
    )
    .await
    .expect("check handler");
    assert_eq!(check["evaluator"], "reaper_dsl", "compiled driver expected");
    assert_eq!(check["allowed"], false);
    assert_eq!(
        check["violations"].as_array().map(Vec::len),
        Some(5),
        "all five deny rules violate: {}",
        check["violations"]
    );

    // Same driver, same verdict through the admission envelope.
    let body = admission(state, POLICY_NAME, fixture("pod-violating.json")).await;
    assert_eq!(body["response"]["allowed"], false);
}
