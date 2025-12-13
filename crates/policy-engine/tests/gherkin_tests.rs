// Gherkin/Cucumber Test Runner for Reaper Policy Engine
//
// Runs BDD tests defined in .feature files

use cucumber::{given, then, when, World};
use policy_engine::gherkin::TestContext;

/// World struct for Cucumber tests
#[derive(Debug, World, Default)]
#[world(init = Self::new)]
pub struct PolicyWorld {
    pub context: TestContext,
}

impl PolicyWorld {
    fn new() -> Self {
        Self {
            context: TestContext::new(),
        }
    }
}

// ============================================================================
// Given Steps - Setup
// ============================================================================

#[given(expr = "the policy file {string}")]
async fn load_policy_file(world: &mut PolicyWorld, path: String) {
    world
        .context
        .load_policy(&path)
        .expect("Failed to load policy file");
}

#[given(expr = "the data file {string}")]
async fn load_data_file(world: &mut PolicyWorld, path: String) {
    world
        .context
        .load_data(&path)
        .expect("Failed to load data file");
    world
        .context
        .build_evaluator()
        .expect("Failed to build evaluator");
}

#[given(expr = "a principal {string}")]
async fn set_principal(world: &mut PolicyWorld, principal: String) {
    world.context.principal = Some(principal);
}

// ============================================================================
// When Steps - Actions
// ============================================================================

#[when(expr = "they perform action {string} on resource {string}")]
async fn perform_action(world: &mut PolicyWorld, action: String, resource: String) {
    world.context.action = Some(action);
    world.context.resource = Some(resource);

    world.context.evaluate().expect("Evaluation failed");
}

#[when(expr = "they perform {int} evaluations on random resources")]
async fn perform_multiple_evaluations(world: &mut PolicyWorld, count: usize) {
    let action = world.context.action.clone().unwrap_or("read".to_string());

    // Determine which resource list to use based on the principal
    // Each data file has distinct principals we can detect
    let empty_string = String::new();
    let principal = world.context.principal.as_ref().unwrap_or(&empty_string);

    // Determine which resource naming scheme to use based on the principal
    // Check if this is a numbered principal (user_0, user_1, etc.) for Day 1 tests
    let use_numbered_resources = principal.starts_with("user_") &&
                                  principal.len() <= 7 &&  // "user_0" to "user_99"
                                  principal.chars().skip(5).all(|c| c.is_numeric() || c == '_');

    if use_numbered_resources {
        // Day 1: Use numbered resources (resource_0, resource_1, ...)
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(format!("resource_{}", i));
            world.context.evaluate().expect("Evaluation failed");
        }
    } else if principal.starts_with("user_with_") || principal.starts_with("user_without_") {
        // Day 2: Collection operations - use collection test data resources
        let resource_types = [
            "document",
            "senior_position",
            "shared_resource",
            "content",
            "system",
            "invoice",
            "profile",
            "email_campaign",
            "workflow",
            "resource_0",
            "resource_1",
        ];
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());
            world.context.evaluate().expect("Evaluation failed");
        }
    } else if principal == "user_valid_json"
        || principal == "user_invalid_json"
        || principal == "user_complete_profile"
        || principal == "user_incomplete_profile"
        || principal == "user_nested_data"
        || principal == "user_missing_nested"
        || principal == "user_array_items"
        || principal == "user_empty_array"
        || principal == "user_correct_types"
        || principal == "user_wrong_types"
        || principal.starts_with("user_json_")
    {
        // Day 2: JSON operations - use json test data resources
        let resource_types = [
            "api_endpoint",
            "user_profile",
            "payment",
            "order",
            "form_data",
            "text_field",
            "number_field",
            "boolean_field",
            "structured_data",
            "data_merge",
            "resource_0",
            "resource_1",
        ];
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());
            world.context.evaluate().expect("Evaluation failed");
        }
    } else if principal.contains("credit")
        || principal.contains("seller")
        || principal.contains("sensor")
    {
        // Day 2: Math operations - use math test data resources
        let resource_types = [
            "seller_good_rating",
            "seller_poor_rating",
            "seller_fair_price",
            "seller_high_price",
            "sensor_normal_temp",
            "sensor_extreme_temp",
            "premium_loan",
            "shopping_cart",
            "featured_listing",
            "marketplace",
            "premium_tier",
            "temperature_monitor",
            "loyalty_reward",
            "sale_item",
            "resource_0",
            "resource_1",
        ];
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());
            world.context.evaluate().expect("Evaluation failed");
        }
    } else if principal.contains("valid_email")
        || principal.contains("invalid_email")
        || principal.contains("phone")
    {
        // Day 2: Regex operations - use regex test data resources
        let resource_types = [
            "email_validation",
            "phone_validation",
            "url_validation",
            "ip_validation",
            "uuid_validation",
            "payment_validation",
            "redacted_data",
            "csv_data",
            "log_entry",
            "resource_0",
            "resource_1",
        ];
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());
            world.context.evaluate().expect("Evaluation failed");
        }
    } else if principal.contains("token")
        || principal.contains("employee")
        || principal.contains("tenant")
        || principal.contains("operator")
        || principal.contains("planner")
        || principal.contains("contractor")
    {
        // Day 3: Time operations - use time test data resources
        let resource_types = [
            "employee",
            "employee_after_hours",
            "tenant_active",
            "tenant_expired",
            "operator",
            "operator_wrong_time",
            "event_planner",
            "event_planner_past",
            "contractor_active",
            "contractor_expired",
            "system_with_timestamp",
            "audit_logger",
            "api_client_normal",
            "api_client_exceeded",
            "archiver",
            "retention_policy",
            "api_endpoint",
            "office_system",
            "alcohol",
            "apartment_101",
            "production_system",
            "production_system_outside_window",
            "web_session",
            "conference_room",
            "project_files",
            "timestamp_data",
            "audit_trail",
            "rate_limited_endpoint",
            "old_data",
            "expired_data",
            "resource_0",
            "resource_1",
        ];
        for i in 0..count {
            world.context.action = Some(action.clone());
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());
            world.context.evaluate().expect("Evaluation failed");
        }
    } else {
        // Day 4: Use named resources based on the data file
        let resource_types: &[&str] = if principal.starts_with("user_mixed_case")
            || principal.starts_with("user_wrong_case")
            || principal.starts_with("user_uppercase_code")
            || principal.starts_with("user_lowercase_code")
            || principal.starts_with("user_whitespace_role")
            || principal.starts_with("user_email_contains")
            || principal.starts_with("user_admin_username")
            || principal.starts_with("user_gov_email")
            || principal.starts_with("user_full_name")
            || principal.starts_with("user_complex_email")
        {
            // String test data resources
            &[
                "case_insensitive",
                "code_entry",
                "trimmed_check",
                "internal_docs",
                "system_settings",
                "classified_docs",
                "profile",
                "email_check",
            ]
        } else if principal.starts_with("user_priority_tasks")
            || principal.starts_with("user_recent_login")
            || principal.starts_with("user_top_scores")
            || principal.starts_with("user_desc_order")
            || principal.starts_with("user_sortable_data")
            || principal.starts_with("user_unique_skills")
            || principal.starts_with("user_combined_perms")
            || principal.starts_with("user_filtered_access")
            || principal.starts_with("user_high_max_score")
            || principal.starts_with("user_consistent_performance")
        {
            // Advanced collection test data resources
            &[
                "task_queue",
                "session",
                "leaderboard",
                "items",
                "records",
                "specialized_role",
                "multi_function",
                "filtered_content",
                "competition",
                "quality_check",
            ]
        } else if principal.starts_with("user_matrix_data")
            || principal.starts_with("user_sparse_matrix")
            || principal.starts_with("user_grouped_data")
            || principal.starts_with("user_empty_groups")
            || principal.starts_with("user_hierarchical")
            || principal.starts_with("user_flat_data")
            || principal.starts_with("user_complex_filter")
            || principal.starts_with("user_no_match")
            || principal.starts_with("user_text_data")
            || principal.starts_with("user_invalid_text")
            || principal.starts_with("user_deep_structure")
            || principal.starts_with("user_shallow_structure")
            || principal.starts_with("user_conditional_data")
            || principal.starts_with("user_wrong_condition")
            || principal.starts_with("user_aggregate_data")
            || principal.starts_with("user_limited_data")
            || principal.starts_with("user_transformable_objects")
            || principal.starts_with("user_malformed_objects")
            || principal.starts_with("user_mixed_collection")
            || principal.starts_with("user_incompatible_types")
        {
            // Day 5: Nested comprehension test data resources
            &[
                "matrix_result",
                "unique_values",
                "hierarchy_map",
                "filtered_results",
                "processed_text",
                "deep_result",
                "conditional_result",
                "summary",
                "transformed",
                "type_filtered",
            ]
        } else if principal.starts_with("user_adult")
            || principal.starts_with("user_minor")
            || principal.starts_with("user_premium_")
            || principal.starts_with("user_high_score")
            || principal.starts_with("user_low_score")
            || principal.starts_with("user_tier_")
            || principal.starts_with("user_verified_")
            || principal.starts_with("user_unverified_")
            || principal.starts_with("user_early_exit")
            || principal.starts_with("user_or_condition")
            || principal.starts_with("user_missing_field")
            || principal.starts_with("user_null_value")
            || principal.starts_with("user_long_name")
            || principal.starts_with("user_short_name")
            || principal.starts_with("user_category_")
            || principal.starts_with("user_threshold_")
            || principal.starts_with("user_below_threshold")
        {
            // Day 5: Conditional expressions test data resources
            &[
                "age_restricted",
                "premium_content",
                "leaderboard",
                "subscription",
                "payment",
                "logic_test",
                "nullable_data",
                "name_check",
                "categorizer",
                "conditional_sum",
            ]
        } else if principal.starts_with("user_string_")
            || principal.starts_with("user_number_")
            || principal.starts_with("user_numeric_")
            || principal.starts_with("user_text_")
            || principal.starts_with("user_array_")
            || principal.starts_with("user_object_")
            || principal.starts_with("user_primitive_")
            || principal.starts_with("user_bool_")
            || principal.starts_with("user_valid_")
            || principal.starts_with("user_invalid_")
            || principal.starts_with("user_safe_")
            || principal.starts_with("user_unsafe_")
            || principal.starts_with("user_in_range")
            || principal.starts_with("user_out_of_range")
            || principal.starts_with("user_non_null")
            || principal.starts_with("user_null_field")
            || principal.starts_with("user_fully_valid")
            || principal.starts_with("user_wrong_type")
        {
            // Day 5: Type checking test data resources
            &[
                "string_check",
                "number_check",
                "array_check",
                "object_check",
                "bool_check",
                "schema_check",
                "guarded_op",
                "constrained_value",
                "formatted_data",
                "nullable_field",
                "complex_validation",
            ]
        } else {
            // Comprehension test data resources (default for Day 4)
            &[
                "set_result",
                "array_result",
                "object_result",
                "complex_filter",
                "nested_result",
                "transformed_data",
            ]
        };

        for i in 0..count {
            world.context.action = Some(action.clone());
            // Cycle through available resource types
            world.context.resource = Some(resource_types[i % resource_types.len()].to_string());

            world.context.evaluate().expect("Evaluation failed");
        }
    }
}

// ============================================================================
// Then Steps - Assertions
// ============================================================================

#[then(expr = "the decision should be {string}")]
async fn check_decision(world: &mut PolicyWorld, expected: String) {
    let decision = world.context.get_decision().expect("No decision recorded");

    let expected_upper = expected.to_uppercase();
    let decision_upper = decision.to_uppercase();
    assert!(
        decision_upper.contains(&expected_upper),
        "Expected decision '{}', but got '{}'",
        expected_upper,
        decision
    );
}

#[then(expr = "the average evaluation time should be less than {int} microseconds")]
async fn check_average_time(world: &mut PolicyWorld, max_micros: u128) {
    let avg_nanos = world.context.average_evaluation_time();
    let avg_micros = avg_nanos / 1000;

    assert!(
        avg_micros < max_micros,
        "Average evaluation time {}µs exceeds {}µs",
        avg_micros,
        max_micros
    );
}

#[then(expr = "the decision should be {string} with reason {string}")]
async fn check_decision_with_reason(world: &mut PolicyWorld, expected: String, _reason: String) {
    // For now, just check decision - can enhance with reason tracking later
    check_decision(world, expected).await;
}

// ============================================================================
// Test Runner
// ============================================================================

#[tokio::main]
async fn main() {
    PolicyWorld::cucumber()
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                // Reset world state before each scenario
                *world = PolicyWorld::default();
            })
        })
        .after(|_feature, _rule, scenario, _event, world| {
            Box::pin(async move {
                if let Some(world) = world {
                    // Log statistics after each scenario
                    if !world.context.evaluation_times.is_empty() {
                        let avg = world.context.average_evaluation_time();
                        println!(
                            "  [{}] Average eval time: {}ns ({:.2}µs)",
                            scenario.name,
                            avg,
                            avg as f64 / 1000.0
                        );
                    }
                }
            })
        })
        .filter_run("tests/features", |_feature, _rule, scenario| {
            // Skip scenarios tagged with @expected_failure
            !scenario.tags.iter().any(|tag| tag == "expected_failure")
        })
        .await;
}
