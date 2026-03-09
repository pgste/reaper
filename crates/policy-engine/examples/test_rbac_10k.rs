/// 10k iteration performance test for RBAC (Role-Based Access Control)
///
/// Tests the rbac.reap policy against rbac-test-data.json
/// Measures performance and decision patterns
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 RBAC Policy - 10k Iteration Performance Test\n");
    println!("{}", "=".repeat(70));

    // Load data
    println!("\n📊 Loading test data...");
    let data_content = fs::read_to_string("test-data/rbac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    // Load and compile policy
    println!("📜 Loading RBAC policy...");
    let policy = ReaperPolicy::from_file("crates/policy-engine/examples/policies/rbac.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled successfully");

    // Run 10k evaluations
    let iterations = 10_000;
    println!("\n🚀 Running {} policy evaluations...\n", iterations);

    let mut latencies = Vec::with_capacity(iterations);
    let mut allow_count = 0;
    let mut deny_count = 0;

    // Test different scenarios
    let test_cases = vec![
        // Admin accessing any resource (should allow)
        (
            "user_0",
            "resource_100",
            "Expected: ALLOW (admin full access)",
        ),
        // Manager accessing reports (should allow)
        (
            "user_1",
            "resource_0",
            "Expected: ALLOW (manager accessing report)",
        ),
        // User accessing own resource (should allow)
        (
            "user_10",
            "resource_10",
            "Expected: ALLOW (user owns resource)",
        ),
        // User accessing other's non-report resource (should deny)
        (
            "user_50",
            "resource_100",
            "Expected: DENY (no relationship)",
        ),
        // Manager accessing non-report (should deny unless owner)
        (
            "user_1",
            "resource_2",
            "Expected: DENY (manager, not report, not owner)",
        ),
    ];

    println!("📋 Sample Test Cases:");
    for (principal, resource, expected) in &test_cases {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal.to_string());

        let request = PolicyRequest {
            resource: resource.to_string(),
            action: "read".to_string(),
            context,
        };

        let start = Instant::now();
        let decision = evaluator.evaluate(&request)?;
        let elapsed = start.elapsed().as_nanos();

        let decision_str = format!("{:?}", decision);
        println!(
            "   {} → {} ({}ns) - {}",
            principal, resource, elapsed, expected
        );
        println!("      Result: {}", decision_str);
    }

    println!("\n🔄 Running full {} iteration test...", iterations);

    let start_time = Instant::now();

    for i in 0..iterations {
        // Vary the test scenarios
        let user_id = format!("user_{}", i % 1000);
        let resource_id = format!("resource_{}", i % 2000);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,
        };

        let eval_start = Instant::now();
        let decision = evaluator.evaluate(&request)?;
        let elapsed = eval_start.elapsed().as_nanos();

        latencies.push(elapsed);

        match format!("{:?}", decision).as_str() {
            "Allow" => allow_count += 1,
            _ => deny_count += 1,
        }

        if (i + 1) % 1000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }

    let total_time = start_time.elapsed();
    println!("\n   ✓ Completed");

    // Calculate statistics
    latencies.sort();
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

    let sum_sq_diff: f64 = latencies
        .iter()
        .map(|&x| {
            let diff = x as f64 - mean as f64;
            diff * diff
        })
        .sum();
    let std_dev = (sum_sq_diff / latencies.len() as f64).sqrt();

    // Print results
    println!("\n{}", "=".repeat(70));
    println!("📊 RBAC Policy - Performance Results");
    println!("{}", "=".repeat(70));

    println!("\n⏱️  Latency Statistics:");
    println!("   Total time:     {:?}", total_time);
    println!("   Iterations:     {}", iterations);
    println!("   Min latency:    {} ns", min);
    println!("   Mean latency:   {} ns", mean);
    println!("   Median latency: {} ns", median);
    println!("   P95 latency:    {} ns", p95);
    println!("   P99 latency:    {} ns", p99);
    println!("   Max latency:    {} ns", max);
    println!("   Std deviation:  {:.2} ns", std_dev);

    println!("\n🚀 Throughput:");
    println!(
        "   Ops/second:     {:.0}",
        iterations as f64 / total_time.as_secs_f64()
    );
    println!("   Avg per op:     {:.2} µs", mean as f64 / 1000.0);

    println!("\n✅ Decision Distribution:");
    println!(
        "   ALLOW:          {} ({:.1}%)",
        allow_count,
        (allow_count as f64 / iterations as f64) * 100.0
    );
    println!(
        "   DENY:           {} ({:.1}%)",
        deny_count,
        (deny_count as f64 / iterations as f64) * 100.0
    );

    // Analyze performance buckets
    let mut buckets = vec![
        (500, 0),
        (1000, 0),
        (2000, 0),
        (5000, 0),
        (10000, 0),
        (u128::MAX, 0),
    ];

    for &latency in &latencies {
        for (threshold, count) in &mut buckets {
            if latency <= *threshold {
                *count += 1;
                break;
            }
        }
    }

    println!("\n📈 Latency Distribution:");
    println!(
        "   < 500 ns:       {} ({:.1}%)",
        buckets[0].1,
        (buckets[0].1 as f64 / iterations as f64) * 100.0
    );
    println!(
        "   < 1 µs:         {} ({:.1}%)",
        buckets[1].1,
        (buckets[1].1 as f64 / iterations as f64) * 100.0
    );
    println!(
        "   < 2 µs:         {} ({:.1}%)",
        buckets[2].1,
        (buckets[2].1 as f64 / iterations as f64) * 100.0
    );
    println!(
        "   < 5 µs:         {} ({:.1}%)",
        buckets[3].1,
        (buckets[3].1 as f64 / iterations as f64) * 100.0
    );
    println!(
        "   < 10 µs:        {} ({:.1}%)",
        buckets[4].1,
        (buckets[4].1 as f64 / iterations as f64) * 100.0
    );
    println!(
        "   >= 10 µs:       {} ({:.1}%)",
        buckets[5].1,
        (buckets[5].1 as f64 / iterations as f64) * 100.0
    );

    println!("\n{}", "=".repeat(70));
    println!("✅ RBAC Policy Test Complete!");
    println!("{}", "=".repeat(70));

    Ok(())
}
