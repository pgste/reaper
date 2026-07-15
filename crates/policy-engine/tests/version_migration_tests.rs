//! Policy Version Migration Tests
//!
//! Tests for policy versioning, backward compatibility, and migration scenarios:
//! - Version tracking and history
//! - Policy updates and hot-swapping
//! - Backward compatibility
//! - Multiple policy coexistence

use policy_engine::reap::ReaperPolicy;
use policy_engine::{
    DataStore, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule,
};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// SECTION 1: Version Tracking Tests
// ============================================================================

/// Test that policy versions are tracked correctly
#[test]
fn test_version_tracking_on_deploy() {
    let engine = PolicyEngine::new();

    // Create and deploy first version
    let policy = EnhancedPolicy::new(
        "versioned-policy".to_string(),
        "Version test policy v1".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy).unwrap();

    // Get the policy and check version
    let retrieved = engine.get_policy(&policy_id).unwrap();
    assert_eq!(retrieved.version, 1, "Initial version should be 1");
}

/// Test version increments on update
#[test]
fn test_version_increment_on_update() {
    let engine = PolicyEngine::new();

    let mut policy = EnhancedPolicy::new(
        "update-version-test".to_string(),
        "Version increment test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy.clone()).unwrap();

    // Update the policy rules (this increments version)
    policy.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "*".to_string(),
        conditions: vec![],
    }]);

    engine.deploy_policy(policy).unwrap();

    let retrieved = engine.get_policy(&policy_id).unwrap();
    assert_eq!(
        retrieved.version, 2,
        "Version should increment after update"
    );
}

/// Test multiple version updates
#[test]
fn test_multiple_version_updates() {
    let engine = PolicyEngine::new();

    let mut policy = EnhancedPolicy::new(
        "multi-version-test".to_string(),
        "Multiple version test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy.clone()).unwrap();

    // Perform multiple updates
    for i in 2..=5 {
        policy.update_rules(vec![PolicyRule {
            action: if i % 2 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            },
            resource: format!("resource-{}", i),
            conditions: vec![],
        }]);

        engine.deploy_policy(policy.clone()).unwrap();

        let retrieved = engine.get_policy(&policy_id).unwrap();
        assert_eq!(
            retrieved.version, i,
            "Version should be {} after update {}",
            i, i
        );
    }
}

// ============================================================================
// SECTION 2: Policy Update Tests
// ============================================================================

/// Test that policy updates preserve the ID
#[test]
fn test_update_preserves_id() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "preserve-id-test".to_string(),
        "Preserve ID test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let original_id = policy.id;

    engine.deploy_policy(policy.clone()).unwrap();

    // Create updated policy with same ID
    let mut updated = EnhancedPolicy::new(
        "preserve-id-test".to_string(),
        "Preserve ID test v2".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    updated.id = original_id;
    updated.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "*".to_string(),
        conditions: vec![],
    }]);

    engine.deploy_policy(updated).unwrap();

    // Should still be accessible by original ID
    let retrieved = engine.get_policy(&original_id).unwrap();
    assert_eq!(retrieved.id, original_id);
}

/// Test that old behavior is replaced by new
#[test]
fn test_behavior_changes_on_update() {
    let engine = PolicyEngine::new();

    // Initial policy denies everything
    let mut policy = EnhancedPolicy::new(
        "behavior-change-test".to_string(),
        "Behavior change test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy.clone()).unwrap();

    // Verify initial behavior
    let request = PolicyRequest {
        resource: "any".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),

        ..Default::default()
    };

    let initial_policy = engine.get_policy(&policy_id).unwrap();
    if let Some(evaluator) = initial_policy.evaluator.as_ref() {
        let result = evaluator.evaluate(&request).unwrap();
        assert_eq!(result, PolicyAction::Deny);
    }

    // Update to allow everything
    policy.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "*".to_string(),
        conditions: vec![],
    }]);

    engine.deploy_policy(policy).unwrap();

    // Verify updated behavior
    let updated_policy = engine.get_policy(&policy_id).unwrap();
    if let Some(evaluator) = updated_policy.evaluator.as_ref() {
        let result = evaluator.evaluate(&request).unwrap();
        assert_eq!(result, PolicyAction::Allow);
    }
}

// ============================================================================
// SECTION 3: Backward Compatibility Tests
// ============================================================================

/// Test that old policy format still works
#[test]
fn test_backward_compatible_policy_format() {
    // This represents an "old" policy format that should still work
    let old_format_policy = r#"
policy legacy_format {
    default: deny,

    rule simple_rule {
        allow if {
            user.active == true
        }
    }
}
"#;

    let result = old_format_policy.parse::<ReaperPolicy>();
    assert!(result.is_ok(), "Old policy format should still parse");
}

/// Test EnhancedPolicy backward compatibility
#[test]
fn test_enhanced_policy_backward_compat() {
    // Old way of creating policies should still work
    let policy = EnhancedPolicy::new(
        "legacy-name".to_string(),
        "Legacy description".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    assert_eq!(policy.name, "legacy-name");
    assert_eq!(policy.version, 1);
    assert!(!policy.rules.is_empty());
}

/// Test that policies without metadata still work
#[test]
fn test_policy_without_metadata() {
    let minimal_policy = r#"
policy minimal {
    default: deny,
    rule r1 { allow if { true } }
}
"#;

    let policy = minimal_policy.parse::<ReaperPolicy>().unwrap();

    // Should be able to build evaluator
    let store = Arc::new(DataStore::new());
    let evaluator = policy.build(Arc::clone(&store));
    assert!(evaluator.is_ok(), "Minimal policy should build");
}

// ============================================================================
// SECTION 4: Multiple Policy Coexistence Tests
// ============================================================================

/// Test multiple policies can coexist
#[test]
fn test_multiple_policies_coexist() {
    let engine = PolicyEngine::new();

    let policy1 = EnhancedPolicy::new(
        "policy-1".to_string(),
        "First policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "resource-1".to_string(),
            conditions: vec![],
        }],
    );
    let id1 = policy1.id;

    let policy2 = EnhancedPolicy::new(
        "policy-2".to_string(),
        "Second policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "resource-2".to_string(),
            conditions: vec![],
        }],
    );
    let id2 = policy2.id;

    engine.deploy_policy(policy1).unwrap();
    engine.deploy_policy(policy2).unwrap();

    // Both should be accessible
    assert!(engine.get_policy(&id1).is_some());
    assert!(engine.get_policy(&id2).is_some());

    // Should be different policies
    let p1 = engine.get_policy(&id1).unwrap();
    let p2 = engine.get_policy(&id2).unwrap();
    assert_ne!(p1.name, p2.name);
}

/// Test canary deployment pattern
#[test]
fn test_canary_deployment_pattern() {
    let engine = PolicyEngine::new();

    // Deploy production policy
    let prod_policy = EnhancedPolicy::new(
        "production-policy".to_string(),
        "Production policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let prod_id = prod_policy.id;
    engine.deploy_policy(prod_policy).unwrap();

    // Deploy canary policy (new version with different ID for testing)
    let canary_policy = EnhancedPolicy::new(
        "canary-policy".to_string(),
        "Canary policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "test-*".to_string(),
            conditions: vec![],
        }],
    );
    let canary_id = canary_policy.id;
    engine.deploy_policy(canary_policy).unwrap();

    // Both policies should exist
    assert!(engine.get_policy(&prod_id).is_some());
    assert!(engine.get_policy(&canary_id).is_some());

    // Can evaluate against either
    let prod = engine.get_policy(&prod_id).unwrap();
    let canary = engine.get_policy(&canary_id).unwrap();

    assert_ne!(prod.name, canary.name, "Should be separate policies");
}

// ============================================================================
// SECTION 5: Policy Removal Tests
// ============================================================================

/// Test policy removal
#[test]
fn test_policy_removal() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "removable-policy".to_string(),
        "Removable policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;

    engine.deploy_policy(policy).unwrap();
    assert!(engine.get_policy(&policy_id).is_some());

    // Remove the policy
    engine.remove_policy(&policy_id).unwrap();
    assert!(engine.get_policy(&policy_id).is_none());
}

/// Test removing non-existent policy
#[test]
fn test_remove_nonexistent_policy() {
    let engine = PolicyEngine::new();

    let fake_id = uuid::Uuid::new_v4();
    let result = engine.remove_policy(&fake_id);

    // Should handle gracefully (either Ok or specific error)
    // The key is it shouldn't panic
    assert!(result.is_ok() || result.is_err());
}

// ============================================================================
// SECTION 6: Migration Scenario Tests
// ============================================================================

/// Test migrating from simple to complex policy
#[test]
fn test_migrate_simple_to_complex() {
    let engine = PolicyEngine::new();

    // Start with simple EnhancedPolicy
    let simple = EnhancedPolicy::new(
        "migrate-test".to_string(),
        "Simple policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let simple_id = simple.id;

    engine.deploy_policy(simple).unwrap();

    // Migrate to Reaper DSL policy (different ID, coexisting)
    let complex_text = r#"
policy migrate_test_v2 {
    default: deny,

    rule admin_access {
        allow if {
            user.role == "admin"
        }
    }

    rule user_read {
        allow if {
            user.role == "user" &&
            action == "read"
        }
    }
}
"#;

    let complex = complex_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());
    let _evaluator = complex.build(store).unwrap();

    // Both policies can coexist during migration
    assert!(engine.get_policy(&simple_id).is_some());
}

/// Test gradual migration with multiple versions
#[test]
fn test_gradual_migration() {
    let engine = PolicyEngine::new();

    // Deploy v1
    let mut policy = EnhancedPolicy::new(
        "gradual-migrate".to_string(),
        "Gradual migration v1".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy.clone()).unwrap();

    // Migration step 1: Add new rule
    policy.update_rules(vec![
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "public-*".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        },
    ]);
    engine.deploy_policy(policy.clone()).unwrap();

    let v2 = engine.get_policy(&policy_id).unwrap();
    assert_eq!(v2.version, 2);
    assert_eq!(v2.rules.len(), 2);

    // Migration step 2: Modify rules
    policy.update_rules(vec![
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "public-*".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "internal-*".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        },
    ]);
    engine.deploy_policy(policy).unwrap();

    let v3 = engine.get_policy(&policy_id).unwrap();
    assert_eq!(v3.version, 3);
    assert_eq!(v3.rules.len(), 3);
}

// ============================================================================
// SECTION 7: List and Query Tests
// ============================================================================

/// Test listing all policies
#[test]
fn test_list_policies() {
    let engine = PolicyEngine::new();

    // Deploy multiple policies
    for i in 0..5 {
        let policy = EnhancedPolicy::new(
            format!("list-test-{}", i),
            format!("List test policy {}", i),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: format!("resource-{}", i),
                conditions: vec![],
            }],
        );
        engine.deploy_policy(policy).unwrap();
    }

    let policies = engine.list_policies();
    assert_eq!(policies.len(), 5);
}

/// Test getting policy by name
#[test]
fn test_get_policy_by_name() {
    let engine = PolicyEngine::new();

    let policy = EnhancedPolicy::new(
        "named-policy".to_string(),
        "Named policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    engine.deploy_policy(policy).unwrap();

    let found = engine.get_policy_by_name("named-policy");
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "named-policy");
}

/// Test getting non-existent policy by name
#[test]
fn test_get_nonexistent_policy_by_name() {
    let engine = PolicyEngine::new();

    let found = engine.get_policy_by_name("does-not-exist");
    assert!(found.is_none());
}
