/// Micro-benchmark: Policy Lookup Performance
///
/// Tests the core value proposition of indexing:
/// How fast can we find matching policies in a large set?
///
/// Baseline: Linear scan through all N policies
/// Optimized: Index-based lookup to find only matching policies
use policy_engine::{EnhancedPolicy, IndexedPolicyEngine, PolicyAction, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(80));
    println!("⚡ MICRO-BENCHMARK: Policy Lookup Performance");
    println!("{}", "=".repeat(80));

    let policy_sizes = vec![10, 100, 1000];
    let iterations = 10_000;

    println!("\nTest: Find matching policy in set of N policies");
    println!("Iterations per test: {}\n", iterations);

    for policy_count in policy_sizes {
        println!("{}", "=".repeat(80));
        println!("📊 Testing with {} policies", policy_count);
        println!("{}", "=".repeat(80));

        // Create test policies
        let mut policies = Vec::new();
        for i in 0..policy_count {
            let policy = EnhancedPolicy::new(
                format!("policy-{}", i),
                format!("Policy {}", i),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource_{}", i),
                    conditions: vec!["role==user".to_string()],
                }],
            );
            policies.push(policy);
        }

        // Add a wildcard admin policy at the end
        policies.push(EnhancedPolicy::new(
            "admin-wildcard".to_string(),
            "Admin full access".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec!["role==admin".to_string()],
            }],
        ));

        // ====================================================================
        // BASELINE: Linear scan through all policies
        // ====================================================================
        println!("\n📝 BASELINE: Linear Scan");

        let mut baseline_latencies = Vec::new();
        let mut baseline_matches = 0;

        let test_resource = format!("/api/resource_{}", policy_count / 2);
        let mut test_context = HashMap::new();
        test_context.insert("role".to_string(), "user".to_string());

        let request = PolicyRequest {
            resource: test_resource.clone(),
            action: "read".to_string(),
            context: test_context.clone(),
        };

        for _ in 0..iterations {
            let start = Instant::now();

            // Linear scan: check every policy
            let mut matched = false;
            for policy in &policies {
                for rule in &policy.rules {
                    let resource_matches = if rule.resource == "*" {
                        true
                    } else if rule.resource.ends_with('*') {
                        let prefix = rule.resource.trim_end_matches('*');
                        request.resource.starts_with(prefix)
                    } else {
                        rule.resource == request.resource
                    };

                    if resource_matches {
                        // Check conditions
                        let conditions_met = rule.conditions.iter().all(|cond| {
                            if let Some((key, value)) = cond.split_once("==") {
                                request
                                    .context
                                    .get(key)
                                    .map(|v| v == value)
                                    .unwrap_or(false)
                            } else {
                                false
                            }
                        });

                        if conditions_met {
                            matched = true;
                            break;
                        }
                    }
                }
                if matched {
                    break;
                }
            }

            let elapsed = start.elapsed().as_nanos();
            baseline_latencies.push(elapsed);
            if matched {
                baseline_matches += 1;
            }
        }

        baseline_latencies.sort();
        let baseline_min = baseline_latencies[0];
        let baseline_mean =
            baseline_latencies.iter().sum::<u128>() / baseline_latencies.len() as u128;
        let baseline_median = baseline_latencies[baseline_latencies.len() / 2];
        let baseline_p99 = baseline_latencies[(baseline_latencies.len() as f64 * 0.99) as usize];

        println!("   Min:     {:>8} ns", baseline_min);
        println!("   Mean:    {:>8} ns", baseline_mean);
        println!("   Median:  {:>8} ns", baseline_median);
        println!("   P99:     {:>8} ns", baseline_p99);
        println!("   Matches: {}/{}", baseline_matches, iterations);

        // ====================================================================
        // OPTIMIZED: Index-based lookup
        // ====================================================================
        println!("\n⚡ OPTIMIZED: Indexed Lookup");

        let indexed_engine = IndexedPolicyEngine::new();
        for policy in policies {
            indexed_engine.deploy_policy(policy)?;
        }

        let mut optimized_latencies = Vec::new();
        let mut optimized_matches = 0;

        for _ in 0..iterations {
            let start = Instant::now();

            let decision = indexed_engine.evaluate(&request)?;

            let elapsed = start.elapsed().as_nanos();
            optimized_latencies.push(elapsed);

            if matches!(decision.decision, PolicyAction::Allow) {
                optimized_matches += 1;
            }
        }

        optimized_latencies.sort();
        let optimized_min = optimized_latencies[0];
        let optimized_mean =
            optimized_latencies.iter().sum::<u128>() / optimized_latencies.len() as u128;
        let optimized_median = optimized_latencies[optimized_latencies.len() / 2];
        let optimized_p99 = optimized_latencies[(optimized_latencies.len() as f64 * 0.99) as usize];

        println!("   Min:     {:>8} ns", optimized_min);
        println!("   Mean:    {:>8} ns", optimized_mean);
        println!("   Median:  {:>8} ns", optimized_median);
        println!("   P99:     {:>8} ns", optimized_p99);
        println!("   Matches: {}/{}", optimized_matches, iterations);

        // Index stats
        let stats = indexed_engine.get_index_stats();
        println!("\n📈 Index Stats:");
        println!("   Policies:      {}", stats.total_policies);
        println!("   Index size:    {}", stats.resource_index_size);
        println!("   Hit rate:      {:.2}%", stats.hit_rate);
        println!("   Avg checked:   {:.2}", stats.avg_policies_per_request);

        // ====================================================================
        // COMPARISON
        // ====================================================================
        println!("\n🏆 Results:");
        let mean_speedup = baseline_mean as f64 / optimized_mean as f64;
        let p99_speedup = baseline_p99 as f64 / optimized_p99 as f64;

        println!("   Mean speedup:  {:.2}x", mean_speedup);
        println!("   P99 speedup:   {:.2}x", p99_speedup);

        if mean_speedup > 2.0 {
            println!("   ✅ Indexing provides {:.2}x speedup!", mean_speedup);
        } else if mean_speedup > 1.0 {
            println!("   ⚠️  Modest speedup: {:.2}x", mean_speedup);
        } else {
            println!("   ❌ No improvement: {:.2}x", mean_speedup);
        }
    }

    println!("\n{}", "=".repeat(80));

    Ok(())
}
