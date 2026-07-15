// Integration test for object comprehension with filters
//
// Tests the full pipeline: parse policy, load data, compile, evaluate

use policy_engine::data::{AttributeValue, DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::PolicyEvaluator;
use policy_engine::PolicyRequest;
use std::collections::HashMap;
use std::sync::Arc;

fn main() {
    // Create data store
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Load test data with an array of objects
    let data = r#"{
        "entities": [
            {
                "id": "user_map_data",
                "type": "User",
                "attributes": {
                    "records": [
                        {"id": "rec1", "value": 100, "active": true},
                        {"id": "rec2", "value": 200, "active": false},
                        {"id": "rec3", "value": 300, "active": true}
                    ],
                    "role": "data_mapper"
                }
            },
            {
                "id": "object_result",
                "type": "Result",
                "attributes": {
                    "type": "object_result",
                    "format": "object"
                }
            }
        ]
    }"#;

    loader.load_json(data).expect("Failed to load data");
    let store_arc = Arc::new(store);
    let interner = store_arc.interner();

    // Verify data was loaded correctly
    println!("=== DATA VERIFICATION ===");
    let user_id = interner.intern("user_map_data");
    if let Some(user) = store_arc.get(user_id) {
        println!("User entity found");

        let records_key = interner.intern("records");
        if let Some(AttributeValue::List(records)) = user.get_attribute(records_key) {
            println!("Records array has {} items", records.len());
            for (i, record) in records.iter().enumerate() {
                if let AttributeValue::Object(obj) = record {
                    println!("  Record {}: {} keys", i, obj.len());

                    // Print the keys to check interning
                    for (k, v) in obj.iter() {
                        let key_name = interner.resolve(*k).unwrap_or_default();
                        println!("    Key {:?} ('{}') => {:?}", k, key_name, v);
                    }

                    // Try to look up 'active' using the interner
                    let active_key = interner.intern("active");
                    println!("    Interned 'active' key: {:?}", active_key);

                    if let Some(active_val) = obj.get(&active_key) {
                        println!("    Found 'active' via lookup: {:?}", active_val);
                    } else {
                        println!("    WARNING: 'active' NOT FOUND via obj.get()!");
                    }
                }
            }
        } else {
            println!("Records attribute not found!");
        }
    } else {
        println!("User entity NOT found!");
    }

    // Test the policy
    let policy_content = r#"
policy test_comprehension {
    default: deny,

    rule object_mapping {
        allow if {
            user.records != null &&
            resource.type == "object_result" &&
            mapping := {r.id: r.value | r := user.records[_]; r.active == true} &&
            map_count := mapping.count() &&
            map_count >= 2
        }
    }
}
"#;

    let policy: ReaperPolicy = policy_content.parse().expect("Failed to parse policy");

    // Try compiled evaluator
    println!("\n=== COMPILED EVALUATOR ===");
    let compiled_result = policy.clone().build(store_arc.clone());
    match compiled_result {
        Ok(compiled) => {
            println!("Compilation succeeded");

            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user_map_data".to_string());

            let request = PolicyRequest {
                resource: "object_result".to_string(),
                action: "transform".to_string(),
                context,

                ..Default::default()
            };

            match compiled.evaluate(&request) {
                Ok(decision) => println!("Compiled decision: {:?}", decision),
                Err(e) => println!("Compiled evaluation error: {:?}", e),
            }
        }
        Err(e) => {
            println!("Compilation failed: {:?}", e);
        }
    }

    // Try AST evaluator
    println!("\n=== AST EVALUATOR ===");
    let ast_eval = policy.build_ast_evaluator(store_arc.clone());

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_map_data".to_string());

    let request = PolicyRequest {
        resource: "object_result".to_string(),
        action: "transform".to_string(),
        context,

        ..Default::default()
    };

    match ast_eval.evaluate(&request) {
        Ok(decision) => println!("AST decision: {:?}", decision),
        Err(e) => println!("AST evaluation error: {:?}", e),
    }
}
