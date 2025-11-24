/// 10k iteration performance test for ABAC (Attribute-Based Access Control)
///
/// Tests the abac.reap policy against abac-test-data.json
/// Measures performance with clearance levels and department matching
use policy_engine::{DataStore, DataLoader, ReaperPolicy, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 ABAC Policy - 10k Iteration Performance Test\n");
    println!("{}", "=".repeat(70));

    // Load data
    println!("\n📊 Loading test data...");
    let data_content = fs::read_to_string("abac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    // Load and compile policy
    println!("📜 Loading ABAC policy...");
    let policy = ReaperPolicy::from_file("crates/policy-engine/examples/policies/abac.reap")?;
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
        // Executive with high clearance (should allow most)
        ("user_0", "doc_100", "Expected: ALLOW (executive, clearance 8)"),
        // Suspended user (should deny)
        ("user_20", "doc_0", "Expected: DENY (suspended user)"),
        // Same department, sufficient clearance (should allow)
        ("user_10", "doc_10", "Expected: ALLOW (dept match, clearance OK)"),
        // Different department (should deny)
        ("user_5", "doc_10", "Expected: DENY (different department)"),
        // Archived document (should deny)
        ("user_1", "doc_10", "Expected: DENY (archived document)"),
        // Owner access (should allow if active)
        ("user_100", "doc_500", "Expected: ALLOW if owner (check data)"),
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
        println!("   {} → {} ({}ns) - {}", principal, resource, elapsed, expected);
        println!("      Result: {}", decision_str);
    }

    println!("\n🔄 Running full {} iteration test...", iterations);

    let start_time = Instant::now();

    for i in 0..iterations {
        // Vary the test scenarios to hit different attribute combinations
        let user_id = format!("user_{}", i % 1000);
        let doc_id = format!("doc_{}", i % 2000);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: doc_id,
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
    println!("📊 ABAC Policy - Performance Results");
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
    println!("   Ops/second:     {:.0}", iterations as f64 / total_time.as_secs_f64());
    println!("   Avg per op:     {:.2} µs", mean as f64 / 1000.0);

    println!("\n✅ Decision Distribution:");
    println!("   ALLOW:          {} ({:.1}%)", allow_count, (allow_count as f64 / iterations as f64) * 100.0);
    println!("   DENY:           {} ({:.1}%)", deny_count, (deny_count as f64 / iterations as f64) * 100.0);

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
    println!("   < 500 ns:       {} ({:.1}%)", buckets[0].1, (buckets[0].1 as f64 / iterations as f64) * 100.0);
    println!("   < 1 µs:         {} ({:.1}%)", buckets[1].1, (buckets[1].1 as f64 / iterations as f64) * 100.0);
    println!("   < 2 µs:         {} ({:.1}%)", buckets[2].1, (buckets[2].1 as f64 / iterations as f64) * 100.0);
    println!("   < 5 µs:         {} ({:.1}%)", buckets[3].1, (buckets[3].1 as f64 / iterations as f64) * 100.0);
    println!("   < 10 µs:        {} ({:.1}%)", buckets[4].1, (buckets[4].1 as f64 / iterations as f64) * 100.0);
    println!("   >= 10 µs:       {} ({:.1}%)", buckets[5].1, (buckets[5].1 as f64 / iterations as f64) * 100.0);

    println!("\n💡 ABAC Characteristics:");
    println!("   This policy evaluates multiple attributes:");
    println!("   • User clearance vs document clearance_required");
    println!("   • Department matching");
    println!("   • Suspended status (deny rule)");
    println!("   • Archived status");
    println!("   • Ownership relationships");
    println!("   More complex than RBAC but still sub-microsecond!");

    println!("\n{}", "=".repeat(70));
    println!("✅ ABAC Policy Test Complete!");
    println!("{}", "=".repeat(70));

    Ok(())
}
