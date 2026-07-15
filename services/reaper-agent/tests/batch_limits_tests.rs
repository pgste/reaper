//! Batch endpoint bounds (Plan 05, Step 3): a batch larger than
//! `performance.max_batch_requests` is rejected with 413 *before* any
//! evaluation, and the eval routes carry a tighter per-route body limit than
//! the bulk-data routes.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Json, State},
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use policy_engine::{
    cache_config::CacheConfig, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule,
};
use reaper_agent::handlers::{batch_evaluate_policy, evaluate_policy};
use reaper_agent::management::verify::BundleVerifier;
use reaper_agent::state::{AgentState, AgentStats, DataSyncState};
use reaper_agent::types::{BatchEvaluateRequest, BatchRequestItem, EvaluateRequest};
use reaper_core::config::{ManagementSettings, ReaperAgentConfig};
use tower::ServiceExt; // for `oneshot`

fn state_with_max_batch(max_batch_requests: usize) -> Arc<AgentState> {
    let mut agent_config = ReaperAgentConfig::default();
    agent_config.performance.max_batch_requests = max_batch_requests;

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

fn batch_of(n: usize) -> BatchEvaluateRequest {
    BatchEvaluateRequest {
        policy_id: None,
        policy_name: None,
        requests: (0..n)
            .map(|i| BatchRequestItem {
                id: format!("r{i}"),
                principal: "alice".to_string(),
                resource: "/doc".to_string(),
                action: "read".to_string(),
                context: None,
                actor: None,
                context_provenance: None,
                capability: None,
            })
            .collect(),
    }
}

#[tokio::test]
async fn batch_over_cap_is_rejected_with_413() {
    let state = state_with_max_batch(2);
    // cap + 1 requests → rejected before evaluation, regardless of policy state.
    let result = batch_evaluate_policy(State(state), Json(batch_of(3))).await;
    match result {
        Err(status) => assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE),
        Ok(_) => panic!("expected 413 for an over-cap batch"),
    }
}

#[tokio::test]
async fn batch_at_cap_is_not_rejected_by_the_cap() {
    let state = state_with_max_batch(2);
    // Exactly at the cap: the count guard must not fire. With no policy loaded
    // the handler returns an Ok JSON body ("No policies loaded") — the point is
    // it is not the 413 the cap produces.
    let result = batch_evaluate_policy(State(state), Json(batch_of(2))).await;
    assert!(
        result.is_ok(),
        "a batch at the cap must pass the count guard"
    );
}

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

/// Plan 08 Phase B: the batch loop fans out across the rayon pool. Parallel
/// execution must not reorder results or mix up per-request decisions.
#[tokio::test]
async fn parallel_batch_preserves_order_and_decisions() {
    let state = state_with_max_batch(1000);
    state
        .policy_engine
        .deploy_policy(simple_allow("batch-par", "/doc"))
        .unwrap();

    // Mixed batch: even indices hit /doc (allow), odd hit /other (default deny).
    let requests = (0..300)
        .map(|i| BatchRequestItem {
            id: format!("r{i}"),
            principal: "alice".to_string(),
            resource: if i % 2 == 0 { "/doc" } else { "/other" }.to_string(),
            action: "read".to_string(),
            context: None,
            actor: None,
            context_provenance: None,
            capability: None,
        })
        .collect();
    let req = BatchEvaluateRequest {
        policy_id: None,
        policy_name: Some("batch-par".to_string()),
        requests,
    };

    let Json(body) = batch_evaluate_policy(State(state), Json(req))
        .await
        .expect("batch evaluation failed");
    let results = body["results"].as_array().expect("results array");
    assert_eq!(results.len(), 300);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["index"].as_u64().unwrap() as usize, i, "order preserved");
        let expected = if i % 2 == 0 { "allow" } else { "deny" };
        assert_eq!(r["decision"], expected, "decision for request {i}");
    }
    assert_eq!(body["summary"]["allowed"], 150);
    assert_eq!(body["summary"]["denied"], 150);
}

/// Functional head-of-line check (Plan 08 Phase B): while a full-cap batch
/// runs on the blocking/rayon pools, concurrent single evaluations on the
/// async runtime still complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_evals_complete_while_large_batch_runs() {
    let state = state_with_max_batch(1000);
    state
        .policy_engine
        .deploy_policy(simple_allow("hol", "/doc"))
        .unwrap();

    let mut batch = batch_of(1000);
    batch.policy_name = Some("hol".to_string());
    let batch_state = state.clone();
    let batch_task =
        tokio::spawn(async move { batch_evaluate_policy(State(batch_state), Json(batch)).await });

    let singles: Vec<_> = (0..8)
        .map(|_| {
            let s = state.clone();
            tokio::spawn(async move {
                evaluate_policy(
                    State(s),
                    Json(EvaluateRequest {
                        policy_id: None,
                        policy_name: Some("hol".to_string()),
                        principal: "alice".to_string(),
                        resource: "/doc".to_string(),
                        action: "read".to_string(),
                        context: None,
                        actor: None,
                        context_provenance: None,
                        capability: None,
                    }),
                )
                .await
            })
        })
        .collect();

    for t in singles {
        assert!(t.await.expect("single eval task panicked").is_ok());
    }
    assert!(batch_task.await.expect("batch task panicked").is_ok());
}

/// The production router applies a global 256 MB body limit and a tighter
/// per-route limit on the eval endpoints via `route_layer`. This proves the
/// wiring the agent relies on: the inner per-route limit wins over the global
/// one for the eval route, while other routes keep the larger limit.
#[tokio::test]
async fn per_route_body_limit_overrides_the_global_limit() {
    // `Bytes` (like the real handlers' `Json`) is a limit-aware extractor, so
    // it returns 413 when the body exceeds the route's DefaultBodyLimit. A raw
    // `Body` extractor would bypass the limit entirely.
    async fn echo(_body: axum::body::Bytes) -> StatusCode {
        StatusCode::OK
    }

    const SMALL: usize = 1024; // eval-style tight limit
    const LARGE: usize = 1024 * 1024; // bulk-data limit

    let eval = Router::new()
        .route("/eval", post(echo))
        .route_layer(DefaultBodyLimit::max(SMALL));
    let app = Router::new()
        .route("/data", post(echo))
        .merge(eval)
        .layer(DefaultBodyLimit::max(LARGE));

    let big_body = vec![b'x'; 4096]; // over SMALL, under LARGE

    // Eval route: the 1 KiB per-route limit rejects a 4 KiB body.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/eval")
                .body(Body::from(big_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "eval route must enforce the tighter per-route limit"
    );

    // Data route: the same body is well under the 1 MiB global limit.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/data")
                .body(Body::from(big_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "data route keeps the larger global limit"
    );
}
