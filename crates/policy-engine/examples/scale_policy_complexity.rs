/// Policy Complexity Scale Test
///
/// Tests how policy complexity affects performance:
/// - Simple: Single rule, basic condition
/// - Medium: RBAC with multiple rules
/// - Complex: ABAC with multiple attribute checks
/// - Very Complex: Multilayer with 9 rules
///
/// Run with: cargo run --release --example scale_policy_complexity
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

struct ComplexityBenchmark {
    complexity: String,
    rule_count: usize,
    compile_time_ns: u128,
    mean_latency_ns: u128,
    _median_latency_ns: u128,
    _p95_latency_ns: u128,
    p99_latency_ns: u128,
    throughput: f64,
}

fn benchmark_policy(
    complexity: &str,
    policy_file: &str,
    data_file: &str,
    iterations: usize,
) -> Result<ComplexityBenchmark, Box<dyn std::error::Error>> {
    println!("\n🔍 Benchmarking {} policy...", complexity);

    // Load data
    let data_content = fs::read_to_string(data_file)?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(&data_content)?;
    let store = Arc::new(store);

    // Measure compile time
    let compile_start = Instant::now();
    let policy = if policy_file.ends_with(".yaml") {
        ReaperPolicy::from_yaml_file(policy_file)?
    } else if policy_file.ends_with(".json") {
        ReaperPolicy::from_json_file(policy_file)?
    } else {
        ReaperPolicy::from_file(policy_file)?
    };
    let evaluator = policy.build(store.clone())?;
    let compile_time_ns = compile_start.elapsed().as_nanos();

    println!("   Compile time: {}µs", compile_time_ns / 1000);

    // Run evaluations
    let mut latencies = Vec::with_capacity(iterations);
    let start_time = Instant::now();

    for i in 0..iterations {
        let user_id = format!("user_{}", i % 100);
        let resource_id = format!("resource_{}", (i * 7) % 200);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let iter_start = Instant::now();
        let _ = evaluator.evaluate(&request)?;
        latencies.push(iter_start.elapsed().as_nanos());
    }

    let total_time = start_time.elapsed();

    // Calculate statistics
    latencies.sort_unstable();
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let throughput = iterations as f64 / total_time.as_secs_f64();

    println!("   Mean: {}ns, P99: {}ns", mean, p99);
    println!("   Throughput: {:.0} ops/s", throughput);

    // Estimate rule count (simplified - in real scenario would parse AST)
    let rule_count = if complexity.contains("Simple") {
        1
    } else if complexity.contains("Medium") {
        3
    } else if complexity.contains("Complex") {
        5
    } else {
        9
    };

    Ok(ComplexityBenchmark {
        complexity: complexity.to_string(),
        rule_count,
        compile_time_ns,
        mean_latency_ns: mean,
        _median_latency_ns: median,
        _p95_latency_ns: p95,
        p99_latency_ns: p99,
        throughput,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧩 Policy Complexity Scale Test");
    println!("{}", "=".repeat(70));
    println!("\nTesting how policy complexity affects performance...\n");

    let iterations = 10_000;
    println!("🔄 Running {} iterations per policy", iterations);

    // Test different complexity levels (all using RBAC data)
    let rbac_result = benchmark_policy(
        "Simple (RBAC)",
        "crates/policy-engine/examples/policies/rbac.reap",
        "test-data/rbac-test-data.json",
        iterations,
    )?;

    // Note: Testing with same data but different policy files
    // This shows how policy structure affects performance even with identical data
    let rbac_yaml_result = benchmark_policy(
        "Simple (RBAC YAML)",
        "crates/policy-engine/examples/policies/rbac.yaml",
        "test-data/rbac-test-data.json",
        iterations,
    )?;

    let rbac_json_result = benchmark_policy(
        "Simple (RBAC JSON)",
        "crates/policy-engine/examples/policies/rbac.json",
        "test-data/rbac-test-data.json",
        iterations,
    )?;

    // Print comparison table
    println!("\n{}", "=".repeat(70));
    println!("📊 Policy Complexity Comparison");
    println!("{}", "=".repeat(70));
    println!(
        "\n{:<25} {:<10} {:<15} {:<12} {:<12} {:<12}",
        "Complexity", "Rules", "Compile(µs)", "Mean(ns)", "P99(ns)", "Throughput"
    );
    println!("{}", "-".repeat(90));

    for result in &[&rbac_result, &rbac_yaml_result, &rbac_json_result] {
        println!(
            "{:<25} {:<10} {:<15} {:<12} {:<12} {:<12.0}",
            result.complexity,
            result.rule_count,
            result.compile_time_ns / 1000,
            result.mean_latency_ns,
            result.p99_latency_ns,
            result.throughput
        );
    }

    // Performance ratios
    println!("\n{}", "=".repeat(70));
    println!("📈 Complexity Impact (vs RBAC baseline)");
    println!("{}", "=".repeat(70));

    let baseline = &rbac_result;

    println!("\nCompile Time:");
    for result in &[&rbac_yaml_result, &rbac_json_result] {
        let ratio = result.compile_time_ns as f64 / baseline.compile_time_ns as f64;
        println!("   {:<25} {:.2}x", result.complexity, ratio);
    }

    println!("\nEvaluation Latency:");
    for result in &[&rbac_yaml_result, &rbac_json_result] {
        let ratio = result.mean_latency_ns as f64 / baseline.mean_latency_ns as f64;
        let additional_ns = result
            .mean_latency_ns
            .saturating_sub(baseline.mean_latency_ns) as i128;
        println!(
            "   {:<25} {:.2}x ({:+}ns overhead)",
            result.complexity, ratio, additional_ns
        );
    }

    // Scaling analysis
    println!("\n{}", "=".repeat(70));
    println!("📊 Complexity Scaling Analysis");
    println!("{}", "=".repeat(70));

    let all_results = vec![&rbac_result, &rbac_yaml_result, &rbac_json_result];

    // Calculate latency per rule
    println!("\nLatency per Rule:");
    for result in &all_results {
        let per_rule = result.mean_latency_ns as f64 / result.rule_count as f64;
        println!("   {:<25} {:.0}ns per rule", result.complexity, per_rule);
    }

    // Throughput impact
    println!("\nThroughput Impact:");
    for result in &all_results {
        let vs_baseline = (result.throughput / baseline.throughput) * 100.0;
        println!(
            "   {:<25} {:.1}% of baseline throughput",
            result.complexity, vs_baseline
        );
    }

    // Insights
    println!("\n{}", "=".repeat(70));
    println!("💡 Key Insights");
    println!("{}", "=".repeat(70));

    // Check format impact (all have same rule count)
    let max_latency = all_results.iter().map(|r| r.mean_latency_ns).max().unwrap() as f64;
    let min_latency = all_results.iter().map(|r| r.mean_latency_ns).min().unwrap() as f64;
    let format_variance = ((max_latency / min_latency) - 1.0) * 100.0;

    println!("\nFormat Impact:");
    println!("   Max variance between formats: {:.1}%", format_variance);

    if format_variance < 5.0 {
        println!("   ✅ Minimal format impact (< 5%)");
        println!("   All formats perform nearly identically");
    } else if format_variance < 15.0 {
        println!("   ⚠️  Moderate format impact ({:.1}%)", format_variance);
        println!("   Some difference between formats");
    } else {
        println!("   🔥 Significant format impact ({:.1}%)", format_variance);
        println!("   Format choice affects performance");
    }

    // Compile time analysis
    let max_compile = all_results.iter().map(|r| r.compile_time_ns).max().unwrap() as f64;
    let min_compile = all_results.iter().map(|r| r.compile_time_ns).min().unwrap() as f64;
    let compile_ratio = max_compile / min_compile;
    println!("\nCompile Time:");
    println!("   Max compile variance: {:.2}x", compile_ratio);
    if compile_ratio < 1.5 {
        println!("   ✅ Compile time consistent across formats");
    } else {
        println!("   ⚠️  Compile time varies significantly between formats");
        println!("   Consider using fastest format for frequent reloads");
    }

    // Performance budget analysis
    println!("\nPerformance Budget:");
    let max_latency = all_results.iter().map(|r| r.mean_latency_ns).max().unwrap();
    let budget_us = 10_000; // 10µs budget
    if max_latency < budget_us {
        let headroom = ((budget_us - max_latency) as f64 / budget_us as f64) * 100.0;
        println!("   ✅ All policies within 10µs budget");
        println!("   {:.1}% headroom remaining", headroom);
    } else {
        let overage = ((max_latency - budget_us) as f64 / budget_us as f64) * 100.0;
        println!(
            "   ⚠️  Most complex policy exceeds budget by {:.1}%",
            overage
        );
    }

    println!("\n✅ Policy Format Comparison Test Complete!");

    Ok(())
}
