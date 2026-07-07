/// 10k-iteration volume test for the compiled DSL utility functions.
///
/// Exercises intersection / difference / count / has_key / values / any / all /
/// find / find_all / replace in a single policy — all on the COMPILED fast path
/// (this test fails if the policy falls back to AST) — and reports the
/// compiled-vs-AST speedup for the same policy.
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

const POLICY: &str = r#"
policy functions_volume {
    version: "1.0",
    default: deny,

    rule all_functions {
        allow if {
            i := user.skills.intersection(["rust", "python"]) && ic := i.count() && ic == 2 &&
            d := user.skills.difference(["go"]) && dc := d.count() && dc == 2 &&
            user.profile.has_key("tier") &&
            v := user.profile.values() && vc := v.count() && vc == 2 &&
            user.flags.any() &&
            user.all_flags.all() &&
            m := user.email.find("corp") && m == "corp" &&
            fa := user.csv.find_all("[a-z]") && fc := fa.count() && fc == 3 &&
            rp := user.name.replace("temp", "perm") && rp == "perm"
        }
    }
}
"#;

fn percentiles(mut lat: Vec<u128>) -> (u128, u128, u128, u128) {
    lat.sort();
    let mean = lat.iter().sum::<u128>() / lat.len() as u128;
    let p50 = lat[lat.len() / 2];
    let p95 = lat[(lat.len() as f64 * 0.95) as usize];
    let p99 = lat[(lat.len() as f64 * 0.99) as usize];
    (mean, p50, p95, p99)
}

fn run(evaluator: &dyn PolicyEvaluator, iterations: usize) -> (Vec<u128>, usize) {
    let mut latencies = Vec::with_capacity(iterations);
    let mut allow = 0;
    for i in 0..iterations {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", i % 1000));
        let request = PolicyRequest {
            resource: format!("resource_{}", i % 2000),
            action: "read".to_string(),
            context,
        };
        let start = Instant::now();
        let decision = evaluator.evaluate(&request).expect("evaluate");
        latencies.push(start.elapsed().as_nanos());
        if format!("{:?}", decision) == "Allow" {
            allow += 1;
        }
    }
    (latencies, allow)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧩 DSL Functions - 10k Iteration Volume Test\n");
    println!("{}", "=".repeat(70));

    let data_content = fs::read_to_string("test-data/functions-test-data.json")?;
    let store = DataStore::new();
    let entity_count = DataLoader::new(store.clone()).load_json(&data_content)?;
    let store = Arc::new(store);
    println!("📊 Loaded {} entities", entity_count);

    let policy = POLICY.parse::<ReaperPolicy>()?;

    // Compiled fast path — the whole point of the work. Fail loudly if this
    // policy does not land on the compiled evaluator.
    let compiled = policy.clone().build(store.clone())?;
    assert_eq!(
        compiled.evaluator_type(),
        "reaper_dsl",
        "functions policy must run on the compiled fast path, got {}",
        compiled.evaluator_type()
    );
    let ast = policy.build_ast_evaluator(store.clone());
    println!(
        "📜 Policy built (compiled='{}', ast='{}')",
        compiled.evaluator_type(),
        ast.evaluator_type()
    );

    let iterations = 10_000;
    println!(
        "\n🚀 Running {} evaluations on each evaluator...",
        iterations
    );

    // Warm up regex/interner caches so we time steady state, not first-touch.
    let _ = run(&compiled, 1000);
    let _ = run(&ast, 1000);

    // Eval-path interner bounding: this policy produces string results
    // (find/find_all/replace + set ops) every eval. With per-eval transient
    // reclamation the interner must NOT grow across the run.
    let interner_before = store.interner().stats().unique_strings;
    let (c_lat, c_allow) = run(&compiled, iterations);
    let interner_after = store.interner().stats().unique_strings;
    let (a_lat, _a_allow) = run(&ast, iterations);

    let (c_mean, c_p50, c_p95, c_p99) = percentiles(c_lat);
    let (a_mean, a_p50, _a_p95, a_p99) = percentiles(a_lat);

    println!("\n{}", "=".repeat(70));
    println!("📊 Functions Policy - Performance Results (compiled fast path)");
    println!("{}", "=".repeat(70));
    println!("\n⏱️  Latency Statistics:");
    println!("   Iterations:     {}", iterations);
    println!("   Mean latency:   {} ns", c_mean);
    println!("   Median latency: {} ns", c_p50);
    println!("   P95 latency:    {} ns", c_p95);
    println!("   P99 latency:    {} ns", c_p99);
    let interner_growth = interner_after.saturating_sub(interner_before);
    println!(
        "   Interner growth: {} strings over {} evals (bounded — transient results reclaimed)",
        interner_growth, iterations
    );
    assert!(
        interner_growth == 0,
        "eval path leaked the interner: grew {interner_before} -> {interner_after} over {iterations} evals"
    );
    println!(
        "   Allow rate:     {:.1}%",
        (c_allow as f64 / iterations as f64) * 100.0
    );

    println!("\n⚖️  Compiled vs AST (same policy, same data):");
    println!(
        "   Compiled  mean {:>6} ns   p50 {:>6} ns   p99 {:>6} ns",
        c_mean, c_p50, c_p99
    );
    println!(
        "   AST       mean {:>6} ns   p50 {:>6} ns   p99 {:>6} ns",
        a_mean, a_p50, a_p99
    );
    if c_mean > 0 {
        println!(
            "   Speedup (mean): {:.2}x    (p50): {:.2}x",
            a_mean as f64 / c_mean as f64,
            a_p50 as f64 / c_p50.max(1) as f64
        );
    }

    println!("\n{}", "=".repeat(70));
    println!("✅ Functions Volume Test Complete!");
    println!("{}", "=".repeat(70));
    Ok(())
}
