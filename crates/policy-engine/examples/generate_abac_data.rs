/// Generate test data for ABAC (Attribute-Based Access Control) testing
///
/// Creates users with clearances/departments and resources with requirements
///
/// User attributes:
/// - clearance: 1-10 (security clearance level)
/// - department: engineering, sales, hr, finance, operations
/// - role: executive, manager, analyst, staff
/// - status: active, suspended
///
/// Resource attributes:
/// - clearance_required: 1-10
/// - department: engineering, sales, hr, finance, operations
/// - classification: public, internal, confidential, secret
/// - archived: true/false
use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Generating ABAC test data...\n");

    let mut entities = Vec::new();
    let num_users = 1000;
    let num_resources = 2000;

    let departments = ["engineering", "sales", "hr", "finance", "operations"];
    let roles = ["executive", "manager", "analyst", "staff"];
    let classifications = ["public", "internal", "confidential", "secret"];

    // Generate users with attributes
    println!("👥 Generating {} users with attributes...", num_users);
    for i in 0..num_users {
        let department = departments[i % departments.len()];
        let role = roles[i % roles.len()];

        // Clearance based on role
        let clearance = match role {
            "executive" => 8 + (i % 3),  // 8-10
            "manager" => 5 + (i % 3),    // 5-7
            "analyst" => 3 + (i % 3),    // 3-5
            _ => 1 + (i % 3),            // 1-3
        };

        // 5% suspended users
        let status = if i % 20 == 0 { "suspended" } else { "active" };
        let suspended = status == "suspended";

        // High clearance flag (clearance >= 5)
        let high_clearance = clearance >= 5;

        // Clearance match flag (for exact matching with resources)
        let clearance_match = clearance >= 3;

        entities.push(json!({
            "id": format!("user_{}", i),
            "type": "User",
            "attributes": {
                "id": format!("user_{}", i),
                "name": format!("User {}", i),
                "role": role,
                "department": department,
                "clearance": clearance,
                "high_clearance": high_clearance,
                "clearance_match": clearance_match,
                "status": status,
                "suspended": suspended,
                "email": format!("user{}@{}.example.com", i, department),
            }
        }));
    }

    // Generate resources with attribute requirements
    println!("📄 Generating {} resources with attribute requirements...", num_resources);
    for i in 0..num_resources {
        let department = departments[i % departments.len()];
        let classification = classifications[i % classifications.len()];

        // Clearance required based on classification
        let clearance_required = match classification {
            "public" => 1,
            "internal" => 3,
            "confidential" => 5,
            "secret" => 8,
            _ => 1,
        };

        // 10% archived
        let archived = i % 10 == 0;

        // Owner from same department
        let owner_id = format!("user_{}", (i * 5) % num_users);

        entities.push(json!({
            "id": format!("doc_{}", i),
            "type": "Resource",
            "attributes": {
                "id": format!("doc_{}", i),
                "name": format!("Document {}", i),
                "type": "document",
                "department": department,
                "classification": classification,
                "clearance_required": clearance_required,
                "owner_id": owner_id,
                "archived": archived,
                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
            }
        }));
    }

    // Write to file
    let output = json!({
        "entities": entities
    });

    let filename = "abac-test-data.json";
    fs::write(filename, serde_json::to_string_pretty(&output)?)?;

    println!("\n✅ Generated ABAC test data:");
    println!("   Users:      {}", num_users);
    println!("   Resources:  {}", num_resources);
    println!("   Total:      {}", num_users + num_resources);
    println!("   File:       {}", filename);
    println!("\n👥 User Distribution:");
    println!("   Active:     {} (95%)", num_users * 95 / 100);
    println!("   Suspended:  {} (5%)", num_users * 5 / 100);
    println!("\n🔐 Clearance Levels:");
    println!("   High (8-10): Executives");
    println!("   Med (5-7):   Managers");
    println!("   Low (3-5):   Analysts");
    println!("   Min (1-3):   Staff");
    println!("\n📄 Resource Classification:");
    println!("   Public:       {} (25%)", num_resources / 4);
    println!("   Internal:     {} (25%)", num_resources / 4);
    println!("   Confidential: {} (25%)", num_resources / 4);
    println!("   Secret:       {} (25%)", num_resources / 4);
    println!("   Archived:     {} (10%)", num_resources / 10);

    Ok(())
}
