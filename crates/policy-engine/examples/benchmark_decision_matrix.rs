/// Benchmark: Decision Matrix Performance
///
/// Verify that precomputed decision matrix actually provides sub-100ns lookups
use policy_engine::{DecisionMatrix, EnhancedPolicy, PolicyAction, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(80));
    println!("📊 Decision Matrix Performance Verification");
    println!("{}", "=".repeat(80));

    // Create a simple RBAC policy
    let policy = EnhancedPolicy::new(
        "rbac-policy".to_string(),
        "Simple RBAC".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/api/users".to_string(),
            conditions: vec!["role==admin".to_string()],
        }],
    );

    // Define bounded space
    let principals = vec![
        "user1".to_string(),
        "user2".to_string(),
        "user3".to_string(),
    ];
    let resources = vec!["/api/users".to_string(), "/api/posts".to_string()];
    let actions = vec!["read".to_string(), "write".to_string()];

    // Contexts with different roles
    let mut contexts = Vec::new();
    for role in &["admin", "user", "guest"] {
        let mut context = HashMap::new();
        context.insert("role".to_string(), role.to_string());
        contexts.push(context);
    }

    println!("\n📋 Precomputing decision matrix...");
    println!("   Principals: {}", principals.len());
    println!("   Resources:  {}", resources.len());
    println!("   Actions:    {}", actions.len());
    println!("   Contexts:   {}", contexts.len());
    println!(
        "   Total combinations: {}",
        principals.len() * resources.len() * actions.len() * contexts.len()
    );

    let matrix = DecisionMatrix::new();

    let precompute_start = Instant::now();
    let count = matrix.precompute(
        &policy,
        principals.clone(),
        resources.clone(),
        actions.clone(),
        contexts.clone(),
    )?;
    let precompute_time = precompute_start.elapsed();

    println!("\n✅ Precomputation complete:");
    println!("   Decisions computed: {}", count);
    println!("   Time: {:?}", precompute_time);
    println!(
        "   Per decision: {:.2}ns",
        precompute_time.as_nanos() as f64 / count as f64
    );

    // Now benchmark lookups
    println!("\n🔍 Benchmarking lookups...");
    let iterations = 10_000;

    let mut latencies = Vec::new();
    let mut hits = 0;

    for i in 0..iterations {
        let principal = &principals[i % principals.len()];
        let resource = &resources[i % resources.len()];
        let action = &actions[i % actions.len()];
        let context = &contexts[i % contexts.len()];

        let request = PolicyRequest {
            resource: resource.clone(),
            action: action.clone(),
            context: context.clone(),
        };

        let start = Instant::now();
        let decision = matrix.lookup(&request, principal);
        let elapsed = start.elapsed().as_nanos();

        latencies.push(elapsed);

        if decision.is_some() {
            hits += 1;
        }
    }

    latencies.sort();
    let min = latencies[0];
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let max = latencies[latencies.len() - 1];

    println!("\n📊 Lookup Performance ({} iterations):", iterations);
    println!("   Min:    {:>6} ns", min);
    println!("   Mean:   {:>6} ns", mean);
    println!("   Median: {:>6} ns", median);
    println!("   P95:    {:>6} ns", p95);
    println!("   P99:    {:>6} ns", p99);
    println!("   Max:    {:>6} ns", max);
    println!("\n   Hits:   {}/{}", hits, iterations);

    // Verdict
    println!("\n{}", "=".repeat(80));
    println!("🏆 VERDICT:");
    println!("{}", "=".repeat(80));

    if mean < 100 {
        println!("✅ SUCCESS! Mean lookup time: {}ns (< 100ns target)", mean);
        println!("   Decision Matrix works as designed!");
    } else if mean < 500 {
        println!(
            "⚠️  ACCEPTABLE: Mean lookup time: {}ns (close to target)",
            mean
        );
    } else {
        println!("❌ FAILED: Mean lookup time: {}ns (>> 100ns target)", mean);
    }

    println!("\n{}", "=".repeat(80));

    Ok(())
}
