// Integration test for nested comprehension
use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::PolicyEvaluator;
use policy_engine::PolicyRequest;
use std::collections::HashMap;
use std::sync::Arc;

fn main() {
    // Create data store
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Load test data
    let data = r#"{
        "entities": [
            {
                "id": "user_hierarchical",
                "type": "User",
                "attributes": {
                    "categories": [
                        {"name": "cat1", "items": ["item1", "item2", "item3"]},
                        {"name": "cat2", "items": ["item4", "item5"]},
                        {"name": "cat3", "items": ["item6"]}
                    ],
                    "role": "organizer"
                }
            },
            {
                "id": "hierarchy_map",
                "type": "Result",
                "attributes": {
                    "type": "hierarchy_map"
                }
            }
        ]
    }"#;

    loader.load_json(data).expect("Failed to load data");
    let store_arc = Arc::new(store);

    // Test the policy
    let policy_content = r#"
policy test_nested {
    default: deny,

    rule hierarchy_builder {
        allow if {
            user.categories != null &&
            resource.type == "hierarchy_map" &&
            first_cat := user.categories[0] &&
            has_items := first_cat.items != null &&
            has_items == true
        }
    }
}
"#;

    let policy: ReaperPolicy = policy_content.parse().expect("Failed to parse policy");

    // Try compiled evaluator
    println!("=== COMPILED EVALUATOR ===");
    let compiled_result = policy.clone().build(store_arc.clone());
    match compiled_result {
        Ok(compiled) => {
            println!("Compilation succeeded");

            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user_hierarchical".to_string());

            let request = PolicyRequest {
                resource: "hierarchy_map".to_string(),
                action: "build".to_string(),
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
    context.insert("principal".to_string(), "user_hierarchical".to_string());

    let request = PolicyRequest {
        resource: "hierarchy_map".to_string(),
        action: "build".to_string(),
        context,

        ..Default::default()
    };

    match ast_eval.evaluate(&request) {
        Ok(decision) => println!("AST decision: {:?}", decision),
        Err(e) => println!("AST evaluation error: {:?}", e),
    }
}
