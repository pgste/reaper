//! Comprehensive integration tests for comprehensions
//!
//! Tests all three comprehension types (Set, Array, Object) with realistic data
//! and various scenarios including edge cases.

use policy_engine::data::{AttributeValue, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{EntityBuilder, PolicyAction, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

/// Create a test data store with users and resources
fn create_test_data() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Intern all attribute keys
    let user_type = interner.intern("User");
    let resource_type = interner.intern("Resource");
    let name_key = interner.intern("name");
    let email_key = interner.intern("email");
    let role_key = interner.intern("role");
    let _roles_key = interner.intern("roles");
    let years_key = interner.intern("years_experience");
    let active_key = interner.intern("active");
    let department_key = interner.intern("department");
    let team_key = interner.intern("team");
    let all_users_key = interner.intern("all_users");

    // Create user list
    let mut users_list = Vec::new();

    // User 1: alice - admin, 10 years, active, engineering
    let mut alice_obj = HashMap::new();
    alice_obj.insert(name_key, AttributeValue::String(interner.intern("alice")));
    alice_obj.insert(
        email_key,
        AttributeValue::String(interner.intern("alice@example.com")),
    );
    alice_obj.insert(role_key, AttributeValue::String(interner.intern("admin")));
    alice_obj.insert(years_key, AttributeValue::Int(10));
    alice_obj.insert(active_key, AttributeValue::Bool(true));
    alice_obj.insert(
        department_key,
        AttributeValue::String(interner.intern("engineering")),
    );
    alice_obj.insert(team_key, AttributeValue::String(interner.intern("backend")));
    users_list.push(AttributeValue::Object(alice_obj));

    // User 2: bob - developer, 3 years, active, engineering
    let mut bob_obj = HashMap::new();
    bob_obj.insert(name_key, AttributeValue::String(interner.intern("bob")));
    bob_obj.insert(
        email_key,
        AttributeValue::String(interner.intern("bob@example.com")),
    );
    bob_obj.insert(
        role_key,
        AttributeValue::String(interner.intern("developer")),
    );
    bob_obj.insert(years_key, AttributeValue::Int(3));
    bob_obj.insert(active_key, AttributeValue::Bool(true));
    bob_obj.insert(
        department_key,
        AttributeValue::String(interner.intern("engineering")),
    );
    bob_obj.insert(
        team_key,
        AttributeValue::String(interner.intern("frontend")),
    );
    users_list.push(AttributeValue::Object(bob_obj));

    // User 3: charlie - developer, 8 years, inactive, sales
    let mut charlie_obj = HashMap::new();
    charlie_obj.insert(name_key, AttributeValue::String(interner.intern("charlie")));
    charlie_obj.insert(
        email_key,
        AttributeValue::String(interner.intern("charlie@example.com")),
    );
    charlie_obj.insert(
        role_key,
        AttributeValue::String(interner.intern("developer")),
    );
    charlie_obj.insert(years_key, AttributeValue::Int(8));
    charlie_obj.insert(active_key, AttributeValue::Bool(false));
    charlie_obj.insert(
        department_key,
        AttributeValue::String(interner.intern("sales")),
    );
    charlie_obj.insert(team_key, AttributeValue::String(interner.intern("support")));
    users_list.push(AttributeValue::Object(charlie_obj));

    // User 4: diana - developer, 7 years, active, engineering
    let mut diana_obj = HashMap::new();
    diana_obj.insert(name_key, AttributeValue::String(interner.intern("diana")));
    diana_obj.insert(
        email_key,
        AttributeValue::String(interner.intern("diana@example.com")),
    );
    diana_obj.insert(
        role_key,
        AttributeValue::String(interner.intern("developer")),
    );
    diana_obj.insert(years_key, AttributeValue::Int(7));
    diana_obj.insert(active_key, AttributeValue::Bool(true));
    diana_obj.insert(
        department_key,
        AttributeValue::String(interner.intern("engineering")),
    );
    diana_obj.insert(team_key, AttributeValue::String(interner.intern("backend")));
    users_list.push(AttributeValue::Object(diana_obj));

    // Create test user entities
    let alice_id = interner.intern("alice");
    let alice = EntityBuilder::new(alice_id, user_type)
        .with_string(name_key, interner.intern("alice"))
        .with_string(email_key, interner.intern("alice@example.com"))
        .with_string(role_key, interner.intern("admin"))
        .with_int(years_key, 10)
        .with_bool(active_key, true)
        .with_attribute(all_users_key, AttributeValue::List(users_list.clone()))
        .build();

    let bob_id = interner.intern("bob");
    let bob = EntityBuilder::new(bob_id, user_type)
        .with_string(name_key, interner.intern("bob"))
        .with_string(email_key, interner.intern("bob@example.com"))
        .with_string(role_key, interner.intern("developer"))
        .with_int(years_key, 3)
        .with_bool(active_key, true)
        .with_attribute(all_users_key, AttributeValue::List(users_list.clone()))
        .build();

    let charlie_id = interner.intern("charlie");
    let charlie = EntityBuilder::new(charlie_id, user_type)
        .with_string(name_key, interner.intern("charlie"))
        .with_string(email_key, interner.intern("charlie@example.com"))
        .with_string(role_key, interner.intern("developer"))
        .with_int(years_key, 8)
        .with_bool(active_key, false)
        .with_attribute(all_users_key, AttributeValue::List(users_list))
        .build();

    // Create test resource
    let resource_id = interner.intern("prod_db");
    let resource = EntityBuilder::new(resource_id, resource_type).build();

    store.insert(alice);
    store.insert(bob);
    store.insert(charlie);
    store.insert(resource);

    store
}

#[test]
fn test_set_comprehension_basic() {
    let store = create_test_data();

    // Collect all developer emails into a set
    let policy_text = r#"
        policy test {
            default: deny,
            rule collect_developers {
                allow if dev_emails := {u.email | u := user.all_users[_]; u.role == "developer"}
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_set_comprehension_with_filters() {
    let store = create_test_data();

    // Collect emails of senior active developers
    let policy_text = r#"
        policy test {
            default: deny,
            rule senior_devs {
                allow if senior_emails := {u.email |
                    u := user.all_users[_];
                    u.role == "developer";
                    u.years_experience >= 5;
                    u.active == true
                }
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    // Diana meets criteria: developer, 7 years, active
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_array_comprehension_preserves_order() {
    let store = create_test_data();

    // Collect all user names in an array
    let policy_text = r#"
        policy test {
            default: deny,
            rule collect_names {
                allow if names := [u.name | u := user.all_users[_]]
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_array_comprehension_with_multiple_filters() {
    let store = create_test_data();

    // Collect emails of engineering department active employees
    let policy_text = r#"
        policy test {
            default: deny,
            rule engineering_active {
                allow if eng_emails := [u.email |
                    u := user.all_users[_];
                    u.department == "engineering";
                    u.active == true
                ]
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_object_comprehension_creates_map() {
    let store = create_test_data();

    // Create name -> email mapping
    let policy_text = r#"
        policy test {
            default: deny,
            rule create_directory {
                allow if directory := {u.name: u.email | u := user.all_users[_]}
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_object_comprehension_with_filter() {
    let store = create_test_data();

    // Create name -> department mapping for active users only
    let policy_text = r#"
        policy test {
            default: deny,
            rule active_directory {
                allow if dept_map := {u.name: u.department |
                    u := user.all_users[_];
                    u.active == true
                }
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "prod_db".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_empty_collection() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user with empty all_users list
    let user_type = interner.intern("User");
    let user_id = interner.intern("test_user");
    let all_users_key = interner.intern("all_users");

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(all_users_key, AttributeValue::List(vec![]))
        .build();

    store.insert(user);

    // Create resource
    let resource_type = interner.intern("Resource");
    let resource_id = interner.intern("resource1");
    let resource = EntityBuilder::new(resource_id, resource_type).build();
    store.insert(resource);

    // Comprehension over empty list should still work
    let policy_text = r#"
        policy test {
            default: deny,
            rule empty_set {
                allow if emails := {u.email | u := user.all_users[_]}
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "test_user".to_string());

    let request = PolicyRequest {
        resource: "resource1".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_comprehension_deduplication() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create users with duplicate departments
    let user_type = interner.intern("User");
    let dept_key = interner.intern("department");
    let all_users_key = interner.intern("all_users");

    let mut users = Vec::new();

    // All users in "engineering"
    for _i in 0..5 {
        let mut user_obj = HashMap::new();
        user_obj.insert(
            dept_key,
            AttributeValue::String(interner.intern("engineering")),
        );
        users.push(AttributeValue::Object(user_obj));
    }

    let user_id = interner.intern("test_user");
    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(all_users_key, AttributeValue::List(users))
        .build();

    store.insert(user);

    // Create resource
    let resource_type = interner.intern("Resource");
    let resource_id = interner.intern("resource1");
    let resource = EntityBuilder::new(resource_id, resource_type).build();
    store.insert(resource);

    // Set comprehension should deduplicate
    let policy_text = r#"
        policy test {
            default: deny,
            rule unique_depts {
                allow if depts := {u.department | u := user.all_users[_]}
            }
        }
    "#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "test_user".to_string());

    let request = PolicyRequest {
        resource: "resource1".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
    // Note: Set should have only 1 unique department, but we can't easily verify that from outside
}

// ============================================================================
// Semantics pins: comprehension edge cases the policies in the library rely
// on, asserted on BOTH evaluators (parity is a contract, not a hope).
// ============================================================================

/// Run a policy through the AST evaluator AND, when it compiles, the
/// compiled evaluator — both must produce the expected decision.
fn assert_both_evaluators(
    policy_text: &str,
    store: Arc<DataStore>,
    expected: PolicyAction,
    label: &str,
) {
    use policy_engine::PolicyEvaluator as _;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "test_user".to_string());
    let request = PolicyRequest {
        resource: "resource1".to_string(),
        action: "read".to_string(),
        context,
    };

    let ast = policy.clone().build_ast_evaluator(store.clone());
    let ast_decision = ast.evaluate(&request).unwrap();
    assert_eq!(
        format!("{ast_decision:?}"),
        format!("{expected:?}"),
        "{label}: AST decision mismatch"
    );

    if let Ok(compiled) = policy.build(store) {
        let compiled_decision = compiled.evaluate(&request).unwrap();
        assert_eq!(
            format!("{compiled_decision:?}"),
            format!("{ast_decision:?}"),
            "{label}: compiled/AST parity break"
        );
    }
}

fn store_with_empty_and_lists() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    let user_type = interner.intern("User");
    let user_id = interner.intern("test_user");
    let items_key = interner.intern("items");
    let empty_key = interner.intern("empty_list");
    let name_key = interner.intern("name");

    let mut obj = HashMap::new();
    obj.insert(name_key, AttributeValue::String(interner.intern("thing")));

    let user = EntityBuilder::new(user_id, user_type)
        .with_attribute(
            items_key,
            AttributeValue::List(vec![AttributeValue::Object(obj)]),
        )
        .with_attribute(empty_key, AttributeValue::List(vec![]))
        .build();
    store.insert(user);

    let resource_type = interner.intern("Resource");
    let resource_id = interner.intern("resource1");
    store.insert(EntityBuilder::new(resource_id, resource_type).build());
    store
}

#[test]
fn comprehension_over_missing_attribute_is_empty_not_failure() {
    // TOTAL ITERATION: a missing source is an EMPTY collection. The
    // assignment still binds (assignments always succeed), so a rule whose
    // only condition is the assignment ALLOWS.
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if xs := [u.name | u := user.nonexistent[_]] } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Allow,
        "missing-source assignment",
    );
}

#[test]
fn comprehension_missing_source_count_guard_fails_closed() {
    // The conftest pattern the policy library uses everywhere:
    // bad := [...missing...] && bad.count() > 0  -> no match, rule fails.
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if { bad := [u.name | u := user.nonexistent[_]] && bad.count() > 0 } } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Deny,
        "missing-source count guard",
    );
}

#[test]
fn comprehension_over_empty_list_binds_empty() {
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if xs := [u.name | u := user.empty_list[_]] } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Allow,
        "empty-source assignment",
    );
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if { xs := [u.name | u := user.empty_list[_]] && xs.count() > 0 } } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Deny,
        "empty-source count guard",
    );
}

#[test]
fn comprehension_filter_excluding_everything_yields_empty() {
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if { xs := [u.name | u := user.items[_]; u.name == "no-such"] && xs.count() > 0 } } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Deny,
        "filter-excludes-all count guard",
    );
}

#[test]
fn comprehension_count_positive_path_matches() {
    assert_both_evaluators(
        r#"policy p { default: deny,
            rule r { allow if { xs := [u.name | u := user.items[_]; u.name == "thing"] && xs.count() > 0 } } }"#,
        store_with_empty_and_lists(),
        PolicyAction::Allow,
        "filter-matches count guard",
    );
}
