/// Cache Performance Scale Test
///
/// Tests cache behavior and hot vs cold path performance:
/// - Cold path: First evaluation (cache miss)
/// - Hot path: Repeated evaluations (cache hit)
/// - Mixed path: Random access patterns
///
/// Run with: cargo run --release --example scale_cache_performance
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

struct CacheBenchmark {
    pattern: String,
    mean_latency_ns: u128,
    p50_latency_ns: u128,
    p95_latency_ns: u128,
    p99_latency_ns: u128,
    max_latency_ns: u128,
}

fn benchmark_pattern(
    pattern: &str,
    evaluator: &impl PolicyEvaluator,
    iterations: usize,
    access_fn: impl Fn(usize) -> (String, String),
) -> CacheBenchmark {
    let mut latencies = Vec::with_capacity(iterations);

    for i in 0..iterations {
        let (user_id, resource_id) = access_fn(i);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: resource_id,
            action: "read".to_string(),
            context,
        };

        let start = Instant::now();
        let _ = evaluator.evaluate(&request);
        latencies.push(start.elapsed().as_nanos());
    }

    // Calculate statistics
    latencies.sort_unstable();
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let p50 = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let max = *latencies.last().unwrap();

    CacheBenchmark {
        pattern: pattern.to_string(),
        mean_latency_ns: mean,
        p50_latency_ns: p50,
        p95_latency_ns: p95,
        p99_latency_ns: p99,
        max_latency_ns: max,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔥 Cache Performance Scale Test");
    println!("{}", "=".repeat(70));
    println!("\nTesting hot path vs cold path performance...\n");

    // Load data
    println!("📂 Loading test data...");
    let data_content = fs::read_to_string("test-data/rbac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    // Load policy
    println!("📜 Loading RBAC policy...");
    let policy = ReaperPolicy::from_file("crates/policy-engine/examples/policies/rbac.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled");

    let iterations = 10_000;

    println!("\n{}", "=".repeat(70));
    println!("🔄 Testing Access Patterns");
    println!("{}", "=".repeat(70));

    // Pattern 1: Repeated (Hot Path - same request over and over)
    println!("\n1️⃣  Hot Path (Same Request Repeated)");
    println!("   Testing: user_0 → resource_0 ({}x)", iterations);

    let hot_result = benchmark_pattern("Hot Path", &evaluator, iterations, |_| {
        ("user_0".to_string(), "resource_0".to_string())
    });

    println!(
        "   Mean: {}ns, P99: {}ns",
        hot_result.mean_latency_ns, hot_result.p99_latency_ns
    );

    // Pattern 2: Sequential (Cold Path - different request each time)
    println!("\n2️⃣  Cold Path (Sequential Access)");
    println!("   Testing: user_i → resource_i (unique requests)");

    let sequential_result = benchmark_pattern("Sequential", &evaluator, iterations, |i| {
        let user = format!("user_{}", i % 1000);
        let resource = format!("resource_{}", i % 2000);
        (user, resource)
    });

    println!(
        "   Mean: {}ns, P99: {}ns",
        sequential_result.mean_latency_ns, sequential_result.p99_latency_ns
    );

    // Pattern 3: Random (Mixed - some cache hits, some misses)
    println!("\n3️⃣  Mixed Path (Random Access)");
    println!("   Testing: Random user/resource pairs");

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let random_result = benchmark_pattern("Random", &evaluator, iterations, |i| {
        let mut hasher = DefaultHasher::new();
        i.hash(&mut hasher);
        let hash = hasher.finish();

        let user = format!("user_{}", hash % 100);
        let resource = format!("resource_{}", (hash / 100) % 200);
        (user, resource)
    });

    println!(
        "   Mean: {}ns, P99: {}ns",
        random_result.mean_latency_ns, random_result.p99_latency_ns
    );

    // Pattern 4: Burst (Repeated small batches)
    println!("\n4️⃣  Burst Path (Repeated Batches)");
    println!("   Testing: 10 requests per user, then switch");

    let burst_result = benchmark_pattern("Burst", &evaluator, iterations, |i| {
        let user = format!("user_{}", (i / 10) % 100);
        let resource = format!("resource_{}", i % 200);
        (user, resource)
    });

    println!(
        "   Mean: {}ns, P99: {}ns",
        burst_result.mean_latency_ns, burst_result.p99_latency_ns
    );

    // Print comparison table
    println!("\n{}", "=".repeat(70));
    println!("📊 Cache Performance Comparison");
    println!("{}", "=".repeat(70));
    println!(
        "\n{:<15} {:<12} {:<12} {:<12} {:<12} {:<12}",
        "Pattern", "Mean", "P50", "P95", "P99", "Max"
    );
    println!("{}", "-".repeat(70));

    for result in &[
        &hot_result,
        &sequential_result,
        &random_result,
        &burst_result,
    ] {
        println!(
            "{:<15} {:<12} {:<12} {:<12} {:<12} {:<12}",
            result.pattern,
            format!("{}ns", result.mean_latency_ns),
            format!("{}ns", result.p50_latency_ns),
            format!("{}ns", result.p95_latency_ns),
            format!("{}ns", result.p99_latency_ns),
            format!("{}ns", result.max_latency_ns),
        );
    }

    // Performance ratios
    println!("\n{}", "=".repeat(70));
    println!("📈 Performance Ratios (vs Hot Path)");
    println!("{}", "=".repeat(70));

    let baseline = hot_result.mean_latency_ns as f64;

    for result in &[&sequential_result, &random_result, &burst_result] {
        let ratio = result.mean_latency_ns as f64 / baseline;
        println!("{:<15} {:.2}x slower than hot path", result.pattern, ratio);
    }

    // Insights
    println!("\n{}", "=".repeat(70));
    println!("💡 Key Insights");
    println!("{}", "=".repeat(70));

    let hot_vs_cold = sequential_result.mean_latency_ns as f64 / hot_result.mean_latency_ns as f64;

    if hot_vs_cold < 1.2 {
        println!("✅ Minimal cache effect (< 20% difference)");
        println!("   Engine performs consistently regardless of cache state");
    } else if hot_vs_cold < 2.0 {
        println!("⚠️  Moderate cache effect (20-100% difference)");
        println!("   Some caching benefits observed");
    } else {
        println!("🔥 Strong cache effect (> 2x difference)");
        println!("   Hot path significantly faster than cold path");
    }

    println!("\n📊 Latency Stability:");
    for result in &[
        &hot_result,
        &sequential_result,
        &random_result,
        &burst_result,
    ] {
        let variance = result.p99_latency_ns as f64 / result.mean_latency_ns as f64;
        println!("   {:<15} P99/Mean ratio: {:.2}x", result.pattern, variance);
    }

    println!("\n✅ Cache Performance Test Complete!");

    Ok(())
}
