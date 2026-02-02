//! Debug test for first() collection method

use std::sync::Arc;
use std::collections::HashMap;
use policy_engine::{
    DataLoader, DataStore, ReaperPolicy, PolicyRequest, PolicyEvaluator, PolicyAction,
    reap::ReapParser,
};

#[test]
fn test_parse_first_method_ast() {
    let policy_content = r#"
policy test_first {
    default: deny,
    rule first_task_priority {
        allow if {
            user.tasks != null &&
            resource.type == "task_queue" &&
            first_task := user.tasks.first() &&
            first_task == "urgent"
        }
    }
}"#;

    let ast = ReapParser::parse(policy_content).expect("parse policy");
    eprintln!("Policy AST: {:#?}", ast);
}

#[test]
fn test_type_checking_bdd_scenario() {
    // Reproduce the failing BDD test: user_string_field / string_check
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Load the same data as the BDD test
    let data = std::fs::read_to_string("/Volumes/PGS-E/Code/reaper/test-data/type-checking-test-data.json")
        .expect("read data file");
    loader.load_json(&data).expect("load data");

    let store_arc = Arc::new(store);

    // Load the same policy
    let policy = ReaperPolicy::from_file_auto("/Volumes/PGS-E/Code/reaper/crates/policy-engine/examples/policies/type_checking_policy.reap")
        .expect("load policy");

    eprintln!("Policy name: {}", policy.name());

    // Force AST evaluator to test
    let ast_evaluator = policy.clone().build_ast_evaluator(store_arc.clone());
    eprintln!("Testing AST evaluator first...");

    let mut context_ast = HashMap::new();
    context_ast.insert("principal".to_string(), "user_string_field".to_string());
    let request_ast = PolicyRequest {
        resource: "string_check".to_string(),
        action: "validate".to_string(),
        context: context_ast,
    };
    let ast_result = ast_evaluator.evaluate(&request_ast);
    eprintln!("AST result: {:?}", ast_result);

    // Now try compiled
    let evaluator: Box<dyn PolicyEvaluator> = match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            eprintln!("Using COMPILED evaluator");
            Box::new(compiled)
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}, using AST evaluator", e);
            Box::new(policy.build_ast_evaluator(store_arc.clone()))
        }
    };

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_string_field".to_string());

    let request = PolicyRequest {
        resource: "string_check".to_string(),
        action: "validate".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request);
    eprintln!("Type checking result: {:?}", result);
}

#[test]
fn test_string_count_method() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_string", "type": "User", "attributes": {"data": "hello world"}},
            {"id": "string_check", "type": "Resource", "attributes": {"type": "string_check"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    // Policy that checks string.count() > 0
    let policy_content = r#"
policy test_string {
    default: deny,
    rule string_check {
        allow if {
            user.data != null &&
            resource.type == "string_check" &&
            has_length := user.data.count() &&
            has_length > 0
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");
    let ast_evaluator = policy.build_ast_evaluator(store_arc.clone());

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_string".to_string());

    let request = PolicyRequest {
        resource: "string_check".to_string(),
        action: "validate".to_string(),
        context,
    };

    let result = ast_evaluator.evaluate(&request);
    eprintln!("String count result: {:?}", result);
    match result {
        Ok(decision) => eprintln!("Decision: {:?}", decision),
        Err(e) => eprintln!("Error: {:?}", e),
    }
}

#[test]
fn test_two_level_nested_access() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_json", "type": "User", "attributes": {"payload": {"valid": true, "data": "test"}}},
            {"id": "api_endpoint", "type": "Resource", "attributes": {"type": "api_endpoint"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");

    // Debug: Print the payload attribute
    let interner = store.interner();
    let user_id = interner.intern("user_json");
    let user = store.get(user_id).expect("user entity");
    let payload_attr = interner.intern("payload");
    let payload = user.get_attribute(payload_attr);
    eprintln!("Payload attribute: {:?}", payload);

    let store_arc = Arc::new(store);

    // Policy that checks two-level nested access
    let policy_content = r#"
policy test_two_level {
    default: deny,
    rule json_valid {
        allow if {
            user.payload != null &&
            resource.type == "api_endpoint" &&
            user.payload.valid == true
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");
    let ast_evaluator = policy.build_ast_evaluator(store_arc.clone());

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_json".to_string());

    let request = PolicyRequest {
        resource: "api_endpoint".to_string(),
        action: "submit".to_string(),
        context,
    };

    let result = ast_evaluator.evaluate(&request);
    eprintln!("Two-level nested result: {:?}", result);
    match result {
        Ok(decision) => {
            eprintln!("Decision: {:?}", decision);
            assert!(matches!(decision, PolicyAction::Allow), "Expected Allow but got {:?}", decision);
        },
        Err(e) => panic!("Error: {:?}", e),
    }
}

#[test]
fn test_nested_object_access() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_obj", "type": "User", "attributes": {"config": {"name": "test", "value": 42}}},
            {"id": "object_check", "type": "Resource", "attributes": {"type": "object_check"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");

    // Debug: Print the config attribute
    let interner = store.interner();
    let user_id = interner.intern("user_obj");
    let user = store.get(user_id).expect("user entity");
    let config_attr = interner.intern("config");
    let config = user.get_attribute(config_attr);
    eprintln!("Config attribute: {:?}", config);

    let store_arc = Arc::new(store);

    // Policy that checks nested object access
    let policy_content = r#"
policy test_nested {
    default: deny,
    rule object_check {
        allow if {
            user.config != null &&
            resource.type == "object_check" &&
            user.config.name != null
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");
    let ast_evaluator = policy.build_ast_evaluator(store_arc.clone());

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_obj".to_string());

    let request = PolicyRequest {
        resource: "object_check".to_string(),
        action: "validate".to_string(),
        context,
    };

    let result = ast_evaluator.evaluate(&request);
    eprintln!("Nested object result: {:?}", result);
}

#[test]
fn test_nested_object_access_compiled() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_obj", "type": "User", "attributes": {"config": {"name": "test", "value": 42}}},
            {"id": "object_check", "type": "Resource", "attributes": {"type": "object_check"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    // Policy that checks nested object access
    let policy_content = r#"
policy test_nested {
    default: deny,
    rule object_check {
        allow if {
            user.config != null &&
            resource.type == "object_check" &&
            user.config.name != null
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");

    // Try compiled evaluator
    eprintln!("Trying to compile...");
    match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            eprintln!("Compiled successfully");
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user_obj".to_string());

            let request = PolicyRequest {
                resource: "object_check".to_string(),
                action: "validate".to_string(),
                context,
            };

            let result = compiled.evaluate(&request);
            eprintln!("Compiled nested object result: {:?}", result);
            assert!(matches!(result, Ok(PolicyAction::Allow)), "Expected Allow but got {:?}", result);
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}", e);
            panic!("Compilation should succeed");
        }
    }
}

#[test]
fn test_null_safety_compiled() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Mimics the failing null_safety test
    let data = r#"{
        "entities": [
            {"id": "user_non_null", "type": "User", "attributes": {"required_field": "value present"}},
            {"id": "nullable_field", "type": "Resource", "attributes": {"type": "nullable_field"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    let policy_content = r#"
policy test_null_safety {
    default: deny,
    rule null_safety {
        allow if {
            resource.type == "nullable_field" &&
            user.required_field != null &&
            value_exists := user.required_field != "" &&
            value_exists == true
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");

    // Test AST first
    eprintln!("Testing AST evaluator...");
    let ast_eval = policy.clone().build_ast_evaluator(store_arc.clone());
    let mut context_ast = HashMap::new();
    context_ast.insert("principal".to_string(), "user_non_null".to_string());
    let request_ast = PolicyRequest {
        resource: "nullable_field".to_string(),
        action: "access".to_string(),
        context: context_ast,
    };
    let ast_result = ast_eval.evaluate(&request_ast);
    eprintln!("AST null_safety result: {:?}", ast_result);

    // Now try compiled evaluator
    eprintln!("\nTrying to compile...");
    match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            eprintln!("Compiled successfully");
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user_non_null".to_string());

            let request = PolicyRequest {
                resource: "nullable_field".to_string(),
                action: "access".to_string(),
                context,
            };

            let result = compiled.evaluate(&request);
            eprintln!("Compiled null_safety result: {:?}", result);
            assert!(matches!(result, Ok(PolicyAction::Allow)), "Expected Allow but got {:?}", result);
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}", e);
            // This is ok if compilation fails - the BDD test would use AST
            // But the AST result should still be Allow
            assert!(matches!(ast_result, Ok(PolicyAction::Allow)), "AST Expected Allow but got {:?}", ast_result);
        }
    }
}

#[test]
fn test_string_not_empty_compiled() {
    // Simplified test to isolate the string != "" issue
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "test_user", "type": "User", "attributes": {"name": "hello"}},
            {"id": "test_resource", "type": "Resource", "attributes": {"type": "test"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    // Simple policy: user.name != ""
    let policy_content = r#"
policy test_not_empty {
    default: deny,
    rule check_not_empty {
        allow if {
            resource.type == "test" &&
            user.name != ""
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");

    // Test AST first
    let ast_eval = policy.clone().build_ast_evaluator(store_arc.clone());
    let mut context_ast = HashMap::new();
    context_ast.insert("principal".to_string(), "test_user".to_string());
    let request_ast = PolicyRequest {
        resource: "test_resource".to_string(),
        action: "test".to_string(),
        context: context_ast,
    };
    let ast_result = ast_eval.evaluate(&request_ast);
    eprintln!("AST string != '' result: {:?}", ast_result);

    // Now compiled
    match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "test_user".to_string());
            let request = PolicyRequest {
                resource: "test_resource".to_string(),
                action: "test".to_string(),
                context,
            };
            let result = compiled.evaluate(&request);
            eprintln!("Compiled string != '' result: {:?}", result);
            assert!(matches!(result, Ok(PolicyAction::Allow)), "Expected Allow but got {:?}", result);
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}", e);
        }
    }
}

#[test]
fn test_assignment_comparison_compiled() {
    // Test assignment of comparison result
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "test_user", "type": "User", "attributes": {"name": "hello"}},
            {"id": "test_resource", "type": "Resource", "attributes": {"type": "test"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    // Policy with assignment: is_valid := user.name != "" && is_valid == true
    let policy_content = r#"
policy test_assignment {
    default: deny,
    rule check_assignment {
        allow if {
            resource.type == "test" &&
            is_valid := user.name != "" &&
            is_valid == true
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");

    // Test AST first
    let ast_eval = policy.clone().build_ast_evaluator(store_arc.clone());
    let mut context_ast = HashMap::new();
    context_ast.insert("principal".to_string(), "test_user".to_string());
    let request_ast = PolicyRequest {
        resource: "test_resource".to_string(),
        action: "test".to_string(),
        context: context_ast,
    };
    let ast_result = ast_eval.evaluate(&request_ast);
    eprintln!("AST assignment result: {:?}", ast_result);

    // Now compiled
    match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "test_user".to_string());
            let request = PolicyRequest {
                resource: "test_resource".to_string(),
                action: "test".to_string(),
                context,
            };
            let result = compiled.evaluate(&request);
            eprintln!("Compiled assignment result: {:?}", result);
            assert!(matches!(result, Ok(PolicyAction::Allow)), "Expected Allow but got {:?}", result);
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}", e);
            panic!("Compilation should succeed");
        }
    }
}

#[test]
fn test_bdd_object_validation() {
    // Use the actual BDD test data and policy
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Load the same data as the BDD test
    let data = std::fs::read_to_string("/Volumes/PGS-E/Code/reaper/test-data/type-checking-test-data.json")
        .expect("read data file");
    loader.load_json(&data).expect("load data");

    let store_arc = Arc::new(store);

    // Load the same policy
    let policy = ReaperPolicy::from_file_auto("/Volumes/PGS-E/Code/reaper/crates/policy-engine/examples/policies/type_checking_policy.reap")
        .expect("load policy");

    // Test AST first
    let ast_eval = policy.clone().build_ast_evaluator(store_arc.clone());
    let mut context_ast = HashMap::new();
    context_ast.insert("principal".to_string(), "user_object_value".to_string());
    let request_ast = PolicyRequest {
        resource: "object_check".to_string(),
        action: "validate".to_string(),
        context: context_ast,
    };
    let ast_result = ast_eval.evaluate(&request_ast);
    eprintln!("AST object_validation result: {:?}", ast_result);

    // Now compiled
    match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => {
            eprintln!("Compiled successfully");
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user_object_value".to_string());
            let request = PolicyRequest {
                resource: "object_check".to_string(),
                action: "validate".to_string(),
                context,
            };
            let result = compiled.evaluate(&request);
            eprintln!("Compiled object_validation result: {:?}", result);
        }
        Err(e) => {
            eprintln!("Compilation failed: {:?}, falling back to AST", e);
        }
    }
}

#[test]
fn test_nested_attr_null_check_ast() {
    // Simpler test for nested attribute null check
    let policy_content = r#"
policy test_nested_null {
    default: deny,
    rule check_nested {
        allow if {
            user.config != null &&
            user.config.name != null
        }
    }
}"#;

    let ast = ReapParser::parse(policy_content).expect("parse policy");
    eprintln!("Policy AST: {:#?}", ast);
}

#[test]
fn test_first_collection_method_compiled() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_priority_tasks", "type": "User", "attributes": {"tasks": ["urgent", "normal", "low"]}},
            {"id": "task_queue", "type": "Queue", "attributes": {"type": "task_queue"}}
        ]
    }"#;

    loader.load_json(data).expect("load data");
    let store_arc = Arc::new(store);

    let policy_content = r#"
policy test_first {
    default: deny,
    rule first_task_priority {
        allow if {
            user.tasks != null &&
            resource.type == "task_queue" &&
            first_task := user.tasks.first() &&
            first_task == "urgent"
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");

    // Try compiled evaluator
    let result = policy.clone().build(store_arc.clone());
    eprintln!("Compilation result: {:?}", result.is_ok());

    if let Err(ref e) = result {
        eprintln!("Compilation error: {:?}", e);
    }

    let compiled = result.expect("should compile successfully");

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_priority_tasks".to_string());

    let request = PolicyRequest {
        resource: "task_queue".to_string(),
        action: "check".to_string(),
        context,
    };

    let decision = compiled.evaluate(&request).expect("evaluation failed");
    eprintln!("Decision: {:?}", decision);
    assert!(matches!(decision, PolicyAction::Allow), "Expected Allow but got {:?}", decision);
}

#[test]
fn test_first_collection_method_ast() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let data = r#"{
        "entities": [
            {"id": "user_priority_tasks", "type": "User", "attributes": {"tasks": ["urgent", "normal", "low"]}},
            {"id": "task_queue", "type": "Queue", "attributes": {"type": "task_queue"}}
        ]
    }"#;

    let count = loader.load_json(data).expect("load data");
    eprintln!("Loaded {} entities", count);

    // Debug: check if we can retrieve the entity
    let interner = store.interner();
    let user_id = interner.intern("user_priority_tasks");
    eprintln!("User ID (interned): {:?}", user_id);

    let user_entity = store.get(user_id);
    eprintln!("User entity found: {:?}", user_entity.is_some());
    if let Some(ref entity) = user_entity {
        let tasks_attr = interner.intern("tasks");
        let tasks = entity.get_attribute(tasks_attr);
        eprintln!("Tasks attribute: {:?}", tasks);

        // Try to resolve the interned strings
        if let Some(policy_engine::AttributeValue::Set(set)) = tasks {
            eprintln!("Set iteration order:");
            for (idx, item) in set.iter().enumerate() {
                if let policy_engine::AttributeValue::String(s) = item {
                    eprintln!("  [{}]: {:?} = {:?}", idx, s, interner.resolve(*s));
                }
            }
        }
    }

    let store_arc = Arc::new(store);

    let policy_content = r#"
policy test_first {
    default: deny,
    rule first_task_priority {
        allow if {
            user.tasks != null &&
            resource.type == "task_queue" &&
            first_task := user.tasks.first() &&
            first_task == "urgent"
        }
    }
}"#;

    let policy: ReaperPolicy = policy_content.parse().expect("parse policy");
    let ast_evaluator = policy.build_ast_evaluator(store_arc.clone());

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_priority_tasks".to_string());

    let request = PolicyRequest {
        resource: "task_queue".to_string(),
        action: "check".to_string(),
        context,
    };

    let decision = ast_evaluator.evaluate(&request).expect("ast evaluation failed");
    eprintln!("AST Decision: {:?}", decision);
    assert!(matches!(decision, PolicyAction::Allow), "Expected Allow but got {:?}", decision);
}
