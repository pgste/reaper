/// Decision Distribution Scale Test
///
/// Tests how decision distribution affects performance:
/// - Allow-heavy (90% allow, 10% deny)
/// - Balanced (50% allow, 50% deny)
/// - Deny-heavy (10% allow, 90% deny)
///
/// Tests if short-circuit evaluation affects performance.
/// Run with: cargo run --release --example scale_decision_distribution
use policy_engine::{
    DataLoader, DataStore, PolicyAction, PolicyEvaluator, PolicyRequest, ReaperPolicy,
};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

struct DistributionBenchmark {
    scenario: String,
    _iterations: usize,
    _allow_count: usize,
    _deny_count: usize,
    allow_percent: f64,
    mean_allow_ns: u128,
    mean_deny_ns: u128,
    overall_mean_ns: u128,
    p99_ns: u128,
}

fn benchmark_scenario(
    scenario: &str,
    evaluator: &impl PolicyEvaluator,
    test_cases: &[(String, String, bool)], // (user, resource, expected_allow)
) -> DistributionBenchmark {
    let mut allow_latencies = Vec::new();
    let mut deny_latencies = Vec::new();
    let mut all_latencies = Vec::new();

    let mut allow_count = 0;
    let mut deny_count = 0;

    for (user_id, resource_id, _expected) in test_cases {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id.clone());

        let request = PolicyRequest {
            resource: resource_id.clone(),
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let start = Instant::now();
        let decision = evaluator.evaluate(&request).unwrap();
        let latency = start.elapsed().as_nanos();

        all_latencies.push(latency);

        match decision {
            PolicyAction::Allow => {
                allow_count += 1;
                allow_latencies.push(latency);
            }
            PolicyAction::Deny | PolicyAction::Log => {
                deny_count += 1;
                deny_latencies.push(latency);
            }
        }
    }

    // Calculate statistics
    all_latencies.sort_unstable();
    let overall_mean = all_latencies.iter().sum::<u128>() / all_latencies.len() as u128;
    let p99 = all_latencies[(all_latencies.len() as f64 * 0.99) as usize];

    let mean_allow = if !allow_latencies.is_empty() {
        allow_latencies.iter().sum::<u128>() / allow_latencies.len() as u128
    } else {
        0
    };

    let mean_deny = if !deny_latencies.is_empty() {
        deny_latencies.iter().sum::<u128>() / deny_latencies.len() as u128
    } else {
        0
    };

    let allow_percent = (allow_count as f64 / test_cases.len() as f64) * 100.0;

    DistributionBenchmark {
        scenario: scenario.to_string(),
        _iterations: test_cases.len(),
        _allow_count: allow_count,
        _deny_count: deny_count,
        allow_percent,
        mean_allow_ns: mean_allow,
        mean_deny_ns: mean_deny,
        overall_mean_ns: overall_mean,
        p99_ns: p99,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("⚖️  Decision Distribution Scale Test");
    println!("{}", "=".repeat(70));
    println!("\nTesting if decision distribution affects performance...\n");

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

    let iterations_per_scenario = 1000;

    println!("\n{}", "=".repeat(70));
    println!("🔄 Generating Test Scenarios");
    println!("{}", "=".repeat(70));

    // Scenario 1: Allow-heavy (admins accessing resources - should mostly allow)
    println!("\n1️⃣  Allow-Heavy Scenario (Admins)");
    let mut allow_heavy_cases = Vec::new();
    for i in 0..iterations_per_scenario {
        // Users 0-99 are admins (10%), admins can access everything
        let user = format!("user_{}", i % 10); // Admin users
        let resource = format!("resource_{}", i % 200);
        allow_heavy_cases.push((user, resource, true));
    }

    let allow_heavy_result = benchmark_scenario("Allow-Heavy", &evaluator, &allow_heavy_cases);
    println!("   Generated {} test cases", allow_heavy_cases.len());
    println!(
        "   Actual distribution: {:.1}% allow",
        allow_heavy_result.allow_percent
    );

    // Scenario 2: Deny-heavy (regular users accessing random resources - should mostly deny)
    println!("\n2️⃣  Deny-Heavy Scenario (Random Access)");
    let mut deny_heavy_cases = Vec::new();
    for i in 0..iterations_per_scenario {
        // Regular users accessing resources they don't own
        let user = format!("user_{}", 200 + (i % 100)); // Non-admin users
        let resource = format!("resource_{}", (i * 7) % 200); // Unlikely to match
        deny_heavy_cases.push((user, resource, false));
    }

    let deny_heavy_result = benchmark_scenario("Deny-Heavy", &evaluator, &deny_heavy_cases);
    println!("   Generated {} test cases", deny_heavy_cases.len());
    println!(
        "   Actual distribution: {:.1}% allow",
        deny_heavy_result.allow_percent
    );

    // Scenario 3: Balanced (mix of admins, managers, and users)
    println!("\n3️⃣  Balanced Scenario (Mixed Access)");
    let mut balanced_cases = Vec::new();
    for i in 0..iterations_per_scenario {
        let user = format!("user_{}", i % 300); // Mix of all user types
        let resource = if i % 2 == 0 {
            // 50% chance of accessing own resource
            format!("resource_{}", i % 300)
        } else {
            // 50% chance of accessing random resource
            format!("resource_{}", (i * 13) % 2000)
        };
        balanced_cases.push((user, resource, false)); // Expected varies
    }

    let balanced_result = benchmark_scenario("Balanced", &evaluator, &balanced_cases);
    println!("   Generated {} test cases", balanced_cases.len());
    println!(
        "   Actual distribution: {:.1}% allow",
        balanced_result.allow_percent
    );

    // Scenario 4: Alternating (allow, deny, allow, deny)
    println!("\n4️⃣  Alternating Scenario (Predictable Pattern)");
    let mut alternating_cases = Vec::new();
    for i in 0..iterations_per_scenario {
        if i % 2 == 0 {
            // Even: admin user (should allow)
            let user = format!("user_{}", i % 10);
            let resource = format!("resource_{}", i % 200);
            alternating_cases.push((user, resource, true));
        } else {
            // Odd: non-owner user (should deny)
            let user = format!("user_{}", 200 + (i % 100));
            let resource = format!("resource_{}", (i * 7) % 200);
            alternating_cases.push((user, resource, false));
        }
    }

    let alternating_result = benchmark_scenario("Alternating", &evaluator, &alternating_cases);
    println!("   Generated {} test cases", alternating_cases.len());
    println!(
        "   Actual distribution: {:.1}% allow",
        alternating_result.allow_percent
    );

    // Print comparison table
    println!("\n{}", "=".repeat(70));
    println!("📊 Decision Distribution Performance");
    println!("{}", "=".repeat(70));
    println!(
        "\n{:<15} {:<12} {:<12} {:<12} {:<12} {:<12}",
        "Scenario", "Allow%", "Mean All", "Mean Allow", "Mean Deny", "P99"
    );
    println!("{}", "-".repeat(70));

    for result in &[
        &allow_heavy_result,
        &balanced_result,
        &deny_heavy_result,
        &alternating_result,
    ] {
        println!(
            "{:<15} {:<12.1} {:<12} {:<12} {:<12} {:<12}",
            result.scenario,
            result.allow_percent,
            format!("{}ns", result.overall_mean_ns),
            format!("{}ns", result.mean_allow_ns),
            format!("{}ns", result.mean_deny_ns),
            format!("{}ns", result.p99_ns),
        );
    }

    // Performance analysis
    println!("\n{}", "=".repeat(70));
    println!("📈 Performance Analysis");
    println!("{}", "=".repeat(70));

    println!("\n1️⃣  Allow vs Deny Performance:");
    for result in &[
        &allow_heavy_result,
        &balanced_result,
        &deny_heavy_result,
        &alternating_result,
    ] {
        if result.mean_allow_ns > 0 && result.mean_deny_ns > 0 {
            let ratio = result.mean_allow_ns as f64 / result.mean_deny_ns as f64;
            println!(
                "   {:<15} Allow/Deny ratio: {:.2}x {}",
                result.scenario,
                ratio,
                if ratio > 1.0 {
                    "(Allow slower)"
                } else {
                    "(Deny slower)"
                }
            );
        }
    }

    println!("\n2️⃣  Distribution Impact:");
    let baseline = balanced_result.overall_mean_ns as f64;
    for result in &[&allow_heavy_result, &deny_heavy_result, &alternating_result] {
        let diff = ((result.overall_mean_ns as f64 / baseline) - 1.0) * 100.0;
        println!("   {:<15} {:+.1}% vs balanced", result.scenario, diff);
    }

    // Insights
    println!("\n{}", "=".repeat(70));
    println!("💡 Key Insights");
    println!("{}", "=".repeat(70));

    let max_mean = [
        &allow_heavy_result,
        &balanced_result,
        &deny_heavy_result,
        &alternating_result,
    ]
    .iter()
    .map(|r| r.overall_mean_ns)
    .max()
    .unwrap() as f64;

    let min_mean = [
        &allow_heavy_result,
        &balanced_result,
        &deny_heavy_result,
        &alternating_result,
    ]
    .iter()
    .map(|r| r.overall_mean_ns)
    .min()
    .unwrap() as f64;

    let variance = (max_mean / min_mean - 1.0) * 100.0;

    if variance < 10.0 {
        println!("✅ Decision distribution has minimal impact (< 10% variance)");
        println!("   Engine performs consistently regardless of allow/deny ratio");
    } else if variance < 25.0 {
        println!(
            "⚠️  Moderate distribution impact ({:.1}% variance)",
            variance
        );
        println!("   Some difference between allow-heavy and deny-heavy scenarios");
    } else {
        println!(
            "🔥 Significant distribution impact ({:.1}% variance)",
            variance
        );
        println!("   Performance varies based on decision distribution");
    }

    // Check for short-circuit evaluation
    let allow_faster = allow_heavy_result.overall_mean_ns < deny_heavy_result.overall_mean_ns;
    println!("\n📊 Short-Circuit Behavior:");
    if allow_faster {
        let speedup =
            deny_heavy_result.overall_mean_ns as f64 / allow_heavy_result.overall_mean_ns as f64;
        println!("   Allow decisions are {:.2}x faster than deny", speedup);
        println!("   ✅ Policy likely short-circuits on first allow rule");
    } else {
        let speedup =
            allow_heavy_result.overall_mean_ns as f64 / deny_heavy_result.overall_mean_ns as f64;
        println!("   Deny decisions are {:.2}x faster than allow", speedup);
        println!("   ⚠️  Possible early-exit on deny rules");
    }

    println!("\n✅ Decision Distribution Test Complete!");

    Ok(())
}
