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
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use uuid::Uuid;

use policy_engine::{
    cache_config::CacheConfig, DataLoader, DataStore, EnhancedPolicy, PolicyEngine, PolicyLanguage,
    ReaperPolicy,
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

fn build_state() -> Arc<AgentState> {
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
        decision_buffer: None,
        agent_id: "bench".to_string(),
    })
}

fn bench_fast_evaluate(c: &mut Criterion) {
    let state = build_state();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let body = Bytes::from(
        r#"{"principal":"alice","action":"read","resource":"doc1","policy_name":"bench"}"#,
    );

    c.bench_function("fast_evaluate_policy/compiled_abac", |b| {
        b.iter(|| {
            let fut = fast_evaluate_policy(State(state.clone()), black_box(body.clone()));
            let resp = rt.block_on(fut);
            black_box(resp.is_ok())
        })
    });
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
