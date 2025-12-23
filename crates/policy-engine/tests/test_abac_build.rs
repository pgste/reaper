use policy_engine::{DataStore, ReaperPolicy};
use std::str::FromStr;
use std::sync::Arc;

#[test]
fn test_build_abac_evaluator() {
    let policy_text = r#"
policy abac_benchmark {
    version: "1.0.0",
    description: "ABAC benchmark",
    default: deny,

    rule deny_suspended {
        deny if user.suspended == true
    }

    rule admin_access {
        allow if user.role == "admin"
    }

    rule department_clearance {
        allow if {
            user.department == resource.department &&
            user.clearance_level >= resource.clearance_level
        }
    }
}
"#;

    let policy = ReaperPolicy::from_str(policy_text).expect("Failed to parse policy");
    let store = Arc::new(DataStore::new());

    let _evaluator = policy.build(store).expect("Failed to build evaluator");

    println!("\n✓ Evaluator built successfully!");
    // Can't easily inspect the evaluator's rule count since it's private,
    // but we know it compiled without errors
}
