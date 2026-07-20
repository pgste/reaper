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
        capability_gate: std::sync::Arc::new(
            reaper_agent::capability_cache::CapabilityGateRuntime::from_auth(
                &reaper_core::config::AgentAuthSettings::default(),
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

/// Plan 06 Phase D DoD: steady-state agentic throughput is not gated by
/// ed25519 cost. Benches the two capability-gate path bodies side by side:
/// - `verdict_cache_hit`: digest + cache lookup + window/revocation check —
///   the ENTIRE per-request crypto-path work once a capability's verdict is
///   cached (expected ~hundreds of ns);
/// - `full_verify`: one `verify_capability_with` — the ed25519 cost every
///   request paid before Phase D (expected tens of µs).
fn bench_capability_gate(c: &mut Criterion) {
    use reaper_agent::capability_cache::CapabilityVerdictCache;
    use reaper_agent::management::verify::BundleVerifier;
    use reaper_core::bundle_signing::SigningKey;
    use reaper_core::capability::{issue, Grant};
    use reaper_core::config::ManagementSettings;

    let key = SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])));
    let verifier = BundleVerifier::from_config(&ManagementSettings {
        enabled: true,
        bundle_public_key: Some(key.public_key_hex()),
        bundle_key_id: Some("k1".to_string()),
        ..Default::default()
    });
    let now = reaper_agent::capability_gate::now_unix();
    let cap = issue(
        &key,
        "k1",
        "alice",
        "agent-1",
        vec![Grant::new("read", "/doc/*")],
        now - 300,
        now + 3600,
    )
    .expect("issue capability");
    let revoked = std::collections::HashSet::new();

    let cache = CapabilityVerdictCache::new(1024, 300);
    let cache_key = (cap.cache_digest(), 0u64);
    cache.insert(cache_key, now);
    c.bench_function("capability/verdict_cache_hit", |b| {
        b.iter(|| {
            let key = (black_box(&cap).cache_digest(), 0u64);
            assert!(cache.check(&key, now));
            black_box(&cap).check_validity_at(now, &revoked).unwrap();
        })
    });

    c.bench_function("capability/full_verify", |b| {
        b.iter(|| {
            verifier
                .verify_capability_with(black_box(&cap), now, &revoked)
                .unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_fast_evaluate,
    bench_decision_id,
    bench_capability_gate
);
criterion_main!(benches);
