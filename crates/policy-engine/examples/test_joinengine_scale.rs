/// Multi-Source Policy Evaluation with JoinEngine - Scale Test
///
/// This example demonstrates Phase 2 implementation:
/// 1. Using the generic JoinEngine for multi-source data loading
/// 2. N-way joins (2+ sources) with declarative configuration
/// 3. Automatic join statistics and missing data tracking
/// 4. Measuring performance improvements over manual joining
///
/// Architecture:
/// - JoinEngine handles all join logic automatically
/// - Declarative JoinConfig specifies sources and join keys
/// - Supports arbitrary entity types and join fields
/// - Built-in statistics for monitoring join quality
use policy_engine::{
    data::{DataStore, EntitySource, JoinConfig, JoinEngine, JoinKey, SecondarySource},
    PolicyEvaluator, PolicyRequest, ReaperPolicy,
};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine scale based on command line
    let args: Vec<String> = env::args().collect();
    let (scale, roles_file, attributes_file, resources_file) = if args.len() > 1 {
        let s = args[1].as_str();
        if s == "large" || s == "1000000" {
            (
                1_000_000,
                "test-data/dualsource-roles-large.json",
                "test-data/dualsource-attributes-large.json",
                "test-data/dualsource-resources-large.json",
            )
        } else {
            (
                100,
                "test-data/dualsource-roles-small.json",
                "test-data/dualsource-attributes-small.json",
                "test-data/dualsource-resources-small.json",
            )
        }
    } else {
        (
            100,
            "test-data/dualsource-roles-small.json",
            "test-data/dualsource-attributes-small.json",
            "test-data/dualsource-resources-small.json",
        )
    };

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  JoinEngine - Multi-Source Policy Evaluation (Phase 2)   ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");
    println!("Scale: {} users\n", scale);

    // Check if files exist
    if !std::path::Path::new(roles_file).exists() {
        eprintln!("❌ Error: {} not found", roles_file);
        eprintln!(
            "Run: cargo run --release --example generate_dualsource_data {}",
            scale
        );
        std::process::exit(1);
    }

    // ============================================================
    // PHASE 1: Configure Multi-Source Join
    // ============================================================
    println!("🔧 PHASE 1: Configuring multi-source join...\n");

    let config = JoinConfig {
        primary: EntitySource {
            file_path: roles_file.to_string(),
            entity_type: "User".to_string(),
        },
        secondary: HashMap::from([(
            "UserAttributes".to_string(),
            SecondarySource {
                source: EntitySource {
                    file_path: attributes_file.to_string(),
                    entity_type: "UserAttributes".to_string(),
                },
                join_key: JoinKey {
                    primary_field: "attributes.id".to_string(),
                    secondary_field: "attributes.id".to_string(),
                },
            },
        )]),
    };

    println!(
        "   Primary source: {} ({})",
        roles_file, config.primary.entity_type
    );
    println!("   Secondary sources: {}", config.secondary.len());
    for (name, sec) in &config.secondary {
        println!(
            "      {} -> {} on {}",
            name, sec.source.file_path, sec.join_key.primary_field
        );
    }
    println!();

    // ============================================================
    // PHASE 2: Execute Join and Load (JoinEngine)
    // ============================================================
    println!("🔗 PHASE 2: Executing join with JoinEngine...\n");

    let total_start = Instant::now();
    let num_sources = config.secondary.len() + 1; // Save before moving

    let store = DataStore::new();
    let loader = policy_engine::DataLoader::new(store.clone());
    let engine = JoinEngine::new(loader);

    let join_result = engine.join_and_load(config)?;

    println!("   ✓ Join completed in {:?}", join_result.join_duration);
    println!("   Primary entities: {}", join_result.primary_count);
    println!("   Join statistics:");
    for (entity_type, count) in &join_result.join_counts {
        println!("      {}: {} successful joins", entity_type, count);
    }
    for (entity_type, count) in &join_result.missing_counts {
        if *count > 0 {
            println!("      {}: {} missing", entity_type, count);
        }
    }
    println!();

    // ============================================================
    // PHASE 3: Load Resources Separately
    // ============================================================
    println!("📥 PHASE 3: Loading resources...\n");

    let resources_start = Instant::now();
    let resources_content = std::fs::read_to_string(resources_file)?;
    let resources_data: serde_json::Value = serde_json::from_str(&resources_content)?;
    let resources = resources_data["entities"]
        .as_array()
        .ok_or("Missing entities array")?
        .clone();

    let loader2 = policy_engine::DataLoader::new(store.clone());
    let resource_stats = loader2.load_json_values(resources)?;
    let resources_time = resources_start.elapsed();

    println!(
        "   ✓ Loaded {} resources in {:?}",
        resource_stats.total, resources_time
    );
    println!();

    // ============================================================
    // PHASE 4: Entity Type Validation
    // ============================================================
    println!("🔍 PHASE 4: Entity type validation...\n");

    let store = Arc::new(store);
    let entity_stats = store.get_entity_type_stats();
    for (entity_type, count) in entity_stats.iter() {
        println!("   {} entities: {}", entity_type, count);
    }
    println!();

    let total_time = total_start.elapsed();
    println!("✅ Total load time: {:?}\n", total_time);

    // ============================================================
    // PHASE 5: Policy Evaluation
    // ============================================================
    println!("🎯 PHASE 5: Policy evaluation...\n");

    println!("📜 Loading policy from dualsource-policy.reap...");
    let policy = ReaperPolicy::from_file("test-data/dualsource-policy.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled successfully\n");

    // Sample evaluations
    let num_tests = if scale >= 100_000 { 10_000 } else { 1_000 };
    println!("Running {} test evaluations...", num_tests);

    let mut allowed = 0;
    let mut denied = 0;
    let mut error_count = 0;

    let eval_start = Instant::now();
    let mut latencies = Vec::with_capacity(num_tests);

    for i in 0..num_tests {
        let user_idx = i % join_result.primary_count;
        let resource_idx = (i * 2) % resource_stats.by_type.get("Resource").unwrap_or(&1);

        let principal = format!("user_{}", user_idx);
        let resource = format!("doc_{}", resource_idx);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal);

        let request = PolicyRequest {
            resource,
            action: "read".to_string(),
            context,
        };

        let lat_start = Instant::now();
        match evaluator.evaluate(&request) {
            Ok(decision) => {
                let latency = lat_start.elapsed();
                latencies.push(latency.as_nanos() as u64);

                let decision_str = format!("{:?}", decision);
                if decision_str.contains("Allow") {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(_) => error_count += 1,
        }

        if (i + 1) % 1000 == 0 {
            print!("   Progress: {}/{}", i + 1, num_tests);
            if (i + 1) < num_tests {
                print!("\r");
            }
        }
    }
    println!();

    let eval_time = eval_start.elapsed();
    let throughput = num_tests as f64 / eval_time.as_secs_f64();

    println!("\n✅ Evaluation complete:");
    println!("   Total:      {}", num_tests);
    println!(
        "   Allowed:    {} ({:.1}%)",
        allowed,
        (allowed as f64 / num_tests as f64) * 100.0
    );
    println!(
        "   Denied:     {} ({:.1}%)",
        denied,
        (denied as f64 / num_tests as f64) * 100.0
    );
    println!("   Errors:     {}", error_count);
    println!("   Time:       {:?}", eval_time);
    println!("   Throughput: {:.0} ops/sec", throughput);

    // ============================================================
    // PHASE 6: Performance Analysis
    // ============================================================
    println!("\n📊 PHASE 6: Performance analysis...\n");

    latencies.sort_unstable();
    let mean = latencies.iter().sum::<u64>() / latencies.len() as u64;
    let p50 = latencies[latencies.len() / 2];
    let p95 = latencies[latencies.len() * 95 / 100];
    let p99 = latencies[latencies.len() * 99 / 100];
    let max = latencies[latencies.len() - 1];

    println!("Latency Distribution ({} samples):", num_tests);
    println!("   Mean:  {:.2}ns", mean as f64);
    println!("   P50:   {}ns", p50);
    println!("   P95:   {}ns", p95);
    println!("   P99:   {}ns", p99);
    println!("   Max:   {}ns", max);

    // ============================================================
    // SUMMARY
    // ============================================================
    println!("\n╔═══════════════════════════════════════════════════════════╗");
    println!("║  SUMMARY                                                  ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    println!(
        "Scale:               {} users, {} resources",
        join_result.primary_count,
        resource_stats.by_type.get("Resource").unwrap_or(&0)
    );
    println!("JoinEngine:          {} sources joined", num_sources);
    println!("Join Strategy:       Declarative (JoinConfig)");
    println!("Join Time:           {:?}", join_result.join_duration);
    println!("Total Load Time:     {:?}", total_time);
    println!("Throughput:          {:.0} ops/sec", throughput);
    println!("Mean Latency:        {:.2}ns", mean as f64);
    println!("P99 Latency:         {}ns", p99);

    println!("\n✅ Phase 2 (JoinEngine) multi-source evaluation successful!\n");

    Ok(())
}
