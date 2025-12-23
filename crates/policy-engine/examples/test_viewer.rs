use policy_engine::{
    DataLoader, DataStore, PolicyAction, PolicyEvaluator, PolicyRequest, ReaperPolicy,
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn main() {
    // Load data
    let data = r#"{
        "entities": [
            {"id": "user_viewer", "type": "User", "attributes": {"role": "viewer"}},
            {"id": "/api/test", "type": "Resource", "attributes": {"name": "Test"}}
        ]
    }"#;

    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(data).unwrap();
    let store = Arc::new(store);

    // Load policy
    let policy_text = r#"
policy test {
    version: "1.0.0",
    default: deny,
    
    rule viewer_read {
        allow if {
            user.role == "viewer" &&
            action == "read"
        }
    }
}
"#;

    let policy = ReaperPolicy::from_str(policy_text).unwrap();
    let evaluator = policy.build(store.clone()).unwrap();

    // Test viewer + write (should DENY)
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user_viewer".to_string());

    let request = PolicyRequest {
        resource: "/api/test".to_string(),
        action: "write".to_string(),
        context,
    };

    match evaluator.evaluate(&request) {
        Ok(decision) => {
            println!("Decision: {:?}", decision);
            if matches!(decision, PolicyAction::Deny) {
                println!("✓ PASS: viewer + write correctly denied");
            } else {
                println!(
                    "✗ FAIL: viewer + write should be denied, got {:?}",
                    decision
                );
            }
        }
        Err(e) => {
            println!("✗ ERROR: {}", e);
        }
    }
}
