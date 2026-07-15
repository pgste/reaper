/// Join-Based Policy Scale Test - Comprehensive Example
///
/// This example demonstrates complete end-to-end multi-source policy evaluation:
/// 1. Generate realistic multi-source test data
/// 2. Configure and execute JoinEngine for N-way joins
/// 3. Evaluate policies requiring data from multiple sources
/// 4. Measure performance and memory at scale (100 to 100k entities)
///
/// Use Cases Demonstrated:
/// - User identity + attributes + device trust scores
/// - Context-aware access control (who + what + where + when)
/// - Multi-factor policy decisions
use policy_engine::{
    data::{DataStore, EntitySource, JoinConfig, JoinEngine, JoinKey, SecondarySource},
    PolicyEvaluator, PolicyRequest, ReaperPolicy,
};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse scale from command line
    let args: Vec<String> = env::args().collect();
    let scale = if args.len() > 1 {
        args[1].parse::<usize>().unwrap_or(100)
    } else {
        100
    };

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  Join-Based Policy Evaluation - Scale Test               ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");
    println!("Testing scale: {} entities\n", scale);

    // ============================================================
    // PHASE 1: Generate Multi-Source Test Data
    // ============================================================
    println!("📝 PHASE 1: Generating multi-source test data...\n");

    let gen_start = Instant::now();

    // Generate user identities (SOURCE 1)
    let mut user_identities = Vec::new();
    let roles = ["admin", "analyst", "viewer", "guest"];

    for i in 0..scale {
        let user_id = format!("user_{}", i);
        let role = roles[i % roles.len()];

        user_identities.push(json!({
            "id": user_id.clone(),
            "type": "UserIdentity",
            "attributes": {
                "id": user_id,
                "role": role,
                "team": format!("team_{}", i % 10),
            }
        }));
    }

    // Generate user attributes (SOURCE 2)
    let mut user_attributes = Vec::new();
    let departments = ["engineering", "security", "hr", "finance"];

    for i in 0..scale {
        let user_id = format!("user_{}", i);
        let clearance_level = (i % 5) + 1; // 1-5
        let is_active = i % 10 != 0; // 90% active

        user_attributes.push(json!({
            "id": user_id.clone(),
            "type": "UserAttributes",
            "attributes": {
                "id": user_id,
                "clearance": clearance_level,
                "high_clearance": clearance_level >= 3,
                "department": departments[i % departments.len()],
                "is_active": is_active,
                "certified": i % 3 == 0,
            }
        }));
    }

    // Generate device trust scores (SOURCE 3)
    let mut device_trust = Vec::new();

    for i in 0..(scale / 2) {
        let device_id = format!("device_{}", i);
        let trustscore = 50 + (i % 50); // 50-99

        device_trust.push(json!({
            "id": device_id.clone(),
            "type": "Device",
            "attributes": {
                "id": device_id,
                "trustscore": trustscore,
                "trusted": trustscore >= 75,
                "os": if i % 2 == 0 { "Linux" } else { "Windows" },
                "managed": i % 3 == 0,
            }
        }));
    }

    // Generate resources
    let mut resources = Vec::new();
    let classifications = ["public", "internal", "confidential", "secret"];

    for i in 0..(scale * 2) {
        let doc_id = format!("doc_{}", i);
        let classification = classifications[i % classifications.len()];

        resources.push(json!({
            "id": doc_id.clone(),
            "type": "Resource",
            "attributes": {
                "id": doc_id,
                "classification": classification,
                "is_archived": i % 20 == 0,
                "owner": format!("user_{}", i % scale),
                "sensitive": classification == "confidential" || classification == "secret",
            }
        }));
    }

    let gen_time = gen_start.elapsed();
    println!("   ✓ Generated {} user identities", user_identities.len());
    println!("   ✓ Generated {} user attributes", user_attributes.len());
    println!("   ✓ Generated {} device trust scores", device_trust.len());
    println!("   ✓ Generated {} resources", resources.len());
    println!("   Generation time: {:?}\n", gen_time);

    // Save to files
    let identities_file = format!("join-identities-{}.json", scale);
    let attributes_file = format!("join-attributes-{}.json", scale);
    let devices_file = format!("join-devices-{}.json", scale);
    let resources_file = format!("join-resources-{}.json", scale);

    fs::write(
        &identities_file,
        serde_json::to_string(&json!({"entities": user_identities}))?,
    )?;
    fs::write(
        &attributes_file,
        serde_json::to_string(&json!({"entities": user_attributes}))?,
    )?;
    fs::write(
        &devices_file,
        serde_json::to_string(&json!({"entities": device_trust}))?,
    )?;
    fs::write(
        &resources_file,
        serde_json::to_string(&json!({"entities": resources}))?,
    )?;

    println!("   ✓ Saved test data to files\n");

    // ============================================================
    // PHASE 2: Configure Multi-Source Join
    // ============================================================
    println!("🔧 PHASE 2: Configuring 2-way join (identity + attributes)...\n");

    let config = JoinConfig {
        primary: EntitySource {
            file_path: identities_file.clone(),
            entity_type: "UserIdentity".to_string(),
        },
        secondary: HashMap::from([(
            "UserAttributes".to_string(),
            SecondarySource {
                source: EntitySource {
                    file_path: attributes_file.clone(),
                    entity_type: "UserAttributes".to_string(),
                },
                join_key: JoinKey {
                    primary_field: "attributes.id".to_string(),
                    secondary_field: "attributes.id".to_string(),
                },
            },
        )]),
    };

    println!("   Primary: UserIdentity ({} records)", scale);
    println!("   Secondary:");
    println!("      UserAttributes (join on: id)");
    println!();

    // ============================================================
    // PHASE 3: Execute Join and Load
    // ============================================================
    println!("🔗 PHASE 3: Executing join with JoinEngine...\n");

    let join_start = Instant::now();

    let store = DataStore::new();
    let loader = policy_engine::DataLoader::new(store.clone());
    let engine = JoinEngine::new(loader);

    let join_result = engine.join_and_load(config)?;

    let join_time = join_result.join_duration;
    println!("   ✓ Join completed in {:?}", join_time);
    println!("   Primary entities: {}", join_result.primary_count);

    for (entity_type, count) in &join_result.join_counts {
        println!("   Joined with {}: {}", entity_type, count);
    }

    for (entity_type, count) in &join_result.missing_counts {
        if *count > 0 {
            println!("   ⚠️  Missing {} data: {} records", entity_type, count);
        }
    }
    println!();

    // Load devices separately (not joined with users)
    println!("📥 Loading devices separately...\n");
    let devices_content = fs::read_to_string(&devices_file)?;
    let devices_data: serde_json::Value = serde_json::from_str(&devices_content)?;
    let devices = devices_data["entities"].as_array().unwrap().clone();

    let loader2 = policy_engine::DataLoader::new(store.clone());
    let device_stats = loader2.load_json_values(devices)?;
    println!("   ✓ Loaded {} devices\n", device_stats.total);

    // Load resources
    println!("📥 Loading resources...\n");
    let resources_content = fs::read_to_string(&resources_file)?;
    let resources_data: serde_json::Value = serde_json::from_str(&resources_content)?;
    let resources = resources_data["entities"].as_array().unwrap().clone();

    let loader3 = policy_engine::DataLoader::new(store.clone());
    let resource_stats = loader3.load_json_values(resources)?;
    println!("   ✓ Loaded {} resources\n", resource_stats.total);

    let store = Arc::new(store);

    // Verify entity types
    println!("🔍 PHASE 4: Entity type validation...\n");
    let entity_stats = store.get_entity_type_stats();
    let mut total_entities = 0;
    for (entity_type, count) in entity_stats.iter() {
        println!("   {}: {}", entity_type, count);
        total_entities += count;
    }
    println!("   Total: {} entities\n", total_entities);

    let total_load_time = join_start.elapsed();
    println!("✅ Total load time: {:?}\n", total_load_time);

    // ============================================================
    // PHASE 5: Create Join-Based Policy
    // ============================================================
    println!("📜 PHASE 5: Creating join-based policy...\n");

    let policy_content = r#"
// Multi-Source Join-Based Policy
// Requires attributes from multiple joined sources

policy join_based_access {
    version: "1.0.0",
    description: "Access control using joined user data",

    default: deny,

    // Rule 1: High clearance active users
    // Uses: role (SOURCE 1) + clearance + is_active (SOURCE 2)
    rule high_clearance_access {
        allow if {
            user.role != "guest" &&
            user.high_clearance == true &&
            user.is_active == true &&
            resource.is_archived != true
        }
    }

    // Rule 2: Admin override
    // Uses: role (SOURCE 1)
    rule admin_override {
        allow if {
            user.role == "admin" &&
            resource.is_archived != true
        }
    }

    // Rule 3: Certified analysts for confidential data
    // Uses: role (SOURCE 1) + certified (SOURCE 2)
    rule analyst_certified_access {
        allow if {
            user.role == "analyst" &&
            user.certified == true &&
            user.is_active == true &&
            (resource.classification == "internal" ||
             resource.classification == "confidential")
        }
    }

    // Rule 4: Team access to public resources
    // Uses: is_active (SOURCE 2)
    rule public_access {
        allow if {
            user.is_active == true &&
            resource.classification == "public"
        }
    }
}
"#;

    let policy_file = format!("join-policy-{}.reap", scale);
    fs::write(&policy_file, policy_content)?;

    println!("   ✓ Created policy: {}", policy_file);
    println!("   Policy Rules:");
    println!("      1. High clearance active users");
    println!("      2. Admin override");
    println!("      3. Certified analysts (confidential)");
    println!("      4. Public access for active users");
    println!();

    // ============================================================
    // PHASE 6: Policy Evaluation
    // ============================================================
    println!("🎯 PHASE 6: Policy evaluation...\n");

    let policy = ReaperPolicy::from_file(&policy_file)?;
    let evaluator = policy.build(store.clone())?;
    println!("   ✓ Policy compiled successfully\n");

    // Run evaluations
    let num_tests = if scale >= 10_000 { 10_000 } else { 1_000 };
    println!("Running {} policy evaluations...\n", num_tests);

    let mut allowed = 0;
    let mut denied = 0;
    let mut error_count = 0;

    let eval_start = Instant::now();
    let mut latencies = Vec::with_capacity(num_tests);

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
            print!("   Progress: {}/{}\r", i + 1, num_tests);
        }
    }
    println!("   Progress: {}/{}     ", num_tests, num_tests);

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
    // PHASE 7: Performance Analysis
    // ============================================================
    println!("\n📊 PHASE 7: Performance analysis...\n");

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

    // Memory estimation
    let files_size = fs::metadata(&identities_file)?.len()
        + fs::metadata(&attributes_file)?.len()
        + fs::metadata(&devices_file)?.len()
        + fs::metadata(&resources_file)?.len();
    let estimated_memory_mb = (files_size as f64 * 0.8) / 1_048_576.0;

    println!("\nMemory Analysis:");
    println!("   Files on disk:    {} MB", files_size / 1_048_576);
    println!("   Estimated in-mem: {:.2} MB", estimated_memory_mb);

    // ============================================================
    // SUMMARY
    // ============================================================
    println!("\n╔═══════════════════════════════════════════════════════════╗");
    println!("║  SUMMARY                                                  ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    println!(
        "Scale:               {} users, {} resources",
        scale,
        scale * 2
    );
    println!("Data Sources:        3 (identity + attributes + devices)");
    println!("Join Strategy:       JoinEngine (2-way user join)");
    println!("Policy Rules:        4 multi-source rules");
    println!();
    println!("Performance:");
    println!("   Generation:       {:?}", gen_time);
    println!("   Join:             {:?}", join_time);
    println!("   Total Load:       {:?}", total_load_time);
    println!("   Evaluation:       {:?}", eval_time);
    println!("   Throughput:       {:.0} ops/sec", throughput);
    println!("   Mean Latency:     {:.2}ns", mean as f64);
    println!("   P99 Latency:      {}ns", p99);
    println!();
    println!("Memory:");
    println!("   Estimated:        {:.2} MB", estimated_memory_mb);
    println!();

    println!("✅ Join-based policy evaluation successful!\n");

    // Cleanup
    println!("🧹 Cleaning up test files...");
    fs::remove_file(&identities_file)?;
    fs::remove_file(&attributes_file)?;
    fs::remove_file(&devices_file)?;
    fs::remove_file(&resources_file)?;
    fs::remove_file(&policy_file)?;
    println!("   ✓ Cleanup complete\n");

    println!("💡 To test larger scales:");
    println!("   cargo run --release --example test_join_policy_scale 1000");
    println!("   cargo run --release --example test_join_policy_scale 10000");
    println!("   cargo run --release --example test_join_policy_scale 100000\n");

    Ok(())
}
