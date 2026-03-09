//! Tests for the Reaper DSL evaluator.

use super::*;
use crate::EntityBuilder;
use std::collections::HashMap;

#[test]
fn test_reaper_dsl_simple_rule() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create test entities
    let alice_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin_value = interner.intern("admin");

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_string(role_key, admin_value)
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    // Create policy: admin can do anything (using V2 type)
    let rules = vec![Rule {
        name: "admin_access".to_string(),
        condition: Condition::AttributeCompare(AttributeComparison {
            entity_type: EntityType::User,
            attribute: "role".to_string(),
            op: NumericOp::Equal,
            target: CompareTarget::LiteralString("admin".to_string()),
        }),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Test evaluation
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_reaper_dsl_complex_rule() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user
    let bob_id = interner.intern("bob");
    let user_type = interner.intern("User");
    let dept_key = interner.intern("department");
    let eng_value = interner.intern("engineering");

    let bob = EntityBuilder::new(bob_id, user_type)
        .with_string(dept_key, eng_value)
        .build();

    // Create resource
    let doc_id = interner.intern("doc2");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type)
        .with_string(dept_key, eng_value)
        .build();

    store.insert(bob);
    store.insert(doc);

    // Create policy: same department access (using V2 type)
    let rules = vec![Rule {
        name: "department_access".to_string(),
        condition: Condition::CrossEntityCompare(CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "department".to_string(),
            op: NumericOp::Equal,
            right_entity: EntityType::Resource,
            right_attr: "department".to_string(),
        }),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "bob".to_string());

    let request = PolicyRequest {
        resource: "doc2".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}
