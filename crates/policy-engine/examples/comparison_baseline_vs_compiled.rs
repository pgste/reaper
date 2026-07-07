/// CRITICAL COMPARISON: Baseline vs Compiled Policy Evaluation
///
/// This test answers the question: "Does compilation actually make a difference?"
///
/// Comparison:
/// 1. BASELINE: Standard policy evaluation
/// 2. COMPILED: Optimized compiled evaluator with partial evaluation
///
/// Both use identical policies and requests for fair comparison.
use policy_engine::{
    CompiledPolicyEvaluator, EnhancedPolicy, PolicyAction, PolicyEvaluator, PolicyRequest,
    PolicyRule,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(80));
    println!("🔬 CRITICAL COMPARISON: Baseline vs Compiled Evaluation");
    println!("{}", "=".repeat(80));

    let iterations = 10_000;

    println!("\n📊 Test Configuration:");
    println!("   Iterations: {}", iterations);
    println!("   Goal: Prove compilation provides measurable speedup\n");

    // ========================================================================
    // CREATE TEST POLICY
    // ========================================================================
    println!("Creating test policy with 10 rules...");

    let mut rules = Vec::new();
    for i in 0..10 {
        rules.push(PolicyRule {
            action: PolicyAction::Allow,
            resource: format!("/api/resource_{}", i),
            conditions: vec![
                "role==user".to_string(),
                "department==eng".to_string(),
                "active==true".to_string(),
            ],
        });
    }

    // Add a catch-all deny
    rules.push(PolicyRule {
        action: PolicyAction::Deny,
        resource: "*".to_string(),
        conditions: vec![],
    });

    let policy = EnhancedPolicy::new(
        "test-policy".to_string(),
        "Test policy for comparison".to_string(),
        rules,
    );

    println!("✓ Policy created with {} rules\n", policy.rules.len());

    // ========================================================================
    // BASELINE TEST: Standard Evaluation
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 BASELINE: Standard Policy Evaluation");
    println!("{}", "=".repeat(80));

    let baseline_evaluator = policy.get_evaluator().unwrap();

    println!("\n🔄 Running BASELINE test ({} iterations)...", iterations);
    let mut baseline_latencies = Vec::with_capacity(iterations);
    let mut baseline_allow = 0;

    // Create test request
    let mut context = HashMap::new();
    context.insert("role".to_string(), "user".to_string());
    context.insert("department".to_string(), "eng".to_string());
    context.insert("active".to_string(), "true".to_string());

    let request = PolicyRequest {
        resource: "/api/resource_5".to_string(),
        action: "read".to_string(),
        context: context.clone(),
    };

    for i in 0..iterations {
        let start = Instant::now();
        let decision = baseline_evaluator.evaluate(&request)?;
        let elapsed = start.elapsed().as_nanos();

        baseline_latencies.push(elapsed);

        if matches!(decision, PolicyAction::Allow) {
            baseline_allow += 1;
        }

        if (i + 1) % 2000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }
    println!("\n✓ BASELINE test complete\n");

    baseline_latencies.sort();
    let baseline_min = baseline_latencies[0];
    let baseline_mean = baseline_latencies.iter().sum::<u128>() / baseline_latencies.len() as u128;
    let baseline_median = baseline_latencies[baseline_latencies.len() / 2];
    let baseline_p95 = baseline_latencies[(baseline_latencies.len() as f64 * 0.95) as usize];
    let baseline_p99 = baseline_latencies[(baseline_latencies.len() as f64 * 0.99) as usize];

    // ========================================================================
    // COMPILED TEST: Optimized Evaluation (WITHOUT static context)
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 COMPILED (No Partial Eval): Optimized Evaluation");
    println!("{}", "=".repeat(80));

    let compiled_evaluator = CompiledPolicyEvaluator::compile(&policy, None)?;
    let compiled_evaluator = Arc::new(compiled_evaluator) as Arc<dyn PolicyEvaluator>;

    println!("\n🔄 Running COMPILED test ({} iterations)...", iterations);
    let mut compiled_latencies = Vec::with_capacity(iterations);
    let mut compiled_allow = 0;

    for i in 0..iterations {
        let start = Instant::now();
        let decision = compiled_evaluator.evaluate(&request)?;
        let elapsed = start.elapsed().as_nanos();

        compiled_latencies.push(elapsed);

        if matches!(decision, PolicyAction::Allow) {
            compiled_allow += 1;
        }

        if (i + 1) % 2000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }
    println!("\n✓ COMPILED test complete\n");

    compiled_latencies.sort();
    let compiled_min = compiled_latencies[0];
    let compiled_mean = compiled_latencies.iter().sum::<u128>() / compiled_latencies.len() as u128;
    let compiled_median = compiled_latencies[compiled_latencies.len() / 2];
    let compiled_p95 = compiled_latencies[(compiled_latencies.len() as f64 * 0.95) as usize];
    let compiled_p99 = compiled_latencies[(compiled_latencies.len() as f64 * 0.99) as usize];

    // ========================================================================
    // COMPILED + PARTIAL EVAL TEST: Full Optimization
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 COMPILED + PARTIAL EVAL: Full Optimization");
    println!("{}", "=".repeat(80));

    // Provide static context for partial evaluation
    let mut static_context = HashMap::new();
    static_context.insert("department".to_string(), "eng".to_string());
    static_context.insert("active".to_string(), "true".to_string());

    let optimized_evaluator = CompiledPolicyEvaluator::compile(&policy, Some(&static_context))?;
    let optimized_evaluator = Arc::new(optimized_evaluator) as Arc<dyn PolicyEvaluator>;

    println!("\n🔄 Running OPTIMIZED test ({} iterations)...", iterations);
    let mut optimized_latencies = Vec::with_capacity(iterations);
    let mut optimized_allow = 0;

    for i in 0..iterations {
        let start = Instant::now();
        let decision = optimized_evaluator.evaluate(&request)?;
        let elapsed = start.elapsed().as_nanos();

        optimized_latencies.push(elapsed);

        if matches!(decision, PolicyAction::Allow) {
            optimized_allow += 1;
        }

        if (i + 1) % 2000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }
    println!("\n✓ OPTIMIZED test complete\n");

    optimized_latencies.sort();
    let optimized_min = optimized_latencies[0];
    let optimized_mean =
        optimized_latencies.iter().sum::<u128>() / optimized_latencies.len() as u128;
    let optimized_median = optimized_latencies[optimized_latencies.len() / 2];
    let optimized_p95 = optimized_latencies[(optimized_latencies.len() as f64 * 0.95) as usize];
    let optimized_p99 = optimized_latencies[(optimized_latencies.len() as f64 * 0.99) as usize];

    // ========================================================================
    // COMPARISON RESULTS
    // ========================================================================
    println!("{}", "=".repeat(80));
    println!("📊 COMPARISON RESULTS");
    println!("{}", "=".repeat(80));

    println!("\n⏱️  LATENCY COMPARISON:");
    println!(
        "{:<20} {:>15} {:>15} {:>15}",
        "Metric", "Baseline", "Compiled", "Optimized"
    );
    println!("{}", "-".repeat(80));

    println!(
        "{:<20} {:>12} ns {:>12} ns {:>12} ns",
        "Min", baseline_min, compiled_min, optimized_min
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>12} ns",
        "Mean", baseline_mean, compiled_mean, optimized_mean
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>12} ns",
        "Median", baseline_median, compiled_median, optimized_median
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>12} ns",
        "P95", baseline_p95, compiled_p95, optimized_p95
    );
    println!(
        "{:<20} {:>12} ns {:>12} ns {:>12} ns",
        "P99", baseline_p99, compiled_p99, optimized_p99
    );

    println!("\n🚀 SPEEDUP ANALYSIS:");
    println!("{:<30} {:>20}", "Configuration", "Speedup vs Baseline");
    println!("{}", "-".repeat(80));

    let compiled_speedup = baseline_mean as f64 / compiled_mean as f64;
    let optimized_speedup = baseline_mean as f64 / optimized_mean as f64;

    println!(
        "{:<30} {:>19.2}x",
        "Compiled (no partial eval)", compiled_speedup
    );
    println!(
        "{:<30} {:>19.2}x",
        "Compiled + Partial Eval", optimized_speedup
    );

    println!("\n✅ DECISION DISTRIBUTION:");
    println!(
        "{:<20} {:>15} {:>15} {:>15}",
        "Decision", "Baseline", "Compiled", "Optimized"
    );
    println!("{}", "-".repeat(80));
    println!(
        "{:<20} {:>15} {:>15} {:>15}",
        "ALLOW", baseline_allow, compiled_allow, optimized_allow
    );

    // Verify correctness
    if baseline_allow == compiled_allow && compiled_allow == optimized_allow {
        println!("\n✅ Correctness verified: All evaluators produce identical results");
    } else {
        println!("\n⚠️  WARNING: Decision mismatch detected!");
    }

    // ========================================================================
    // FINAL VERDICT
    // ========================================================================
    println!("\n{}", "=".repeat(80));
    println!("🏆 FINAL VERDICT:");
    println!("{}", "=".repeat(80));

    println!("\n📊 Performance Summary:");
    println!("   Baseline:              {} ns", baseline_mean);
    println!(
        "   Compiled:              {} ns ({:.2}x)",
        compiled_mean, compiled_speedup
    );
    println!(
        "   Compiled + Partial:    {} ns ({:.2}x)",
        optimized_mean, optimized_speedup
    );

    if optimized_speedup >= 2.0 {
        println!(
            "\n🎉 SUCCESS! Compilation provides {:.2}x speedup!",
            optimized_speedup
        );
        println!("   ✅ Compilation + Partial Evaluation is working!");
    } else if optimized_speedup >= 1.5 {
        println!(
            "\n✅ Moderate improvement: {:.2}x speedup",
            optimized_speedup
        );
    } else if optimized_speedup > 1.0 {
        println!(
            "\n⚠️  Modest improvement: {:.2}x speedup",
            optimized_speedup
        );
    } else {
        println!(
            "\n❌ No improvement: {:.2}x - investigation needed",
            optimized_speedup
        );
    }

    println!("\n{}", "=".repeat(80));

    Ok(())
}
