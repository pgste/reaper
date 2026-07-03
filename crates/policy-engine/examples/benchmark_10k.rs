//! Quick 10K benchmark for Phase 1 & 2 optimizations
//!
//! Run with: cargo run --release -p policy-engine --example benchmark_10k
//!
//! Environment variables for cache configuration:
//! - REAPER_CACHE_ENABLED: "true" or "false" (default: "true")
//! - REAPER_CACHE_CAPACITY: positive integer (default: 10000)
//! - REAPER_CACHE_TTL_SECS: seconds, 0 for no TTL (default: 300)
//!
//! Example: REAPER_CACHE_ENABLED=false cargo run --release -p policy-engine --example benchmark_10k

use policy_engine::batch::BatchEvaluator;
use policy_engine::cache_config::CacheConfig;
use policy_engine::data::DataStore;
use policy_engine::decision_cache::DecisionCache;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, PolicyAction, PolicyRequest};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn benchmark_scenario(name: &str, policy_path: &str, data_path: &str, iterations: usize) {
    println!("\n=== {} ===", name);

    // Load policy
    let policy_content = fs::read_to_string(policy_path)
        .unwrap_or_else(|_| panic!("Failed to read policy: {}", policy_path));
    let policy: ReaperPolicy = policy_content
        .parse()
        .unwrap_or_else(|_| panic!("Failed to parse policy: {}", policy_path));

    // Load data
    let data_content = fs::read_to_string(data_path)
        .unwrap_or_else(|_| panic!("Failed to read data: {}", data_path));

    // Setup data store and load entities
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let load_start = Instant::now();
    loader
        .load_json(&data_content)
        .expect("Failed to load data");
    let load_duration = load_start.elapsed();

    let stats = store.stats();
    println!(
        "  Data loaded: {:?} ({} entities)",
        load_duration, stats.total_entities
    );

    // Build evaluator
    let build_start = Instant::now();
    let evaluator = policy.build_ast_evaluator(Arc::new(store.clone()));
    let build_duration = build_start.elapsed();

    println!("  Evaluator built: {:?}", build_duration);

    // Get some entity IDs for testing
    let interner = store.interner();
    let user_type = interner.intern("User");
    let resource_type = interner.intern("Resource");

    let user_ids: Vec<String> = store
        .get_by_type(user_type)
        .iter()
        .take(100)
        .map(|e| {
            interner
                .resolve_str(e.id)
                .unwrap_or_else(|| "unknown".to_string())
        })
        .collect();

    let resource_ids: Vec<String> = store
        .get_by_type(resource_type)
        .iter()
        .take(100)
        .map(|e| {
            interner
                .resolve_str(e.id)
                .unwrap_or_else(|| "unknown".to_string())
        })
        .collect();

    if user_ids.is_empty() {
        println!("  WARNING: No users found");
        return;
    }

    let resource_id = resource_ids
        .first()
        .map(|s| s.as_str())
        .unwrap_or("resource_0");

    let actions = ["read", "write", "delete", "admin"];

    // Build requests for testing
    let requests: Vec<PolicyRequest> = (0..iterations)
        .map(|i| {
            let user_idx = i % user_ids.len();
            let action_idx = i % actions.len();
            let mut ctx = HashMap::new();
            ctx.insert("principal".to_string(), user_ids[user_idx].clone());
            PolicyRequest {
                resource: resource_id.to_string(),
                action: actions[action_idx].to_string(),
                context: ctx,
            }
        })
        .collect();

    // Warmup
    for request in requests.iter().take(100) {
        let _ = evaluator.evaluate(request);
    }

    // ============ SEQUENTIAL BENCHMARK ============
    println!("\n  --- Sequential Evaluation ---");
    let mut latencies: Vec<Duration> = Vec::with_capacity(iterations);
    let mut allow_count = 0;
    let mut deny_count = 0;

    let start = Instant::now();
    for request in &requests {
        let eval_start = Instant::now();
        let result = evaluator.evaluate(request);
        latencies.push(eval_start.elapsed());

        match result {
            Ok(PolicyAction::Allow) => allow_count += 1,
            Ok(PolicyAction::Deny) => deny_count += 1,
            Ok(PolicyAction::Log) => allow_count += 1,
            Err(_) => deny_count += 1,
        }
    }
    let total_duration = start.elapsed();

    latencies.sort();
    let p50 = latencies[iterations / 2];
    let p95 = latencies[(iterations as f64 * 0.95) as usize];
    let p99 = latencies[(iterations as f64 * 0.99) as usize];
    let min = latencies[0];
    let max = latencies[iterations - 1];
    let mean: Duration = latencies.iter().sum::<Duration>() / iterations as u32;
    let throughput = iterations as f64 / total_duration.as_secs_f64();

    println!("    Throughput: {:.0} ops/sec", throughput);
    println!(
        "    Latency - P50: {:.3}µs, P95: {:.3}µs, P99: {:.3}µs",
        p50.as_nanos() as f64 / 1000.0,
        p95.as_nanos() as f64 / 1000.0,
        p99.as_nanos() as f64 / 1000.0
    );
    println!(
        "    Min: {:.3}µs, Mean: {:.3}µs, Max: {:.3}µs",
        min.as_nanos() as f64 / 1000.0,
        mean.as_nanos() as f64 / 1000.0,
        max.as_nanos() as f64 / 1000.0
    );
    println!("    Allow: {}, Deny: {}", allow_count, deny_count);

    // ============ CACHED BENCHMARK ============
    println!("\n  --- With Decision Cache (10K capacity) ---");
    let cache = Arc::new(DecisionCache::new(10000));

    // Single-policy benchmark: one cache scope, generation captured up front.
    let scope = 0u64;
    let generation = cache.generation();

    // First pass - cache misses
    let start = Instant::now();
    for request in &requests {
        if cache.get(request, scope).is_none() {
            let decision = evaluator.evaluate(request).unwrap_or(PolicyAction::Deny);
            cache.insert(request, scope, decision, generation);
        }
    }
    let first_pass = start.elapsed();

    // Second pass - cache hits
    let start = Instant::now();
    for request in &requests {
        let _ = cache.get(request, scope);
    }
    let second_pass = start.elapsed();

    let cache_stats = cache.stats();
    println!(
        "    First pass (cold): {:.0} ops/sec",
        iterations as f64 / first_pass.as_secs_f64()
    );
    println!(
        "    Second pass (hot): {:.0} ops/sec",
        iterations as f64 / second_pass.as_secs_f64()
    );
    println!("    Cache hit rate: {:.1}%", cache_stats.hit_rate * 100.0);

    // ============ PARALLEL BATCH BENCHMARK ============
    println!("\n  --- Parallel Batch Evaluation ---");
    let evaluator_arc = Arc::new(evaluator);
    let batch_evaluator = BatchEvaluator::from_arc(evaluator_arc.clone());

    let start = Instant::now();
    let (_results, batch_stats) = batch_evaluator.evaluate_with_stats(&requests);
    let batch_duration = start.elapsed();

    println!(
        "    Throughput: {:.0} ops/sec",
        iterations as f64 / batch_duration.as_secs_f64()
    );
    println!(
        "    Latency - P50: {:.3}µs, P95: {:.3}µs, P99: {:.3}µs",
        batch_stats.p50_latency.as_nanos() as f64 / 1000.0,
        batch_stats.p95_latency.as_nanos() as f64 / 1000.0,
        batch_stats.p99_latency.as_nanos() as f64 / 1000.0
    );
    println!(
        "    Allow: {}, Deny: {}",
        batch_stats.allowed, batch_stats.denied
    );

    // ============ PARALLEL + CACHED BENCHMARK ============
    println!("\n  --- Parallel Batch + Cache ---");
    let cached_batch = BatchEvaluator::from_arc(evaluator_arc).with_cache(10000);

    // First batch - cache misses
    let start = Instant::now();
    let _ = cached_batch.evaluate_all(&requests);
    let first_batch = start.elapsed();

    // Second batch - cache hits
    let start = Instant::now();
    let _ = cached_batch.evaluate_all(&requests);
    let second_batch = start.elapsed();

    let cache_stats = cached_batch.cache_stats().unwrap();
    println!(
        "    First batch (cold): {:.0} ops/sec",
        iterations as f64 / first_batch.as_secs_f64()
    );
    println!(
        "    Second batch (hot): {:.0} ops/sec",
        iterations as f64 / second_batch.as_secs_f64()
    );
    println!("    Cache hit rate: {:.1}%", cache_stats.hit_rate * 100.0);
}

fn main() {
    println!("========================================");
    println!("  Reaper 10K Benchmark");
    println!("  Phase 1 & 2 Optimization Results");
    println!("========================================");

    // Show cache configuration (loaded from environment)
    let cache_config = CacheConfig::from_env();
    println!("\nCache configuration: {}", cache_config.summary());
    println!("  (Set REAPER_CACHE_ENABLED=false to disable)");

    let iterations = 50000;
    let base_path = "benchmarks/reaper-vs-opa";

    // RBAC
    benchmark_scenario(
        "RBAC (10K entities)",
        &format!("{}/policies/reaper/rbac.reap", base_path),
        &format!("{}/data/10k/rbac.json", base_path),
        iterations,
    );

    // ABAC
    benchmark_scenario(
        "ABAC (10K entities)",
        &format!("{}/policies/reaper/abac.reap", base_path),
        &format!("{}/data/10k/abac.json", base_path),
        iterations,
    );

    // Multilayer
    benchmark_scenario(
        "Multilayer (10K entities)",
        &format!("{}/policies/reaper/multilayer.reap", base_path),
        &format!("{}/data/10k/multilayer.json", base_path),
        iterations,
    );

    // ReBAC
    benchmark_scenario(
        "ReBAC (10K entities)",
        &format!("{}/policies/reaper/rebac.reap", base_path),
        &format!("{}/data/10k/rebac.json", base_path),
        iterations,
    );

    println!("\n========================================");
    println!("  Benchmark Complete");
    println!("========================================");
}
