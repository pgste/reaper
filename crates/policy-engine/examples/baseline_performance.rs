/// Baseline Performance Benchmark
///
/// Measures current performance BEFORE optimizations
/// This is our reference point to ensure no regressions
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(80));
    println!("📊 BASELINE PERFORMANCE - Before Optimizations");
    println!("{}", "=".repeat(80));

    let iterations = 10_000;

    // ========================================================================
    // Test 1: RBAC Policy with Data Store
    // ========================================================================
    println!("\n📋 Test 1: RBAC Policy Evaluation");
    println!("{}", "-".repeat(80));

    let data_content = fs::read_to_string("test-data/rbac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   Loaded {} entities", entity_count);

    let policy = ReaperPolicy::from_file("crates/policy-engine/examples/policies/rbac.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   Policy compiled");

    let mut latencies = Vec::with_capacity(iterations);
    let mut allow_count = 0;

    for i in 0..iterations {
        let user_id = format!("user_{}", i % 100);
        let resource_id = format!("resource_{}", i % 100);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let start = Instant::now();
        let decision = evaluator.evaluate(&request)?;
        let elapsed = start.elapsed().as_nanos();

        latencies.push(elapsed);

        if format!("{:?}", decision).contains("Allow") {
            allow_count += 1;
        }

        if (i + 1) % 2000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }
    println!();

    latencies.sort();
    let min = latencies[0];
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let max = latencies[latencies.len() - 1];

    println!("\n   Results:");
    println!("   Min:    {:>8} ns", min);
    println!("   Mean:   {:>8} ns ({:.2} µs)", mean, mean as f64 / 1000.0);
    println!("   Median: {:>8} ns", median);
    println!("   P95:    {:>8} ns", p95);
    println!("   P99:    {:>8} ns", p99);
    println!("   Max:    {:>8} ns", max);
    println!("   Allowed: {}/{}", allow_count, iterations);

    // ========================================================================
    // Save baseline for comparison
    // ========================================================================
    println!("\n{}", "=".repeat(80));
    println!("💾 BASELINE METRICS (save these for comparison):");
    println!("{}", "=".repeat(80));
    println!("\nRBAC Policy:");
    println!("  Mean:   {} ns", mean);
    println!("  Median: {} ns", median);
    println!("  P99:    {} ns", p99);

    let throughput = iterations as f64 / (latencies.iter().sum::<u128>() as f64 / 1_000_000_000.0);
    println!("\nThroughput: {:.0} requests/second", throughput);

    println!("\n{}", "=".repeat(80));
    println!("✅ Baseline established. Use these numbers to verify no regressions.");
    println!("{}", "=".repeat(80));

    Ok(())
}
