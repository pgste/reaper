/// CRITICAL COMPARISON TEST: Indexed Engine vs Linear Scan
///
/// This test directly compares:
/// 1. Baseline: PolicyEngine with linear scan through N policies
/// 2. Optimized: IndexedPolicyEngine with O(1) indexed lookup
///
/// Both use identical EnhancedPolicy objects to ensure fair comparison.
/// Goal: Prove that indexing provides dramatic speedup for large policy sets
use policy_engine::{
    EnhancedPolicy, IndexedPolicyEngine, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule,
};
use std::collections::HashMap;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(80));
    println!("🔬 CRITICAL COMPARISON: Indexed Engine vs Linear Scan");
    println!("{}", "=".repeat(80));

    let iterations = 10_000;
    let policy_count = 1000; // Test with 1000 policies

    println!("\n📊 Test Configuration:");
    println!("   Policies: {}", policy_count);
    println!("   Iterations: {}", iterations);
    println!("   Goal: Prove indexing beats linear scan for large policy sets\n");

    // ========================================================================
    // CREATE IDENTICAL POLICIES FOR BOTH TESTS
    // ========================================================================
    println!("Creating {} policies...", policy_count);

    let mut policies = Vec::new();

    // Create diverse policies with different resource patterns
    for i in 0..policy_count {
        let policy = EnhancedPolicy::new(
            format!("policy-{}", i),
            format!("Policy for resource range {}-{}", i * 10, (i + 1) * 10),
            vec![
                PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource_{}", i * 10),
                    conditions: vec!["role==user".to_string()],
                },
                PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource_{}", i * 10 + 5),
                    conditions: vec!["role==admin".to_string()],
                },
            ],
        );
        policies.push(policy);
    }

    // Add wildcard admin policy (should be found quickly by both)
    let admin_policy = EnhancedPolicy::new(
        "admin-wildcard".to_string(),
        "Admin full access".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec!["role==admin".to_string()],
        }],
    );
    policies.push(admin_policy);

    println!("✓ Created {} policies\n", policies.len());

    // ========================================================================
    // BASELINE TEST: Linear Scan with PolicyEngine
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 BASELINE: PolicyEngine (Linear Scan)");
    println!("{}", "=".repeat(80));

    let baseline_engine = PolicyEngine::new();

    // Deploy all policies
    println!(
        "Deploying {} policies to baseline engine...",
        policies.len()
    );
    for policy in policies.clone() {
        baseline_engine.deploy_policy(policy)?;
    }
    println!("✓ Policies deployed\n");

    // Get policy IDs for evaluation
    let policy_ids: Vec<_> = baseline_engine
        .list_policies()
        .iter()
        .map(|p| p.id)
        .collect();

    println!("🔄 Running BASELINE test ({} iterations)...", iterations);
    let baseline_start = Instant::now();
    let mut baseline_latencies = Vec::with_capacity(iterations);
    let mut baseline_allow = 0;
    let mut baseline_deny = 0;

    for i in 0..iterations {
        // Test against policy in middle of set (worst case for linear scan)
        let target_policy_idx = policy_count / 2;
        let policy_id = policy_ids[target_policy_idx];

        let resource_id = format!("/api/resource_{}", target_policy_idx * 10);

        let mut context = HashMap::new();
        context.insert("role".to_string(), "user".to_string());

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,
        };

        let eval_start = Instant::now();
        let decision = baseline_engine.evaluate(&policy_id, &request)?;
        let elapsed = eval_start.elapsed().as_nanos();

        baseline_latencies.push(elapsed);

        match decision.decision {
            PolicyAction::Allow => baseline_allow += 1,
            _ => baseline_deny += 1,
        }

        if (i + 1) % 1000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }

    let baseline_total = baseline_start.elapsed();
    println!("\n✓ BASELINE test complete\n");

    // Calculate baseline stats
    baseline_latencies.sort();
    let baseline_min = baseline_latencies[0];
    let baseline_mean = baseline_latencies.iter().sum::<u128>() / baseline_latencies.len() as u128;
    let baseline_median = baseline_latencies[baseline_latencies.len() / 2];
    let baseline_p95 = baseline_latencies[(baseline_latencies.len() as f64 * 0.95) as usize];
    let baseline_p99 = baseline_latencies[(baseline_latencies.len() as f64 * 0.99) as usize];
    let baseline_max = baseline_latencies[baseline_latencies.len() - 1];

    // ========================================================================
    // OPTIMIZED TEST: Indexed Lookup with IndexedPolicyEngine
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 OPTIMIZED: IndexedPolicyEngine (O(1) Indexed Lookup)");
    println!("{}", "=".repeat(80));

    let indexed_engine = IndexedPolicyEngine::new();

    // Deploy same policies to indexed engine
    println!("Deploying {} policies to indexed engine...", policies.len());
    for policy in policies {
        indexed_engine.deploy_policy(policy)?;
    }
    println!("✓ Policies deployed and indexed\n");

    println!("🔄 Running OPTIMIZED test ({} iterations)...", iterations);
    let optimized_start = Instant::now();
    let mut optimized_latencies = Vec::with_capacity(iterations);
    let mut optimized_allow = 0;
    let mut optimized_deny = 0;

    for i in 0..iterations {
        // Test same scenarios as baseline
        let target_policy_idx = policy_count / 2;
        let resource_id = format!("/api/resource_{}", target_policy_idx * 10);

        let mut context = HashMap::new();
        context.insert("role".to_string(), "user".to_string());

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,
        };

        let eval_start = Instant::now();
        let decision = indexed_engine.evaluate(&request)?;
        let elapsed = eval_start.elapsed().as_nanos();

        optimized_latencies.push(elapsed);

        match decision.decision {
            PolicyAction::Allow => optimized_allow += 1,
            _ => optimized_deny += 1,
        }

        if (i + 1) % 1000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }

    let optimized_total = optimized_start.elapsed();
    println!("\n✓ OPTIMIZED test complete\n");

    // Calculate optimized stats
    optimized_latencies.sort();
    let optimized_min = optimized_latencies[0];
    let optimized_mean =
        optimized_latencies.iter().sum::<u128>() / optimized_latencies.len() as u128;
    let optimized_median = optimized_latencies[optimized_latencies.len() / 2];
    let optimized_p95 = optimized_latencies[(optimized_latencies.len() as f64 * 0.95) as usize];
    let optimized_p99 = optimized_latencies[(optimized_latencies.len() as f64 * 0.99) as usize];
    let optimized_max = optimized_latencies[optimized_latencies.len() - 1];

    // ========================================================================
    // COMPARISON RESULTS
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 COMPARISON RESULTS");
    println!("{}", "=".repeat(80));

    println!("\n⏱️  LATENCY COMPARISON:");
    println!(
        "{:<20} {:>15} {:>15} {:>15}",
        "Metric", "Baseline", "Optimized", "Speedup"
    );
    println!("{}", "-".repeat(80));

    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "Min",
        baseline_min,
        optimized_min,
        baseline_min as f64 / optimized_min as f64
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "Mean",
        baseline_mean,
        optimized_mean,
        baseline_mean as f64 / optimized_mean as f64
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "Median",
        baseline_median,
        optimized_median,
        baseline_median as f64 / optimized_median as f64
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "P95",
        baseline_p95,
        optimized_p95,
        baseline_p95 as f64 / optimized_p95 as f64
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "P99",
        baseline_p99,
        optimized_p99,
        baseline_p99 as f64 / optimized_p99 as f64
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>14.2}x",
        "Max",
        baseline_max,
        optimized_max,
        baseline_max as f64 / optimized_max as f64
    );

    println!("\n🚀 THROUGHPUT COMPARISON:");
    let baseline_ops_sec = iterations as f64 / baseline_total.as_secs_f64();
    let optimized_ops_sec = iterations as f64 / optimized_total.as_secs_f64();

    println!(
        "{:<20} {:>15} {:>15} {:>15}",
        "Metric", "Baseline", "Optimized", "Improvement"
    );
    println!("{}", "-".repeat(80));
    println!(
        "{:<20} {:>12.0} /s {:>12.0} /s {:>14.2}x",
        "Operations/sec",
        baseline_ops_sec,
        optimized_ops_sec,
        optimized_ops_sec / baseline_ops_sec
    );
    println!(
        "{:<20} {:>12.2} µs {:>12.2} µs {:>14.2}x",
        "Avg per op",
        baseline_mean as f64 / 1000.0,
        optimized_mean as f64 / 1000.0,
        baseline_mean as f64 / optimized_mean as f64
    );

    println!("\n✅ DECISION DISTRIBUTION:");
    println!("{:<20} {:>15} {:>15}", "Decision", "Baseline", "Optimized");
    println!("{}", "-".repeat(80));
    println!(
        "{:<20} {:>15} {:>15}",
        "ALLOW", baseline_allow, optimized_allow
    );
    println!(
        "{:<20} {:>15} {:>15}",
        "DENY", baseline_deny, optimized_deny
    );

    // Verify decisions match
    if baseline_allow == optimized_allow && baseline_deny == optimized_deny {
        println!("\n✅ Decision parity verified: Both engines produce identical results");
    } else {
        println!("\n⚠️  WARNING: Decision mismatch detected!");
    }

    // Get index stats
    let stats = indexed_engine.get_index_stats();
    println!("\n📈 INDEXED ENGINE STATS:");
    println!("   Total policies:        {}", stats.total_policies);
    println!("   Resource index size:   {}", stats.resource_index_size);
    println!("   Index hits:            {}", stats.index_hits);
    println!("   Index misses:          {}", stats.index_misses);
    println!("   Hit rate:              {:.2}%", stats.hit_rate);
    println!(
        "   Avg policies checked:  {:.2}",
        stats.avg_policies_per_request
    );

    // Final verdict
    println!("\n{}", "=".repeat(80));
    println!("🏆 FINAL VERDICT:");
    println!("{}", "=".repeat(80));

    let mean_speedup = baseline_mean as f64 / optimized_mean as f64;
    let throughput_improvement = optimized_ops_sec / baseline_ops_sec;

    println!("\n✨ OPTIMIZATION EFFECTIVENESS:");
    println!("   Mean latency speedup:      {:.2}x faster", mean_speedup);
    println!(
        "   Throughput improvement:    {:.2}x more ops/sec",
        throughput_improvement
    );
    println!(
        "   P99 latency improvement:   {:.2}x faster",
        baseline_p99 as f64 / optimized_p99 as f64
    );

    if mean_speedup > 10.0 {
        println!(
            "\n🎉 INDEXING IS HIGHLY EFFECTIVE! {:.2}x speedup achieved!",
            mean_speedup
        );
    } else if mean_speedup > 2.0 {
        println!(
            "\n✅ Good improvement: {:.2}x speedup - indexing working as designed",
            mean_speedup
        );
    } else if mean_speedup > 1.0 {
        println!(
            "\n⚠️  Moderate improvement: {:.2}x speedup - may need tuning",
            mean_speedup
        );
    } else {
        println!(
            "\n❌ No improvement: {:.2}x - indexing not effective for this workload",
            mean_speedup
        );
    }

    println!("\n{}", "=".repeat(80));

    Ok(())
}
