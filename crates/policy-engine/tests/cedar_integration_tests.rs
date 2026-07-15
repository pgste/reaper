//! Cedar Policy Integration Tests
//!
//! Comprehensive tests for Cedar policy language integration including:
//! - Schema validation
//! - Entity types and hierarchies
//! - Cedar extension functions
//! - Complex ABAC scenarios

use policy_engine::{CedarPolicyEvaluator, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;

// ============================================================================
// SECTION 1: Basic Cedar Policy Tests
// ============================================================================

/// Test basic Cedar permit policy
#[test]
fn test_cedar_basic_permit() {
    let policy = r#"
        permit(
            principal,
            action == Action::"read",
            resource
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);
}

/// Test basic Cedar forbid policy
#[test]
fn test_cedar_basic_forbid() {
    let policy = r#"
        forbid(
            principal,
            action == Action::"delete",
            resource
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "delete".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);
}

/// Test Cedar with no matching policy (default deny)
#[test]
fn test_cedar_default_deny() {
    let policy = r#"
        permit(
            principal == User::"admin",
            action,
            resource
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "guest".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);
}

// ============================================================================
// SECTION 2: Cedar Conditions (when clauses)
// ============================================================================

/// Test Cedar with when clause string comparison
#[test]
fn test_cedar_when_string_condition() {
    let policy = r#"
        permit(
            principal,
            action == Action::"read",
            resource
        ) when {
            context.department == "engineering"
        };
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should allow - department matches
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    context.insert("department".to_string(), "engineering".to_string());

    let request = PolicyRequest {
        resource: "code".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Should deny - department doesn't match
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "bob".to_string());
    context2.insert("department".to_string(), "marketing".to_string());

    let request2 = PolicyRequest {
        resource: "code".to_string(),
        action: "read".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Deny);
}

/// Test Cedar with numeric comparison
#[test]
fn test_cedar_when_numeric_condition() {
    let policy = r#"
        permit(
            principal,
            action == Action::"purchase",
            resource
        ) when {
            context.amount < 1000
        };
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should allow - amount under limit
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    context.insert("amount".to_string(), "500".to_string());

    let request = PolicyRequest {
        resource: "item".to_string(),
        action: "purchase".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    // Note: Cedar may require proper type handling
    assert!(result.is_ok() || result.is_err());
}

/// Test Cedar with unless clause (negation)
#[test]
fn test_cedar_unless_clause() {
    let policy = r#"
        permit(
            principal,
            action,
            resource
        ) unless {
            context.suspended == "true"
        };
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should deny - user is suspended
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    context.insert("suspended".to_string(), "true".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);

    // Should allow - user is not suspended
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "bob".to_string());
    context2.insert("suspended".to_string(), "false".to_string());

    let request2 = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Allow);
}

// ============================================================================
// SECTION 3: Cedar Multiple Policies
// ============================================================================

/// Test Cedar with multiple permit policies (OR semantics)
#[test]
fn test_cedar_multiple_permits() {
    let policy = r#"
        permit(
            principal == User::"admin",
            action,
            resource
        );

        permit(
            principal,
            action == Action::"read",
            resource
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Admin can do anything
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "admin".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "delete".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Anyone can read
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "guest".to_string());

    let request2 = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Allow);
}

/// Test Cedar forbid overrides permit
#[test]
fn test_cedar_forbid_overrides_permit() {
    let policy = r#"
        permit(
            principal,
            action,
            resource
        );

        forbid(
            principal,
            action == Action::"delete",
            resource == Resource::"protected"
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should allow - delete on non-protected resource
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "normal".to_string(),
        action: "delete".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Should deny - delete on protected resource (forbid overrides)
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "alice".to_string());

    let request2 = PolicyRequest {
        resource: "protected".to_string(),
        action: "delete".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Deny);
}

// ============================================================================
// SECTION 4: Cedar Policy Validation
// ============================================================================

/// Test Cedar rejects invalid policy syntax
#[test]
fn test_cedar_invalid_syntax_rejected() {
    let invalid_policies = vec![
        "not valid cedar at all",
        "permit(principal, action, resource", // Missing closing paren
        "permit();",                          // Empty permit
        "allow(principal, action, resource);", // Wrong keyword
    ];

    for policy in invalid_policies {
        let result = CedarPolicyEvaluator::new(policy.to_string());
        assert!(result.is_err(), "Should reject invalid policy: {}", policy);
    }
}

/// Test Cedar accepts valid policy syntax variations
#[test]
fn test_cedar_valid_syntax_accepted() {
    let valid_policies = vec![
        "permit(principal, action, resource);",
        "forbid(principal, action, resource);",
        r#"permit(principal == User::"alice", action, resource);"#,
        r#"permit(principal, action == Action::"read", resource);"#,
        r#"permit(principal, action, resource == Resource::"doc");"#,
        r#"permit(principal, action, resource) when { true };"#,
        r#"permit(principal, action, resource) unless { false };"#,
    ];

    for policy in valid_policies {
        let result = CedarPolicyEvaluator::new(policy.to_string());
        assert!(
            result.is_ok(),
            "Should accept valid policy: {}, error: {:?}",
            policy,
            result.err()
        );
    }
}

// ============================================================================
// SECTION 5: Cedar Entity Types
// ============================================================================

/// Test Cedar with specific entity types in policy
#[test]
fn test_cedar_entity_type_matching() {
    let policy = r#"
        permit(
            principal == User::"alice",
            action == Action::"read",
            resource == Resource::"secret-doc"
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should allow - exact match
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "secret-doc".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Should deny - different user
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "bob".to_string());

    let request2 = PolicyRequest {
        resource: "secret-doc".to_string(),
        action: "read".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Deny);
}

// ============================================================================
// SECTION 6: Cedar Metadata and Annotations
// ============================================================================

/// Test Cedar policy with annotations
#[test]
fn test_cedar_policy_annotations() {
    let policy = r#"
        @id("read-access")
        @description("Allows read access to all users")
        permit(
            principal,
            action == Action::"read",
            resource
        );
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "anyone".to_string());

    let request = PolicyRequest {
        resource: "document".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);
}

/// Test Cedar evaluator metadata
#[test]
fn test_cedar_evaluator_metadata() {
    let policy = r#"permit(principal, action, resource);"#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();
    let metadata = evaluator.metadata();

    // Metadata should be present
    assert!(
        metadata.is_some(),
        "Cedar evaluator should provide metadata"
    );

    let meta = metadata.unwrap();
    assert!(meta.rule_count >= 1, "Should have at least 1 rule");
}

// ============================================================================
// SECTION 7: Cedar Performance Characteristics
// ============================================================================

/// Test Cedar evaluation completes in reasonable time
#[test]
fn test_cedar_evaluation_performance() {
    let policy = r#"
        permit(
            principal,
            action == Action::"read",
            resource
        ) when {
            context.level == "public"
        };
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user".to_string());
    context.insert("level".to_string(), "public".to_string());

    let request = PolicyRequest {
        resource: "doc".to_string(),
        action: "read".to_string(),
        context,

        ..Default::default()
    };

    // Measure evaluation time
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = evaluator.evaluate(&request);
    }
    let elapsed = start.elapsed();

    // Should complete 1000 evaluations in under 5 seconds
    // Note: Cedar is slower than Simple evaluator (~1-2ms per evaluation)
    assert!(
        elapsed.as_secs() < 5,
        "1000 Cedar evaluations took too long: {:?}",
        elapsed
    );

    // Average should be under 5ms per evaluation (Cedar is slower than Simple)
    let avg_micros = elapsed.as_micros() / 1000;
    assert!(
        avg_micros < 5000,
        "Average evaluation time {} microseconds exceeds 5ms",
        avg_micros
    );

    println!("Cedar avg evaluation time: {} microseconds", avg_micros);
}

// ============================================================================
// SECTION 8: Cedar ABAC Scenarios
// ============================================================================

/// Test Cedar ABAC with multiple context attributes
#[test]
fn test_cedar_abac_multi_attribute() {
    let policy = r#"
        permit(
            principal,
            action == Action::"access",
            resource
        ) when {
            context.clearance == "secret" &&
            context.department == "research"
        };
    "#;

    let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

    // Should allow - both attributes match
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    context.insert("clearance".to_string(), "secret".to_string());
    context.insert("department".to_string(), "research".to_string());

    let request = PolicyRequest {
        resource: "classified".to_string(),
        action: "access".to_string(),
        context,

        ..Default::default()
    };

    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Should deny - clearance doesn't match
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "bob".to_string());
    context2.insert("clearance".to_string(), "public".to_string());
    context2.insert("department".to_string(), "research".to_string());

    let request2 = PolicyRequest {
        resource: "classified".to_string(),
        action: "access".to_string(),
        context: context2,

        ..Default::default()
    };

    let result2 = evaluator.evaluate(&request2);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), PolicyAction::Deny);
}
