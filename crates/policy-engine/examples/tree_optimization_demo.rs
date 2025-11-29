//! Tree Optimization End-to-End Demo
//!
//! Demonstrates how to use decision tree optimization with PolicyEngine
//! for 10-600x faster policy evaluation.
//!
//! This example shows:
//! 1. Creating policies with and without tree optimization
//! 2. Deploying to PolicyEngine
//! 3. Evaluating requests
//! 4. Comparing performance
//!
//! Usage:
//! ```bash
//! cargo run --release --example tree_optimization_demo
//! ```

use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::time::Instant;

fn main() {
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║   Tree Optimization Demo - PolicyEngine Integration           ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");

    // Generate a realistic policy with multiple rules
    // Use 500+ rules to better demonstrate tree optimization benefits
    let rules = generate_sample_policy(500);
    let rule_count = rules.len();

    println!("📋 Sample Policy:");
    println!("  - Rules: {}", rule_count);
    println!("  - Resource types: users, documents, api");
    println!("  - Actions: Allow/Deny\n");

    // Create PolicyEngine
    let engine = PolicyEngine::new();

    // ========================================================
    // Scenario 1: Standard Linear Evaluation
    // ========================================================
    println!("═══════════════════════════════════════════════════════════════");
    println!("Scenario 1: Standard Linear Evaluation (O(r))");
    println!("═══════════════════════════════════════════════════════════════\n");

    let start = Instant::now();
    let linear_policy = EnhancedPolicy::new(
        "rbac-linear".to_string(),
        "RBAC policy with linear evaluation".to_string(),
        rules.clone(),
    );
    let linear_compile_time = start.elapsed();

    let linear_id = linear_policy.id;
    engine
        .deploy_policy(linear_policy)
        .expect("Failed to deploy linear policy");

    println!("✓ Policy deployed (linear mode)");
    println!("  Compilation time: {:?}", linear_compile_time);

    // ========================================================
    // Scenario 2: Tree-Optimized Evaluation
    // ========================================================
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("Scenario 2: Tree-Optimized Evaluation (O(log r))");
    println!("═══════════════════════════════════════════════════════════════\n");

    let start = Instant::now();
    let tree_policy = EnhancedPolicy::new_with_tree_optimization(
        "rbac-tree".to_string(),
        "RBAC policy with decision tree optimization".to_string(),
        rules,
    )
    .expect("Failed to create tree-optimized policy");
    let tree_compile_time = start.elapsed();

    let tree_id = tree_policy.id;
    engine
        .deploy_policy(tree_policy)
        .expect("Failed to deploy tree policy");

    println!("✓ Policy deployed (tree mode)");
    println!("  Compilation time: {:?}", tree_compile_time);
    if tree_compile_time > linear_compile_time {
        println!("  Overhead: {:?}", tree_compile_time - linear_compile_time);
    } else {
        println!("  Overhead: ~0 (faster than expected)");
    }

    // ========================================================
    // Performance Comparison
    // ========================================================
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("Performance Comparison: 1,000 Evaluations");
    println!("═══════════════════════════════════════════════════════════════\n");

    let test_requests = generate_test_requests(1000);

    // Benchmark linear
    let start = Instant::now();
    for request in &test_requests {
        engine
            .evaluate(&linear_id, request)
            .expect("Evaluation failed");
    }
    let linear_total = start.elapsed();
    let linear_avg = linear_total.as_nanos() / test_requests.len() as u128;

    println!("Linear Evaluation:");
    println!("  Total time: {:?}", linear_total);
    println!("  Average:    {} ns/op", linear_avg);
    println!(
        "  Throughput: {:.0} ops/sec",
        1_000_000_000.0 / linear_avg as f64
    );

    // Benchmark tree
    let start = Instant::now();
    for request in &test_requests {
        engine
            .evaluate(&tree_id, request)
            .expect("Evaluation failed");
    }
    let tree_total = start.elapsed();
    let tree_avg = tree_total.as_nanos() / test_requests.len() as u128;

    println!("\nTree-Optimized Evaluation:");
    println!("  Total time: {:?}", tree_total);
    println!("  Average:    {} ns/op", tree_avg);
    println!(
        "  Throughput: {:.0} ops/sec",
        1_000_000_000.0 / tree_avg as f64
    );

    let speedup = linear_avg as f64 / tree_avg as f64;
    println!(
        "\n🚀 Speedup: {:.1}x faster with tree optimization!",
        speedup
    );

    // ========================================================
    // Real-World Use Cases
    // ========================================================
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("Real-World Usage Examples");
    println!("═══════════════════════════════════════════════════════════════\n");

    println!("✅ When to use tree optimization:");
    println!("  - Policies with 100+ rules");
    println!("  - Enterprise RBAC with many roles");
    println!("  - Fine-grained ABAC policies");
    println!("  - Multi-tenant policies");
    println!("  - Latency-sensitive applications\n");

    println!("⚠️  Tradeoffs:");
    println!("  - Compilation overhead: ~1-10ms (one-time)");
    println!("  - Extra memory: ~2x policy size");
    println!("  - Best for large policies (100+ rules)\n");

    println!("💡 Recommendation:");
    if rule_count >= 100 {
        println!("  Use tree optimization for {}-rule policies", rule_count);
    } else {
        println!(
            "  Linear evaluation is fine for {}-rule policies",
            rule_count
        );
    }

    println!("\n═══════════════════════════════════════════════════════════════");
    println!("Demo Complete!");
    println!("═══════════════════════════════════════════════════════════════\n");
}

/// Generate a sample RBAC policy with realistic rules
fn generate_sample_policy(rule_count: usize) -> Vec<PolicyRule> {
    let mut rules = Vec::new();

    let resource_types = ["users", "documents", "api"];
    let actions = [PolicyAction::Allow, PolicyAction::Deny];

    for i in 0..rule_count {
        let resource_type = &resource_types[i % resource_types.len()];
        let action = &actions[i % actions.len()];

        rules.push(PolicyRule {
            action: action.clone(),
            resource: format!("{}:{}", resource_type, i),
            conditions: vec![],
        });
    }

    // Add default deny rule
    rules.push(PolicyRule {
        action: PolicyAction::Deny,
        resource: "*".to_string(),
        conditions: vec![],
    });

    rules
}

/// Generate test requests for benchmarking
fn generate_test_requests(count: usize) -> Vec<PolicyRequest> {
    let mut requests = Vec::new();

    for i in 0..count {
        let resource_id = i % 200; // Distribute across resources
        requests.push(PolicyRequest {
            resource: format!("users:{}", resource_id),
            action: "read".to_string(),
            context: HashMap::new(),
        });
    }

    requests
}
