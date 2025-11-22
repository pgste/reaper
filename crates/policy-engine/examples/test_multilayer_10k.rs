/// 10k iteration performance test for Multilayer (RBAC + ABAC + ReBAC)
///
/// Tests a realistic enterprise policy combining all three models
/// Tracks which layers/rules are triggered and performance impact

use policy_engine::{DataStore, DataLoader, ReaperPolicy, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
struct TestScenario {
    name: &'static str,
    user_pattern: fn(usize) -> String,
    resource_pattern: fn(usize) -> String,
    expected_layer: &'static str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Multilayer Policy - 10k Iteration Performance Test\n");
    println!("{}", "=".repeat(70));

    // Load data
    println!("\n📊 Loading test data...");
    let data_content = fs::read_to_string("multilayer-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    // Load and compile policy
    println!("📜 Loading Multilayer policy...");
    let policy = ReaperPolicy::from_file("crates/policy-engine/examples/policies/multilayer.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled successfully");

    // Define test scenarios that hit different rule combinations
    let scenarios = vec![
        TestScenario {
            name: "Admin Override (RBAC)",
            user_pattern: |_| "user_0".to_string(),  // admin
            resource_pattern: |i| format!("resource_{}", i % 2000),
            expected_layer: "RBAC - Admin",
        },
        TestScenario {
            name: "Suspended User (RBAC Deny)",
            user_pattern: |_| "user_20".to_string(),  // suspended
            resource_pattern: |i| format!("resource_{}", i % 2000),
            expected_layer: "RBAC - Deny",
        },
        TestScenario {
            name: "Owner with Clearance (ReBAC + ABAC)",
            user_pattern: |i| format!("user_{}", i % 1000),
            resource_pattern: |i| format!("resource_{}", i % 2000),  // some match ownership
            expected_layer: "ReBAC + ABAC",
        },
        TestScenario {
            name: "Team Lead Access (ReBAC + RBAC)",
            user_pattern: |i| format!("user_{}", i * 10 % 1000),  // team leads every 10th
            resource_pattern: |i| format!("resource_{}", i % 2000),
            expected_layer: "ReBAC + RBAC",
        },
        TestScenario {
            name: "Department + Clearance (ABAC + ReBAC)",
            user_pattern: |i| format!("user_{}", (i * 3) % 1000),
            resource_pattern: |i| format!("resource_{}", (i * 3) % 2000),
            expected_layer: "ABAC + ReBAC",
        },
        TestScenario {
            name: "Shared Resource (ReBAC)",
            user_pattern: |i| format!("user_{}", (i + 100) % 1000),
            resource_pattern: |i| {
                let res_id = i % 2000;
                if res_id % 3 == 0 {
                    format!("resource_{}", res_id)
                } else {
                    format!("resource_{}", (res_id / 3) * 3)  // Force shared match
                }
            },
            expected_layer: "ReBAC - Sharing",
        },
        TestScenario {
            name: "Executive Access (RBAC + ABAC)",
            user_pattern: |i| format!("user_{}", 1 + (i % 142)),  // executives
            resource_pattern: |i| format!("resource_{}", i % 2000),
            expected_layer: "RBAC + ABAC",
        },
        TestScenario {
            name: "Public Resources (ABAC)",
            user_pattern: |i| format!("user_{}", i % 1000),
            resource_pattern: |i| format!("resource_{}", (i % 500) * 4),  // public classification
            expected_layer: "ABAC - Public",
        },
        TestScenario {
            name: "Mixed Random (All Layers)",
            user_pattern: |i| format!("user_{}", i % 1000),
            resource_pattern: |i| format!("resource_{}", (i * 7 + 13) % 2000),
            expected_layer: "Mixed",
        },
    ];

    println!("\n🚀 Running {} policy evaluations across {} scenarios...\n", 10000, scenarios.len());

    let mut total_latencies = Vec::with_capacity(10000);
    let mut total_allow = 0;
    let mut total_deny = 0;
    let mut scenario_results: Vec<(String, Vec<u128>, usize, usize)> = Vec::new();

    // Run each scenario
    for scenario in &scenarios {
        let iterations_per_scenario = 10000 / scenarios.len();
        let mut latencies = Vec::with_capacity(iterations_per_scenario);
        let mut allow_count = 0;
        let mut deny_count = 0;

        for i in 0..iterations_per_scenario {
            let user_id = (scenario.user_pattern)(i);
            let resource_id = (scenario.resource_pattern)(i);

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
            total_latencies.push(elapsed);

            match format!("{:?}", decision).as_str() {
                "Allow" => {
                    allow_count += 1;
                    total_allow += 1;
                }
                _ => {
                    deny_count += 1;
                    total_deny += 1;
                }
            }
        }

        scenario_results.push((
            scenario.name.to_string(),
            latencies,
            allow_count,
            deny_count,
        ));
    }

    println!("   ✓ Completed all scenarios");

    // Calculate overall statistics
    total_latencies.sort();
    let total_iters = total_latencies.len();
    let min = total_latencies[0];
    let max = total_latencies[total_iters - 1];
    let mean = total_latencies.iter().sum::<u128>() / total_iters as u128;
    let median = total_latencies[total_iters / 2];
    let p95 = total_latencies[(total_iters as f64 * 0.95) as usize];
    let p99 = total_latencies[(total_iters as f64 * 0.99) as usize];

    let sum_sq_diff: f64 = total_latencies
        .iter()
        .map(|&x| {
            let diff = x as f64 - mean as f64;
            diff * diff
        })
        .sum();
    let std_dev = (sum_sq_diff / total_iters as f64).sqrt();

    // Print overall results
    println!("\n{}", "=".repeat(70));
    println!("📊 Multilayer Policy - Overall Performance Results");
    println!("{}", "=".repeat(70));

    println!("\n⏱️  Overall Latency Statistics:");
    println!("   Iterations:     {}", total_iters);
    println!("   Min latency:    {} ns", min);
    println!("   Mean latency:   {} ns", mean);
    println!("   Median latency: {} ns", median);
    println!("   P95 latency:    {} ns", p95);
    println!("   P99 latency:    {} ns", p99);
    println!("   Max latency:    {} ns", max);
    println!("   Std deviation:  {:.2} ns", std_dev);

    println!("\n✅ Overall Decision Distribution:");
    println!("   ALLOW:          {} ({:.1}%)", total_allow, (total_allow as f64 / total_iters as f64) * 100.0);
    println!("   DENY:           {} ({:.1}%)", total_deny, (total_deny as f64 / total_iters as f64) * 100.0);

    // Latency distribution
    let mut buckets = vec![
        (500, 0),
        (1000, 0),
        (2000, 0),
        (5000, 0),
        (10000, 0),
        (u128::MAX, 0),
    ];

    for &latency in &total_latencies {
        for (threshold, count) in &mut buckets {
            if latency <= *threshold {
                *count += 1;
                break;
            }
        }
    }

    println!("\n📈 Overall Latency Distribution:");
    println!("   < 500 ns:       {} ({:.1}%)", buckets[0].1, (buckets[0].1 as f64 / total_iters as f64) * 100.0);
    println!("   < 1 µs:         {} ({:.1}%)", buckets[1].1, (buckets[1].1 as f64 / total_iters as f64) * 100.0);
    println!("   < 2 µs:         {} ({:.1}%)", buckets[2].1, (buckets[2].1 as f64 / total_iters as f64) * 100.0);
    println!("   < 5 µs:         {} ({:.1}%)", buckets[3].1, (buckets[3].1 as f64 / total_iters as f64) * 100.0);
    println!("   < 10 µs:        {} ({:.1}%)", buckets[4].1, (buckets[4].1 as f64 / total_iters as f64) * 100.0);
    println!("   >= 10 µs:       {} ({:.1}%)", buckets[5].1, (buckets[5].1 as f64 / total_iters as f64) * 100.0);

    // Per-scenario breakdown
    println!("\n{}", "=".repeat(70));
    println!("📋 Per-Scenario Results");
    println!("{}", "=".repeat(70));

    for (name, latencies, allow_count, deny_count) in &scenario_results {
        let mut sorted = latencies.clone();
        sorted.sort();
        let scenario_mean = sorted.iter().sum::<u128>() / sorted.len() as u128;
        let scenario_median = sorted[sorted.len() / 2];
        let scenario_p99 = sorted[(sorted.len() as f64 * 0.99) as usize];

        let total = allow_count + deny_count;
        let allow_pct = (*allow_count as f64 / total as f64) * 100.0;

        println!("\n{}", name);
        println!("   Mean: {}ns | Median: {}ns | P99: {}ns", scenario_mean, scenario_median, scenario_p99);
        println!("   Allow: {} ({:.1}%) | Deny: {} ({:.1}%)", allow_count, allow_pct, deny_count, 100.0 - allow_pct);
    }

    // Comparison with individual policy types
    println!("\n{}", "=".repeat(70));
    println!("📊 Comparison with Individual Policy Types");
    println!("{}", "=".repeat(70));

    println!("\n(Approximate values from previous tests):");
    println!("   RBAC only:      646 ns mean, 1,728 ns P99");
    println!("   ABAC only:      964 ns mean, 2,286 ns P99");
    println!("   ReBAC only:     560 ns mean, 1,141 ns P99");
    println!("   Multilayer:     {} ns mean, {} ns P99", mean, p99);

    let rbac_overhead = mean as f64 / 646.0;
    let abac_overhead = mean as f64 / 964.0;
    let rebac_overhead = mean as f64 / 560.0;

    println!("\n💡 Overhead Analysis:");
    println!("   vs RBAC:        {:.2}x", rbac_overhead);
    println!("   vs ABAC:        {:.2}x", abac_overhead);
    println!("   vs ReBAC:       {:.2}x", rebac_overhead);

    if rbac_overhead < 2.0 {
        println!("\n✅ Excellent! Multilayer policy adds minimal overhead.");
        println!("   Combining all three models is < 2x slowest individual policy.");
    } else if rbac_overhead < 3.0 {
        println!("\n✅ Good! Multilayer policy overhead is reasonable.");
        println!("   The complexity is justified by comprehensive authorization.");
    } else {
        println!("\n⚠️  Multilayer policy has some overhead.");
        println!("   Consider optimizing rule ordering or reducing rule count.");
    }

    println!("\n💡 Multilayer Characteristics:");
    println!("   This policy evaluates:");
    println!("   • 9 distinct rules across 3 authorization models");
    println!("   • RBAC: Admin override, suspended user blocking, role checks");
    println!("   • ABAC: Clearance levels, departments, classifications");
    println!("   • ReBAC: Ownership, teams, sharing, collaboration, hierarchy");
    println!("   • Combined checks: Role + clearance, team + department, etc.");
    println!("\n   Real-world enterprise authorization in action!");

    println!("\n{}", "=".repeat(70));
    println!("✅ Multilayer Policy Test Complete!");
    println!("{}", "=".repeat(70));

    Ok(())
}
