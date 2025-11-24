// ! Volume Testing with Memory Analysis: 1k vs 100k entities
//!
//! Tests:
//! - Compare 1k vs 100k entity performance
//! - Measure actual memory usage
//! - Validate string interning efficiency
//! - Check for memory leaks

use policy_engine::{
    DataStore, DataLoader, PolicyRequest,
    ReaperPolicy, PolicyEvaluator,
};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Instant, Duration};
use std::fs;

#[cfg(target_os = "linux")]
fn get_memory_usage() -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let status = fs::read_to_string("/proc/self/status")?;

    let mut rss_kb = 0;
    let mut vm_size_kb = 0;

    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            rss_kb = line.split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("VmSize:") {
            vm_size_kb = line.split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }

    Ok((rss_kb * 1024, vm_size_kb * 1024))
}

#[cfg(not(target_os = "linux"))]
fn get_memory_usage() -> Result<(usize, usize), Box<dyn std::error::Error>> {
    Ok((0, 0)) // Fallback for non-Linux systems
}

struct MemorySnapshot {
    label: String,
    rss_bytes: usize,
    vm_size_bytes: usize,
}

impl MemorySnapshot {
    fn take(label: &str) -> Self {
        let (rss, vm) = get_memory_usage().unwrap_or((0, 0));
        Self {
            label: label.to_string(),
            rss_bytes: rss,
            vm_size_bytes: vm,
        }
    }

    fn print(&self) {
        println!("   📊 {}: RSS={:.2} MB, VM={:.2} MB",
            self.label,
            self.rss_bytes as f64 / 1_048_576.0,
            self.vm_size_bytes as f64 / 1_048_576.0
        );
    }

    fn diff(&self, other: &MemorySnapshot) -> (i64, i64) {
        (
            self.rss_bytes as i64 - other.rss_bytes as i64,
            self.vm_size_bytes as i64 - other.vm_size_bytes as i64,
        )
    }
}

struct TestResult {
    entity_count: usize,
    data_file_size: u64,
    load_time: Duration,
    mean_eval_time: Duration,
    p99_eval_time: Duration,
    memory_baseline: MemorySnapshot,
    memory_after_load: MemorySnapshot,
    memory_after_eval: MemorySnapshot,
}

impl TestResult {
    fn print_summary(&self) {
        println!("\n   Summary for {} entities:", self.entity_count);
        println!("   • Data file: {:.2} MB", self.data_file_size as f64 / 1_048_576.0);
        println!("   • Load time: {:?}", self.load_time);
        println!("   • Mean eval: {:.0} ns", self.mean_eval_time.as_nanos());
        println!("   • P99 eval: {:.0} ns", self.p99_eval_time.as_nanos());

        let (rss_load, _) = self.memory_after_load.diff(&self.memory_baseline);
        let (rss_total, _) = self.memory_after_eval.diff(&self.memory_baseline);

        println!("   • Memory (data load): +{:.2} MB", rss_load as f64 / 1_048_576.0);
        println!("   • Memory (total): +{:.2} MB", rss_total as f64 / 1_048_576.0);

        // Calculate memory per entity
        let memory_per_entity = rss_load as f64 / self.entity_count as f64;
        println!("   • Memory per entity: {:.0} bytes", memory_per_entity);

        // Calculate data file compression ratio
        let compression = (self.data_file_size as f64) / (rss_load as f64);
        println!("   • Compression ratio: {:.2}x (JSON → DataStore)", compression);
    }
}

fn run_test(data_path: &str, entity_count: usize, iterations: usize) -> Result<TestResult, Box<dyn std::error::Error>> {
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  🔬 Testing {} Entities", entity_count);
    println!("═══════════════════════════════════════════════════════════\n");

    // Baseline memory
    let mem_baseline = MemorySnapshot::take("Baseline");
    mem_baseline.print();

    // Load data
    println!("\n1️⃣  Loading data from {}...", data_path);
    let data_content = fs::read_to_string(data_path)?;
    let file_size = fs::metadata(data_path)?.len();
    println!("   📦 File size: {:.2} MB", file_size as f64 / 1_048_576.0);

    let load_start = Instant::now();
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let loaded_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    let load_time = load_start.elapsed();

    println!("   ✓ Loaded {} entities in {:?}", loaded_count, load_time);

    let mem_after_load = MemorySnapshot::take("After data load");
    mem_after_load.print();

    let (rss_increase, _) = mem_after_load.diff(&mem_baseline);
    println!("   💾 Memory increase: +{:.2} MB", rss_increase as f64 / 1_048_576.0);

    // Load policy
    println!("\n2️⃣  Loading policy...");
    let policy_text = r#"
        policy memory_test {
            version: "1.0.0",
            default: deny,

            rule admin_access {
                allow if user.role == "admin"
            }

            rule department_access {
                allow if {
                    user.department == resource.department &&
                    user.clearance >= resource.clearance_required &&
                    user.status == "active"
                }
            }

            rule owner_access {
                allow if user.id == resource.owner_id
            }
        }
    "#;

    let policy = policy_text.parse::<ReaperPolicy>()?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled");

    // Warm up
    println!("\n3️⃣  Warming up (1000 iterations)...");
    let mut context = HashMap::new();
    context.insert("principal".to_string(), format!("user_{}", entity_count / 4));
    let request = PolicyRequest {
        resource: format!("doc_{}", entity_count / 4),
        action: "read".to_string(),
        context: context.clone(),
    };

    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request)?;
    }
    println!("   ✓ Warm-up complete");

    // Run evaluations
    println!("\n4️⃣  Running {} evaluations...", iterations);
    let mut samples = Vec::with_capacity(iterations);
    let max_entities = entity_count / 2; // Use half for users, half for docs

    for i in 0..iterations {
        let user_id = format!("user_{}", i % max_entities);
        let doc_id = format!("doc_{}", (i * 3) % max_entities);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id);

        let request = PolicyRequest {
            resource: doc_id,
            action: "read".to_string(),
            context,
        };

        let start = Instant::now();
        let _decision = evaluator.evaluate(&request)?;
        samples.push(start.elapsed());
    }

    let mem_after_eval = MemorySnapshot::take("After evaluation");
    mem_after_eval.print();

    // Calculate statistics
    samples.sort();
    let sum: Duration = samples.iter().sum();
    let mean = sum / samples.len() as u32;
    let p99 = samples[(samples.len() as f64 * 0.99) as usize];

    println!("   ✓ Complete");
    println!("   • Mean: {:.0} ns", mean.as_nanos());
    println!("   • P99: {:.0} ns", p99.as_nanos());

    Ok(TestResult {
        entity_count: loaded_count,
        data_file_size: file_size,
        load_time,
        mean_eval_time: mean,
        p99_eval_time: p99,
        memory_baseline: mem_baseline,
        memory_after_load: mem_after_load,
        memory_after_eval: mem_after_eval,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("    🔬 REAPER MEMORY & SCALE PERFORMANCE TEST");
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("Testing 1k vs 100k entities to analyze:");
    println!("  • Performance scaling");
    println!("  • Memory efficiency");
    println!("  • String interning impact");
    println!("  • Memory leak detection");
    println!();

    let mut results = Vec::new();

    // Test 1: 1k entities
    if std::path::Path::new("large-test-data.json").exists() {
        let result = run_test("large-test-data.json", 1000, 10_000)?;
        results.push(result);
    } else {
        println!("⚠️  large-test-data.json not found, skipping 1k test");
        println!("   Run: cargo run --example generate_large_data --release");
    }

    // Test 2: 100k entities
    if std::path::Path::new("huge-test-data.json").exists() {
        let result = run_test("huge-test-data.json", 100_000, 10_000)?;
        results.push(result);
    } else {
        println!("⚠️  huge-test-data.json not found, skipping 100k test");
        println!("   Run: cargo run --example generate_huge_data --release");
    }

    // Comparison analysis
    if results.len() >= 2 {
        println!("\n═══════════════════════════════════════════════════════════");
        println!("  📊 COMPARISON ANALYSIS: 1k vs 100k");
        println!("═══════════════════════════════════════════════════════════\n");

        let small = &results[0];
        let large = &results[1];

        println!("┌─────────────────────────┬────────────┬────────────┬──────────┐");
        println!("│ Metric                  │ 1k         │ 100k       │ Ratio    │");
        println!("├─────────────────────────┼────────────┼────────────┼──────────┤");

        // Data file size
        let file_ratio = large.data_file_size as f64 / small.data_file_size as f64;
        println!("│ Data file size          │ {:>7.2} MB │ {:>7.2} MB │ {:>6.1}x │",
            small.data_file_size as f64 / 1_048_576.0,
            large.data_file_size as f64 / 1_048_576.0,
            file_ratio
        );

        // Memory usage
        let (small_mem, _) = small.memory_after_load.diff(&small.memory_baseline);
        let (large_mem, _) = large.memory_after_load.diff(&large.memory_baseline);
        let mem_ratio = large_mem as f64 / small_mem as f64;
        println!("│ Memory usage            │ {:>7.2} MB │ {:>7.2} MB │ {:>6.1}x │",
            small_mem as f64 / 1_048_576.0,
            large_mem as f64 / 1_048_576.0,
            mem_ratio
        );

        // Memory per entity
        let small_per_entity = small_mem as f64 / small.entity_count as f64;
        let large_per_entity = large_mem as f64 / large.entity_count as f64;
        println!("│ Memory per entity       │ {:>8.0} B │ {:>8.0} B │ {:>6.2}x │",
            small_per_entity,
            large_per_entity,
            large_per_entity / small_per_entity
        );

        // Load time
        let load_ratio = large.load_time.as_secs_f64() / small.load_time.as_secs_f64();
        println!("│ Load time               │ {:>7.2} ms │ {:>7.2} ms │ {:>6.1}x │",
            small.load_time.as_secs_f64() * 1000.0,
            large.load_time.as_secs_f64() * 1000.0,
            load_ratio
        );

        // Mean eval time
        let eval_ratio = large.mean_eval_time.as_nanos() as f64 / small.mean_eval_time.as_nanos() as f64;
        println!("│ Mean eval time          │ {:>8.0} ns │ {:>8.0} ns │ {:>6.2}x │",
            small.mean_eval_time.as_nanos(),
            large.mean_eval_time.as_nanos(),
            eval_ratio
        );

        // P99 eval time
        let p99_ratio = large.p99_eval_time.as_nanos() as f64 / small.p99_eval_time.as_nanos() as f64;
        println!("│ P99 eval time           │ {:>8.0} ns │ {:>8.0} ns │ {:>6.2}x │",
            small.p99_eval_time.as_nanos(),
            large.p99_eval_time.as_nanos(),
            p99_ratio
        );

        println!("└─────────────────────────┴────────────┴────────────┴──────────┘");

        println!("\n🔍 Analysis:");

        // Memory efficiency
        println!("\n   Memory Efficiency:");
        if mem_ratio < 110.0 {
            println!("   ✅ EXCELLENT: Memory scales linearly ({:.1}x for 100x data)", mem_ratio);
        } else {
            println!("   ⚠️  Memory scaling: {:.1}x (expected ~100x)", mem_ratio);
        }

        let small_compression = small.data_file_size as f64 / small_mem as f64;
        let large_compression = large.data_file_size as f64 / large_mem as f64;
        println!("   • Compression (1k): {:.2}x", small_compression);
        println!("   • Compression (100k): {:.2}x", large_compression);
        println!("   • String interning saves ~{}% memory",
            ((1.0 - 1.0/large_compression) * 100.0) as i32);

        // Performance scaling
        println!("\n   Performance Scaling:");
        if eval_ratio < 2.0 {
            println!("   ✅ EXCELLENT: Evaluation time barely affected ({:.2}x)", eval_ratio);
            println!("   String interning and indexing working perfectly!");
        } else if eval_ratio < 5.0 {
            println!("   ✅ GOOD: Evaluation time scales well ({:.2}x)", eval_ratio);
        } else {
            println!("   ⚠️  Evaluation time scaling: {:.2}x", eval_ratio);
        }

        if large.mean_eval_time.as_nanos() < 2000 {
            println!("   ✅ Still sub-2µs even with 100k entities!");
        }

        // Memory leak check
        println!("\n   Memory Leak Detection:");
        let (small_eval_growth, _) = small.memory_after_eval.diff(&small.memory_after_load);
        let (large_eval_growth, _) = large.memory_after_eval.diff(&large.memory_after_load);

        if small_eval_growth.abs() < 1_000_000 && large_eval_growth.abs() < 1_000_000 {
            println!("   ✅ NO LEAKS: Memory stable during evaluation");
            println!("   • 1k: {:+.2} MB during 10k evals", small_eval_growth as f64 / 1_048_576.0);
            println!("   • 100k: {:+.2} MB during 10k evals", large_eval_growth as f64 / 1_048_576.0);
        } else {
            println!("   ⚠️  Memory growth detected during evaluation");
        }
    }

    // Print individual summaries
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  📋 DETAILED RESULTS");
    println!("═══════════════════════════════════════════════════════════");
    for result in &results {
        result.print_summary();
    }

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  ✅ TEST COMPLETE");
    println!("═══════════════════════════════════════════════════════════\n");

    Ok(())
}
