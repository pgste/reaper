//! Policy-specific scenario generators for comparison benchmarks.
//!
//! Each policy needs scenarios that use entity IDs from its data file,
//! producing a meaningful mix of allow/deny decisions for both Reaper and eOPA.

use crate::scenarios::TestRequest;
use std::collections::HashMap;

/// A scenario entry: (principal_id, action, resource_id)
struct Scenario {
    principal: &'static str,
    action: &'static str,
    resource: &'static str,
}

/// Generate comparison requests for a specific policy.
/// Falls back to generic scenarios if the policy has no specific generator.
pub fn generate_comparison_requests(policy_name: &str, count: usize) -> Vec<TestRequest> {
    let scenarios = match policy_name {
        "rbac_simple" => rbac_scenarios(),
        "abac_clearance" => abac_scenarios(),
        "rebac_relationships" => rebac_scenarios(),
        "multilayer_enterprise" => multilayer_scenarios(),
        "string_operations" => string_scenarios(),
        "math_validation" => math_scenarios(),
        "regex_validation" => regex_scenarios(),
        "collection_operations" => collection_scenarios(),
        "time_based_access" => time_scenarios(),
        "comprehensions" | "comprehension_test" => comprehension_scenarios(),
        "json_operations" | "json_processing" => json_scenarios(),
        "mega_policy" => mega_scenarios(),
        _ => return crate::scenarios::generate_requests(count),
    };

    (0..count)
        .map(|i| {
            let s = &scenarios[i % scenarios.len()];
            TestRequest {
                principal: s.principal.to_string(),
                action: s.action.to_string(),
                resource: s.resource.to_string(),
                context: Some(HashMap::new()),
            }
        })
        .collect()
}

// =============================================================================
// RBAC — rbac_data.json
// Rules: admin→allow, manager+report→allow, owner→allow, else→deny
// =============================================================================
fn rbac_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — admin (rule 1)
        Scenario {
            principal: "admin_alice",
            action: "read",
            resource: "report_001",
        },
        Scenario {
            principal: "admin_alice",
            action: "write",
            resource: "api_data",
        },
        Scenario {
            principal: "admin_bob",
            action: "delete",
            resource: "dashboard",
        },
        // ALLOW — manager + report (rule 2)
        Scenario {
            principal: "manager_carol",
            action: "read",
            resource: "report_001",
        },
        Scenario {
            principal: "manager_carol",
            action: "read",
            resource: "report_002",
        },
        // ALLOW — owner (rule 3)
        Scenario {
            principal: "engineer_eve",
            action: "read",
            resource: "report_001",
        },
        Scenario {
            principal: "viewer_grace",
            action: "read",
            resource: "dashboard",
        },
        Scenario {
            principal: "manager_dave",
            action: "read",
            resource: "sales_data",
        },
        // DENY — wrong role, wrong resource, not owner
        Scenario {
            principal: "guest_jack",
            action: "read",
            resource: "api_data",
        },
        Scenario {
            principal: "viewer_grace",
            action: "write",
            resource: "report_001",
        },
        Scenario {
            principal: "rbac_frank",
            action: "read",
            resource: "dashboard",
        },
        Scenario {
            principal: "viewer_henry",
            action: "delete",
            resource: "sales_data",
        },
        Scenario {
            principal: "marketing_iris",
            action: "read",
            resource: "api_data",
        },
        Scenario {
            principal: "manager_dave",
            action: "write",
            resource: "api_data",
        },
        Scenario {
            principal: "rbac_frank",
            action: "read",
            resource: "sales_data",
        },
        Scenario {
            principal: "guest_jack",
            action: "delete",
            resource: "dashboard",
        },
    ]
}

// =============================================================================
// ABAC — abac_data.json
// Rules: deny if suspended; allow if clearance_match+dept+!archived;
//        allow if high_clearance+dept+!secret+!archived; allow if owner+active;
//        allow if executive+!archived
// =============================================================================
fn abac_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — executive, not archived
        Scenario {
            principal: "exec_alice",
            action: "read",
            resource: "doc_public",
        },
        Scenario {
            principal: "exec_alice",
            action: "read",
            resource: "doc_confidential",
        },
        Scenario {
            principal: "exec_alice",
            action: "read",
            resource: "finance_report",
        },
        // ALLOW — clearance_match + same dept + not archived
        Scenario {
            principal: "manager_bob",
            action: "read",
            resource: "doc_public",
        },
        Scenario {
            principal: "manager_bob",
            action: "read",
            resource: "doc_confidential",
        },
        Scenario {
            principal: "engineer_frank",
            action: "read",
            resource: "doc_public",
        },
        // ALLOW — owner + active
        Scenario {
            principal: "manager_bob",
            action: "read",
            resource: "doc_public",
        },
        Scenario {
            principal: "analyst_carol",
            action: "read",
            resource: "finance_report",
        },
        // DENY — suspended
        Scenario {
            principal: "suspended_eve",
            action: "read",
            resource: "doc_public",
        },
        Scenario {
            principal: "suspended_eve",
            action: "read",
            resource: "doc_confidential",
        },
        // DENY — no clearance + not executive + not owner
        Scenario {
            principal: "intern_dave",
            action: "read",
            resource: "doc_public",
        },
        Scenario {
            principal: "intern_dave",
            action: "read",
            resource: "doc_confidential",
        },
        Scenario {
            principal: "low_clear_grace",
            action: "read",
            resource: "doc_public",
        },
        // DENY — archived
        Scenario {
            principal: "exec_alice",
            action: "read",
            resource: "doc_archived",
        },
        Scenario {
            principal: "manager_bob",
            action: "read",
            resource: "doc_archived",
        },
        // DENY — cross-department
        Scenario {
            principal: "analyst_carol",
            action: "read",
            resource: "doc_public",
        },
    ]
}

// =============================================================================
// ReBAC — rebac_data.json
// Rules: owner→allow, team_lead+same_team→allow, team_member+same_team→allow,
//        shared_with→allow, group_member+public_in_dept→allow,
//        senior_manager+same_dept→allow, parent_inherit→allow
// =============================================================================
fn rebac_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — owner
        Scenario {
            principal: "owner_alice",
            action: "read",
            resource: "team_doc_1",
        },
        Scenario {
            principal: "member_bob",
            action: "read",
            resource: "team_doc_2",
        },
        // ALLOW — team lead + same team
        Scenario {
            principal: "owner_alice",
            action: "read",
            resource: "team_doc_2",
        },
        // ALLOW — team member + same team
        Scenario {
            principal: "member_bob",
            action: "read",
            resource: "team_doc_1",
        },
        // ALLOW — shared_with
        Scenario {
            principal: "collaborator_frank",
            action: "read",
            resource: "team_doc_1",
        },
        Scenario {
            principal: "owner_alice",
            action: "read",
            resource: "sales_doc",
        },
        // ALLOW — senior_manager + same dept
        Scenario {
            principal: "manager_eve",
            action: "read",
            resource: "sales_doc",
        },
        Scenario {
            principal: "manager_eve",
            action: "read",
            resource: "private_doc",
        },
        // DENY — pending member
        Scenario {
            principal: "pending_carol",
            action: "read",
            resource: "team_doc_1",
        },
        Scenario {
            principal: "pending_carol",
            action: "write",
            resource: "team_doc_2",
        },
        // DENY — different team, no share
        Scenario {
            principal: "other_dave",
            action: "read",
            resource: "team_doc_1",
        },
        Scenario {
            principal: "other_dave",
            action: "write",
            resource: "team_doc_2",
        },
        // DENY — external without share
        Scenario {
            principal: "collaborator_frank",
            action: "read",
            resource: "team_doc_2",
        },
        Scenario {
            principal: "collaborator_frank",
            action: "read",
            resource: "private_doc",
        },
        // DENY — cross-team no permissions
        Scenario {
            principal: "member_bob",
            action: "read",
            resource: "sales_doc",
        },
        Scenario {
            principal: "member_bob",
            action: "read",
            resource: "private_doc",
        },
    ]
}

// =============================================================================
// Multilayer — multilayer_data.json
// Combined RBAC+ABAC+ReBAC: deny if suspended; admin→allow;
// executive→allow; clearance+dept+!archived; owner+active; team+dept; etc.
// =============================================================================
fn multilayer_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — admin
        Scenario {
            principal: "admin_1",
            action: "read",
            resource: "public_doc",
        },
        Scenario {
            principal: "admin_1",
            action: "write",
            resource: "secret_doc",
        },
        Scenario {
            principal: "admin_1",
            action: "read",
            resource: "confidential_doc",
        },
        // ALLOW — executive
        Scenario {
            principal: "exec_1",
            action: "read",
            resource: "ops_doc",
        },
        Scenario {
            principal: "exec_1",
            action: "read",
            resource: "public_doc",
        },
        // ALLOW — clearance_match + same dept
        Scenario {
            principal: "manager_1",
            action: "read",
            resource: "public_doc",
        },
        Scenario {
            principal: "engineer_1",
            action: "read",
            resource: "public_doc",
        },
        // ALLOW — owner
        Scenario {
            principal: "engineer_1",
            action: "read",
            resource: "public_doc",
        },
        // DENY — suspended
        Scenario {
            principal: "suspended_1",
            action: "read",
            resource: "public_doc",
        },
        Scenario {
            principal: "suspended_1",
            action: "read",
            resource: "confidential_doc",
        },
        // DENY — no clearance
        Scenario {
            principal: "intern_1",
            action: "read",
            resource: "confidential_doc",
        },
        Scenario {
            principal: "intern_1",
            action: "write",
            resource: "secret_doc",
        },
        // DENY — external without share
        Scenario {
            principal: "collaborator_1",
            action: "read",
            resource: "secret_doc",
        },
        Scenario {
            principal: "collaborator_1",
            action: "read",
            resource: "public_doc",
        },
        // DENY — archived
        Scenario {
            principal: "manager_1",
            action: "read",
            resource: "archived_doc",
        },
        Scenario {
            principal: "engineer_1",
            action: "read",
            resource: "archived_doc",
        },
    ]
}

// =============================================================================
// String — string_data.json
// Rules check: lower, upper, trim, split, contains, startswith, endswith, length
// =============================================================================
fn string_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — contains(@company.com) + internal_docs
        Scenario {
            principal: "str_user3",
            action: "read",
            resource: "str_docs",
        },
        // ALLOW — contains("admin") + code_entry
        Scenario {
            principal: "str_user2",
            action: "read",
            resource: "str_gate1",
        },
        // ALLOW — startswith("admin_") + system_settings
        Scenario {
            principal: "str_user5",
            action: "read",
            resource: "str_settings",
        },
        // ALLOW — endswith(".gov") + classified_docs
        Scenario {
            principal: "str_user6",
            action: "read",
            resource: "str_classified",
        },
        // ALLOW — case insensitive name check
        Scenario {
            principal: "str_user1",
            action: "read",
            resource: "str_doc1",
        },
        // DENY — external domain + internal_docs
        Scenario {
            principal: "str_user4",
            action: "read",
            resource: "str_docs",
        },
        // DENY — no admin in access_code + system_settings
        Scenario {
            principal: "str_user1",
            action: "read",
            resource: "str_settings",
        },
        // DENY — wrong resource type
        Scenario {
            principal: "str_user3",
            action: "read",
            resource: "str_gate1",
        },
        Scenario {
            principal: "str_user6",
            action: "read",
            resource: "str_docs",
        },
        Scenario {
            principal: "str_user2",
            action: "read",
            resource: "str_classified",
        },
        // DENY — mismatched attributes
        Scenario {
            principal: "str_user4",
            action: "read",
            resource: "str_classified",
        },
        Scenario {
            principal: "str_user1",
            action: "read",
            resource: "str_classified",
        },
    ]
}

// =============================================================================
// Math — math_data.json
// Rules: credit_score>=700+premium_loan, order<=budget+shopping_cart,
//        rating>=4.0+featured_listing, etc.
// =============================================================================
fn math_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — credit_score 750 >= 700
        Scenario {
            principal: "math_user1",
            action: "apply",
            resource: "math_loan1",
        },
        // ALLOW — order_total 150 <= budget_limit 200
        Scenario {
            principal: "math_user3",
            action: "checkout",
            resource: "math_cart1",
        },
        // ALLOW — average_rating 4.5 >= 4.0
        Scenario {
            principal: "math_user5",
            action: "list",
            resource: "math_listing1",
        },
        // ALLOW — total_points 1500 >= 1000
        Scenario {
            principal: "math_user6",
            action: "redeem",
            resource: "math_reward1",
        },
        // DENY — credit_score 650 < 700
        Scenario {
            principal: "math_user2",
            action: "apply",
            resource: "math_loan1",
        },
        // DENY — order_total 250 > budget_limit 200
        Scenario {
            principal: "math_user4",
            action: "checkout",
            resource: "math_cart1",
        },
        // DENY — wrong resource for user
        Scenario {
            principal: "math_user1",
            action: "checkout",
            resource: "math_cart1",
        },
        Scenario {
            principal: "math_user2",
            action: "list",
            resource: "math_listing1",
        },
        Scenario {
            principal: "math_user3",
            action: "apply",
            resource: "math_loan1",
        },
        Scenario {
            principal: "math_user6",
            action: "apply",
            resource: "math_loan1",
        },
        Scenario {
            principal: "math_user5",
            action: "checkout",
            resource: "math_cart1",
        },
        Scenario {
            principal: "math_user4",
            action: "redeem",
            resource: "math_reward1",
        },
    ]
}

// =============================================================================
// Regex — regex_data.json
// Rules: regex match email, phone, url, uuid, credit_card format
// =============================================================================
fn regex_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — valid email format
        Scenario {
            principal: "regex_user1",
            action: "validate",
            resource: "regex_val1",
        },
        // ALLOW — valid phone format
        Scenario {
            principal: "regex_user3",
            action: "validate",
            resource: "regex_val2",
        },
        // ALLOW — valid url format
        Scenario {
            principal: "regex_user4",
            action: "validate",
            resource: "regex_val3",
        },
        // ALLOW — valid uuid format
        Scenario {
            principal: "regex_user5",
            action: "validate",
            resource: "regex_val4",
        },
        // ALLOW — valid credit card format
        Scenario {
            principal: "regex_user6",
            action: "validate",
            resource: "regex_val5",
        },
        // DENY — invalid email format
        Scenario {
            principal: "regex_user2",
            action: "validate",
            resource: "regex_val1",
        },
        // DENY — wrong attribute for resource
        Scenario {
            principal: "regex_user1",
            action: "validate",
            resource: "regex_val2",
        },
        Scenario {
            principal: "regex_user3",
            action: "validate",
            resource: "regex_val1",
        },
        Scenario {
            principal: "regex_user4",
            action: "validate",
            resource: "regex_val1",
        },
        Scenario {
            principal: "regex_user5",
            action: "validate",
            resource: "regex_val1",
        },
        Scenario {
            principal: "regex_user6",
            action: "validate",
            resource: "regex_val1",
        },
        Scenario {
            principal: "regex_user2",
            action: "validate",
            resource: "regex_val5",
        },
    ]
}

// =============================================================================
// Collection — collection_data.json
// Rules: permissions contains "read"/"write"/"admin", group intersection, roles
// =============================================================================
fn collection_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — "read" in permissions + document + view action
        Scenario {
            principal: "coll_user1",
            action: "view",
            resource: "coll_doc1",
        },
        // ALLOW — "write" in permissions + document + edit action
        Scenario {
            principal: "coll_user1",
            action: "edit",
            resource: "coll_doc1",
        },
        // ALLOW — "admin" in permissions + document (any action)
        Scenario {
            principal: "coll_user2",
            action: "delete",
            resource: "coll_doc1",
        },
        // ALLOW — engineering in groups + shared_resource
        Scenario {
            principal: "coll_user4",
            action: "read",
            resource: "coll_resource1",
        },
        // ALLOW — "admin" in roles + system
        Scenario {
            principal: "coll_user5",
            action: "admin",
            resource: "coll_sys1",
        },
        // DENY — "view" only permissions + write
        Scenario {
            principal: "coll_user3",
            action: "edit",
            resource: "coll_doc1",
        },
        // DENY — no groups + shared_resource
        Scenario {
            principal: "coll_user1",
            action: "read",
            resource: "coll_resource1",
        },
        // DENY — no admin role + system
        Scenario {
            principal: "coll_user6",
            action: "admin",
            resource: "coll_sys1",
        },
        // DENY — wrong resource
        Scenario {
            principal: "coll_user2",
            action: "read",
            resource: "coll_sys1",
        },
        Scenario {
            principal: "coll_user5",
            action: "read",
            resource: "coll_doc1",
        },
        Scenario {
            principal: "coll_user3",
            action: "view",
            resource: "coll_sys1",
        },
        Scenario {
            principal: "coll_user6",
            action: "view",
            resource: "coll_resource1",
        },
    ]
}

// =============================================================================
// Time — time_data.json
// Rules: token not expired, lease active, event future, access grant window
// =============================================================================
fn time_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — token valid (future expiry)
        Scenario {
            principal: "time_user1",
            action: "call",
            resource: "time_api1",
        },
        // ALLOW — lease active (future end)
        Scenario {
            principal: "time_user3",
            action: "enter",
            resource: "time_apt1",
        },
        // ALLOW — event scheduled (future)
        Scenario {
            principal: "time_user5",
            action: "book",
            resource: "time_room1",
        },
        // ALLOW — access grant window active
        Scenario {
            principal: "time_user6",
            action: "access",
            resource: "time_files1",
        },
        // DENY — token expired (past expiry)
        Scenario {
            principal: "time_user2",
            action: "call",
            resource: "time_api1",
        },
        // DENY — lease expired (past end)
        Scenario {
            principal: "time_user4",
            action: "enter",
            resource: "time_apt1",
        },
        // DENY — wrong resource for user
        Scenario {
            principal: "time_user1",
            action: "enter",
            resource: "time_apt1",
        },
        Scenario {
            principal: "time_user3",
            action: "call",
            resource: "time_api1",
        },
        Scenario {
            principal: "time_user5",
            action: "enter",
            resource: "time_apt1",
        },
        Scenario {
            principal: "time_user2",
            action: "enter",
            resource: "time_apt1",
        },
        Scenario {
            principal: "time_user4",
            action: "book",
            resource: "time_room1",
        },
        Scenario {
            principal: "time_user6",
            action: "call",
            resource: "time_api1",
        },
    ]
}

// =============================================================================
// Comprehension — comprehension_data.json
// Rules: set filter count, array filter count, object mapping count, etc.
// =============================================================================
fn comprehension_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — user1 has 3+ numbers > 5 (6,8,10) + set_result
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_set",
        },
        // ALLOW — user1 has 2+ high priority items + array_result
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_arr",
        },
        // ALLOW — user1 has 2+ active records + object_result
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_obj",
        },
        // ALLOW — user1 has 2+ score>80 && verified + complex_filter
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_complex",
        },
        // ALLOW — user1 has 2+ groups with items + nested_result
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_nested",
        },
        // ALLOW — user1 has 2+ strings containing "a" + transformed
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_transform",
        },
        // ALLOW — user3 (all pass)
        Scenario {
            principal: "comp_user3",
            action: "read",
            resource: "comp_res_set",
        },
        Scenario {
            principal: "comp_user3",
            action: "read",
            resource: "comp_res_arr",
        },
        // DENY — user2 has <3 numbers > 5
        Scenario {
            principal: "comp_user2",
            action: "read",
            resource: "comp_res_set",
        },
        // DENY — user2 has <2 high priority items
        Scenario {
            principal: "comp_user2",
            action: "read",
            resource: "comp_res_arr",
        },
        // DENY — user2 has <2 active records
        Scenario {
            principal: "comp_user2",
            action: "read",
            resource: "comp_res_obj",
        },
        // DENY — empty user
        Scenario {
            principal: "comp_empty",
            action: "read",
            resource: "comp_res_set",
        },
        // DENY — wrong resource type
        Scenario {
            principal: "comp_user1",
            action: "read",
            resource: "comp_res_none",
        },
        Scenario {
            principal: "comp_user3",
            action: "read",
            resource: "comp_res_none",
        },
    ]
}

// =============================================================================
// JSON — json_data.json
// Rules: valid payload, complete profile, nested payment, order items, etc.
// =============================================================================
fn json_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — valid payload + api_endpoint
        Scenario {
            principal: "json_user1",
            action: "post",
            resource: "json_res_api",
        },
        // ALLOW — complete profile + user_profile
        Scenario {
            principal: "json_user1",
            action: "read",
            resource: "json_res_profile",
        },
        // ALLOW — payment data + payment
        Scenario {
            principal: "json_user2",
            action: "pay",
            resource: "json_res_payment",
        },
        // ALLOW — order items + order
        Scenario {
            principal: "json_user2",
            action: "checkout",
            resource: "json_res_order",
        },
        // ALLOW — form data + form_data
        Scenario {
            principal: "json_user2",
            action: "submit",
            resource: "json_res_form",
        },
        // ALLOW — name + text_field
        Scenario {
            principal: "json_user1",
            action: "edit",
            resource: "json_res_text",
        },
        // ALLOW — age >= 18 + number_field
        Scenario {
            principal: "json_user1",
            action: "check",
            resource: "json_res_number",
        },
        // ALLOW — verified + boolean_field
        Scenario {
            principal: "json_user1",
            action: "check",
            resource: "json_res_bool",
        },
        // ALLOW — address + structured_data
        Scenario {
            principal: "json_user1",
            action: "read",
            resource: "json_res_struct",
        },
        // ALLOW — primary+secondary data + data_merge
        Scenario {
            principal: "json_user3",
            action: "merge",
            resource: "json_res_merge",
        },
        // DENY — invalid payload
        Scenario {
            principal: "json_user4",
            action: "post",
            resource: "json_res_api",
        },
        // DENY — incomplete profile
        Scenario {
            principal: "json_user4",
            action: "read",
            resource: "json_res_profile",
        },
        // DENY — empty order items
        Scenario {
            principal: "json_user4",
            action: "checkout",
            resource: "json_res_order",
        },
        // DENY — age < 18
        Scenario {
            principal: "json_user3",
            action: "check",
            resource: "json_res_number",
        },
        // DENY — verified == false
        Scenario {
            principal: "json_user3",
            action: "check",
            resource: "json_res_bool",
        },
        // DENY — empty user
        Scenario {
            principal: "json_empty",
            action: "read",
            resource: "json_res_api",
        },
    ]
}

// =============================================================================
// Mega — mega_data.json (50+ rules, uses input.resource for resource matching)
// =============================================================================
fn mega_scenarios() -> Vec<Scenario> {
    vec![
        // ALLOW — credit_score 750 >= 700
        Scenario {
            principal: "mega_user1",
            action: "apply",
            resource: "premium_loan",
        },
        // ALLOW — order_total 150 <= budget_limit 200
        Scenario {
            principal: "mega_user1",
            action: "checkout",
            resource: "shopping_cart",
        },
        // ALLOW — average_rating 4.5 >= 4.0
        Scenario {
            principal: "mega_user1",
            action: "list",
            resource: "featured_listing",
        },
        // ALLOW — score 92 >= 90 → gold
        Scenario {
            principal: "mega_user1",
            action: "upgrade",
            resource: "gold_tier",
        },
        // ALLOW — email contains @company.com
        Scenario {
            principal: "mega_user1",
            action: "read",
            resource: "company_docs",
        },
        // ALLOW — username startswith admin_
        Scenario {
            principal: "mega_user1",
            action: "access",
            resource: "admin_panel",
        },
        // ALLOW — valid email regex
        Scenario {
            principal: "mega_user1",
            action: "validate",
            resource: "email_validation",
        },
        // ALLOW — token not expired
        Scenario {
            principal: "mega_user1",
            action: "call",
            resource: "api_endpoint",
        },
        // ALLOW — "read" in permissions
        Scenario {
            principal: "mega_user1",
            action: "read",
            resource: "doc_read",
        },
        // ALLOW — "write" in permissions
        Scenario {
            principal: "mega_user1",
            action: "write",
            resource: "doc_write",
        },
        // ALLOW — skills count >= 5
        Scenario {
            principal: "mega_user1",
            action: "apply",
            resource: "senior_position",
        },
        // ALLOW — verified == true
        Scenario {
            principal: "mega_user1",
            action: "check",
            resource: "verification_check",
        },
        // DENY — credit_score 650 < 700
        Scenario {
            principal: "mega_user2",
            action: "apply",
            resource: "premium_loan",
        },
        // DENY — order_total 250 > budget_limit 200
        Scenario {
            principal: "mega_user2",
            action: "checkout",
            resource: "shopping_cart",
        },
        // DENY — token expired
        Scenario {
            principal: "mega_user2",
            action: "call",
            resource: "api_endpoint",
        },
        // DENY — empty user, no attributes
        Scenario {
            principal: "mega_empty",
            action: "read",
            resource: "company_docs",
        },
        Scenario {
            principal: "mega_empty",
            action: "apply",
            resource: "premium_loan",
        },
        Scenario {
            principal: "mega_empty",
            action: "validate",
            resource: "email_validation",
        },
        // DENY — partner domain, not company
        Scenario {
            principal: "mega_user2",
            action: "read",
            resource: "company_docs",
        },
        // DENY — mgr_ prefix, not admin_
        Scenario {
            principal: "mega_user2",
            action: "access",
            resource: "admin_panel",
        },
    ]
}
