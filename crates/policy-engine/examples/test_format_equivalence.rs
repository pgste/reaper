/// Test Format Equivalence
///
/// Verifies that .reap, YAML, and JSON policies produce identical evaluation results

use policy_engine::{DataStore, DataLoader, ReaperPolicy, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Format Equivalence Test - RBAC Policy\n");
    println!("{}", "=".repeat(70));

    // Load test data
    println!("\n📊 Loading RBAC test data...");
    let data_content = std::fs::read_to_string("rbac-test-data.json")?;
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   ✓ Loaded {} entities", entity_count);

    // Load all three policy formats
    println!("\n📜 Loading policies in all formats...");

    let policy_reap = ReaperPolicy::from_file("crates/policy-engine/examples/policies/rbac.reap")?;
    println!("   ✓ Loaded rbac.reap");

    let policy_yaml = ReaperPolicy::from_yaml_file("crates/policy-engine/examples/policies/rbac.yaml")?;
    println!("   ✓ Loaded rbac.yaml");

    let policy_json = ReaperPolicy::from_json_file("crates/policy-engine/examples/policies/rbac.json")?;
    println!("   ✓ Loaded rbac.json");

    // Compile all three
    println!("\n🔧 Compiling policies...");
    let eval_reap = policy_reap.build(store.clone())?;
    let eval_yaml = policy_yaml.build(store.clone())?;
    let eval_json = policy_json.build(store.clone())?;
    println!("   ✓ All policies compiled successfully");

    // Test scenarios
    let test_cases = vec![
        ("Admin access", "user_0", "resource_100", "read"),
        ("Manager report access", "user_1", "resource_50", "read"),
        ("User own resource", "user_50", "resource_50", "read"),
        ("User other resource", "user_50", "resource_51", "read"),
        ("No role match", "user_700", "resource_900", "read"),
    ];

    println!("\n🧪 Running {} test scenarios...\n", test_cases.len());

    let mut all_match = true;

    for (scenario, user_id, resource_id, action) in &test_cases {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), user_id.to_string());

        let request = PolicyRequest {
            resource: resource_id.to_string(),
            action: action.to_string(),
            context,
        };

        // Evaluate with all three formats
        let decision_reap = eval_reap.evaluate(&request)?;
        let decision_yaml = eval_yaml.evaluate(&request)?;
        let decision_json = eval_json.evaluate(&request)?;

        // Compare results
        let reap_str = format!("{:?}", decision_reap);
        let yaml_str = format!("{:?}", decision_yaml);
        let json_str = format!("{:?}", decision_json);

        let matches = reap_str == yaml_str && yaml_str == json_str;

        if matches {
            println!("✅ {}", scenario);
            println!("   All formats agree: {}", reap_str);
        } else {
            println!("❌ {} - MISMATCH!", scenario);
            println!("   .reap: {}", reap_str);
            println!("   .yaml: {}", yaml_str);
            println!("   .json: {}", json_str);
            all_match = false;
        }
        println!();
    }

    println!("{}", "=".repeat(70));
    if all_match {
        println!("✅ SUCCESS: All formats produce identical results!");
        println!("\nThe following formats are equivalent:");
        println!("  • .reap (native Reaper DSL)");
        println!("  • .yaml (YAML policy format)");
        println!("  • .json (JSON policy format)");
        println!("\nAll three compile to the same AST and produce identical decisions.");
    } else {
        println!("❌ FAILURE: Formats produced different results!");
        return Err("Format equivalence test failed".into());
    }
    println!("{}", "=".repeat(70));

    Ok(())
}
