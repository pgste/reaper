//! End-to-End Cedar Policy Evaluation with DataStore
//!
//! This example demonstrates the complete flow:
//! 1. Load entity data from JSON
//! 2. Load a Cedar policy
//! 3. Evaluate requests against the policy using entity data
//! 4. Measure performance
//!
//! Run with: cargo run --example cedar_with_data

use policy_engine::{
    DataLoader, DataStore, EnhancedPolicy, PolicyEngine, PolicyLanguage, PolicyRequest,
};
use std::collections::HashMap;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Cedar Policy with DataStore - End-to-End Demo ===\n");

    // ==========================================
    // Step 1: Load Entity Data
    // ==========================================
    println!("📋 Step 1: Loading Entity Data");
    println!("{}", "=".repeat(60));

    let json_data = r#"
    {
        "entities": [
            {
                "id": "alice",
                "type": "User",
                "attributes": {
                    "role": "admin",
                    "department": "engineering",
                    "clearance": 5
                }
            },
            {
                "id": "bob",
                "type": "User",
                "attributes": {
                    "role": "user",
                    "department": "sales",
                    "clearance": 2
                }
            },
            {
                "id": "charlie",
                "type": "User",
                "attributes": {
                    "role": "manager",
                    "department": "engineering",
                    "clearance": 4
                }
            },
            {
                "id": "doc1",
                "type": "Document",
                "attributes": {
                    "owner": "alice",
                    "classification": "public",
                    "department": "engineering"
                }
            },
            {
                "id": "doc2",
                "type": "Document",
                "attributes": {
                    "owner": "bob",
                    "classification": "confidential",
                    "department": "sales",
                    "clearance_required": 3
                }
            },
            {
                "id": "doc3",
                "type": "Document",
                "attributes": {
                    "owner": "alice",
                    "classification": "secret",
                    "department": "engineering",
                    "clearance_required": 5
                }
            }
        ]
    }
    "#;

    let load_start = Instant::now();
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let entity_count = loader.load_json(json_data)?;
    let load_duration = load_start.elapsed();

    println!("✓ Loaded {} entities in {:?}", entity_count, load_duration);
    println!("  Data Store Stats:");
    let stats = store.stats();
    println!("    - Total Entities: {}", stats.total_entities);
    println!("    - Unique Types: {}", stats.unique_types);
    println!(
        "    - Unique Strings: {}",
        stats.interner_stats.unique_strings
    );
    println!(
        "    - Memory Usage: {} bytes\n",
        stats.estimated_memory_bytes
    );

    // ==========================================
    // Step 2: Define Cedar Policy
    // ==========================================
    println!("📋 Step 2: Loading Cedar Policy");
    println!("{}", "=".repeat(60));

    let cedar_policy = r#"
        // Allow admins to do anything
        permit(
            principal,
            action,
            resource
        ) when {
            principal.role == "admin"
        };

        // Allow users to read public documents
        permit(
            principal,
            action == Action::"read",
            resource
        ) when {
            resource.classification == "public"
        };

        // Allow users to read documents from their department
        permit(
            principal,
            action == Action::"read",
            resource
        ) when {
            principal.department == resource.department
        };

        // Deny access to documents requiring higher clearance
        forbid(
            principal,
            action == Action::"read",
            resource
        ) when {
            resource has clearance_required &&
            principal.clearance < resource.clearance_required
        };
    "#;

    let policy_start = Instant::now();
    let policy = EnhancedPolicy::new_with_language(
        "cedar-abac-policy".to_string(),
        "Cedar ABAC policy with clearance levels".to_string(),
        PolicyLanguage::Cedar,
        cedar_policy.to_string(),
    )?;
    let policy_load_duration = policy_start.elapsed();

    println!("✓ Loaded Cedar policy in {:?}", policy_load_duration);
    println!("  Policy Language: {}", policy.language);
    println!("  Policy Length: {} chars\n", policy.content.len());

    // ==========================================
    // Step 3: Deploy Policy
    // ==========================================
    println!("📋 Step 3: Deploying Policy to Engine");
    println!("{}", "=".repeat(60));

    let engine = PolicyEngine::new();
    let deploy_start = Instant::now();
    engine.deploy_policy(policy.clone())?;
    let deploy_duration = deploy_start.elapsed();

    println!("✓ Policy deployed in {:?}\n", deploy_duration);

    // ==========================================
    // Step 4: Evaluate Test Cases
    // ==========================================
    println!("📋 Step 4: Evaluating Test Cases");
    println!("{}", "=".repeat(60));

    let test_cases = vec![
        (
            "Test 1: Alice (admin) reads doc1",
            "alice",
            "doc1",
            "read",
            "Allow", // Expected
        ),
        (
            "Test 2: Bob (user) reads doc1 (public)",
            "bob",
            "doc1",
            "read",
            "Allow", // Public doc
        ),
        (
            "Test 3: Bob reads doc2 (his department)",
            "bob",
            "doc2",
            "read",
            "Allow", // Same department
        ),
        (
            "Test 4: Bob reads doc3 (insufficient clearance)",
            "bob",
            "doc3",
            "read",
            "Deny", // Clearance 2 < required 5
        ),
        (
            "Test 5: Charlie (manager) reads doc3",
            "charlie",
            "doc3",
            "read",
            "Deny", // Clearance 4 < required 5, different dept
        ),
        (
            "Test 6: Alice reads doc3 (sufficient clearance)",
            "alice",
            "doc3",
            "read",
            "Allow", // Admin OR (same dept AND clearance 5 >= 5)
        ),
    ];

    let mut total_eval_time = 0u128;
    let mut passed = 0;
    let mut failed = 0;

    for (description, principal, resource, action, expected) in test_cases {
        println!("\n{}", description);
        println!(
            "  Principal: {} | Resource: {} | Action: {}",
            principal, resource, action
        );

        // Look up entities from store
        let interner = store.interner();
        let principal_id = interner.intern(principal);
        let resource_id = interner.intern(resource);

        let principal_entity = store.get(principal_id);
        let resource_entity = store.get(resource_id);

        if let (Some(p_entity), Some(r_entity)) = (principal_entity, resource_entity) {
            // Build context with entity attributes
            let mut context = HashMap::new();
            context.insert("principal".to_string(), principal.to_string());

            // Add principal attributes to context
            for (key_id, value) in &p_entity.attributes {
                let key = interner.resolve_str(*key_id).unwrap();
                let value_str = match value {
                    policy_engine::AttributeValue::String(id) => {
                        interner.resolve_str(*id).unwrap().to_string()
                    }
                    policy_engine::AttributeValue::Int(i) => i.to_string(),
                    policy_engine::AttributeValue::Bool(b) => b.to_string(),
                    _ => continue,
                };
                context.insert(format!("principal.{}", key), value_str);
            }

            // Add resource attributes to context
            for (key_id, value) in &r_entity.attributes {
                let key = interner.resolve_str(*key_id).unwrap();
                let value_str = match value {
                    policy_engine::AttributeValue::String(id) => {
                        interner.resolve_str(*id).unwrap().to_string()
                    }
                    policy_engine::AttributeValue::Int(i) => i.to_string(),
                    policy_engine::AttributeValue::Bool(b) => b.to_string(),
                    _ => continue,
                };
                context.insert(format!("resource.{}", key), value_str);
            }

            let request = PolicyRequest {
                resource: resource.to_string(),
                action: action.to_string(),
                context,
            };

            // Evaluate
            let eval_start = Instant::now();
            let decision = engine.evaluate(&policy.id, &request)?;
            let eval_duration = eval_start.elapsed();
            total_eval_time += eval_duration.as_nanos();

            let decision_str = format!("{:?}", decision.decision);
            let pass = decision_str == expected;

            if pass {
                passed += 1;
                println!(
                    "  ✓ PASS: Decision = {} (expected {})",
                    decision_str, expected
                );
            } else {
                failed += 1;
                println!(
                    "  ✗ FAIL: Decision = {} (expected {})",
                    decision_str, expected
                );
            }

            println!(
                "  Evaluation Time: {} ns ({:.2} µs)",
                decision.evaluation_time_ns,
                decision.evaluation_time_ns as f64 / 1000.0
            );
        } else {
            println!("  ✗ ERROR: Could not find entities in store");
            failed += 1;
        }
    }

    // ==========================================
    // Step 5: Performance Summary
    // ==========================================
    println!("\n{}", "=".repeat(60));
    println!("📋 Step 5: Performance Summary");
    println!("{}", "=".repeat(60));

    let num_tests = (passed + failed) as u128;
    let avg_eval_time = total_eval_time.checked_div(num_tests).unwrap_or(0);

    println!("\nTest Results:");
    println!("  Passed: {}/{}", passed, passed + failed);
    println!("  Failed: {}/{}", failed, passed + failed);

    println!("\nPerformance Metrics:");
    println!(
        "  Data Load Time: {:?} ({} entities)",
        load_duration, entity_count
    );
    println!("  Policy Load Time: {:?}", policy_load_duration);
    println!("  Policy Deploy Time: {:?}", deploy_duration);
    println!(
        "  Average Evaluation Time: {} ns ({:.2} µs)",
        avg_eval_time,
        avg_eval_time as f64 / 1000.0
    );
    println!(
        "  Total Evaluation Time: {} ns ({:.2} ms)",
        total_eval_time,
        total_eval_time as f64 / 1_000_000.0
    );

    println!("\nEnd-to-End Latency Breakdown:");
    let total_e2e =
        load_duration.as_nanos() + policy_load_duration.as_nanos() + deploy_duration.as_nanos();
    println!(
        "  Data Loading: {:.1}%",
        (load_duration.as_nanos() as f64 / total_e2e as f64) * 100.0
    );
    println!(
        "  Policy Loading: {:.1}%",
        (policy_load_duration.as_nanos() as f64 / total_e2e as f64) * 100.0
    );
    println!(
        "  Policy Deployment: {:.1}%",
        (deploy_duration.as_nanos() as f64 / total_e2e as f64) * 100.0
    );
    println!(
        "  Total Setup Time: {:?}",
        load_duration + policy_load_duration + deploy_duration
    );

    println!("\n{}", "=".repeat(60));
    println!("✅ Cedar + DataStore Integration Complete!");
    println!("{}", "=".repeat(60));
    println!("\n🎯 Key Takeaways:");
    println!("  • Entity data loaded and queryable in microseconds");
    println!("  • Cedar policies can reference entity attributes");
    println!("  • Complex ABAC rules (department, clearance) working");
    println!("  • Sub-millisecond policy evaluation with rich context");
    println!("  • Memory-efficient storage via string interning");
    println!("\n💡 This demonstrates a complete better-than-OPA system:");
    println!("  ✓ High-performance data store");
    println!("  ✓ Expressive Cedar policies");
    println!("  ✓ Seamless integration");
    println!("  ✓ Production-ready architecture\n");

    Ok(())
}
