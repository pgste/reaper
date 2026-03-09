/// Policy Format Comparison Scale Test
///
/// Compares performance of the same policy in different formats:
/// - REAP native format
/// - YAML format
/// - JSON format
///
/// Tests if format affects evaluation performance.
/// Run with: cargo run --release --example scale_policy_format_comparison
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

struct FormatBenchmark {
    format: String,
    compile_time_ns: u128,
    mean_latency_ns: u128,
    p99_latency_ns: u128,
    throughput: f64,
}

fn benchmark_format(
    format: &str,
    policy_file: &str,
    store: Arc<DataStore>,
    iterations: usize,
) -> Result<FormatBenchmark, Box<dyn std::error::Error>> {
    println!("\n🔍 Benchmarking {} format...", format);

    // Measure compile time
    let compile_start = Instant::now();
    let policy = match format {
        "YAML" => ReaperPolicy::from_yaml_file(policy_file)?,
        "JSON" => ReaperPolicy::from_json_file(policy_file)?,
        _ => ReaperPolicy::from_file(policy_file)?, // REAP format
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
        };

        let iter_start = Instant::now();
        let _ = evaluator.evaluate(&request)?;
        latencies.push(iter_start.elapsed().as_nanos());
    }

    let total_time = start_time.elapsed();

    // Calculate statistics
    latencies.sort_unstable();
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let p99_index = (latencies.len() as f64 * 0.99) as usize;
    let p99 = latencies[p99_index];
    let throughput = iterations as f64 / total_time.as_secs_f64();

    println!("   Mean latency: {}ns", mean);
    println!("   P99 latency: {}ns", p99);
    println!("   Throughput: {:.0} ops/s", throughput);

    Ok(FormatBenchmark {
        format: format.to_string(),
        compile_time_ns,
        mean_latency_ns: mean,
        p99_latency_ns: p99,
        throughput,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("📊 Policy Format Comparison Scale Test");
    println!("{}", "=".repeat(70));
    println!("\nTesting if policy format affects evaluation performance...\n");

    // Load data
    println!("📂 Loading test data...");
    let data_content = fs::read_to_string("test-data/rbac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    let iterations = 10_000;
    println!("\n🔄 Running {} iterations per format", iterations);

    // Test each format
    let reap_result = benchmark_format(
        "REAP",
        "crates/policy-engine/examples/policies/rbac.reap",
        store.clone(),
        iterations,
    )?;

    let yaml_result = benchmark_format(
        "YAML",
        "crates/policy-engine/examples/policies/rbac.yaml",
        store.clone(),
        iterations,
    )?;

    let json_result = benchmark_format(
        "JSON",
        "crates/policy-engine/examples/policies/rbac.json",
        store.clone(),
        iterations,
    )?;

    // Print comparison table
    println!("\n{}", "=".repeat(70));
    println!("📊 Format Comparison Results");
    println!("{}", "=".repeat(70));
    println!(
        "\n{:<10} {:<15} {:<15} {:<15} {:<15}",
        "Format", "Compile (µs)", "Mean (ns)", "P99 (ns)", "Throughput"
    );
    println!("{}", "-".repeat(70));

    for result in &[&reap_result, &yaml_result, &json_result] {
        println!(
            "{:<10} {:<15} {:<15} {:<15} {:<15.0}",
            result.format,
            result.compile_time_ns / 1000,
            result.mean_latency_ns,
            result.p99_latency_ns,
            result.throughput
        );
    }

    // Calculate differences
    println!("\n{}", "=".repeat(70));
    println!("📈 Performance Differences");
    println!("{}", "=".repeat(70));

    let baseline = &reap_result;

    for (name, result) in [("YAML", &yaml_result), ("JSON", &json_result)] {
        let compile_diff =
            (result.compile_time_ns as f64 / baseline.compile_time_ns as f64 - 1.0) * 100.0;
        let mean_diff =
            (result.mean_latency_ns as f64 / baseline.mean_latency_ns as f64 - 1.0) * 100.0;
        let p99_diff =
            (result.p99_latency_ns as f64 / baseline.p99_latency_ns as f64 - 1.0) * 100.0;

        println!("\n{} vs REAP:", name);
        println!("   Compile time: {:+.1}%", compile_diff);
        println!("   Mean latency: {:+.1}%", mean_diff);
        println!("   P99 latency: {:+.1}%", p99_diff);
    }

    println!("\n{}", "=".repeat(70));
    println!("💡 Key Insights:");
    println!("{}", "=".repeat(70));

    // Determine if formats have significant difference
    let max_mean_diff = [&reap_result, &yaml_result, &json_result]
        .iter()
        .map(|r| r.mean_latency_ns)
        .max()
        .unwrap() as f64
        / [&reap_result, &yaml_result, &json_result]
            .iter()
            .map(|r| r.mean_latency_ns)
            .min()
            .unwrap() as f64;

    if max_mean_diff < 1.1 {
        println!("✅ All formats perform within 10% of each other");
        println!("   Format choice does NOT significantly affect runtime performance");
    } else {
        println!("⚠️  Formats show >10% performance difference");
        println!("   Consider using fastest format for production");
    }

    println!("\n💡 All formats compile to identical internal representation");
    println!("   Choose based on readability and tooling support, not performance");

    println!("\n✅ Format Comparison Test Complete!");

    Ok(())
}
