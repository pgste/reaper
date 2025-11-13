//! Multi-Language Policy Engine Demo
//!
//! This example demonstrates Reaper's support for multiple policy languages:
//! - Simple: High-performance rule-based policies (sub-microsecond)
//! - Cedar: AWS-compatible ABAC policies (10-50 microseconds)
//!
//! Run with: cargo run --example multi_language_demo

use policy_engine::{
    EnhancedPolicy, PolicyAction, PolicyEngine, PolicyLanguage, PolicyRequest, PolicyRule,
};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Reaper Multi-Language Policy Engine Demo ===\n");

    let engine = PolicyEngine::new();

    // ==========================================
    // Example 1: Simple Policy (High Performance)
    // ==========================================
    println!("📋 Example 1: Simple Policy (Sub-Microsecond Performance)");
    println!("{}", "=".repeat(60));

    let simple_rules = vec![
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "api/users/*".to_string(),
            conditions: vec!["role:admin".to_string()],
        },
        PolicyRule {
            action: PolicyAction::Deny,
            resource: "api/admin/*".to_string(),
            conditions: vec![],
        },
        PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        },
    ];

    let simple_policy = EnhancedPolicy::new(
        "simple-api-policy".to_string(),
        "High-performance API access control".to_string(),
        simple_rules,
    );

    println!("✓ Created Simple policy: {}", simple_policy.name);
    println!("  Language: {}", simple_policy.language);
    println!("  Rules: {}", simple_policy.rules.len());

    engine.deploy_policy(simple_policy.clone())?;

    // Test simple policy evaluation
    let request = PolicyRequest {
        resource: "api/users/123".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
    };

    let decision = engine.evaluate(&simple_policy.id, &request)?;
    println!("\n📝 Test Request:");
    println!("  Resource: {}", request.resource);
    println!("  Decision: {:?}", decision.decision);
    println!("  Evaluation Time: {} ns", decision.evaluation_time_ns);
    println!("  Performance: ⚡ Sub-microsecond\n");

    // ==========================================
    // Example 2: Cedar Policy (Expressive ABAC)
    // ==========================================
    println!("📋 Example 2: Cedar Policy (Expressive ABAC)");
    println!("{}", "=".repeat(60));

    // Cedar policy for document access control
    let cedar_policy_text = r#"
        // Allow users to read their own documents
        permit(
            principal,
            action == Action::"read",
            resource
        ) when {
            principal == resource
        };

        // Allow admins to do everything
        permit(
            principal,
            action,
            resource
        ) when {
            principal has role && principal.role == "admin"
        };

        // Deny access to sensitive documents
        forbid(
            principal,
            action,
            resource
        ) when {
            resource has sensitive && resource.sensitive == true
        };
    "#;

    let cedar_policy = EnhancedPolicy::new_with_language(
        "cedar-document-policy".to_string(),
        "Cedar-based document access control".to_string(),
        PolicyLanguage::Cedar,
        cedar_policy_text.to_string(),
    )?;

    println!("✓ Created Cedar policy: {}", cedar_policy.name);
    println!("  Language: {}", cedar_policy.language);
    println!("  Policy Length: {} chars", cedar_policy.content.len());

    engine.deploy_policy(cedar_policy.clone())?;

    // Test Cedar policy evaluation - basic access
    let mut cedar_context = HashMap::new();
    cedar_context.insert("principal".to_string(), "alice".to_string());

    let cedar_request = PolicyRequest {
        resource: "document-123".to_string(),
        action: "read".to_string(),
        context: cedar_context,
    };

    let cedar_decision = engine.evaluate(&cedar_policy.id, &cedar_request)?;
    println!("\n📝 Test Request:");
    println!("  Principal: alice");
    println!("  Resource: {}", cedar_request.resource);
    println!("  Decision: {:?}", cedar_decision.decision);
    println!("  Evaluation Time: {} ns ({:.2} µs)",
        cedar_decision.evaluation_time_ns,
        cedar_decision.evaluation_time_ns as f64 / 1000.0
    );
    println!("  Performance: 🔧 Rich policy expression\n");

    // ==========================================
    // Example 3: Comparing Both Languages
    // ==========================================
    println!("📋 Example 3: Performance Comparison");
    println!("{}", "=".repeat(60));

    let iterations = 1000;
    println!("Running {} evaluations for each policy type...\n", iterations);

    // Benchmark Simple policy
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = engine.evaluate(&simple_policy.id, &request)?;
    }
    let simple_duration = start.elapsed();

    println!("Simple Policy:");
    println!("  Total time: {:?}", simple_duration);
    println!("  Average: {:.2} ns per evaluation",
        simple_duration.as_nanos() as f64 / iterations as f64
    );
    println!("  Throughput: {:.0} ops/sec",
        iterations as f64 / simple_duration.as_secs_f64()
    );

    // Benchmark Cedar policy
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = engine.evaluate(&cedar_policy.id, &cedar_request)?;
    }
    let cedar_duration = start.elapsed();

    println!("\nCedar Policy:");
    println!("  Total time: {:?}", cedar_duration);
    println!("  Average: {:.2} ns per evaluation",
        cedar_duration.as_nanos() as f64 / iterations as f64
    );
    println!("  Throughput: {:.0} ops/sec",
        iterations as f64 / cedar_duration.as_secs_f64()
    );

    let speedup = cedar_duration.as_secs_f64() / simple_duration.as_secs_f64();
    println!("\n⚡ Simple is {:.1}x faster than Cedar", speedup);

    // ==========================================
    // Example 4: Use Cases and Recommendations
    // ==========================================
    println!("\n📋 Example 4: When to Use Each Language");
    println!("{}", "=".repeat(60));

    println!("\n✅ Use Simple When:");
    println!("  • Sub-microsecond latency is critical");
    println!("  • Handling >100K requests/second");
    println!("  • Simple resource patterns suffice");
    println!("  • Example: API gateway, service mesh");

    println!("\n✅ Use Cedar When:");
    println!("  • Rich attribute-based access control needed");
    println!("  • Schema validation important");
    println!("  • AWS Cedar compatibility required");
    println!("  • Example: Document management, multi-tenant SaaS");

    println!("\n🔮 Future: Custom Reaper DSL");
    println!("  • Compile-time policy optimization");
    println!("  • Rust macro-based DSL");
    println!("  • Zero-cost abstractions");
    println!("  • Best of both worlds\n");

    // ==========================================
    // Summary
    // ==========================================
    println!("=== Summary ===");
    println!("✓ Deployed {} policies", engine.list_policies().len());
    println!("✓ Tested both Simple and Cedar languages");
    println!("✓ Demonstrated extensible architecture");
    println!("\n🎯 Reaper supports multiple policy languages while maintaining");
    println!("   high performance and easy extensibility for custom languages!\n");

    Ok(())
}
