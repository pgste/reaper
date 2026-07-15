//! D2 impact measurement — evidence that compiled DSL resource-literal
//! extraction actually reduces evaluate-all work at scale.
//!
//! Deploys N DSL policies, each `allow if resource == "/res/i"`, with a real
//! populated DataStore so evaluation runs to a genuine Allow decision, then
//! compares the two candidate sets the evaluate-all handler could iterate:
//!   - PRE-D2  (full scan): every policy in `list_policies()` — what a DSL fleet
//!     evaluated before D2 (all policies were `unprunable`).
//!   - POST-D2 (pruned):    only `candidate_policy_ids(resource)` — the
//!     resource-bucketed candidates D2 makes available for DSL.
//!
//! Run: `cargo run --release --example d2_pruning_impact`
use policy_engine::data::entity::EntityBuilder;
use policy_engine::{
    DataStore, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyLanguage, PolicyRequest,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    const N: usize = 10_000;
    const ITERS: usize = 500;

    // A populated store so the DSL evaluator resolves the principal and runs a
    // real evaluation (it fails closed on an unknown principal otherwise).
    let store = Arc::new(DataStore::new());
    {
        let interner = store.interner();
        let id = interner.intern_counted("alice");
        let etype = interner.intern("User");
        store.insert(EntityBuilder::new(id, etype).build());
    }

    let engine = PolicyEngine::new();
    for i in 0..N {
        let content = format!(
            "policy d{i} {{\n    default: deny,\n    rule r {{\n        allow if resource == \"/res/{i}\"\n    }}\n}}"
        );
        let mut p = EnhancedPolicy::new_with_language(
            format!("d{i}"),
            String::new(),
            PolicyLanguage::ReaperDsl,
            content,
        )
        .unwrap();
        p.build_evaluator_with_data(Some(store.clone())).unwrap();
        engine.deploy_policy(p).unwrap();
    }

    let target = "/res/5000";
    let request = PolicyRequest {
        resource: target.to_string(),
        action: "read".to_string(),
        context: {
            let mut c = HashMap::new();
            c.insert("principal".to_string(), "alice".to_string());
            c
        },

        ..Default::default()
    };

    let stats = engine.get_index_stats();
    let all_ids: Vec<_> = engine.list_policies().iter().map(|p| p.id).collect();

    // Correctness first: both paths must reach a real Allow (pruning changes
    // cost, never the decision). If the candidate path can't produce the allow,
    // the whole measurement is meaningless — assert it up front.
    let candidate_ids = engine.candidate_policy_ids(target);
    let full_allow = all_ids
        .iter()
        .filter_map(|id| engine.evaluate(id, &request).ok())
        .any(|d| matches!(d.decision, PolicyAction::Allow));
    let pruned_allow = candidate_ids
        .iter()
        .filter_map(|id| engine.evaluate(id, &request).ok())
        .any(|d| matches!(d.decision, PolicyAction::Allow));
    assert!(
        full_allow && pruned_allow,
        "both paths must yield a real Allow (full={full_allow}, pruned={pruned_allow})"
    );

    println!("=== D2 pruning impact — {N} DSL policies, each `resource == \"/res/i\"` ===");
    println!(
        "index: {} resource_buckets, {} unprunable, {} total",
        stats.resource_buckets, stats.unprunable_policies, stats.total_policies
    );
    println!(
        "evaluate-all candidate set for {target}: PRE-D2 = {} policies, POST-D2 = {} policies (both decide Allow)",
        all_ids.len(),
        candidate_ids.len()
    );

    // PRE-D2: evaluate-all iterates every policy.
    let t0 = Instant::now();
    let mut sink = 0u64;
    for _ in 0..ITERS {
        for id in &all_ids {
            if let Ok(d) = engine.evaluate(id, &request) {
                sink += d.evaluation_time_ns;
            }
        }
    }
    let pre = t0.elapsed();
    std::hint::black_box(sink);

    // POST-D2: evaluate-all iterates only the candidates candidate_policy_ids
    // returns (index lookup + eval of the bucketed candidates).
    let t1 = Instant::now();
    let mut sink2 = 0u64;
    for _ in 0..ITERS {
        for id in engine.candidate_policy_ids(target) {
            if let Ok(d) = engine.evaluate(&id, &request) {
                sink2 += d.evaluation_time_ns;
            }
        }
    }
    let post = t1.elapsed();
    std::hint::black_box(sink2);

    let pre_us = pre.as_nanos() as f64 / ITERS as f64 / 1000.0;
    let post_us = post.as_nanos() as f64 / ITERS as f64 / 1000.0;
    println!("\nper evaluate-all request (mean over {ITERS} iters, real Allow decisions):");
    println!(
        "  PRE-D2  (scan {:>5} policies): {:>10.2} µs",
        all_ids.len(),
        pre_us
    );
    println!(
        "  POST-D2 (index + {:>2} candidate): {:>10.2} µs",
        candidate_ids.len(),
        post_us
    );
    println!(
        "  → {:.0}x fewer evaluations, {:.0}x faster wall-clock",
        all_ids.len() as f64 / candidate_ids.len().max(1) as f64,
        pre_us / post_us
    );
}
