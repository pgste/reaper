//! Security-focused tests for the policy engine
//!
//! These tests verify that the policy engine handles adversarial input safely,
//! prevents injection attacks, and resists denial-of-service scenarios.

use policy_engine::data::DataLoader;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// SECTION 1: Adversarial Input Tests
// ============================================================================

/// Test that extremely long strings don't cause stack overflow or excessive memory
#[test]
fn test_adversarial_long_string_in_policy() {
    // Create a policy with a very long resource pattern
    let long_string = "a".repeat(100_000);
    let policy_text = format!(
        r#"
policy long_string_test {{
    default: deny,

    rule allow_long {{
        allow if {{
            resource.id == "{}"
        }}
    }}
}}
"#,
        long_string
    );

    // Should parse without crashing
    let result = policy_text.parse::<ReaperPolicy>();
    assert!(result.is_ok(), "Should handle long strings in policy");

    let policy = result.unwrap();
    let store = Arc::new(DataStore::new());

    // Should build without crashing
    let evaluator = policy.build(Arc::clone(&store));
    assert!(evaluator.is_ok(), "Should build evaluator with long strings");
}

/// Test that deeply nested conditions don't cause stack overflow
#[test]
fn test_adversarial_deeply_nested_conditions() {
    // Create a policy with deeply nested AND conditions
    let mut conditions = String::from("true");
    for _ in 0..100 {
        conditions = format!("({} && true)", conditions);
    }

    let policy_text = format!(
        r#"
policy nested_test {{
    default: deny,

    rule nested {{
        allow if {{
            {}
        }}
    }}
}}
"#,
        conditions
    );

    let result = policy_text.parse::<ReaperPolicy>();
    // May fail to parse due to depth limits, which is acceptable
    // The key is it shouldn't crash
    if let Ok(policy) = result {
        let store = Arc::new(DataStore::new());
        let _ = policy.build(Arc::clone(&store));
    }
}

/// Test that null bytes in input don't cause issues
#[test]
fn test_adversarial_null_bytes_in_request() {
    let policy_text = r#"
policy null_byte_test {
    default: deny,

    rule allow_read {
        allow if {
            resource.type == "document"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());
    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Create request with null bytes
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user\0injected".to_string());

    let request = PolicyRequest {
        resource: "resource\0with\0nulls".to_string(),
        action: "read\0action".to_string(),
        context,
    };

    // Should evaluate without crashing - result doesn't matter, safety does
    let _ = evaluator.evaluate(&request);
}

/// Test Unicode edge cases
#[test]
fn test_adversarial_unicode_edge_cases() {
    let policy_text = r#"
policy unicode_test {
    default: deny,

    rule allow_unicode {
        allow if {
            user.name != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load entity with various Unicode edge cases
    let json = r#"
{
    "entities": [
        {"id": "user_unicode", "type": "User", "attributes": {
            "id": "user_unicode",
            "name": "Test\u0000User\u200B\uFEFF\u202E"
        }}
    ]
}
"#;
    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_unicode".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    // Should evaluate without crashing
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
}

// ============================================================================
// SECTION 2: Injection Prevention Tests
// ============================================================================

/// Test that policy text injection is prevented
#[test]
fn test_injection_policy_text_in_string() {
    // Attempt to inject policy syntax through a string literal
    let policy_text = r#"
policy injection_test {
    default: deny,

    rule check_name {
        allow if {
            user.name == "admin\" } rule injected { allow if { true } } rule fake {"
        }
    }
}
"#;

    // Should parse as a single rule with a weird string, not inject a new rule
    let result = policy_text.parse::<ReaperPolicy>();
    if let Ok(policy) = result {
        // If it parses, verify it only has one meaningful rule
        let store = Arc::new(DataStore::new());
        let evaluator = policy.build(Arc::clone(&store));

        // The key is that injected rules don't execute
        if let Ok(eval) = evaluator {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "nonexistent".to_string());

            let request = PolicyRequest {
                resource: "test".to_string(),
                action: "read".to_string(),
                context,
            };

            // Should deny (default), not allow from "injected" rule
            let result = eval.evaluate(&request);
            if let Ok(decision) = result {
                assert_eq!(decision, PolicyAction::Deny, "Injection attempt should not create allow rule");
            }
        }
    }
}

/// Test that attribute names can't inject code
#[test]
fn test_injection_attribute_name() {
    let policy_text = r#"
policy attr_injection_test {
    default: deny,

    rule check_attr {
        allow if {
            user.role == "admin"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Try to create entity with suspicious attribute name
    let json = r#"
{
    "entities": [
        {"id": "evil_user", "type": "User", "attributes": {
            "id": "evil_user",
            "role == \"admin\" || true || role": "irrelevant"
        }}
    ]
}
"#;
    let loader = DataLoader::new((*store).clone());
    let load_result = loader.load_json(json);

    // Loading should succeed (attribute names are just strings)
    if load_result.is_ok() {
        let evaluator = policy.build(Arc::clone(&store)).unwrap();

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "evil_user".to_string());

        let request = PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        // Should deny - the injected attribute name shouldn't match "role"
        let result = evaluator.evaluate(&request);
        if let Ok(decision) = result {
            assert_eq!(decision, PolicyAction::Deny, "Attribute name injection should not grant access");
        }
    }
}

// ============================================================================
// SECTION 3: Regex DoS Prevention Tests
// ============================================================================

/// Test that catastrophic backtracking regex patterns are handled safely
#[test]
fn test_regex_dos_catastrophic_backtracking() {
    let policy_text = r#"
policy regex_dos_test {
    default: deny,

    rule regex_check {
        allow if {
            user.input.matches("^(a+)+$")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Create input designed to trigger catastrophic backtracking
    // "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaX" - many 'a's followed by non-matching char
    let evil_input = format!("{}X", "a".repeat(30));

    let json = format!(
        r#"
{{
    "entities": [
        {{"id": "regex_user", "type": "User", "attributes": {{
            "id": "regex_user",
            "input": "{}"
        }}}}
    ]
}}
"#,
        evil_input
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "regex_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    // Should complete in reasonable time (< 1 second)
    let start = Instant::now();
    let _ = evaluator.evaluate(&request);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "Regex evaluation took too long: {:?} - possible ReDoS vulnerability",
        elapsed
    );
}

/// Test that extremely long regex patterns are handled
#[test]
fn test_regex_dos_long_pattern() {
    // Create a very long but valid regex pattern
    let long_pattern = format!("^{}$", "a".repeat(10_000));

    let policy_text = format!(
        r#"
policy long_regex_test {{
    default: deny,

    rule regex_check {{
        allow if {{
            user.data.matches("{}")
        }}
    }}
}}
"#,
        long_pattern
    );

    let result = policy_text.parse::<ReaperPolicy>();
    // Should either parse and work, or reject gracefully
    if let Ok(policy) = result {
        let store = Arc::new(DataStore::new());
        let build_result = policy.build(Arc::clone(&store));

        // Building may fail due to regex complexity, which is acceptable
        if let Ok(evaluator) = build_result {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "test".to_string());

            let request = PolicyRequest {
                resource: "test".to_string(),
                action: "read".to_string(),
                context,
            };

            let start = Instant::now();
            let _ = evaluator.evaluate(&request);
            let elapsed = start.elapsed();

            assert!(
                elapsed < Duration::from_secs(1),
                "Long regex pattern caused slow evaluation: {:?}",
                elapsed
            );
        }
    }
}

/// Test nested quantifiers in regex (common ReDoS pattern)
#[test]
fn test_regex_dos_nested_quantifiers() {
    let policy_text = r#"
policy nested_quantifier_test {
    default: deny,

    rule regex_check {
        allow if {
            user.email.matches("^([a-zA-Z0-9]+)*@example.com$")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Evil input: many characters that match [a-zA-Z0-9] but no @
    let evil_input = format!("{}!", "a".repeat(25));

    let json = format!(
        r#"
{{
    "entities": [
        {{"id": "regex_user2", "type": "User", "attributes": {{
            "id": "regex_user2",
            "email": "{}"
        }}}}
    ]
}}
"#,
        evil_input
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "regex_user2".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let start = Instant::now();
    let _ = evaluator.evaluate(&request);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "Nested quantifier regex took too long: {:?}",
        elapsed
    );
}

// ============================================================================
// SECTION 4: Resource Exhaustion Tests
// ============================================================================

/// Test that large comprehensions don't exhaust memory
#[test]
fn test_resource_large_array_in_data() {
    let policy_text = r#"
policy large_array_test {
    default: deny,

    rule check_permissions {
        allow if {
            "admin" in user.roles
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Create entity with large array (but not unreasonably so)
    let large_array: Vec<String> = (0..10_000).map(|i| format!("role_{}", i)).collect();
    let roles_json = serde_json::to_string(&large_array).unwrap();

    let json = format!(
        r#"
{{
    "entities": [
        {{"id": "large_array_user", "type": "User", "attributes": {{
            "id": "large_array_user",
            "roles": {}
        }}}}
    ]
}}
"#,
        roles_json
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "large_array_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let start = Instant::now();
    let result = evaluator.evaluate(&request);
    let elapsed = start.elapsed();

    // Should complete in reasonable time
    assert!(
        elapsed < Duration::from_millis(100),
        "Large array evaluation took too long: {:?}",
        elapsed
    );

    // Should deny (admin not in roles)
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);
}

/// Test handling of many entities
#[test]
fn test_resource_many_entities() {
    let policy_text = r#"
policy many_entities_test {
    default: deny,

    rule allow_active {
        allow if {
            user.status == "active"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Create many entities
    let mut entities = Vec::new();
    for i in 0..10_000 {
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "status": "active"}}}}"#,
            i, i
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));

    let start = Instant::now();
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();
    let load_elapsed = start.elapsed();

    // Loading should be reasonably fast
    assert!(
        load_elapsed < Duration::from_secs(5),
        "Loading 10K entities took too long: {:?}",
        load_elapsed
    );

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Evaluate against a specific user
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_5000".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let start = Instant::now();
    let result = evaluator.evaluate(&request);
    let eval_elapsed = start.elapsed();

    // Evaluation should be sub-millisecond
    assert!(
        eval_elapsed < Duration::from_millis(1),
        "Evaluation with 10K entities took too long: {:?}",
        eval_elapsed
    );

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);
}

// ============================================================================
// SECTION 5: Boundary Value Tests
// ============================================================================

/// Test integer boundary values
#[test]
fn test_boundary_integer_values() {
    let policy_text = r#"
policy integer_boundary_test {
    default: deny,

    rule check_score {
        allow if {
            user.score >= 0 && user.score <= 9223372036854775807
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Test with i64::MAX
    let json = r#"
{
    "entities": [
        {"id": "max_user", "type": "User", "attributes": {"id": "max_user", "score": 9223372036854775807}},
        {"id": "min_user", "type": "User", "attributes": {"id": "min_user", "score": 0}},
        {"id": "neg_user", "type": "User", "attributes": {"id": "neg_user", "score": -1}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Test max value
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "max_user".to_string());
    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Test min value (0)
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "min_user".to_string());
    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);

    // Test negative value (should deny)
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "neg_user".to_string());
    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);
}

/// Test empty string handling
#[test]
fn test_boundary_empty_strings() {
    let policy_text = r#"
policy empty_string_test {
    default: deny,

    rule check_name {
        allow if {
            user.name != "" && user.name != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "empty_user", "type": "User", "attributes": {"id": "empty_user", "name": ""}},
        {"id": "valid_user", "type": "User", "attributes": {"id": "valid_user", "name": "Alice"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Empty name should deny
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "empty_user".to_string());
    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);

    // Valid name should allow
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "valid_user".to_string());
    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Allow);
}

// ============================================================================
// SECTION 6: Error Recovery Tests
// ============================================================================

/// Test graceful handling of missing entities
#[test]
fn test_error_missing_principal() {
    let policy_text = r#"
policy missing_entity_test {
    default: deny,

    rule allow_active {
        allow if {
            user.status == "active"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());
    // Note: NOT loading any data

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "nonexistent_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    // Should not crash - may return error or deny (both are acceptable)
    // The key security property is that it doesn't panic or allow access
    let result = evaluator.evaluate(&request);
    match result {
        Ok(decision) => {
            assert_eq!(decision, PolicyAction::Deny, "Missing principal should not be allowed");
        }
        Err(_) => {
            // Returning an error is also acceptable behavior
        }
    }
}

/// Test handling of malformed JSON in data loading
#[test]
fn test_error_malformed_json() {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    // Various malformed JSON inputs
    let malformed_inputs = vec![
        r#"{"entities": [{"id": "user1" "type": "User"}]}"#,  // Missing comma
        r#"{"entities": [{"id": "user1", "type": }]}"#,       // Missing value
        r#"{"entities": [{"id": "user1", "type": "User",}]}"#, // Trailing comma
        r#"not json at all"#,                                  // Not JSON
        r#"{"entities": null}"#,                               // Null entities
    ];

    for input in malformed_inputs {
        let result = loader.load_json(input);
        // Should return error, not panic
        assert!(
            result.is_err(),
            "Malformed JSON should return error: {}",
            input
        );
    }
}

/// Test handling of type mismatches in conditions
#[test]
fn test_error_type_mismatch_in_condition() {
    let policy_text = r#"
policy type_mismatch_test {
    default: deny,

    rule check_score {
        allow if {
            user.score > 100
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Score is a string, not a number
    let json = r#"
{
    "entities": [
        {"id": "mismatch_user", "type": "User", "attributes": {"id": "mismatch_user", "score": "not a number"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "mismatch_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    // Should not crash, should deny (type mismatch fails condition)
    let result = evaluator.evaluate(&request);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PolicyAction::Deny);
}
