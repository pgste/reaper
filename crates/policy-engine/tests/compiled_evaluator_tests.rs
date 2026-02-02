//! Comprehensive tests for compiled evaluator against all benchmark policies
//!
//! These tests load policies from the reaper-bench folder and validate
//! that the compiled evaluator produces correct results for all scenarios.
//! Tests run locally without Docker.

use policy_engine::data::DataLoader;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Get the project root directory (reaper/)
fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/policy-engine -> reaper/
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Get the path to a benchmark policy file
fn policy_path(name: &str) -> PathBuf {
    project_root()
        .join("services/reaper-bench/policies")
        .join(name)
}

/// Get the path to a benchmark data file
fn data_path(name: &str) -> PathBuf {
    project_root()
        .join("services/reaper-bench/policies/data")
        .join(name)
}

/// Test result for easier assertions
#[derive(Debug, Clone, PartialEq)]
enum Expected {
    Allow,
    Deny,
}

impl Expected {
    fn matches(&self, decision: &PolicyAction) -> bool {
        match (self, decision) {
            (Expected::Allow, PolicyAction::Allow) => true,
            (Expected::Deny, PolicyAction::Deny) => true,
            _ => false,
        }
    }
}

/// Helper to load policy and data, then run test scenarios
fn run_policy_test(
    policy_file: &str,
    data_file: Option<&str>,
    scenarios: Vec<(&str, &str, &str, Expected)>,
) {
    let policy_path = policy_path(policy_file);
    let policy = ReaperPolicy::from_file_auto(&policy_path)
        .unwrap_or_else(|e| panic!("Failed to load policy {}: {}", policy_file, e));

    let store = Arc::new(DataStore::new());

    if let Some(data_name) = data_file {
        let data_path = data_path(data_name);
        let json = std::fs::read_to_string(&data_path)
            .unwrap_or_else(|e| panic!("Failed to read data {}: {}", data_name, e));
        let loader = DataLoader::new((*store).clone());
        loader
            .load_json(&json)
            .unwrap_or_else(|e| panic!("Failed to load data {}: {}", data_name, e));
    }

    // Build the compiled evaluator
    let evaluator = policy
        .build(Arc::clone(&store))
        .unwrap_or_else(|e| panic!("Failed to build evaluator: {}", e));

    for (principal, action, resource, expected) in scenarios {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal.to_string());

        let request = PolicyRequest {
            resource: resource.to_string(),
            action: action.to_string(),
            context,
        };

        let result = evaluator.evaluate(&request);
        let decision = result.unwrap_or_else(|e| {
            panic!(
                "Evaluation error for principal={}, action={}, resource={}: {}",
                principal, action, resource, e
            )
        });

        assert!(
            expected.matches(&decision),
            "Policy: {}, Principal: {}, Action: {}, Resource: {}\n  Expected: {:?}\n  Got: {:?}",
            policy_file,
            principal,
            action,
            resource,
            expected,
            decision
        );
    }
}

// ============ String Operations Tests ============

#[test]
fn test_string_lowercase_match() {
    run_policy_test(
        "string_policy.reap",
        Some("string_data.json"),
        vec![
            // str_user1 has name "John Doe" -> lower() == "john doe"
            ("str_user1", "access", "str_doc1", Expected::Allow),
        ],
    );
}

#[test]
fn test_string_uppercase_code() {
    run_policy_test(
        "string_policy.reap",
        Some("string_data.json"),
        vec![
            // str_user2 has access_code "admin123" -> upper() == "ADMIN123"
            ("str_user2", "enter", "str_gate1", Expected::Allow),
        ],
    );
}

#[test]
fn test_string_email_contains() {
    // Debug version of the test to understand the failure
    let policy_path = policy_path("string_policy.reap");
    let policy = ReaperPolicy::from_file_auto(&policy_path)
        .unwrap_or_else(|e| panic!("Failed to load policy: {}", e));

    let store = Arc::new(DataStore::new());
    let data_path = data_path("string_data.json");
    let json = std::fs::read_to_string(&data_path)
        .unwrap_or_else(|e| panic!("Failed to read data: {}", e));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).expect("Failed to load data");

    // Check data loaded correctly
    let interner = store.interner();
    let user_id = interner.intern("str_user3");
    let user = store.get(user_id).expect("str_user3 should exist");
    let email_key = interner.intern("email");
    let email = user.get_attribute(email_key);
    eprintln!("DEBUG: str_user3 email attribute key = {:?}", email_key);
    eprintln!("DEBUG: str_user3 email value = {:?}", email);

    // Print all user attributes
    eprintln!("DEBUG: str_user3 all attributes: {:?}", user.attributes);

    let resource_id = interner.intern("str_docs");
    let resource = store.get(resource_id).expect("str_docs should exist");
    let type_key = interner.intern("type");
    let resource_type = resource.get_attribute(type_key);
    eprintln!("DEBUG: str_docs type attribute key = {:?}", type_key);
    eprintln!("DEBUG: str_docs type value = {:?}", resource_type);

    // Print all resource attributes
    eprintln!("DEBUG: str_docs all attributes: {:?}", resource.attributes);

    // Build AST evaluator first to test (AST should work)
    let ast_evaluator = policy.clone().build_ast_evaluator(Arc::clone(&store));

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "str_user3".to_string());

    let request = PolicyRequest {
        resource: "str_docs".to_string(),
        action: "read".to_string(),
        context: context.clone(),
    };

    let ast_result = ast_evaluator.evaluate(&request);
    eprintln!("DEBUG: AST evaluator result = {:?}", ast_result);

    // Build compiled evaluator
    let compiled_evaluator = policy.build(Arc::clone(&store))
        .unwrap_or_else(|e| panic!("Failed to build evaluator: {}", e));

    let compiled_result = compiled_evaluator.evaluate(&request);
    eprintln!("DEBUG: Compiled evaluator result = {:?}", compiled_result);

    // Simpler test: just check user.email.contains("@company.com") works
    let simple_policy_text = r#"
policy test {
    default: deny,
    rule test_contains {
        allow if {
            user.email.contains("@company.com")
        }
    }
}
"#;
    let simple_policy: ReaperPolicy = simple_policy_text.parse().expect("parse simple policy");
    let simple_ast_evaluator = simple_policy.clone().build_ast_evaluator(Arc::clone(&store));
    let simple_compiled_evaluator = simple_policy.build(Arc::clone(&store)).expect("build simple evaluator");

    eprintln!("DEBUG: Simple AST evaluator = {:?}", simple_ast_evaluator.evaluate(&request));
    eprintln!("DEBUG: Simple compiled evaluator = {:?}", simple_compiled_evaluator.evaluate(&request));

    // Run the original tests
    run_policy_test(
        "string_policy.reap",
        Some("string_data.json"),
        vec![
            // str_user3 has email "alice@company.com" which contains "@company.com"
            ("str_user3", "read", "str_docs", Expected::Allow),
            // str_user4 has email "bob@external.org" which doesn't contain "@company.com"
            ("str_user4", "read", "str_docs", Expected::Deny),
        ],
    );
}

#[test]
fn test_string_startswith() {
    run_policy_test(
        "string_policy.reap",
        Some("string_data.json"),
        vec![
            // str_user5 has username "admin_jones" which starts with "admin_"
            ("str_user5", "modify", "str_settings", Expected::Allow),
        ],
    );
}

#[test]
fn test_string_endswith() {
    run_policy_test(
        "string_policy.reap",
        Some("string_data.json"),
        vec![
            // str_user6 has email "agent@fbi.gov" which ends with ".gov"
            ("str_user6", "read", "str_classified", Expected::Allow),
        ],
    );
}

// ============ Math Operations Tests ============

#[test]
fn test_math_credit_score() {
    run_policy_test(
        "math_policy.reap",
        Some("math_data.json"),
        vec![
            // math_user1 has credit_score 750 >= 700
            ("math_user1", "apply", "math_loan1", Expected::Allow),
            // math_user2 has credit_score 650 < 700
            ("math_user2", "apply", "math_loan1", Expected::Deny),
        ],
    );
}

#[test]
fn test_math_budget_check() {
    run_policy_test(
        "math_policy.reap",
        Some("math_data.json"),
        vec![
            // math_user3 has order_total 150 <= budget_limit 200
            ("math_user3", "checkout", "math_cart1", Expected::Allow),
            // math_user4 has order_total 250 > budget_limit 200
            ("math_user4", "checkout", "math_cart1", Expected::Deny),
        ],
    );
}

#[test]
fn test_math_rating_check() {
    run_policy_test(
        "math_policy.reap",
        Some("math_data.json"),
        vec![
            // math_user5 has average_rating 4.5 >= 4.0
            ("math_user5", "feature", "math_listing1", Expected::Allow),
        ],
    );
}

#[test]
fn test_math_loyalty_points() {
    run_policy_test(
        "math_policy.reap",
        Some("math_data.json"),
        vec![
            // math_user6 has total_points 1500 >= 1000
            ("math_user6", "redeem", "math_reward1", Expected::Allow),
        ],
    );
}

// ============ Regex Validation Tests ============

#[test]
fn test_regex_email_validation() {
    run_policy_test(
        "regex_policy.reap",
        Some("regex_data.json"),
        vec![
            // regex_user1 has email "test@example.com" - valid format
            ("regex_user1", "validate", "regex_val1", Expected::Allow),
            // regex_user2 has email "not-an-email" - invalid format
            ("regex_user2", "validate", "regex_val1", Expected::Deny),
        ],
    );
}

#[test]
fn test_regex_phone_validation() {
    run_policy_test(
        "regex_policy.reap",
        Some("regex_data.json"),
        vec![
            // regex_user3 has phone "+1 (555) 123-4567" - valid US format
            ("regex_user3", "validate", "regex_val2", Expected::Allow),
        ],
    );
}

#[test]
fn test_regex_url_validation() {
    run_policy_test(
        "regex_policy.reap",
        Some("regex_data.json"),
        vec![
            // regex_user4 has url "https://example.com/path" - valid format
            ("regex_user4", "validate", "regex_val3", Expected::Allow),
        ],
    );
}

#[test]
fn test_regex_uuid_validation() {
    run_policy_test(
        "regex_policy.reap",
        Some("regex_data.json"),
        vec![
            // regex_user5 has uuid "550e8400-e29b-41d4-a716-446655440000" - valid format
            ("regex_user5", "validate", "regex_val4", Expected::Allow),
        ],
    );
}

#[test]
fn test_regex_credit_card_validation() {
    run_policy_test(
        "regex_policy.reap",
        Some("regex_data.json"),
        vec![
            // regex_user6 has credit_card "1234-5678-9012-3456" - valid format
            ("regex_user6", "validate", "regex_val5", Expected::Allow),
        ],
    );
}

// ============ Collection Operations Tests ============

#[test]
fn test_collection_array_contains_permission() {
    run_policy_test(
        "collection_policy.reap",
        Some("collection_data.json"),
        vec![
            // coll_user1 has permissions ["read", "write"], "read" in permissions for "view"
            ("coll_user1", "view", "coll_doc1", Expected::Allow),
            // coll_user2 has permissions ["admin"], "admin" in permissions for "delete"
            ("coll_user2", "delete", "coll_doc1", Expected::Allow),
            // coll_user3 has permissions ["view"], no "write" for "edit"
            ("coll_user3", "edit", "coll_doc1", Expected::Deny),
        ],
    );
}

#[test]
fn test_collection_group_overlap() {
    run_policy_test(
        "collection_policy.reap",
        Some("collection_data.json"),
        vec![
            // coll_user4 has groups ["engineering", "frontend"], intersects with allowed groups
            ("coll_user4", "access", "coll_resource1", Expected::Allow),
        ],
    );
}

#[test]
fn test_collection_has_admin_role() {
    run_policy_test(
        "collection_policy.reap",
        Some("collection_data.json"),
        vec![
            // coll_user5 has roles ["user", "admin"], "admin" in roles
            ("coll_user5", "manage", "coll_sys1", Expected::Allow),
            // coll_user6 has roles ["user", "editor"], no "admin"
            ("coll_user6", "manage", "coll_sys1", Expected::Deny),
        ],
    );
}

// ============ Conditional Logic Tests ============

#[test]
fn test_conditional_age_restriction() {
    run_policy_test(
        "conditional_policy.reap",
        Some("conditional_data.json"),
        vec![
            // cond_user1 has age 25 >= 18
            ("cond_user1", "view", "cond_content1", Expected::Allow),
            // cond_user2 has age 16 < 18
            ("cond_user2", "view", "cond_content1", Expected::Deny),
        ],
    );
}

#[test]
fn test_conditional_premium_content() {
    run_policy_test(
        "conditional_policy.reap",
        Some("conditional_data.json"),
        vec![
            // cond_user3 has age 30 >= 18 AND subscription "premium"
            ("cond_user3", "view", "cond_content2", Expected::Allow),
        ],
    );
}

#[test]
fn test_conditional_subscription_upgrade() {
    run_policy_test(
        "conditional_policy.reap",
        Some("conditional_data.json"),
        vec![
            // cond_user4 has tier "gold" - can upgrade
            ("cond_user4", "upgrade", "cond_sub1", Expected::Allow),
            // cond_user5 has tier "bronze" - cannot upgrade
            ("cond_user5", "upgrade", "cond_sub1", Expected::Deny),
        ],
    );
}

#[test]
fn test_conditional_payment_transaction() {
    run_policy_test(
        "conditional_policy.reap",
        Some("conditional_data.json"),
        vec![
            // cond_user6 has verified=true AND status="active"
            ("cond_user6", "process", "cond_pay1", Expected::Allow),
        ],
    );
}

// ============ Time-Based Access Tests ============

#[test]
fn test_time_valid_token() {
    run_policy_test(
        "time_policy.reap",
        Some("time_data.json"),
        vec![
            // time_user1 has token_expires_at in the future
            ("time_user1", "call", "time_api1", Expected::Allow),
            // time_user2 has token_expires_at in the past
            ("time_user2", "call", "time_api1", Expected::Deny),
        ],
    );
}

#[test]
fn test_time_active_lease() {
    run_policy_test(
        "time_policy.reap",
        Some("time_data.json"),
        vec![
            // time_user3 has lease_end_time in the future
            ("time_user3", "access", "time_apt1", Expected::Allow),
            // time_user4 has lease_end_time in the past
            ("time_user4", "access", "time_apt1", Expected::Deny),
        ],
    );
}

#[test]
fn test_time_future_event_scheduling() {
    run_policy_test(
        "time_policy.reap",
        Some("time_data.json"),
        vec![
            // time_user5 has role="event_planner" and event_scheduled_time in future
            ("time_user5", "schedule", "time_room1", Expected::Allow),
        ],
    );
}

#[test]
fn test_time_temporary_access_grant() {
    run_policy_test(
        "time_policy.reap",
        Some("time_data.json"),
        vec![
            // time_user6 has role="contractor" and access window that spans current time
            ("time_user6", "read", "time_files1", Expected::Allow),
        ],
    );
}

// ============ RBAC Tests ============

#[test]
fn test_rbac_admin_full_access() {
    run_policy_test(
        "rbac.reap",
        Some("rbac_data.json"),
        vec![
            // admin_alice has role="admin" - full access
            ("admin_alice", "read", "report_001", Expected::Allow),
            ("admin_alice", "write", "report_001", Expected::Allow),
            ("admin_alice", "delete", "report_001", Expected::Allow),
        ],
    );
}

#[test]
fn test_rbac_manager_access() {
    run_policy_test(
        "rbac.reap",
        Some("rbac_data.json"),
        vec![
            // manager_carol has role="manager" - can read reports
            ("manager_carol", "read", "report_001", Expected::Allow),
            // manager cannot read non-reports
            ("manager_carol", "read", "api_data", Expected::Deny),
        ],
    );
}

#[test]
fn test_rbac_owner_access() {
    run_policy_test(
        "rbac.reap",
        Some("rbac_data.json"),
        vec![
            // engineer_eve owns report_001 (owner_id matches)
            ("engineer_eve", "delete", "report_001", Expected::Allow),
        ],
    );
}

#[test]
fn test_rbac_guest_denied() {
    run_policy_test(
        "rbac.reap",
        Some("rbac_data.json"),
        vec![
            // guest_jack has role="guest" - denied
            ("guest_jack", "read", "report_001", Expected::Deny),
        ],
    );
}

// ============ ABAC Tests ============

#[test]
fn test_abac_suspended_user() {
    run_policy_test(
        "abac.reap",
        Some("abac_data.json"),
        vec![
            // suspended_eve has suspended=true - always denied
            ("suspended_eve", "read", "doc_public", Expected::Deny),
        ],
    );
}

#[test]
fn test_abac_executive_access() {
    run_policy_test(
        "abac.reap",
        Some("abac_data.json"),
        vec![
            // exec_alice has role="executive" and not suspended - can read non-archived
            ("exec_alice", "read", "doc_confidential", Expected::Allow),
            // exec_alice cannot read archived
            ("exec_alice", "read", "doc_archived", Expected::Deny),
        ],
    );
}

#[test]
fn test_abac_department_clearance() {
    run_policy_test(
        "abac.reap",
        Some("abac_data.json"),
        vec![
            // manager_bob has department="engineering" and clearance_match=true
            ("manager_bob", "read", "doc_confidential", Expected::Allow),
            // analyst_carol has wrong department
            ("analyst_carol", "read", "doc_confidential", Expected::Deny),
        ],
    );
}

// ============ ReBAC Tests ============

#[test]
fn test_rebac_owner_access() {
    run_policy_test(
        "rebac.reap",
        Some("rebac_data.json"),
        vec![
            // owner_alice owns team_doc_1
            ("owner_alice", "delete", "team_doc_1", Expected::Allow),
        ],
    );
}

#[test]
fn test_rebac_team_member_access() {
    run_policy_test(
        "rebac.reap",
        Some("rebac_data.json"),
        vec![
            // member_bob is in team_alpha with role="member"
            ("member_bob", "read", "team_doc_1", Expected::Allow),
            // pending_carol is in team_alpha but role="pending" - denied
            ("pending_carol", "read", "team_doc_1", Expected::Deny),
        ],
    );
}

#[test]
fn test_rebac_different_team_denied() {
    run_policy_test(
        "rebac.reap",
        Some("rebac_data.json"),
        vec![
            // other_dave is in team_beta, not team_alpha
            ("other_dave", "read", "team_doc_1", Expected::Deny),
        ],
    );
}

// ============ Multilayer Tests ============

#[test]
fn test_multilayer_suspended_always_denied() {
    run_policy_test(
        "multilayer.reap",
        Some("multilayer_data.json"),
        vec![
            // suspended_1 has suspended=true - always denied even if admin
            ("suspended_1", "read", "public_doc", Expected::Deny),
        ],
    );
}

#[test]
fn test_multilayer_admin_full_access() {
    run_policy_test(
        "multilayer.reap",
        Some("multilayer_data.json"),
        vec![
            // admin_1 has role="admin" and not suspended
            ("admin_1", "read", "secret_doc", Expected::Allow),
        ],
    );
}

#[test]
fn test_multilayer_intern_denied_secret() {
    run_policy_test(
        "multilayer.reap",
        Some("multilayer_data.json"),
        vec![
            // intern_1 has role="intern" - cannot access secret docs
            ("intern_1", "read", "secret_doc", Expected::Deny),
        ],
    );
}

// ============ Benchmark Policy Tests ============

#[test]
fn test_benchmark_admin() {
    run_policy_test(
        "benchmark.reap",
        Some("benchmark_data.json"),
        vec![
            // admin_1 has role="admin" - any action allowed
            ("admin_1", "delete", "any", Expected::Allow),
        ],
    );
}

#[test]
fn test_benchmark_engineer() {
    run_policy_test(
        "benchmark.reap",
        Some("benchmark_data.json"),
        vec![
            // eng_1 has role="engineer" - read/write allowed, delete denied
            ("eng_1", "read", "api", Expected::Allow),
            ("eng_1", "write", "api", Expected::Allow),
            ("eng_1", "delete", "api", Expected::Deny),
        ],
    );
}

#[test]
fn test_benchmark_viewer() {
    run_policy_test(
        "benchmark.reap",
        Some("benchmark_data.json"),
        vec![
            // viewer_1 has role="viewer" - read only
            ("viewer_1", "read", "dashboard", Expected::Allow),
            ("viewer_1", "write", "dashboard", Expected::Deny),
        ],
    );
}

#[test]
fn test_benchmark_guest() {
    run_policy_test(
        "benchmark.reap",
        Some("benchmark_data.json"),
        vec![
            // guest_1 has role="guest" - always denied
            ("guest_1", "read", "any", Expected::Deny),
        ],
    );
}
