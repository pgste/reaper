use policy_engine::ReaperPolicy;
use std::str::FromStr;

#[test]
fn test_parse_abac_benchmark() {
    let policy_text = r#"
// Attribute-Based Access Control (ABAC) Benchmark Policy
// Tests attribute matching with clearance levels, departments, and status

policy abac_benchmark {
    version: "1.0.0",
    description: "ABAC benchmark - clearance levels and department access",

    default: deny,

    // Deny suspended users immediately (highest priority)
    rule deny_suspended {
        deny if user.suspended == true
    }

    // Admin with high clearance can access everything
    rule admin_high_clearance {
        allow if {
            user.role == "admin" &&
            user.high_clearance == true
        }
    }

    // Same department access with clearance match
    rule department_clearance {
        allow if {
            user.department == resource.department &&
            user.clearance_level >= resource.clearance_level &&
            user.status == "active"
        }
    }
}
"#;

    let policy = ReaperPolicy::from_str(policy_text).expect("Failed to parse policy");

    println!("\n✓ Policy parsed successfully!");
    println!("Name: {}", policy.name());
    println!("Version: {:?}", policy.version());
}
