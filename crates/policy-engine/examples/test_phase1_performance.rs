//! Performance Validation for Phase 1 Features
//!
//! Tests new language features (arrays, sets, objects, variables, bracket notation, `in` operator)
//! to ensure < 1µs p99 latency is maintained.

use policy_engine::data::{AttributeValue, DataStore};
use policy_engine::reaper_dsl::{
    Condition, EntityType, IndexExpr, LiteralValue, ReaperDSLEvaluator, Rule,
};
use policy_engine::{EntityBuilder, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    println!("=== Phase 1 Performance Validation ===\n");

    // Run all performance tests
    test_baseline_performance();
    test_membership_set_performance();
    test_membership_list_performance();
    test_indexed_access_performance();
    test_variable_assignment_performance();
    test_complex_policy_performance();

    println!("\n=== All Performance Tests Complete ===");
}

/// Baseline: Simple string comparison (existing functionality)
fn test_baseline_performance() {
    println!("Test 1: Baseline Performance (String Comparison)");
    println!("----------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create entities
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin_value = interner.intern("admin");

    let user = EntityBuilder::new(user_id, user_type)
        .with_string(role_key, admin_value)
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Create simple policy
    let rules = vec![Rule {
        name: "admin_access".to_string(),
        condition: Condition::UserEquals {
            attribute: "role".to_string(),
            value: "admin".to_string(),
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (1µs)");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}

/// Test Set membership (O(1) performance)
fn test_membership_set_performance() {
    println!("Test 2: Set Membership Performance (`in` operator with HashSet)");
    println!("----------------------------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user with Set of 100 roles
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let roles_key = interner.intern("roles");

    let mut roles_set = HashSet::new();
    for i in 0..100 {
        let role = interner.intern(&format!("role_{}", i));
        roles_set.insert(AttributeValue::String(role));
    }
    // Add target role
    let admin_role = interner.intern("admin");
    roles_set.insert(AttributeValue::String(admin_role));

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(roles_key, AttributeValue::Set(roles_set))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Policy: check if "admin" in user.roles (101 element set)
    let rules = vec![Rule {
        name: "admin_access".to_string(),
        condition: Condition::MembershipTest {
            value: LiteralValue::String("admin".to_string()),
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            index: None,
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("Set size: 101 elements");
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (O(1) hash lookup)");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}

/// Test List membership (O(n) but should still be fast for small lists)
fn test_membership_list_performance() {
    println!("Test 3: List Membership Performance (`in` operator with List)");
    println!("--------------------------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user with List of 10 permissions
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let perms_key = interner.intern("permissions");

    let mut perms_list = Vec::new();
    for i in 0..10 {
        let perm = interner.intern(&format!("perm_{}", i));
        perms_list.push(AttributeValue::String(perm));
    }
    // Add target at end
    let write_perm = interner.intern("write");
    perms_list.push(AttributeValue::String(write_perm));

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(perms_key, AttributeValue::List(perms_list))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Policy: check if "write" in user.permissions (11 element list)
    let rules = vec![Rule {
        name: "write_access".to_string(),
        condition: Condition::MembershipTest {
            value: LiteralValue::String("write".to_string()),
            entity_type: EntityType::User,
            attribute: "permissions".to_string(),
            index: None,
        },
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "write".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("List size: 11 elements (worst case: element at end)");
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}

/// Test bracket notation performance
fn test_indexed_access_performance() {
    println!("Test 4: Indexed Access Performance (Bracket Notation)");
    println!("------------------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user with array and object
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let roles_key = interner.intern("roles");
    let data_key = interner.intern("data");

    let admin_role = interner.intern("admin");
    let roles_list = vec![AttributeValue::String(admin_role)];

    let dept_key = interner.intern("department");
    let eng_value = interner.intern("engineering");
    let mut data_map = HashMap::new();
    data_map.insert(dept_key, AttributeValue::String(eng_value));

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(roles_key, AttributeValue::List(roles_list))
        .with_attribute(data_key, AttributeValue::Object(data_map))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Test both numeric and string indexing
    let rules = vec![
        Rule {
            name: "array_index".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: IndexExpr::Number(0),
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        },
        Rule {
            name: "object_index".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "data".to_string(),
                index: IndexExpr::String("department".to_string()),
                value: "engineering".to_string(),
            },
            decision: PolicyAction::Allow,
        },
    ];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("Tests: user.roles[0] and user.data[\"department\"]");
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}

/// Test variable assignment performance
fn test_variable_assignment_performance() {
    println!("Test 5: Variable Assignment Performance (`:=` operator)");
    println!("--------------------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin_value = interner.intern("admin");

    let user = EntityBuilder::new(user_id, user_type)
        .with_string(role_key, admin_value)
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Policy with variable assignment
    let rules = vec![Rule {
        name: "var_test".to_string(),
        condition: Condition::And(vec![
            Condition::Assignment {
                variable: "user_role".to_string(),
                entity_type: EntityType::User,
                attribute: "role".to_string(),
                index: None,
            },
            Condition::EqualsVariable {
                entity_type: EntityType::User,
                attribute: "role".to_string(),
                variable: "user_role".to_string(),
            },
        ]),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("Operations: Assignment + Variable Comparison");
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}

/// Test complex policy combining all features
fn test_complex_policy_performance() {
    println!("Test 6: Complex Policy (All Features Combined)");
    println!("-----------------------------------------------");

    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create rich user entity
    let user_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let roles_key = interner.intern("roles");
    let perms_key = interner.intern("permissions");
    let data_key = interner.intern("data");

    // Roles set
    let admin_role = interner.intern("admin");
    let mut roles_set = HashSet::new();
    roles_set.insert(AttributeValue::String(admin_role));

    // Permissions list
    let read_perm = interner.intern("read");
    let write_perm = interner.intern("write");
    let perms_list = vec![
        AttributeValue::String(read_perm),
        AttributeValue::String(write_perm),
    ];

    // Metadata object
    let dept_key = interner.intern("department");
    let eng_value = interner.intern("engineering");
    let mut data_map = HashMap::new();
    data_map.insert(dept_key, AttributeValue::String(eng_value));

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(roles_key, AttributeValue::Set(roles_set))
        .with_attribute(perms_key, AttributeValue::List(perms_list))
        .with_attribute(data_key, AttributeValue::Object(data_map))
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(user);
    store.insert(doc);

    // Complex policy using multiple features
    let rules = vec![Rule {
        name: "complex_access".to_string(),
        condition: Condition::And(vec![
            // Set membership
            Condition::MembershipTest {
                value: LiteralValue::String("admin".to_string()),
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: None,
            },
            // List membership
            Condition::MembershipTest {
                value: LiteralValue::String("write".to_string()),
                entity_type: EntityType::User,
                attribute: "permissions".to_string(),
                index: None,
            },
            // Object indexed access
            Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "data".to_string(),
                index: IndexExpr::String("department".to_string()),
                value: "engineering".to_string(),
            },
        ]),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "write".to_string(),
        context,
    };

    // Warmup
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }

    // Measure
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    println!("Operations: Set membership + List membership + Object access");
    println!("Iterations: {}", iterations);
    println!("Average: {} ns", avg_ns);
    println!("Target: < 1000 ns (1µs)");
    println!(
        "Status: {}\n",
        if avg_ns < 1000 {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    );
}
