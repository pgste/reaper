/// Performance test for wildcard iteration [_]
///
/// Tests the performance of Rego-style wildcard iteration with:
/// - List iteration (O(n))
/// - Set iteration (O(1))
/// - Various collection sizes
///
/// Target: All operations should remain < 1µs for typical policy scenarios
use policy_engine::{reaper_dsl::*, EntityBuilder, PolicyAction, PolicyEvaluator, PolicyRequest};
use rustc_hash::FxHashSet;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    println!("=== Wildcard Iteration [_] Performance Test ===\n");

    // Test 1: Wildcard on small list (5 elements)
    test_wildcard_small_list();

    // Test 2: Wildcard on medium list (50 elements)
    test_wildcard_medium_list();

    // Test 3: Wildcard on large list (500 elements)
    test_wildcard_large_list();

    // Test 4: Wildcard on HashSet (100 elements, O(1) lookup)
    test_wildcard_set();

    // Test 5: Wildcard with first element match (best case)
    test_wildcard_first_match();

    // Test 6: Wildcard with last element match (worst case)
    test_wildcard_last_match();

    println!("\n=== All Performance Tests Complete ===");
}

fn test_wildcard_small_list() {
    println!("Test 1: Wildcard on Small List (5 elements)");
    println!("----------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    // Create user with 5 roles
    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let roles_key = interner.intern("roles");

    let roles = vec![
        policy_engine::AttributeValue::String(interner.intern("viewer")),
        policy_engine::AttributeValue::String(interner.intern("developer")),
        policy_engine::AttributeValue::String(interner.intern("admin")), // Match this
        policy_engine::AttributeValue::String(interner.intern("manager")),
        policy_engine::AttributeValue::String(interner.intern("owner")),
    ];

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::List(roles))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    // Create rule: user.roles[_] == "admin"
    let rules = vec![Rule {
        name: "wildcard_role_check".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Warm up
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (1µs)");
    if avg_ns < 1000 {
        println!("Status: ✅ PASS\n");
    } else {
        println!("Status: ❌ FAIL\n");
    }
}

fn test_wildcard_medium_list() {
    println!("Test 2: Wildcard on Medium List (50 elements)");
    println!("----------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    // Create user with 50 roles
    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let roles_key = interner.intern("roles");

    let mut roles = Vec::new();
    for i in 0..50 {
        roles.push(policy_engine::AttributeValue::String(
            interner.intern(&format!("role_{}", i)),
        ));
    }
    // Insert target role in the middle (position 25)
    roles[25] = policy_engine::AttributeValue::String(interner.intern("admin"));

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::List(roles))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    let rules = vec![Rule {
        name: "wildcard_role_check".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Warm up
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 2000 ns (2µs) for 50 elements");
    if avg_ns < 2000 {
        println!("Status: ✅ PASS\n");
    } else {
        println!("Status: ❌ FAIL\n");
    }
}

fn test_wildcard_large_list() {
    println!("Test 3: Wildcard on Large List (500 elements)");
    println!("----------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let roles_key = interner.intern("roles");

    let mut roles = Vec::new();
    for i in 0..500 {
        roles.push(policy_engine::AttributeValue::String(
            interner.intern(&format!("role_{}", i)),
        ));
    }
    // Insert target role in the middle (position 250)
    roles[250] = policy_engine::AttributeValue::String(interner.intern("admin"));

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::List(roles))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    let rules = vec![Rule {
        name: "wildcard_role_check".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Warm up
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 10000 ns (10µs) for 500 elements");
    if avg_ns < 10000 {
        println!("Status: ✅ PASS\n");
    } else {
        println!("Status: ❌ FAIL\n");
    }
}

fn test_wildcard_set() {
    println!("Test 4: Wildcard on HashSet (100 elements, O(1) lookup)");
    println!("--------------------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let roles_key = interner.intern("allowed_roles");

    let mut roles_set = FxHashSet::default();
    for i in 0..100 {
        roles_set.insert(policy_engine::AttributeValue::String(
            interner.intern(&format!("role_{}", i)),
        ));
    }
    roles_set.insert(policy_engine::AttributeValue::String(
        interner.intern("admin"),
    ));

    let doc = EntityBuilder::new(doc_id, doc_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::Set(roles_set))
        .build();

    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let alice = EntityBuilder::new(alice_id, user_type).build();

    store.insert(doc);
    store.insert(alice);

    let rules = vec![Rule {
        name: "wildcard_set_check".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::Resource,
            attribute: "allowed_roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Warm up
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (1µs) - O(1) hash lookup");
    if avg_ns < 1000 {
        println!("Status: ✅ PASS (O(1) confirmed!)\n");
    } else {
        println!("Status: ⚠️  Slower than expected\n");
    }
}

fn test_wildcard_first_match() {
    println!("Test 5: Wildcard First Element Match (Best Case)");
    println!("--------------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let roles_key = interner.intern("roles");

    let roles = vec![
        policy_engine::AttributeValue::String(interner.intern("admin")), // First element!
        policy_engine::AttributeValue::String(interner.intern("developer")),
        policy_engine::AttributeValue::String(interner.intern("manager")),
    ];

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::List(roles))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    let rules = vec![Rule {
        name: "wildcard_first_match".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };

    // Warm up
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 500 ns (best case)");
    if avg_ns < 500 {
        println!("Status: ✅ PASS (excellent!)\n");
    } else if avg_ns < 1000 {
        println!("Status: ✅ PASS\n");
    } else {
        println!("Status: ❌ FAIL\n");
    }
}

fn test_wildcard_last_match() {
    println!("Test 6: Wildcard Last Element Match (Worst Case)");
    println!("--------------------------------------------------");

    let store = Arc::new(policy_engine::DataStore::new());
    let interner = store.interner();

    let alice_id = interner.intern("user-alice");
    let user_type = interner.intern("user");
    let roles_key = interner.intern("roles");

    let roles = vec![
        policy_engine::AttributeValue::String(interner.intern("viewer")),
        policy_engine::AttributeValue::String(interner.intern("developer")),
        policy_engine::AttributeValue::String(interner.intern("manager")),
        policy_engine::AttributeValue::String(interner.intern("admin")), // Last element!
    ];

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_attribute(roles_key, policy_engine::AttributeValue::List(roles))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    let rules = vec![Rule {
        name: "wildcard_last_match".to_string(),
        condition: Condition::IndexedEquals {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: IndexExpr::Wildcard,
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user-alice".to_string());
    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context: context.clone(),

        ..Default::default()
    };

    // Warm up
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (worst case for small list)");
    if avg_ns < 1000 {
        println!("Status: ✅ PASS\n");
    } else {
        println!("Status: ❌ FAIL\n");
    }
}
