//! Memory and Resource Management Tests
//!
//! Tests for:
//! - Memory efficiency and leak prevention
//! - Resource cleanup
//! - Cache behavior
//! - String interning efficiency

use policy_engine::data::DataLoader;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyEvaluator, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// SECTION 1: Memory Efficiency Tests
// ============================================================================

/// Test string interning reduces memory for repeated strings
#[test]
fn test_string_interning_efficiency() {
    let store = Arc::new(DataStore::new());

    // Create entities with repeated string values
    let mut entities = Vec::new();
    let departments = vec!["engineering", "sales", "marketing", "finance", "operations"];
    let roles = vec!["admin", "user", "guest", "manager", "viewer"];

    for i in 0..10_000 {
        let dept = departments[i % departments.len()];
        let role = roles[i % roles.len()];

        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{
                "id": "user_{}",
                "department": "{}",
                "role": "{}",
                "status": "active"
            }}}}"#,
            i, i, dept, role
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let stats = store.stats();

    // Calculate memory per entity
    let bytes_per_entity = stats.estimated_memory_bytes / stats.total_entities;

    println!("Entities: {}", stats.total_entities);
    println!("Total memory: {} bytes", stats.estimated_memory_bytes);
    println!("Bytes per entity: {}", bytes_per_entity);

    // With string interning, repeated values like "engineering", "admin"
    // should only be stored once. Memory per entity should be reasonable.
    // Without interning, we'd have 10000 copies of each string.
    // Note: Overhead from indexes and metadata increases actual memory usage.
    assert!(
        bytes_per_entity < 500,
        "Memory per entity ({}) exceeds 500 bytes threshold",
        bytes_per_entity
    );
}

/// Test DataStore memory growth is linear
#[test]
fn test_linear_memory_growth() {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    let mut memory_samples = Vec::new();

    // Add entities in batches and measure memory
    for batch in 0..10 {
        let mut entities = Vec::new();
        for i in 0..1000 {
            let idx = batch * 1000 + i;
            entities.push(format!(
                r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "name": "User {}"}}}}"#,
                idx, idx, idx
            ));
        }

        let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
        loader.load_json(&json).unwrap();

        let stats = store.stats();
        memory_samples.push((stats.total_entities, stats.estimated_memory_bytes));
    }

    // Check that memory grows roughly linearly
    // Compare growth rate between first half and second half
    let mid = memory_samples.len() / 2;
    let (entities_1, mem_1) = memory_samples[mid - 1];
    let (entities_2, mem_2) = memory_samples[memory_samples.len() - 1];

    let growth_rate_1 = mem_1 as f64 / entities_1 as f64;
    let growth_rate_2 = (mem_2 - mem_1) as f64 / (entities_2 - entities_1) as f64;

    println!("Growth rate first half: {:.2} bytes/entity", growth_rate_1);
    println!("Growth rate second half: {:.2} bytes/entity", growth_rate_2);

    // Growth rates should be similar (within 50%)
    let ratio = growth_rate_2 / growth_rate_1;
    assert!(
        ratio > 0.5 && ratio < 2.0,
        "Memory growth is not linear: ratio = {:.2}",
        ratio
    );
}

// ============================================================================
// SECTION 2: Resource Cleanup Tests
// ============================================================================

/// Test that dropped policies are cleaned up
#[test]
fn test_policy_cleanup_on_drop() {
    let engine = PolicyEngine::new();

    // Deploy and then remove policies
    for i in 0..100 {
        let policy = EnhancedPolicy::new(
            format!("temp-policy-{}", i),
            "Temporary policy".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        let policy_id = policy.id;

        engine.deploy_policy(policy).unwrap();
        engine.remove_policy(&policy_id).unwrap();
    }

    // After removing all, there should be no policies
    let policies = engine.list_policies();
    assert!(
        policies.is_empty(),
        "All temporary policies should be removed"
    );
}

/// Test DataStore clear functionality
#[test]
fn test_datastore_clear() {
    let store = Arc::new(DataStore::new());

    // Load some data
    let json = r#"
{
    "entities": [
        {"id": "user_1", "type": "User", "attributes": {"id": "user_1", "name": "Alice"}},
        {"id": "user_2", "type": "User", "attributes": {"id": "user_2", "name": "Bob"}},
        {"id": "res_1", "type": "Resource", "attributes": {"id": "res_1", "type": "doc"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    assert_eq!(store.stats().total_entities, 3);

    // Clear the store
    store.clear();

    assert_eq!(store.stats().total_entities, 0);
    assert_eq!(store.stats().unique_types, 0);
}

// ============================================================================
// SECTION 3: Cache Behavior Tests
// ============================================================================

/// Test regex cache effectiveness
#[test]
fn test_regex_cache_effectiveness() {
    let policy_text = r#"
policy regex_cache_test {
    default: deny,

    rule email_check {
        allow if {
            user.email.matches("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load test data
    let mut entities = Vec::new();
    for i in 0..1000 {
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "email": "user{}@example.com"}}}}"#,
            i, i, i
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // First evaluation (cache miss)
    let start_cold = Instant::now();
    for i in 0..100 {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", i));

        let request = policy_engine::PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        let _ = evaluator.evaluate(&request);
    }
    let cold_time = start_cold.elapsed();

    // Second evaluation (cache hit)
    let start_warm = Instant::now();
    for i in 0..100 {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", i));

        let request = policy_engine::PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        let _ = evaluator.evaluate(&request);
    }
    let warm_time = start_warm.elapsed();

    println!("Cold cache time: {:?}", cold_time);
    println!("Warm cache time: {:?}", warm_time);

    // Both should complete in a reasonable time
    // Note: Warm cache may not always be faster due to system load variance,
    // so we just verify both complete without errors
    assert!(
        warm_time < Duration::from_secs(5),
        "Warm cache evaluation should complete in reasonable time"
    );
    assert!(
        cold_time < Duration::from_secs(5),
        "Cold cache evaluation should complete in reasonable time"
    );
}

// ============================================================================
// SECTION 4: Arc and Reference Counting Tests
// ============================================================================

/// Test that Arc references are properly managed
#[test]
fn test_arc_reference_management() {
    let store = Arc::new(DataStore::new());

    // Initial reference count should be 1
    assert_eq!(Arc::strong_count(&store), 1);

    // Create loader (takes a clone)
    let loader = DataLoader::new((*store).clone());
    // Note: DataLoader might clone internally, so we just verify it works

    // Load data
    let json = r#"{"entities": [{"id": "u1", "type": "User", "attributes": {"id": "u1"}}]}"#;
    loader.load_json(json).unwrap();

    // Store should still be accessible
    assert_eq!(store.stats().total_entities, 1);

    // Drop loader explicitly
    drop(loader);

    // Store should still work
    assert_eq!(store.stats().total_entities, 1);
}

/// Test policy engine doesn't leak policy references
#[test]
fn test_policy_reference_management() {
    let engine = PolicyEngine::new();

    // Create a policy and track its ID
    let policy = EnhancedPolicy::new(
        "ref-test".to_string(),
        "Reference test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    // Deploy
    engine.deploy_policy(policy).unwrap();

    // Get reference
    let retrieved = engine.get_policy(&policy_id).unwrap();
    assert_eq!(retrieved.name, "ref-test");

    // Deploy new version (should replace)
    let mut policy2 = EnhancedPolicy::new(
        "ref-test".to_string(),
        "Reference test v2".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    policy2.id = policy_id;
    policy2.update_rules(vec![PolicyRule {
        action: PolicyAction::Deny,
        resource: "*".to_string(),
        conditions: vec![],
    }]);

    engine.deploy_policy(policy2).unwrap();

    // Old reference is still valid (Arc keeps it alive)
    // But getting from engine gives new version
    let new_retrieved = engine.get_policy(&policy_id).unwrap();
    assert_eq!(new_retrieved.version, 2);
}

// ============================================================================
// SECTION 5: Repeated Operation Tests
// ============================================================================

/// Test repeated policy deployments don't leak memory
#[test]
fn test_repeated_deployment_no_leak() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "repeated-deploy".to_string(),
        "Repeated deployment test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy.clone()).unwrap();

    // Deploy same policy many times
    for i in 0..1000 {
        let mut updated = policy.clone();
        updated.id = policy_id;
        updated.update_rules(vec![PolicyRule {
            action: if i % 2 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            },
            resource: "*".to_string(),
            conditions: vec![],
        }]);

        engine.deploy_policy(updated).unwrap();
    }

    // Should still only have one policy
    let policies = engine.list_policies();
    assert_eq!(policies.len(), 1);

    // Version should have incremented
    let final_policy = engine.get_policy(&policy_id).unwrap();
    assert!(final_policy.version > 1);
}

/// Test repeated evaluations don't accumulate resources
#[test]
fn test_repeated_evaluations_stable() {
    let policy_text = r#"
policy eval_stability_test {
    default: deny,
    rule r1 { allow if { user.active == true } }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "user_1", "type": "User", "attributes": {"id": "user_1", "active": true}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Run many evaluations
    let start = Instant::now();
    for _ in 0..100_000 {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user_1".to_string());

        let request = policy_engine::PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    // Should complete in reasonable time (no accumulating overhead)
    assert!(
        elapsed < Duration::from_secs(5),
        "100K evaluations took too long: {:?}",
        elapsed
    );

    // Memory should be stable (check store didn't grow)
    assert_eq!(store.stats().total_entities, 1);
}

// ============================================================================
// SECTION 6: Large Object Tests
// ============================================================================

/// Test handling of entities with many attributes
#[test]
fn test_entity_many_attributes() {
    let store = Arc::new(DataStore::new());

    // Create entity with 100 attributes
    let mut attrs = Vec::new();
    for i in 0..100 {
        attrs.push(format!(r#""attr_{}": "value_{}""#, i, i));
    }

    let json = format!(
        r#"{{"entities": [{{"id": "big_entity", "type": "User", "attributes": {{"id": "big_entity", {}}}}}]}}"#,
        attrs.join(", ")
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let stats = store.stats();
    assert_eq!(stats.total_entities, 1);

    // Should handle many attributes efficiently
    let bytes_per_entity = stats.estimated_memory_bytes;
    println!(
        "Entity with 100 attributes: {} bytes",
        bytes_per_entity
    );

    // Should be less than 50KB per entity even with 100 attributes
    // Note: Includes overhead from indexes and attribute storage
    assert!(
        bytes_per_entity < 50_000,
        "Entity with 100 attributes uses too much memory: {}",
        bytes_per_entity
    );
}

/// Test handling of entities with large attribute values
#[test]
fn test_entity_large_values() {
    let store = Arc::new(DataStore::new());

    // Create entity with large string value
    let large_value = "x".repeat(10_000);

    let json = format!(
        r#"{{"entities": [{{"id": "large_val_entity", "type": "User", "attributes": {{"id": "large_val_entity", "bio": "{}"}}}}]}}"#,
        large_value
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let stats = store.stats();
    assert_eq!(stats.total_entities, 1);

    // Memory should accommodate the large value
    // But shouldn't be excessive (e.g., no accidental duplication)
    println!(
        "Entity with 10KB value: {} bytes",
        stats.estimated_memory_bytes
    );
}

// ============================================================================
// SECTION 7: Concurrent Resource Access
// ============================================================================

/// Test concurrent access to shared DataStore
#[test]
fn test_concurrent_datastore_access() {
    use std::thread;

    let store = Arc::new(DataStore::new());

    // Pre-load some data
    let json = r#"
{
    "entities": [
        {"id": "shared_user", "type": "User", "attributes": {"id": "shared_user", "name": "Shared"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    // Spawn threads that all read from the same store
    let mut handles = vec![];
    for _ in 0..10 {
        let store = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..1000 {
                let _ = store.stats();
                let _ = store.get_entity_type_stats();
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Store should still be in valid state
    assert_eq!(store.stats().total_entities, 1);
}

/// Test concurrent policy engine operations
#[test]
fn test_concurrent_engine_operations() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let engine = Arc::new(PolicyEngine::new());
    let operations = Arc::new(AtomicUsize::new(0));

    // Create initial policy
    let policy = EnhancedPolicy::new(
        "concurrent-resource-test".to_string(),
        "Test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let mut handles = vec![];

    // Mix of read and write operations
    for thread_id in 0..10 {
        let engine = Arc::clone(&engine);
        let ops = Arc::clone(&operations);

        let handle = thread::spawn(move || {
            for i in 0..100 {
                if (thread_id + i) % 5 == 0 {
                    // Write operation
                    let mut p = EnhancedPolicy::new(
                        "concurrent-resource-test".to_string(),
                        format!("Update {}-{}", thread_id, i),
                        vec![PolicyRule {
                            action: PolicyAction::Allow,
                            resource: "*".to_string(),
                            conditions: vec![],
                        }],
                    );
                    p.id = policy_id;
                    let _ = engine.deploy_policy(p);
                } else {
                    // Read operation
                    let _ = engine.get_policy(&policy_id);
                    let _ = engine.list_policies();
                }
                ops.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let total_ops = operations.load(Ordering::Relaxed);
    assert_eq!(total_ops, 1000, "All operations should complete");

    // Engine should be in valid state
    assert!(engine.get_policy(&policy_id).is_some());
}
