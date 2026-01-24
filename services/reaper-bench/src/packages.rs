//! Policy Packages
//!
//! Defines policy packages that group related policies with their test scenarios.
//! Each package has a set of policies and test cases that exercise different rules.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A policy package groups related policies with test scenarios
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPackage {
    /// Package name (e.g., "rbac", "abac")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Policy names included in this package
    pub policies: Vec<String>,
    /// Test scenarios for this package
    pub scenarios: Vec<TestScenario>,
    /// Data file path (relative to policies directory)
    pub data_file: Option<String>,
}

/// A test scenario exercises specific policy rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestScenario {
    /// Scenario name
    pub name: String,
    /// User attributes for this scenario
    pub user: HashMap<String, serde_json::Value>,
    /// Resource attributes for this scenario
    pub resource: HashMap<String, serde_json::Value>,
    /// Action being performed
    pub action: String,
    /// Expected decision (allow/deny)
    pub expected: String,
}

/// Get all available policy packages
pub fn get_packages() -> Vec<PolicyPackage> {
    vec![
        rbac_package(),
        abac_package(),
        rebac_package(),
        multilayer_package(),
        benchmark_package(),
        string_package(),
        math_package(),
        regex_package(),
        collection_package(),
        conditional_package(),
        time_package(),
    ]
}

/// Get a specific package by name
pub fn get_package(name: &str) -> Option<PolicyPackage> {
    get_packages().into_iter().find(|p| p.name == name)
}

/// RBAC Package - Role-Based Access Control
fn rbac_package() -> PolicyPackage {
    PolicyPackage {
        name: "rbac".to_string(),
        description: "Role-Based Access Control - Tests role permissions and ownership rules".to_string(),
        policies: vec!["rbac_simple".to_string()],
        data_file: Some("data/rbac_data.json".to_string()),
        scenarios: vec![
            // Admin scenarios - should all allow
            TestScenario {
                name: "admin_full_access".to_string(),
                user: [("id".to_string(), json!("admin_alice")), ("role".to_string(), json!("admin"))].into_iter().collect(),
                resource: [("id".to_string(), json!("any_resource")), ("type".to_string(), json!("report"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "allow".to_string(),
            },
            // Manager scenarios
            TestScenario {
                name: "manager_read_report".to_string(),
                user: [("id".to_string(), json!("manager_carol")), ("role".to_string(), json!("manager"))].into_iter().collect(),
                resource: [("id".to_string(), json!("report_001")), ("type".to_string(), json!("report"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "manager_non_report_deny".to_string(),
                user: [("id".to_string(), json!("manager_carol")), ("role".to_string(), json!("manager"))].into_iter().collect(),
                resource: [("id".to_string(), json!("api_data")), ("type".to_string(), json!("api"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Owner scenarios
            TestScenario {
                name: "owner_access_own_resource".to_string(),
                user: [("id".to_string(), json!("engineer_eve")), ("role".to_string(), json!("engineer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("report_001")), ("owner_id".to_string(), json!("engineer_eve"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "allow".to_string(),
            },
            // Guest scenarios - should deny
            TestScenario {
                name: "guest_denied".to_string(),
                user: [("id".to_string(), json!("guest_jack")), ("role".to_string(), json!("guest"))].into_iter().collect(),
                resource: [("id".to_string(), json!("report_001")), ("type".to_string(), json!("report")), ("owner_id".to_string(), json!("other"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
        ],
    }
}

/// ABAC Package - Attribute-Based Access Control
fn abac_package() -> PolicyPackage {
    PolicyPackage {
        name: "abac".to_string(),
        description: "Attribute-Based Access Control - Tests clearance levels, departments, and document classification".to_string(),
        policies: vec!["abac_clearance".to_string()],
        data_file: Some("data/abac_data.json".to_string()),
        scenarios: vec![
            // Suspended user - should always deny
            TestScenario {
                name: "suspended_user_denied".to_string(),
                user: [("id".to_string(), json!("suspended_eve")), ("role".to_string(), json!("manager")), ("suspended".to_string(), json!(true))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_public")), ("classification".to_string(), json!("public"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Executive access
            TestScenario {
                name: "executive_access_non_archived".to_string(),
                user: [("id".to_string(), json!("exec_alice")), ("role".to_string(), json!("executive")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_confidential")), ("archived".to_string(), json!(false))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Executive denied archived
            TestScenario {
                name: "executive_denied_archived".to_string(),
                user: [("id".to_string(), json!("exec_alice")), ("role".to_string(), json!("executive")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_archived")), ("archived".to_string(), json!(true))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Department + clearance match
            TestScenario {
                name: "clearance_department_match".to_string(),
                user: [("id".to_string(), json!("manager_bob")), ("department".to_string(), json!("engineering")), ("clearance_match".to_string(), json!(true)), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_confidential")), ("department".to_string(), json!("engineering")), ("archived".to_string(), json!(false)), ("classification".to_string(), json!("confidential"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Wrong department denied
            TestScenario {
                name: "wrong_department_denied".to_string(),
                user: [("id".to_string(), json!("analyst_carol")), ("department".to_string(), json!("finance")), ("clearance_match".to_string(), json!(true)), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_confidential")), ("department".to_string(), json!("engineering")), ("archived".to_string(), json!(false))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Owner access
            TestScenario {
                name: "owner_can_access".to_string(),
                user: [("id".to_string(), json!("manager_bob")), ("status".to_string(), json!("active")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc_confidential")), ("owner_id".to_string(), json!("manager_bob"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// ReBAC Package - Relationship-Based Access Control
fn rebac_package() -> PolicyPackage {
    PolicyPackage {
        name: "rebac".to_string(),
        description: "Relationship-Based Access Control - Tests team membership, sharing, and organizational hierarchy".to_string(),
        policies: vec!["rebac_relationships".to_string()],
        data_file: Some("data/rebac_data.json".to_string()),
        scenarios: vec![
            // Owner access
            TestScenario {
                name: "owner_full_access".to_string(),
                user: [("id".to_string(), json!("owner_alice"))].into_iter().collect(),
                resource: [("id".to_string(), json!("team_doc_1")), ("owner_id".to_string(), json!("owner_alice"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "allow".to_string(),
            },
            // Team member access
            TestScenario {
                name: "team_member_access".to_string(),
                user: [("id".to_string(), json!("member_bob")), ("team_id".to_string(), json!("team_alpha")), ("team_role".to_string(), json!("member"))].into_iter().collect(),
                resource: [("id".to_string(), json!("team_doc_1")), ("team_id".to_string(), json!("team_alpha"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Pending team member denied
            TestScenario {
                name: "pending_member_denied".to_string(),
                user: [("id".to_string(), json!("pending_carol")), ("team_id".to_string(), json!("team_alpha")), ("team_role".to_string(), json!("pending"))].into_iter().collect(),
                resource: [("id".to_string(), json!("team_doc_1")), ("team_id".to_string(), json!("team_alpha"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Shared access
            TestScenario {
                name: "shared_resource_access".to_string(),
                user: [("id".to_string(), json!("owner_alice"))].into_iter().collect(),
                resource: [("id".to_string(), json!("sales_doc")), ("shared_with_user".to_string(), json!("owner_alice"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Collaborator access
            TestScenario {
                name: "active_collaborator_access".to_string(),
                user: [("id".to_string(), json!("collaborator_frank"))].into_iter().collect(),
                resource: [("id".to_string(), json!("team_doc_1")), ("collaborator_id".to_string(), json!("collaborator_frank")), ("collaboration_status".to_string(), json!("active"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Different team denied
            TestScenario {
                name: "different_team_denied".to_string(),
                user: [("id".to_string(), json!("other_dave")), ("team_id".to_string(), json!("team_beta")), ("team_role".to_string(), json!("member"))].into_iter().collect(),
                resource: [("id".to_string(), json!("team_doc_1")), ("team_id".to_string(), json!("team_alpha")), ("owner_id".to_string(), json!("other")), ("shared_with_user".to_string(), json!(null))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
        ],
    }
}

/// Multilayer Package - Combined RBAC, ABAC, and ReBAC
fn multilayer_package() -> PolicyPackage {
    PolicyPackage {
        name: "multilayer".to_string(),
        description: "Multilayer Access Control - Tests combined RBAC, ABAC, and ReBAC rules in enterprise scenarios".to_string(),
        policies: vec!["multilayer_enterprise".to_string()],
        data_file: Some("data/multilayer_data.json".to_string()),
        scenarios: vec![
            // Suspended user always denied
            TestScenario {
                name: "suspended_always_denied".to_string(),
                user: [("id".to_string(), json!("suspended_1")), ("role".to_string(), json!("admin")), ("suspended".to_string(), json!(true))].into_iter().collect(),
                resource: [("id".to_string(), json!("public_doc")), ("classification".to_string(), json!("public"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Admin full access (not suspended)
            TestScenario {
                name: "admin_full_access".to_string(),
                user: [("id".to_string(), json!("admin_1")), ("role".to_string(), json!("admin")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("secret_doc")), ("classification".to_string(), json!("secret"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Intern denied secret (even same team)
            TestScenario {
                name: "intern_denied_secret".to_string(),
                user: [("id".to_string(), json!("intern_1")), ("role".to_string(), json!("intern")), ("team_id".to_string(), json!("core")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("secret_doc")), ("classification".to_string(), json!("secret")), ("team_id".to_string(), json!("core"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            // Team lead access
            TestScenario {
                name: "team_lead_access".to_string(),
                user: [("id".to_string(), json!("admin_1")), ("team_role".to_string(), json!("lead")), ("team_id".to_string(), json!("core")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("confidential_doc")), ("team_id".to_string(), json!("core")), ("classification".to_string(), json!("confidential"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Collaborator access
            TestScenario {
                name: "collaborator_access".to_string(),
                user: [("id".to_string(), json!("collaborator_1")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("confidential_doc")), ("collaborator_id".to_string(), json!("collaborator_1")), ("collaboration_status".to_string(), json!("active"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Public resource access
            TestScenario {
                name: "public_resource_access".to_string(),
                user: [("id".to_string(), json!("engineer_1")), ("status".to_string(), json!("active")), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("public_doc")), ("classification".to_string(), json!("public")), ("archived".to_string(), json!(false))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            // Executive with clearance
            TestScenario {
                name: "executive_high_clearance".to_string(),
                user: [("id".to_string(), json!("exec_1")), ("role".to_string(), json!("executive")), ("high_clearance".to_string(), json!(true)), ("suspended".to_string(), json!(false))].into_iter().collect(),
                resource: [("id".to_string(), json!("confidential_doc")), ("archived".to_string(), json!(false))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Benchmark Package - Original benchmark policy for performance testing
fn benchmark_package() -> PolicyPackage {
    PolicyPackage {
        name: "benchmark".to_string(),
        description: "Performance Benchmark - Optimized for ~60% allow, 40% deny split".to_string(),
        policies: vec!["benchmark_rbac".to_string()],
        data_file: Some("data/benchmark_data.json".to_string()),
        scenarios: vec![
            // Admin scenarios
            TestScenario {
                name: "admin_any_action".to_string(),
                user: [("id".to_string(), json!("admin_1")), ("role".to_string(), json!("admin"))].into_iter().collect(),
                resource: [("id".to_string(), json!("any"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "allow".to_string(),
            },
            // Engineer scenarios
            TestScenario {
                name: "engineer_read".to_string(),
                user: [("id".to_string(), json!("eng_1")), ("role".to_string(), json!("engineer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("api"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "engineer_write".to_string(),
                user: [("id".to_string(), json!("eng_1")), ("role".to_string(), json!("engineer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("api"))].into_iter().collect(),
                action: "write".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "engineer_delete_denied".to_string(),
                user: [("id".to_string(), json!("eng_1")), ("role".to_string(), json!("engineer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("api"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "deny".to_string(),
            },
            // Viewer scenarios
            TestScenario {
                name: "viewer_read".to_string(),
                user: [("id".to_string(), json!("viewer_1")), ("role".to_string(), json!("viewer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("dashboard"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "viewer_write_denied".to_string(),
                user: [("id".to_string(), json!("viewer_1")), ("role".to_string(), json!("viewer"))].into_iter().collect(),
                resource: [("id".to_string(), json!("dashboard"))].into_iter().collect(),
                action: "write".to_string(),
                expected: "deny".to_string(),
            },
            // Guest denied
            TestScenario {
                name: "guest_denied".to_string(),
                user: [("id".to_string(), json!("guest_1")), ("role".to_string(), json!("guest"))].into_iter().collect(),
                resource: [("id".to_string(), json!("any"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
        ],
    }
}

/// String Operations Package - String manipulation methods
fn string_package() -> PolicyPackage {
    PolicyPackage {
        name: "string".to_string(),
        description: "String Operations - Tests string manipulation methods (lower, upper, contains, startswith, etc.)".to_string(),
        policies: vec!["string_operations".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "lowercase_match".to_string(),
                user: [("id".to_string(), json!("user1")), ("name".to_string(), json!("John Doe"))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc1")), ("type".to_string(), json!("case_insensitive"))].into_iter().collect(),
                action: "access".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "uppercase_code_valid".to_string(),
                user: [("id".to_string(), json!("user2")), ("access_code".to_string(), json!("admin123"))].into_iter().collect(),
                resource: [("id".to_string(), json!("gate1")), ("type".to_string(), json!("code_entry"))].into_iter().collect(),
                action: "enter".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "email_contains_domain".to_string(),
                user: [("id".to_string(), json!("user3")), ("email".to_string(), json!("alice@company.com"))].into_iter().collect(),
                resource: [("id".to_string(), json!("docs")), ("type".to_string(), json!("internal_docs"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "email_wrong_domain_denied".to_string(),
                user: [("id".to_string(), json!("user4")), ("email".to_string(), json!("bob@external.org"))].into_iter().collect(),
                resource: [("id".to_string(), json!("docs")), ("type".to_string(), json!("internal_docs"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "admin_prefix_access".to_string(),
                user: [("id".to_string(), json!("user5")), ("username".to_string(), json!("admin_jones"))].into_iter().collect(),
                resource: [("id".to_string(), json!("settings")), ("type".to_string(), json!("system_settings"))].into_iter().collect(),
                action: "modify".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "gov_email_access".to_string(),
                user: [("id".to_string(), json!("user6")), ("email".to_string(), json!("agent@fbi.gov"))].into_iter().collect(),
                resource: [("id".to_string(), json!("classified")), ("type".to_string(), json!("classified_docs"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Math Operations Package - Numeric validation and comparisons
fn math_package() -> PolicyPackage {
    PolicyPackage {
        name: "math".to_string(),
        description: "Math Operations - Tests numeric comparisons, thresholds, and range checks".to_string(),
        policies: vec!["math_validation".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "credit_score_approved".to_string(),
                user: [("id".to_string(), json!("user1")), ("credit_score".to_string(), json!(750))].into_iter().collect(),
                resource: [("id".to_string(), json!("loan1")), ("type".to_string(), json!("premium_loan"))].into_iter().collect(),
                action: "apply".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "credit_score_denied".to_string(),
                user: [("id".to_string(), json!("user2")), ("credit_score".to_string(), json!(650))].into_iter().collect(),
                resource: [("id".to_string(), json!("loan1")), ("type".to_string(), json!("premium_loan"))].into_iter().collect(),
                action: "apply".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "budget_within_limit".to_string(),
                user: [("id".to_string(), json!("user3")), ("order_total".to_string(), json!(150)), ("budget_limit".to_string(), json!(200))].into_iter().collect(),
                resource: [("id".to_string(), json!("cart1")), ("type".to_string(), json!("shopping_cart"))].into_iter().collect(),
                action: "checkout".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "budget_exceeded".to_string(),
                user: [("id".to_string(), json!("user4")), ("order_total".to_string(), json!(250)), ("budget_limit".to_string(), json!(200))].into_iter().collect(),
                resource: [("id".to_string(), json!("cart1")), ("type".to_string(), json!("shopping_cart"))].into_iter().collect(),
                action: "checkout".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "high_rating_featured".to_string(),
                user: [("id".to_string(), json!("user5")), ("average_rating".to_string(), json!(4.5))].into_iter().collect(),
                resource: [("id".to_string(), json!("listing1")), ("type".to_string(), json!("featured_listing"))].into_iter().collect(),
                action: "feature".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "loyalty_points_reward".to_string(),
                user: [("id".to_string(), json!("user6")), ("total_points".to_string(), json!(1500))].into_iter().collect(),
                resource: [("id".to_string(), json!("reward1")), ("type".to_string(), json!("loyalty_reward"))].into_iter().collect(),
                action: "redeem".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Regex Pattern Package - Regular expression pattern matching
fn regex_package() -> PolicyPackage {
    PolicyPackage {
        name: "regex".to_string(),
        description: "Regex Pattern Matching - Tests regex validation for email, phone, URL, IP, UUID formats".to_string(),
        policies: vec!["regex_validation".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "valid_email_format".to_string(),
                user: [("id".to_string(), json!("user1")), ("email".to_string(), json!("test@example.com"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val1")), ("type".to_string(), json!("email_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "invalid_email_format".to_string(),
                user: [("id".to_string(), json!("user2")), ("email".to_string(), json!("not-an-email"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val1")), ("type".to_string(), json!("email_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "valid_phone_format".to_string(),
                user: [("id".to_string(), json!("user3")), ("phone".to_string(), json!("+1 (555) 123-4567"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val2")), ("type".to_string(), json!("phone_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "valid_url_format".to_string(),
                user: [("id".to_string(), json!("user4")), ("url".to_string(), json!("https://example.com/path"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val3")), ("type".to_string(), json!("url_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "valid_uuid_format".to_string(),
                user: [("id".to_string(), json!("user5")), ("uuid".to_string(), json!("550e8400-e29b-41d4-a716-446655440000"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val4")), ("type".to_string(), json!("uuid_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "valid_credit_card_format".to_string(),
                user: [("id".to_string(), json!("user6")), ("credit_card".to_string(), json!("1234-5678-9012-3456"))].into_iter().collect(),
                resource: [("id".to_string(), json!("val5")), ("type".to_string(), json!("payment_validation"))].into_iter().collect(),
                action: "validate".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Collection Operations Package - Array, set, and map operations
fn collection_package() -> PolicyPackage {
    PolicyPackage {
        name: "collection".to_string(),
        description: "Collection Operations - Tests array contains, set intersection, map keys, and comprehensions".to_string(),
        policies: vec!["collection_operations".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "array_contains_read_permission".to_string(),
                user: [("id".to_string(), json!("user1")), ("permissions".to_string(), json!(["read", "write"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc1")), ("type".to_string(), json!("document"))].into_iter().collect(),
                action: "view".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "array_contains_admin_permission".to_string(),
                user: [("id".to_string(), json!("user2")), ("permissions".to_string(), json!(["admin"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc1")), ("type".to_string(), json!("document"))].into_iter().collect(),
                action: "delete".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "no_permission_denied".to_string(),
                user: [("id".to_string(), json!("user3")), ("permissions".to_string(), json!(["view"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("doc1")), ("type".to_string(), json!("document"))].into_iter().collect(),
                action: "edit".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "group_overlap_engineering".to_string(),
                user: [("id".to_string(), json!("user4")), ("groups".to_string(), json!(["engineering", "frontend"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("resource1")), ("type".to_string(), json!("shared_resource"))].into_iter().collect(),
                action: "access".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "has_admin_role".to_string(),
                user: [("id".to_string(), json!("user5")), ("roles".to_string(), json!(["user", "admin"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("sys1")), ("type".to_string(), json!("system"))].into_iter().collect(),
                action: "manage".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "no_admin_role_denied".to_string(),
                user: [("id".to_string(), json!("user6")), ("roles".to_string(), json!(["user", "editor"]))].into_iter().collect(),
                resource: [("id".to_string(), json!("sys1")), ("type".to_string(), json!("system"))].into_iter().collect(),
                action: "manage".to_string(),
                expected: "deny".to_string(),
            },
        ],
    }
}

/// Conditional Logic Package - Boolean logic and conditional expressions
fn conditional_package() -> PolicyPackage {
    PolicyPackage {
        name: "conditional".to_string(),
        description: "Conditional Logic - Tests if/else expressions, boolean combinations, and nested conditionals".to_string(),
        policies: vec!["conditionals".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "age_over_18_allowed".to_string(),
                user: [("id".to_string(), json!("user1")), ("age".to_string(), json!(25))].into_iter().collect(),
                resource: [("id".to_string(), json!("content1")), ("type".to_string(), json!("age_restricted"))].into_iter().collect(),
                action: "view".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "age_under_18_denied".to_string(),
                user: [("id".to_string(), json!("user2")), ("age".to_string(), json!(16))].into_iter().collect(),
                resource: [("id".to_string(), json!("content1")), ("type".to_string(), json!("age_restricted"))].into_iter().collect(),
                action: "view".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "premium_adult_access".to_string(),
                user: [("id".to_string(), json!("user3")), ("age".to_string(), json!(30)), ("subscription".to_string(), json!("premium"))].into_iter().collect(),
                resource: [("id".to_string(), json!("content2")), ("type".to_string(), json!("premium_content"))].into_iter().collect(),
                action: "view".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "gold_tier_upgrade".to_string(),
                user: [("id".to_string(), json!("user4")), ("tier".to_string(), json!("gold"))].into_iter().collect(),
                resource: [("id".to_string(), json!("sub1")), ("type".to_string(), json!("subscription"))].into_iter().collect(),
                action: "upgrade".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "bronze_tier_denied".to_string(),
                user: [("id".to_string(), json!("user5")), ("tier".to_string(), json!("bronze"))].into_iter().collect(),
                resource: [("id".to_string(), json!("sub1")), ("type".to_string(), json!("subscription"))].into_iter().collect(),
                action: "upgrade".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "verified_active_payment".to_string(),
                user: [("id".to_string(), json!("user6")), ("verified".to_string(), json!(true)), ("status".to_string(), json!("active"))].into_iter().collect(),
                resource: [("id".to_string(), json!("pay1")), ("type".to_string(), json!("payment"))].into_iter().collect(),
                action: "process".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Time-Based Access Package - Time/date based access control
fn time_package() -> PolicyPackage {
    PolicyPackage {
        name: "time".to_string(),
        description: "Time-Based Access Control - Tests token expiration, business hours, and time windows".to_string(),
        policies: vec!["time_based_access".to_string()],
        data_file: None,
        scenarios: vec![
            TestScenario {
                name: "valid_token_access".to_string(),
                user: [("id".to_string(), json!("user1")), ("token_expires_at".to_string(), json!(1800000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("api1")), ("type".to_string(), json!("api_endpoint"))].into_iter().collect(),
                action: "call".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "expired_token_denied".to_string(),
                user: [("id".to_string(), json!("user2")), ("token_expires_at".to_string(), json!(1700000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("api1")), ("type".to_string(), json!("api_endpoint"))].into_iter().collect(),
                action: "call".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "active_lease_access".to_string(),
                user: [("id".to_string(), json!("user3")), ("lease_end_time".to_string(), json!(1800000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("apt1")), ("type".to_string(), json!("apartment"))].into_iter().collect(),
                action: "access".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "expired_lease_denied".to_string(),
                user: [("id".to_string(), json!("user4")), ("lease_end_time".to_string(), json!(1700000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("apt1")), ("type".to_string(), json!("apartment"))].into_iter().collect(),
                action: "access".to_string(),
                expected: "deny".to_string(),
            },
            TestScenario {
                name: "future_event_scheduling".to_string(),
                user: [("id".to_string(), json!("user5")), ("role".to_string(), json!("event_planner")), ("event_scheduled_time".to_string(), json!(1800000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("room1")), ("type".to_string(), json!("conference_room"))].into_iter().collect(),
                action: "schedule".to_string(),
                expected: "allow".to_string(),
            },
            TestScenario {
                name: "contractor_temp_access".to_string(),
                user: [("id".to_string(), json!("user6")), ("role".to_string(), json!("contractor")), ("access_grant_start".to_string(), json!(1700000000000000000i64)), ("access_grant_end".to_string(), json!(1800000000000000000i64))].into_iter().collect(),
                resource: [("id".to_string(), json!("files1")), ("type".to_string(), json!("project_files"))].into_iter().collect(),
                action: "read".to_string(),
                expected: "allow".to_string(),
            },
        ],
    }
}

/// Helper macro for JSON values
macro_rules! json {
    (null) => { serde_json::Value::Null };
    (true) => { serde_json::Value::Bool(true) };
    (false) => { serde_json::Value::Bool(false) };
    ($e:expr) => { serde_json::json!($e) };
}

use json;
