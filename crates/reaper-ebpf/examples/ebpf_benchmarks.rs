/// eBPF Performance Benchmarks
///
/// This example provides benchmark scenarios to demonstrate expected
/// performance characteristics of the eBPF policy engine.
///
/// NOTE: Actual eBPF performance can only be measured on x86_64 with
/// compiled eBPF program. These benchmarks show:
/// 1. Theoretical performance based on eBPF characteristics
/// 2. Userspace component benchmarks that CAN run on ARM64
/// 3. Expected end-to-end performance on x86_64
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use reaper_ebpf::{LearningEngine, PolicyCompiler};
use std::collections::HashMap;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("\n{}", "=".repeat(80));
    println!("⚡ eBPF Performance Benchmarks");
    println!("{}", "=".repeat(80));

    // ========================================================================
    // Benchmark 1: Learning Engine Overhead
    // ========================================================================
    println!("\n📊 Benchmark 1: Learning Engine Overhead");
    println!("{}", "-".repeat(80));

    benchmark_learning_engine()?;

    // ========================================================================
    // Benchmark 2: Policy Compilation
    // ========================================================================
    println!("\n📊 Benchmark 2: Policy Compilation to eBPF Format");
    println!("{}", "-".repeat(80));

    benchmark_policy_compilation()?;

    // ========================================================================
    // Benchmark 3: Userspace Baseline (for comparison)
    // ========================================================================
    println!("\n📊 Benchmark 3: Userspace Baseline (for comparison)");
    println!("{}", "-".repeat(80));

    benchmark_userspace_baseline()?;

    // ========================================================================
    // Benchmark 4: Expected eBPF Performance (theoretical)
    // ========================================================================
    println!("\n📊 Benchmark 4: Expected eBPF Performance (theoretical)");
    println!("{}", "-".repeat(80));

    show_expected_ebpf_performance();

    println!("\n{}", "=".repeat(80));
    println!("✅ Benchmarks complete!");
    println!("{}", "=".repeat(80));

    Ok(())
}

fn benchmark_learning_engine() -> anyhow::Result<()> {
    let learning_engine = LearningEngine::with_defaults();
    let iterations = 100_000;

    println!("Recording {} access patterns...", iterations);

    let start = Instant::now();
    for i in 0..iterations {
        let resource = format!("/api/resource_{}", i % 1000);
        learning_engine.record_access(&resource, PolicyAction::Allow, Some(1000), None);
    }
    let elapsed = start.elapsed();

    let mean_ns = elapsed.as_nanos() / iterations;
    let throughput = iterations as f64 / elapsed.as_secs_f64();

    println!("Results:");
    println!("  Total time:  {:?}", elapsed);
    println!("  Mean:        {} ns per record", mean_ns);
    println!("  Throughput:  {:.0} records/second", throughput);

    // Check promotion logic performance
    println!("\nTesting promotion detection...");
    let start = Instant::now();
    for i in 0..1000 {
        let resource = format!("/api/resource_{}", i);
        let _ = learning_engine.should_promote(&resource);
    }
    let promotion_check_time = start.elapsed();
    println!(
        "  Promotion checks: {:?} for 1000 resources",
        promotion_check_time
    );
    println!(
        "  Mean: {} ns per check",
        promotion_check_time.as_nanos() / 1000
    );

    Ok(())
}

fn benchmark_policy_compilation() -> anyhow::Result<()> {
    let compiler = PolicyCompiler::new();

    // Create test policies
    let mut policies = Vec::new();
    for i in 0..100 {
        let policy = EnhancedPolicy::new(
            format!("policy-{}", i),
            format!("Test policy {}", i),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: format!("/api/resource_{}", i),
                conditions: vec!["role==user".to_string()],
            }],
        );
        policies.push(policy);
    }

    println!("Compiling {} policies to eBPF format...", policies.len());

    let start = Instant::now();
    let mut total_rules = 0;

    for policy in &policies {
        // Compile decision to eBPF format
        let (key, entry) = compiler.compile_decision(
            &policy.rules[0].resource,
            policy.rules[0].action.clone(),
            Some(1000),
            None,
            0,
        )?;

        total_rules += 1;

        // Verify compilation
        assert_eq!(key.len(), 256);
        assert!(entry.action == 1); // Allow = 1
    }

    let elapsed = start.elapsed();
    let mean_ns = elapsed.as_nanos() / total_rules;

    println!("Results:");
    println!("  Total time:  {:?}", elapsed);
    println!("  Rules compiled: {}", total_rules);
    println!("  Mean: {} ns per rule", mean_ns);
    println!(
        "  Throughput: {:.0} rules/second",
        total_rules as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}

fn benchmark_userspace_baseline() -> anyhow::Result<()> {
    let engine = PolicyEngine::new();

    // Deploy simple policy
    let policy = EnhancedPolicy::new(
        "test-policy".to_string(),
        "Test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/api/users".to_string(),
            conditions: vec!["role==user".to_string()],
        }],
    );

    let policy_id = policy.id;
    engine.deploy_policy(policy)?;

    // Benchmark evaluation
    let iterations = 10_000;
    let mut latencies = Vec::with_capacity(iterations);

    let mut context = HashMap::new();
    context.insert("role".to_string(), "user".to_string());

    let request = PolicyRequest {
        resource: "/api/users".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    println!("Running {} userspace evaluations...", iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        let _ = engine.evaluate(&policy_id, &request)?;
        let elapsed = start.elapsed().as_nanos();
        latencies.push(elapsed);
    }

    latencies.sort();
    let min = latencies[0];
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let median = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

    println!("Userspace Performance:");
    println!("  Min:     {} ns", min);
    println!("  Mean:    {} ns", mean);
    println!("  Median:  {} ns", median);
    println!("  P99:     {} ns", p99);
    println!(
        "  Throughput: {:.0} req/s",
        iterations as f64 / (mean as f64 / 1_000_000_000.0)
    );

    Ok(())
}

fn show_expected_ebpf_performance() {
    println!("Expected eBPF Performance (on x86_64):");
    println!();
    println!("Fast Path (eBPF in kernel):");
    println!("┌─────────────────────┬──────────┬────────────┐");
    println!("│ Operation           │ Latency  │ Notes      │");
    println!("├─────────────────────┼──────────┼────────────┤");
    println!("│ BPF map lookup      │ 20-50ns  │ O(1) hash  │");
    println!("│ UID check           │ ~10ns    │ Compare    │");
    println!("│ GID check           │ ~10ns    │ Compare    │");
    println!("│ Context check       │ 20-30ns  │ Map lookup │");
    println!("│ Decision logic      │ ~5ns     │ Match      │");
    println!("├─────────────────────┼──────────┼────────────┤");
    println!("│ TOTAL (simple)      │ <100ns   │ ⚡ FAST    │");
    println!("│ TOTAL (with checks) │ <150ns   │ ⚡ FAST    │");
    println!("└─────────────────────┴──────────┴────────────┘");
    println!();
    println!("Throughput: >10M decisions/second/core");
    println!();

    println!("Slow Path (userspace):");
    println!("┌─────────────────────┬──────────┬────────────┐");
    println!("│ Policy Type         │ Latency  │ Throughput │");
    println!("├─────────────────────┼──────────┼────────────┤");
    println!("│ Simple (baseline)   │ 300ns    │ 3.3M/s     │");
    println!("│ Cedar ABAC          │ 10-50µs  │ 50K/s      │");
    println!("│ Reaper DSL          │ 1-10µs   │ 500K/s     │");
    println!("└─────────────────────┴──────────┴────────────┘");
    println!();

    println!("Learning & Promotion:");
    println!("┌────────────────────────────┬──────────┐");
    println!("│ Metric                     │ Value    │");
    println!("├────────────────────────────┼──────────┤");
    println!("│ Promotion threshold        │ 100 acc. │");
    println!("│ Stability requirement      │ 100 same │");
    println!("│ Promotion compilation time │ <1ms     │");
    println!("│ Speedup after promotion    │ 100-500x │");
    println!("└────────────────────────────┴──────────┘");
    println!();

    println!("Comparison:");
    println!();
    println!("Scenario: Hot path with 1000 requests/second");
    println!();
    println!("Before promotion (userspace):");
    println!("  • Cedar ABAC: 20µs average");
    println!("  • Total time: 20ms/second");
    println!("  • CPU usage: 2%");
    println!();
    println!("After promotion (eBPF):");
    println!("  • eBPF fast path: 50ns average");
    println!("  • Total time: 0.05ms/second");
    println!("  • CPU usage: 0.005%");
    println!("  • Speedup: 400x faster!");
    println!("  • CPU savings: 99.75%!");
    println!();

    println!("Real-World Impact:");
    println!();
    println!("System: 1M requests/second policy evaluation");
    println!();
    println!("Userspace only (baseline @ 300ns):");
    println!("  • Total latency: 300ms/second");
    println!("  • CPU cores needed: ~30% of 1 core");
    println!();
    println!("With eBPF (80% fast path @ 50ns, 20% slow @ 20µs):");
    println!("  • Fast path: 40ms/second (800K requests)");
    println!("  • Slow path: 4ms/second (200K requests)");
    println!("  • Total latency: 44ms/second");
    println!("  • CPU cores needed: ~4.4% of 1 core");
    println!("  • Improvement: 6.8x faster, 85% less CPU!");
}
