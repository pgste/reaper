/// Indexed Query Scale Test - Performance Comparison
///
/// This example demonstrates the performance advantage of attribute indexing:
/// 1. Generate test data at scale (100 to 100k entities)
/// 2. Compare query performance: Indexed vs Full Scan
/// 3. Measure O(m) indexed lookup vs O(n) full scan
/// 4. Show index creation overhead and query speedup
///
/// Use Cases Demonstrated:
/// - Equality queries (role == "admin")
/// - Range queries (trustscore >= 75)
/// - Multiple indexes on same entity type
/// - Index vs no-index performance comparison
use policy_engine::data::{AttributeValue, DataStore, IndexManager};
use policy_engine::EntityBuilder;
use std::env;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse scale from command line
    let args: Vec<String> = env::args().collect();
    let scale = if args.len() > 1 {
        args[1].parse::<usize>().unwrap_or(1000)
    } else {
        1000
    };

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  Attribute Indexing - Performance Scale Test             ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");
    println!("Testing scale: {} entities\n", scale);

    // ============================================================
    // PHASE 1: Generate Test Data
    // ============================================================
    println!("📝 PHASE 1: Generating test data...\n");

    let gen_start = Instant::now();

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Entity types
    let user_type = interner.intern("User");
    let device_type = interner.intern("Device");

    // Attribute keys
    let role_key = interner.intern("role");
    let dept_key = interner.intern("department");
    let clearance_key = interner.intern("clearance");
    let active_key = interner.intern("active");
    let trustscore_key = interner.intern("trustscore");
    let os_key = interner.intern("os");
    let managed_key = interner.intern("managed");

    // Attribute value constants
    let departments = ["engineering", "security", "hr", "finance"];
    let os_types = ["Linux", "Windows", "macOS"];

    // Generate users
    for i in 0..scale {
        let user_id = interner.intern(&format!("user_{}", i));
        let role = if i < scale / 10 {
            "admin"
        } else if i < scale * 3 / 10 {
            "analyst"
        } else {
            "viewer"
        };
        let role_id = interner.intern(role);
        let dept_id = interner.intern(departments[i % departments.len()]);
        let clearance = ((i % 5) + 1) as i64; // 1-5
        let active = i % 10 != 0; // 90% active

        store.insert(
            EntityBuilder::new(user_id, user_type)
                .with_string(role_key, role_id)
                .with_string(dept_key, dept_id)
                .with_int(clearance_key, clearance)
                .with_bool(active_key, active)
                .build(),
        );
    }

    // Generate devices
    for i in 0..(scale / 2) {
        let device_id = interner.intern(&format!("device_{}", i));
        let trustscore = 50 + (i as i64 % 50); // 50-99
        let os_id = interner.intern(os_types[i % os_types.len()]);
        let managed = i % 3 == 0;

        store.insert(
            EntityBuilder::new(device_id, device_type)
                .with_int(trustscore_key, trustscore)
                .with_string(os_key, os_id)
                .with_bool(managed_key, managed)
                .build(),
        );
    }

    let gen_time = gen_start.elapsed();
    println!("   ✓ Generated {} users", scale);
    println!("   ✓ Generated {} devices", scale / 2);
    println!("   Generation time: {:?}\n", gen_time);

    // ============================================================
    // PHASE 2: Baseline - Full Scan Queries (No Index)
    // ============================================================
    println!("🔍 PHASE 2: Baseline performance (full scan, no index)...\n");

    // Query 1: Find all admins (equality)
    let admin_id = interner.intern("admin");
    let scan_start = Instant::now();
    let mut admin_count = 0;

    for entity in store.get_by_type(user_type) {
        if let Some(AttributeValue::String(role_id)) = entity.get_attribute(role_key) {
            if *role_id == admin_id {
                admin_count += 1;
            }
        }
    }

    let scan_time_1 = scan_start.elapsed();
    println!("   Query 1: role == 'admin'");
    println!("      Method: Full scan");
    println!("      Results: {} users", admin_count);
    println!("      Time: {:?}", scan_time_1);

    // Query 2: Find high trust devices (range)
    let scan_start = Instant::now();
    let mut high_trust_count = 0;

    for entity in store.get_by_type(device_type) {
        if let Some(AttributeValue::Int(score)) = entity.get_attribute(trustscore_key) {
            if *score >= 75 {
                high_trust_count += 1;
            }
        }
    }

    let scan_time_2 = scan_start.elapsed();
    println!("\n   Query 2: trustscore >= 75");
    println!("      Method: Full scan");
    println!("      Results: {} devices", high_trust_count);
    println!("      Time: {:?}", scan_time_2);

    // Query 3: Find active engineering users (multiple attributes)
    let eng_id = interner.intern("engineering");
    let scan_start = Instant::now();
    let mut eng_active_count = 0;

    for entity in store.get_by_type(user_type) {
        let has_dept = if let Some(AttributeValue::String(dept_id)) = entity.get_attribute(dept_key)
        {
            *dept_id == eng_id
        } else {
            false
        };

        let is_active = if let Some(AttributeValue::Bool(active)) = entity.get_attribute(active_key)
        {
            *active
        } else {
            false
        };

        if has_dept && is_active {
            eng_active_count += 1;
        }
    }

    let scan_time_3 = scan_start.elapsed();
    println!("\n   Query 3: department == 'engineering' AND active == true");
    println!("      Method: Full scan");
    println!("      Results: {} users", eng_active_count);
    println!("      Time: {:?}", scan_time_3);

    let total_scan_time = scan_time_1 + scan_time_2 + scan_time_3;
    println!("\n   Total scan time: {:?}\n", total_scan_time);

    // ============================================================
    // PHASE 3: Create Indexes
    // ============================================================
    println!("🔧 PHASE 3: Creating attribute indexes...\n");

    let index_manager = IndexManager::new(store.clone());

    // Create indexes
    let index_start = Instant::now();

    let role_stats = index_manager.create_index("User", "role")?;
    println!(
        "   ✓ Index: User.role ({} entities, {} unique values)",
        role_stats.entity_count, role_stats.unique_values
    );

    let dept_stats = index_manager.create_index("User", "department")?;
    println!(
        "   ✓ Index: User.department ({} entities, {} unique values)",
        dept_stats.entity_count, dept_stats.unique_values
    );

    let active_stats = index_manager.create_index("User", "active")?;
    println!(
        "   ✓ Index: User.active ({} entities, {} unique values)",
        active_stats.entity_count, active_stats.unique_values
    );

    let trust_stats = index_manager.create_index("Device", "trustscore")?;
    println!(
        "   ✓ Index: Device.trustscore ({} entities, {} unique values)",
        trust_stats.entity_count, trust_stats.unique_values
    );

    let index_time = index_start.elapsed();
    println!("\n   Index creation time: {:?}", index_time);
    println!("   Total indexes: {}\n", index_manager.index_count());

    // ============================================================
    // PHASE 4: Indexed Queries
    // ============================================================
    println!("⚡ PHASE 4: Indexed query performance...\n");

    // Query 1: Find all admins (indexed)
    let idx_start = Instant::now();
    let admins_indexed =
        index_manager.query_equals("User", "role", &AttributeValue::String(admin_id));
    let idx_time_1 = idx_start.elapsed();

    println!("   Query 1: role == 'admin'");
    println!("      Method: Indexed (equality)");
    println!("      Results: {} users", admins_indexed.len());
    println!("      Time: {:?}", idx_time_1);
    println!(
        "      Speedup: {:.2}x faster",
        scan_time_1.as_nanos() as f64 / idx_time_1.as_nanos() as f64
    );

    // Query 2: Find high trust devices (indexed)
    let idx_start = Instant::now();
    let high_trust_indexed = index_manager.query(
        "Device",
        "trustscore",
        |v| matches!(v, AttributeValue::Int(score) if *score >= 75),
    );
    let idx_time_2 = idx_start.elapsed();

    println!("\n   Query 2: trustscore >= 75");
    println!("      Method: Indexed (range)");
    println!("      Results: {} devices", high_trust_indexed.len());
    println!("      Time: {:?}", idx_time_2);
    println!(
        "      Speedup: {:.2}x faster",
        scan_time_2.as_nanos() as f64 / idx_time_2.as_nanos() as f64
    );

    // Query 3: Find active engineering users (indexed, intersection)
    let idx_start = Instant::now();

    // Query both indexes
    let eng_users =
        index_manager.query_equals("User", "department", &AttributeValue::String(eng_id));
    let active_users = index_manager.query_equals("User", "active", &AttributeValue::Bool(true));

    // Intersect results
    let eng_users_set: std::collections::HashSet<_> = eng_users.into_iter().collect();
    let active_eng: Vec<_> = active_users
        .into_iter()
        .filter(|id| eng_users_set.contains(id))
        .collect();

    let idx_time_3 = idx_start.elapsed();

    println!("\n   Query 3: department == 'engineering' AND active == true");
    println!("      Method: Indexed (intersection)");
    println!("      Results: {} users", active_eng.len());
    println!("      Time: {:?}", idx_time_3);
    println!(
        "      Speedup: {:.2}x faster",
        scan_time_3.as_nanos() as f64 / idx_time_3.as_nanos() as f64
    );

    let total_idx_time = idx_time_1 + idx_time_2 + idx_time_3;
    println!("\n   Total indexed time: {:?}", total_idx_time);
    println!(
        "   Overall speedup: {:.2}x faster\n",
        total_scan_time.as_nanos() as f64 / total_idx_time.as_nanos() as f64
    );

    // ============================================================
    // PHASE 5: Performance Analysis
    // ============================================================
    println!("📊 PHASE 5: Performance analysis...\n");

    println!("Index Overhead:");
    println!("   Creation time: {:?}", index_time);
    println!(
        "   Memory per index: ~{:.2} KB",
        estimate_index_memory_kb(&role_stats)
    );
    println!(
        "   Total memory (4 indexes): ~{:.2} KB\n",
        estimate_index_memory_kb(&role_stats) * 4.0
    );

    println!("Query Performance Comparison:");
    println!("┌────────────────────────────┬──────────────┬──────────────┬───────────┐");
    println!("│ Query                      │ Full Scan    │ Indexed      │ Speedup   │");
    println!("├────────────────────────────┼──────────────┼──────────────┼───────────┤");
    println!(
        "│ role == 'admin'            │ {:>10}ns │ {:>10}ns │ {:>7.2}x │",
        scan_time_1.as_nanos(),
        idx_time_1.as_nanos(),
        scan_time_1.as_nanos() as f64 / idx_time_1.as_nanos() as f64
    );
    println!(
        "│ trustscore >= 75           │ {:>10}ns │ {:>10}ns │ {:>7.2}x │",
        scan_time_2.as_nanos(),
        idx_time_2.as_nanos(),
        scan_time_2.as_nanos() as f64 / idx_time_2.as_nanos() as f64
    );
    println!(
        "│ dept && active             │ {:>10}ns │ {:>10}ns │ {:>7.2}x │",
        scan_time_3.as_nanos(),
        idx_time_3.as_nanos(),
        scan_time_3.as_nanos() as f64 / idx_time_3.as_nanos() as f64
    );
    println!("└────────────────────────────┴──────────────┴──────────────┴───────────┘");
    println!(
        "\nOverall: {:.2}x faster with indexes\n",
        total_scan_time.as_nanos() as f64 / total_idx_time.as_nanos() as f64
    );

    // ============================================================
    // SUMMARY
    // ============================================================
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║  SUMMARY                                                  ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    println!("Scale:               {} entities total", scale + scale / 2);
    println!(
        "Data Sources:        Users ({}), Devices ({})",
        scale,
        scale / 2
    );
    println!("Indexes Created:     4 attribute indexes");
    println!();
    println!("Performance:");
    println!("   Generation:       {:?}", gen_time);
    println!("   Index creation:   {:?}", index_time);
    println!("   Full scan:        {:?}", total_scan_time);
    println!("   Indexed queries:  {:?}", total_idx_time);
    println!(
        "   Speedup:          {:.2}x faster",
        total_scan_time.as_nanos() as f64 / total_idx_time.as_nanos() as f64
    );
    println!();
    println!("Complexity:");
    println!("   Full scan:        O(n) where n = total entities");
    println!("   Indexed query:    O(m) where m = matching entities");
    println!("   Index overhead:   O(n) creation, O(n * unique_values) memory");
    println!();

    println!("✅ Indexed query scale test successful!\n");

    println!("💡 To test larger scales:");
    println!("   cargo run --release --example test_indexed_query_scale 10000");
    println!("   cargo run --release --example test_indexed_query_scale 100000\n");

    Ok(())
}

/// Estimate memory usage for an index (rough approximation)
fn estimate_index_memory_kb(stats: &policy_engine::data::IndexStats) -> f64 {
    // Approximate: 8 bytes per entity ID + 16 bytes HashMap overhead per unique value
    let entity_mem = stats.entity_count * 8;
    let unique_mem = stats.unique_values * 16;
    (entity_mem + unique_mem) as f64 / 1024.0
}
