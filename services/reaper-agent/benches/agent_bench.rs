//! Handler-level microbenchmarks for the Reaper agent hot path.
//!
//! Unlike the network throughput harness (`examples/throughput.rs`,
//! `examples/uds_shard.rs`), this bench calls the real `fast_evaluate_policy`
//! handler *directly* — no sockets, no HTTP framing, no load generator sharing
//! cores. That isolates the per-request CPU work (JSON parse, decision-id
//! generation, evaluation, response serialization) at nanosecond resolution, so
//! small handler-level optimizations are actually measurable here even though
//! they vanish into transport noise end-to-end.
//!
//! Run: `cargo bench -p reaper-agent --bench agent_bench`

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use uuid::Uuid;

use policy_engine::{
    cache_config::CacheConfig, create_shared_buffer, decision_log::DecisionLogConfig,
    decision_log::PrivacyProfile, DataLoader, DataStore, EnhancedPolicy, PolicyEngine,
    PolicyLanguage, ReaperPolicy, SharedDecisionBuffer,
};
use reaper_agent::handlers::fast_evaluate_policy;
use reaper_agent::state::{AgentState, AgentStats};
use reaper_core::config::ReaperAgentConfig;

const POLICY: &str = r#"
policy bench {
    default: deny,
    rule deny_suspended { deny if user.suspended == true }
    rule admin_full { allow if user.role == "admin" }
    rule dept_clearance {
        allow if {
            user.department == resource.department &&
            user.clearance_level >= resource.clearance_level &&
            user.status == "active"
        }
    }
}
"#;

const DATA: &str = r#"{"entities":[
    {"id":"alice","type":"User","attributes":{"role":"engineer","department":"engineering","clearance_level":4,"status":"active"}},
    {"id":"doc1","type":"Resource","attributes":{"department":"engineering","clearance_level":3}}
]}"#;

fn build_state_with_buffer(decision_buffer: Option<SharedDecisionBuffer>) -> Arc<AgentState> {
    let data_store = Arc::new(DataStore::new());
    DataLoader::new((*data_store).clone())
        .load_json(DATA)
        .expect("load data");

    let engine = PolicyEngine::new();
    let reaper_policy: ReaperPolicy = POLICY.parse().expect("parse policy");
    let evaluator = reaper_policy.build(data_store.clone()).expect("compile");
    let mut policy = EnhancedPolicy::new("bench".to_string(), String::new(), vec![]);
    policy.language = PolicyLanguage::ReaperDsl;
    policy.content = POLICY.to_string();
    policy.evaluator = Some(Arc::new(evaluator));
    engine.deploy_policy(policy).expect("deploy");

    Arc::new(AgentState {
        policy_engine: engine,
        data_store,
        stats: Arc::new(AgentStats::new(false)),
        decision_cache: None,
        cache_config: CacheConfig::disabled(),
        agent_config: ReaperAgentConfig::default(),
        policy_cache: None,
        decision_buffer,
        agent_id: "bench".to_string(),
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: Arc::new(reaper_agent::state::DataSyncState::from_env()),
        bundle_verifier: std::sync::Arc::new(
            reaper_agent::management::verify::BundleVerifier::from_config(
                &reaper_core::config::ManagementSettings::default(),
            ),
        ),
    })
}

fn build_state() -> Arc<AgentState> {
    build_state_with_buffer(None)
}

/// The three logging tiers we care about on the hot path:
/// - logging off (baseline)
/// - logging on (entry build + lock-free shard push)
/// - explain tier on (adds the ABAC input-data snapshot: DataStore attribute
///   lookups + JSON for principal and resource)
fn bench_fast_evaluate(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let body = Bytes::from(
        r#"{"principal":"alice","action":"read","resource":"doc1","policy_name":"bench"}"#,
    );

    let mut run = |name: &str, state: Arc<AgentState>| {
        c.bench_function(name, |b| {
            b.iter(|| {
                let fut = fast_evaluate_policy(State(state.clone()), black_box(body.clone()));
                let resp = rt.block_on(fut);
                black_box(resp.is_ok())
            })
        });
    };

    run("fast_evaluate_policy/compiled_abac", build_state());

    // Decision logging on: entry construction + shard push, no sinks. Large
    // ring so eviction churn doesn't dominate.
    let log_config = DecisionLogConfig {
        enabled: true,
        privacy_profile: Some(PrivacyProfile::Raw),
        buffer_capacity: 100_000,
        ..Default::default()
    };
    run(
        "fast_evaluate_policy/compiled_abac_logged",
        build_state_with_buffer(Some(
            create_shared_buffer(log_config.clone()).expect("buffer"),
        )),
    );

    // Explain tier on for ALL decisions (the bench request is an allow, so
    // denies-only must be off to exercise the snapshot): adds the resolved
    // principal/resource attribute capture to every request.
    let explain_config = DecisionLogConfig {
        include_input_data: true,
        input_data_denies_only: false,
        ..log_config
    };
    run(
        "fast_evaluate_policy/compiled_abac_explain",
        build_state_with_buffer(Some(create_shared_buffer(explain_config).expect("buffer"))),
    );
}

/// Directly measures the decision-id RNG cost — this is what the `fast-rng`
/// uuid feature changes (thread-local PRNG vs a `getrandom` syscall per call).
fn bench_decision_id(c: &mut Criterion) {
    c.bench_function("decision_id/uuid_v4", |b| {
        b.iter(|| black_box(Uuid::new_v4()))
    });
}

criterion_group!(benches, bench_fast_evaluate, bench_decision_id);
criterion_main!(benches);
