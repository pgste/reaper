//! Cold-start bench (round-3 Plan 05 §4.6c, Testing T4 / ADR-4).
//!
//! Every other latency path in the repo WARMS UP first (the SLO harness runs
//! `--warmup 1000`, criterion has a warm-up phase), so the documented
//! `<100ms cold start` / `<10ms bundle load` claims were unfalsified. This bench
//! measures the UNWARMED numbers a real process pays on its first request:
//!
//!   1. bundle load  — `ReaperPolicy::from_bundle(bytes)` compiling to a ready
//!      evaluator, and
//!   2. first eval   — the very first `evaluate()` (no prior call primed any
//!      thread-local cache / regex compile / interner path).
//!
//! It deliberately measures ONE iteration each — a second call would be warm.
//! Both are compared against committed thresholds; exceeding either exits
//! non-zero so CI can gate on it. Thresholds are generous by default (shared
//! runners) and overridable via env so a dedicated-hardware run can tighten
//! them toward the real SLA:
//!   COLD_START_BUNDLE_LOAD_MS_MAX (default 10)
//!   COLD_START_FIRST_EVAL_MS_MAX  (default 100)
//!
//! Run: `cargo run -p policy-engine --release --example cold_start_bench`.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyEvaluator, PolicyRequest};

/// A representative RBAC+ABAC policy (not a single-rule toy): several rules, a
/// deny-override, attribute and membership tests — the shape a first request in
/// production actually compiles and evaluates.
const POLICY_SRC: &str = r#"
policy cold_start {
    default: deny,
    rule suspended_never { deny if user.status == "suspended" }
    rule admins_all { allow if "admin" in user.roles }
    rule same_dept_active {
        allow if {
            user.department == resource.department &&
            user.status == "active" &&
            resource.archived != true
        }
    }
    rule clearance_ok {
        allow if user.clearance >= resource.clearance_required
    }
}
"#;

fn dataset() -> serde_json::Value {
    serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {
                "roles": ["engineering"], "status": "active",
                "department": "eng", "clearance": 5
            }},
            {"id": "doc-1", "type": "resource", "attributes": {
                "department": "eng", "archived": false, "clearance_required": 3
            }}
        ]
    })
}

fn env_ms(key: &str, default: u128) -> u128 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let bundle_load_max = env_ms("COLD_START_BUNDLE_LOAD_MS_MAX", 10);
    let first_eval_max = env_ms("COLD_START_FIRST_EVAL_MS_MAX", 100);

    // --- setup (NOT measured): compile a bundle and prepare a fresh store. ---
    let policy = ReaperPolicy::from_str(POLICY_SRC).expect("policy parses");
    let bundle_bytes = policy.compile_to_bundle().expect("compile to bundle");

    let store = Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(&dataset().to_string())
        .expect("load data");

    // --- measured (1): cold bundle load into a ready evaluator. -------------
    let t0 = Instant::now();
    let evaluator = ReaperPolicy::from_bundle(&bundle_bytes, store.clone()).expect("bundle load");
    let bundle_load = t0.elapsed();

    // --- measured (2): the FIRST evaluation (nothing primed before it). -----
    let mut context = std::collections::HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    let request = PolicyRequest {
        resource: "doc-1".to_string(),
        action: "read".to_string(),
        context,
        ..Default::default()
    };
    let t1 = Instant::now();
    let decision = evaluator.evaluate(&request).expect("first eval");
    let first_eval = t1.elapsed();

    let load_ms = bundle_load.as_millis();
    let eval_ms = first_eval.as_millis();

    println!("cold-start measurements (single unwarmed iteration each):");
    println!(
        "  bundle load : {:>8.3} ms  (threshold {} ms)",
        bundle_load.as_secs_f64() * 1e3,
        bundle_load_max
    );
    println!(
        "  first eval  : {:>8.3} ms  (threshold {} ms)  [decision: {:?}]",
        first_eval.as_secs_f64() * 1e3,
        first_eval_max,
        decision
    );

    let mut failed = false;
    if load_ms > bundle_load_max {
        eprintln!("FAIL: bundle load {load_ms} ms exceeds threshold {bundle_load_max} ms");
        failed = true;
    }
    if eval_ms > first_eval_max {
        eprintln!("FAIL: first eval {eval_ms} ms exceeds threshold {first_eval_max} ms");
        failed = true;
    }

    if failed {
        std::process::exit(1);
    }
    println!("OK: cold-start within thresholds.");
}
