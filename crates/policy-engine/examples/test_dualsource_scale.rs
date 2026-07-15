/// Multi-Source Policy Evaluation - Scale Test
///
/// This example demonstrates:
/// 1. Loading data from TWO separate sources (roles + attributes)
/// 2. Joining data sources on user_id at load time
/// 3. Evaluating policies that require data from BOTH sources
/// 4. Measuring memory usage and performance at scale
///
/// Architecture:
/// - SOURCE 1: Role mappings (user_id -> roles[])
/// - SOURCE 2: User attributes (user_id -> department, clearance, etc.)
/// - The policy requires BOTH roles AND clearance checks
///
/// This tests real-world scenarios where user data comes from multiple
/// identity providers, directory services, or databases.
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

/// Memory statistics
#[derive(Debug)]
struct MemoryStats {
    _roles_file_size: u64,
    _attributes_file_size: u64,
    _resources_file_size: u64,
    _total_file_size: u64,
    _estimated_memory_mb: f64,
}

/// Load entities from a JSON file
fn load_entities_from_file(filename: &str) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(filename)?;
    let data: Value = serde_json::from_str(&content)?;
    let entities = data["entities"]
        .as_array()
        .ok_or("Missing entities array")?
        .clone();
    Ok(entities)
}

/// Join two data sources on user_id
/// This combines roles from SOURCE 1 with attributes from SOURCE 2
fn join_data_sources(
    roles: Vec<Value>,
    attributes: Vec<Value>,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    println!("🔗 Joining data sources on user_id...");
    let start = Instant::now();

    // Build a map of user_id -> attributes for fast lookup
    let mut attributes_map: HashMap<String, Value> = HashMap::new();
    for attr_entity in attributes {
        if let Some(id) = attr_entity["attributes"]["id"].as_str() {
            attributes_map.insert(id.to_string(), attr_entity);
        }
    }

    // Join roles with attributes
    let mut joined = Vec::new();
    let mut joined_count = 0;
    let mut missing_count = 0;

    for role_entity in roles {
        if let Some(id) = role_entity["attributes"]["id"].as_str() {
            if let Some(attr_entity) = attributes_map.get(id) {
                // Merge attributes from both sources
                let mut merged = role_entity.clone();

                // Copy all attributes from attributes source
                if let Some(attr_obj) = attr_entity["attributes"].as_object() {
                    if let Some(merged_attrs) = merged["attributes"].as_object_mut() {
                        for (key, value) in attr_obj {
                            if key != "id" {
                                // Don't overwrite id
                                merged_attrs.insert(key.clone(), value.clone());
                            }
                        }
                    }
                }

                joined.push(merged);
                joined_count += 1;
            } else {
                missing_count += 1;
            }
        }
    }

    let elapsed = start.elapsed();
    println!("   ✓ Joined {} users in {:?}", joined_count, elapsed);
    if missing_count > 0 {
        println!(
            "   ⚠️  {} users in roles source had no matching attributes",
            missing_count
        );
    }

    Ok(joined)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine scale based on command line or file existence
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
        // Default to small
        (
            100,
            "test-data/dualsource-roles-small.json",
            "test-data/dualsource-attributes-small.json",
            "test-data/dualsource-resources-small.json",
        )
    };

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  Multi-Source Policy Evaluation - Scale Test             ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");
    println!("Scale: {} users\n", scale);

    // ============================================================
    // PHASE 1: Load Data Sources
    // ============================================================
    println!("📥 PHASE 1: Loading data sources...\n");

    let load_start = Instant::now();

    // Check if files exist
    if !std::path::Path::new(roles_file).exists() {
        eprintln!("❌ Error: {} not found", roles_file);
        eprintln!(
            "Run: cargo run --release --example generate_dualsource_data {}",
            scale
        );
        std::process::exit(1);
    }

    println!("   Loading SOURCE 1: {} (roles)...", roles_file);
    let roles_start = Instant::now();
    let roles = load_entities_from_file(roles_file)?;
    let roles_time = roles_start.elapsed();
    println!(
        "   ✓ Loaded {} role records in {:?}",
        roles.len(),
        roles_time
    );

    println!("   Loading SOURCE 2: {} (attributes)...", attributes_file);
    let attrs_start = Instant::now();
    let attributes = load_entities_from_file(attributes_file)?;
    let attrs_time = attrs_start.elapsed();
    println!(
        "   ✓ Loaded {} attribute records in {:?}",
        attributes.len(),
        attrs_time
    );

    println!("   Loading resources: {}...", resources_file);
    let resources_start = Instant::now();
    let resources = load_entities_from_file(resources_file)?;
    let resources_time = resources_start.elapsed();
    println!(
        "   ✓ Loaded {} resources in {:?}\n",
        resources.len(),
        resources_time
    );

    // ============================================================
    // PHASE 2: Join Data Sources
    // ============================================================
    println!("🔗 PHASE 2: Joining data sources...\n");

    let joined_users = join_data_sources(roles, attributes)?;

    // Combine users and resources into single entity array
    let mut all_entities = joined_users;
    all_entities.extend(resources);

    println!("   ✓ Combined {} total entities\n", all_entities.len());

    // ============================================================
    // PHASE 3: Load into DataStore
    // ============================================================
    println!("🏗️  PHASE 3: Building DataStore...\n");

    let convert_start = Instant::now();

    // Load directly from JSON values (no serialization - saves ~40% memory)
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let stats = loader.load_json_values(all_entities)?;
    let store = Arc::new(store);

    let convert_time = convert_start.elapsed();
    println!("   ✓ Built DataStore in {:?}", convert_time);
    println!("   Total entities: {}", stats.total);
    println!("   Entity types:");
    for (entity_type, count) in stats.by_type.iter() {
        println!("      {}: {}", entity_type, count);
    }
    println!();

    // ============================================================
    // PHASE 3.5: Entity Type Validation
    // ============================================================
    println!("🔍 PHASE 3.5: Entity type validation...\n");

    let entity_stats = store.get_entity_type_stats();
    for (entity_type, count) in entity_stats.iter() {
        println!("   {} entities: {}", entity_type, count);
    }
    println!();

    let total_load_time = load_start.elapsed();
    println!("✅ Total load time: {:?}\n", total_load_time);

    // ============================================================
    // PHASE 4: Memory Analysis
    // ============================================================
    println!("💾 PHASE 4: Memory analysis...\n");

    let roles_size = fs::metadata(roles_file)?.len();
    let attributes_size = fs::metadata(attributes_file)?.len();
    let resources_size = fs::metadata(resources_file)?.len();
    let total_size = roles_size + attributes_size + resources_size;

    // Estimate in-memory size (rough approximation)
    // String interner saves memory, but we have overhead from HashMap
    let estimated_memory_mb = (total_size as f64 * 0.8) / 1_048_576.0;

    let _stats = MemoryStats {
        _roles_file_size: roles_size,
        _attributes_file_size: attributes_size,
        _resources_file_size: resources_size,
        _total_file_size: total_size,
        _estimated_memory_mb: estimated_memory_mb,
    };

    println!("   Roles file:       {} MB", roles_size / 1_048_576);
    println!("   Attributes file:  {} MB", attributes_size / 1_048_576);
    println!("   Resources file:   {} MB", resources_size / 1_048_576);
    println!("   Total on disk:    {} MB", total_size / 1_048_576);
    println!("   Estimated in-mem: {:.2} MB", estimated_memory_mb);
    println!();

    // ============================================================
    // PHASE 5: Policy Evaluation
    // ============================================================
    println!("🎯 PHASE 5: Policy evaluation...\n");

    // Create and save multi-source policy
    let policy_content = r#"// Multi-Source Access Control Policy
// Requires data from BOTH sources:
// - User role (SOURCE 1: identity provider)
// - User clearance and status (SOURCE 2: directory service)
// - Resource attributes

policy multi_source_access {
    version: "1.0.0",
    description: "Multi-source policy demonstrating data joining",

    default: deny,

    // Allow users with high clearance (>=3) and active status
    // Requires data from BOTH SOURCE 1 (primary_role) and SOURCE 2 (high_clearance, is_active)
    rule high_clearance_active_users {
        allow if {
            user.high_clearance == true &&
            user.is_active == true &&
            resource.is_archived != true
        }
    }

    // Admin override - admins can access any non-archived resource
    // Uses primary_role from SOURCE 1
    rule admin_override {
        allow if {
            user.primary_role == "admin" &&
            resource.is_archived != true
        }
    }

    // Analyst access to public/internal resources
    // Uses primary_role from SOURCE 1 and is_active from SOURCE 2
    rule analyst_access {
        allow if {
            user.primary_role == "analyst" &&
            user.is_active == true &&
            (resource.classification == "public" || resource.classification == "internal")
        }
    }
}
"#;

    fs::write("test-data/dualsource-policy.reap", policy_content)?;

    println!("Policy requires data from BOTH sources:");
    println!("   - User roles (from SOURCE 1)");
    println!("   - User clearance, status (from SOURCE 2)");
    println!("   - Resource attributes");
    println!();

    println!("📜 Loading policy from dualsource-policy.reap...");
    let policy = ReaperPolicy::from_file("test-data/dualsource-policy.reap")?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled successfully\n");

    // Run test evaluations
    let num_tests = if scale <= 1000 { 1000 } else { 10000 };
    println!("Running {} test evaluations...", num_tests);

    let mut allow_count = 0;
    let mut deny_count = 0;
    let mut error_count = 0;

    let eval_start = Instant::now();

    for i in 0..num_tests {
        let user_idx = i % scale;
        let resource_idx = (i * 2) % (scale * 2);

        let principal = format!("user_{}", user_idx);
        let resource = format!("doc_{}", resource_idx);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal);

        let request = PolicyRequest {
            resource,
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        match evaluator.evaluate(&request) {
            Ok(decision) => {
                let decision_str = format!("{:?}", decision);
                if decision_str.contains("Allow") {
                    allow_count += 1;
                } else {
                    deny_count += 1;
                }
            }
            Err(_) => error_count += 1,
        }

        if num_tests >= 1000 && (i + 1) % 1000 == 0 {
            print!("\r   Progress: {}/{}", i + 1, num_tests);
        }
    }

    let eval_time = eval_start.elapsed();
    let eval_per_second = num_tests as f64 / eval_time.as_secs_f64();

    println!("\n\n✅ Evaluation complete:");
    println!("   Total:      {}", num_tests);
    println!(
        "   Allowed:    {} ({:.1}%)",
        allow_count,
        (allow_count as f64 / num_tests as f64) * 100.0
    );
    println!(
        "   Denied:     {} ({:.1}%)",
        deny_count,
        (deny_count as f64 / num_tests as f64) * 100.0
    );
    println!("   Errors:     {}", error_count);
    println!("   Time:       {:?}", eval_time);
    println!("   Throughput: {:.0} ops/sec", eval_per_second);
    println!();

    // ============================================================
    // PHASE 6: Performance Analysis
    // ============================================================
    println!("📊 PHASE 6: Performance analysis...\n");

    // Run latency test
    let latency_samples = if scale <= 1000 { 1000 } else { 10000 };
    let mut latencies = Vec::with_capacity(latency_samples);

    for i in 0..latency_samples {
        let user_idx = i % scale;
        let resource_idx = (i * 2) % (scale * 2);

        let principal = format!("user_{}", user_idx);
        let resource = format!("doc_{}", resource_idx);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal);

        let request = PolicyRequest {
            resource,
            action: "read".to_string(),
            context,

            ..Default::default()
        };

        let start = Instant::now();
        let _ = evaluator.evaluate(&request);
        let latency = start.elapsed();

        latencies.push(latency.as_nanos() as u64);
    }

    latencies.sort_unstable();

    let mean = latencies.iter().sum::<u64>() as f64 / latencies.len() as f64;
    let p50 = latencies[latencies.len() / 2];
    let p95 = latencies[latencies.len() * 95 / 100];
    let p99 = latencies[latencies.len() * 99 / 100];
    let max = latencies[latencies.len() - 1];

    println!("Latency Distribution ({} samples):", latency_samples);
    println!("   Mean:  {:.2}ns", mean);
    println!("   P50:   {}ns", p50);
    println!("   P95:   {}ns", p95);
    println!("   P99:   {}ns", p99);
    println!("   Max:   {}ns", max);
    println!();

    // ============================================================
    // Summary
    // ============================================================
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  SUMMARY                                                  ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    println!(
        "Scale:               {} users, {} resources",
        scale,
        scale * 2
    );
    println!("Data Sources:        2 (roles + attributes)");
    println!("Join Strategy:       HashMap lookup on user_id");
    println!("Load Time:           {:?}", total_load_time);
    println!("Memory (estimated):  {:.2} MB", estimated_memory_mb);
    println!("Throughput:          {:.0} ops/sec", eval_per_second);
    println!("Mean Latency:        {:.2}ns", mean);
    println!("P99 Latency:         {}ns", p99);
    println!();

    println!("✅ Multi-source policy evaluation successful!");
    println!();
    println!("💡 To test at 1M scale:");
    println!("   1. cargo run --release --example generate_dualsource_data 1000000");
    println!("   2. cargo run --release --example test_dualsource_scale large");

    Ok(())
}
