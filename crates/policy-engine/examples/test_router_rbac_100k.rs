//! Scale Test: Query Router + RBAC Views with 100K Records
//!
//! This test validates the performance of the intelligent query router
//! combined with RBAC views at scale (100K entities).
//!
//! Test Scenario (based on OPA-equivalent RBAC model):
//! - 10,000 users
//! - 100 roles
//! - 1,000 resources
//! - Each user has 1-5 roles (avg 3) = ~30,000 user-role bindings
//! - Each role has 10-50 permissions (avg 30) = ~3,000 role-permission mappings
//! - Total entities: ~33,000 source + ~90,000 flattened permissions = 123,000 total
//!
//! Performance Targets:
//! - View population: < 100ms for 100K entities
//! - Permission check (Tier 1): < 500ns
//! - Permission check (Tier 2): < 3µs
//! - Throughput: > 1M queries/second
//! - Memory: < 50MB for views

use policy_engine::data::{AttributeValue, DataStore, DataStoreRBACExt, Entity, QueryPattern};
use std::sync::Arc;
use std::time::Instant;

const NUM_USERS: usize = 10_000;
const NUM_ROLES: usize = 100;
const NUM_RESOURCES: usize = 1_000;
const ROLES_PER_USER_MIN: usize = 1;
const ROLES_PER_USER_MAX: usize = 5;
const PERMS_PER_ROLE_MIN: usize = 10;
const PERMS_PER_ROLE_MAX: usize = 50;

fn main() {
    println!("=== Query Router + RBAC Views Scale Test (100K) ===\n");

    // Phase 1: Setup and data generation
    println!("Phase 1: Data Generation");
    let start = Instant::now();
    let store = Arc::new(DataStore::new());
    let _interner = store.interner();
    println!("  ✓ Created data store ({:?})", start.elapsed());

    // Generate test data
    let start = Instant::now();
    let (user_role_count, role_perm_count) = generate_rbac_data(&store);
    let data_gen_time = start.elapsed();
    println!("  ✓ Generated RBAC data:");
    println!("    - {} users", NUM_USERS);
    println!("    - {} roles", NUM_ROLES);
    println!("    - {} resources", NUM_RESOURCES);
    println!("    - {} user-role bindings", user_role_count);
    println!("    - {} role-permission mappings", role_perm_count);
    println!(
        "    - Total source entities: {}",
        user_role_count + role_perm_count
    );
    println!("    - Generation time: {:?}", data_gen_time);

    // Phase 2: Build RBAC views
    println!("\nPhase 2: RBAC View Building");
    let start = Instant::now();
    store
        .setup_rbac_views()
        .expect("Failed to setup RBAC views");
    let view_build_time = start.elapsed();
    println!("  ✓ Built all RBAC views in {:?}", view_build_time);

    // Check view sizes
    if let Some(user_perm_view) = store.get_view("user_permission") {
        let view_size = user_perm_view.len();
        println!("    - user_permission view: {} entries", view_size);

        // Memory estimation
        let est_memory_mb = (view_size * 56) as f64 / 1_024_000.0;
        println!("    - Estimated memory: {:.2} MB", est_memory_mb);
    }

    // Phase 3: Performance testing
    println!("\nPhase 3: Query Performance Testing");

    // Test 3A: Permission checks (Tier 1 - with views)
    println!("\n  Test 3A: Permission Checks (Tier 1 - Pre-Computed View)");
    let test_cases = vec![
        ("user0", "resource0", "read"),
        ("user100", "resource50", "write"),
        ("user500", "resource100", "read"),
        ("user1000", "resource500", "write"),
        ("user5000", "resource250", "read"),
    ];

    let mut tier1_times = Vec::new();
    for (user, resource, action) in &test_cases {
        let start = Instant::now();
        let result = store
            .query(QueryPattern::PermissionCheck {
                user: user.to_string(),
                resource: resource.to_string(),
                action: action.to_string(),
            })
            .expect("Query failed");
        let elapsed = start.elapsed();

        tier1_times.push(elapsed.as_nanos());
        println!(
            "    - {} → {} [{}]: {:?} (Tier: {:?}, {} results)",
            user,
            resource,
            action,
            elapsed,
            result.tier,
            result.entities.len()
        );
    }

    let avg_tier1 = tier1_times.iter().sum::<u128>() / tier1_times.len() as u128;
    let min_tier1 = tier1_times.iter().min().unwrap();
    let max_tier1 = tier1_times.iter().max().unwrap();
    println!(
        "    Summary: avg={:.0}ns, min={}ns, max={}ns",
        avg_tier1, min_tier1, max_tier1
    );

    // Test 3B: Throughput test
    println!("\n  Test 3B: Throughput Test (10,000 permission checks)");
    let num_queries = 10_000;
    let start = Instant::now();

    for i in 0..num_queries {
        let user = format!("user{}", i % NUM_USERS);
        let resource = format!("resource{}", i % NUM_RESOURCES);
        let _result = store
            .query(QueryPattern::PermissionCheck {
                user,
                resource,
                action: "read".to_string(),
            })
            .expect("Query failed");
    }

    let total_time = start.elapsed();
    let qps = num_queries as f64 / total_time.as_secs_f64();
    let avg_latency = total_time.as_nanos() / num_queries as u128;

    println!("    - Total time: {:?}", total_time);
    println!("    - Queries per second: {:.0}", qps);
    println!(
        "    - Average latency: {}ns ({:.2}µs)",
        avg_latency,
        avg_latency as f64 / 1000.0
    );

    // Test 3C: User roles query
    println!("\n  Test 3C: User Roles Query");
    let start = Instant::now();
    let result = store
        .query(QueryPattern::UserRoles {
            user: "user100".to_string(),
        })
        .expect("Query failed");
    let elapsed = start.elapsed();
    println!(
        "    - user100 roles: {} results in {:?} (Tier: {:?})",
        result.entities.len(),
        elapsed,
        result.tier
    );

    // Test 3D: Role members query
    println!("\n  Test 3D: Role Members Query");
    let start = Instant::now();
    let result = store
        .query(QueryPattern::RoleMembers {
            role: "role10".to_string(),
        })
        .expect("Query failed");
    let elapsed = start.elapsed();
    println!(
        "    - role10 members: {} results in {:?} (Tier: {:?})",
        result.entities.len(),
        elapsed,
        result.tier
    );

    // Phase 4: Memory analysis
    println!("\nPhase 4: Memory Analysis");
    let total_entities = user_role_count + role_perm_count;
    println!("  Total source entities in store: {}", total_entities);

    if let Some(user_perm_view) = store.get_view("user_permission") {
        let view_entities = user_perm_view.len();
        let est_source_memory = (user_role_count + role_perm_count) * 120; // ~120 bytes per entity
        let est_view_memory = view_entities * 56; // ~56 bytes per view entity
        let total_memory_mb = (est_source_memory + est_view_memory) as f64 / 1_024_000.0;

        println!("  Memory estimation:");
        println!(
            "    - Source entities: {} (~{:.2} MB)",
            user_role_count + role_perm_count,
            est_source_memory as f64 / 1_024_000.0
        );
        println!(
            "    - View entities: {} (~{:.2} MB)",
            view_entities,
            est_view_memory as f64 / 1_024_000.0
        );
        println!("    - Total estimated: {:.2} MB", total_memory_mb);
    }

    // Phase 5: Performance summary
    println!("\n=== Performance Summary ===");
    println!("✓ Data Generation: {:?}", data_gen_time);
    println!("✓ View Building: {:?}", view_build_time);
    println!(
        "✓ Permission Check (avg): {:.0}ns ({:.2}µs)",
        avg_tier1,
        avg_tier1 as f64 / 1000.0
    );
    println!("✓ Throughput: {:.0} queries/second", qps);

    // Validate against targets
    println!("\n=== Target Validation ===");
    validate_target("View population", view_build_time.as_millis(), 100, "ms");
    validate_target("Permission check", avg_tier1, 500, "ns");
    validate_target("Throughput", qps as u128, 1_000_000, "qps");

    println!("\n=== Test Complete ===");
}

fn generate_rbac_data(store: &DataStore) -> (usize, usize) {
    let interner = store.interner();

    // Pre-intern common strings
    let binding_type = interner.intern("user_role_binding");
    let perm_type = interner.intern("role_permission");
    let user_key = interner.intern("user");
    let role_key = interner.intern("role");
    let resource_key = interner.intern("resource");
    let action_key = interner.intern("action");

    let actions = ["read", "write", "delete", "execute"];
    let action_ids: Vec<_> = actions.iter().map(|a| interner.intern(a)).collect();

    let mut user_role_count = 0;
    let mut role_perm_count = 0;

    // Generate user-role bindings
    for user_idx in 0..NUM_USERS {
        let user_id = interner.intern(&format!("user{}", user_idx));

        // Each user gets 1-5 roles
        let num_roles =
            ROLES_PER_USER_MIN + (user_idx % (ROLES_PER_USER_MAX - ROLES_PER_USER_MIN + 1));

        for role_offset in 0..num_roles {
            let role_idx = (user_idx + role_offset * 7) % NUM_ROLES; // Distribute roles
            let role_id = interner.intern(&format!("role{}", role_idx));

            let binding_id = interner.intern(&format!("binding_{}_{}", user_idx, role_idx));

            let mut attributes = std::collections::HashMap::new();
            attributes.insert(user_key, AttributeValue::String(user_id));
            attributes.insert(role_key, AttributeValue::String(role_id));

            store.insert(Entity::new(binding_id, binding_type, attributes));
            user_role_count += 1;
        }
    }

    // Generate role-permission mappings
    for role_idx in 0..NUM_ROLES {
        let role_id = interner.intern(&format!("role{}", role_idx));

        // Each role gets 10-50 permissions
        let num_perms =
            PERMS_PER_ROLE_MIN + (role_idx % (PERMS_PER_ROLE_MAX - PERMS_PER_ROLE_MIN + 1));

        for perm_offset in 0..num_perms {
            let resource_idx = (role_idx * 13 + perm_offset * 7) % NUM_RESOURCES;
            let resource_id = interner.intern(&format!("resource{}", resource_idx));
            let action_id = action_ids[perm_offset % actions.len()];

            let perm_id = interner.intern(&format!(
                "perm_{}_{}_{}",
                role_idx, resource_idx, perm_offset
            ));

            let mut attributes = std::collections::HashMap::new();
            attributes.insert(role_key, AttributeValue::String(role_id));
            attributes.insert(resource_key, AttributeValue::String(resource_id));
            attributes.insert(action_key, AttributeValue::String(action_id));

            store.insert(Entity::new(perm_id, perm_type, attributes));
            role_perm_count += 1;
        }
    }

    (user_role_count, role_perm_count)
}

fn validate_target(name: &str, actual: u128, target: u128, unit: &str) {
    let met = actual <= target;
    let status = if met { "✓" } else { "✗" };
    let comparison = if met { "≤" } else { ">" };

    println!(
        "{} {}: {} {} {} {}",
        status, name, actual, comparison, target, unit
    );
}
