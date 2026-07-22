//! R4-01 B.3 bench: compiled check driver vs AST interpreter check.
//!
//! Runs `check_with_input` for the full kubernetes admission-control library
//! policy against a representative AdmissionReview-shaped document, on both
//! the compiled evaluator and the AST interpreter, and reports the ratio.
//! Plan target (plans/round-4/01-dsl-parity-and-fast-path.md): compiled
//! check ≥ 5x the interpreter.
//!
//! Run: cargo run --example input_check_bench --release

use policy_engine::data::DataStore;
use policy_engine::reap::ReaperPolicy;
use policy_engine::PolicyRequest;
use serde_json::json;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library");
    let k8s = std::fs::read_to_string(root.join("kubernetes/admission-control/policy.reap"))?;

    // A realistic AdmissionReview pod: two containers, one compliant and one
    // firing three violations (latest tag, unapproved registry, privileged),
    // plus a missing owner label — a mixed match/no-match workload rather
    // than an all-pass or all-fail degenerate case.
    let doc = json!({"request": {"object": {
        "metadata": {"labels": {"env": "prod"}},
        "spec": {"containers": [
            {"name": "app", "image": "registry.corp.internal/app:v1",
             "securityContext": {"privileged": false},
             "resources": {"limits": {"cpu": "1"}}},
            {"name": "sidecar", "image": "docker.io/nginx:latest",
             "securityContext": {"privileged": true}}
        ]}
    }}});

    let request = PolicyRequest {
        resource: "pod".to_string(),
        action: "admit".to_string(),
        context: HashMap::new(),
        ..Default::default()
    };

    let policy = ReaperPolicy::from_str(&k8s)?;
    let compiled = policy
        .clone()
        .build(Arc::new(DataStore::new()))
        .map_err(|e| format!("k8s library policy must compile whole: {e:?}"))?;
    let ast = policy.build_ast_evaluator(Arc::new(DataStore::new()));

    // Equivalence sanity before timing anything.
    let c = compiled.check_with_input(&request, Some(&doc))?;
    let a = ast.check_with_input(&request, Some(&doc))?;
    assert_eq!(c.allowed, a.allowed, "allowed diverged");
    assert_eq!(
        c.violations.len(),
        a.violations.len(),
        "violation count diverged"
    );
    println!(
        "check agrees: allowed={} violations={}",
        c.allowed,
        c.violations.len()
    );

    // Warmup both paths.
    for _ in 0..1_000 {
        let _ = compiled.check_with_input(&request, Some(&doc))?;
        let _ = ast.check_with_input(&request, Some(&doc))?;
    }

    let start = Instant::now();
    for _ in 0..iterations {
        let r = compiled.check_with_input(&request, Some(&doc))?;
        std::hint::black_box(r);
    }
    let compiled_elapsed = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let r = ast.check_with_input(&request, Some(&doc))?;
        std::hint::black_box(r);
    }
    let ast_elapsed = start.elapsed();

    let compiled_ns = compiled_elapsed.as_nanos() as f64 / iterations as f64;
    let ast_ns = ast_elapsed.as_nanos() as f64 / iterations as f64;
    println!("iterations:      {iterations}");
    println!("compiled check:  {compiled_ns:>10.0} ns/check");
    println!("ast check:       {ast_ns:>10.0} ns/check");
    println!("speedup:         {:>10.2}x", ast_ns / compiled_ns);
    Ok(())
}
