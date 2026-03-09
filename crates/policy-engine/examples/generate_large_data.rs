// ! Generate large-scale test data for volume testing

use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🏗️  Generating large-scale test data...\n");

    let mut entities = Vec::new();

    // Generate 500 users with varying attributes
    println!("1️⃣  Generating 500 users...");
    for i in 0..500 {
        let user_id = format!("user_{}", i);
        let department = match i % 5 {
            0 => "engineering",
            1 => "marketing",
            2 => "sales",
            3 => "hr",
            _ => "operations",
        };
        let role = match i % 10 {
            0 => "admin",
            1..=3 => "manager",
            _ => "user",
        };
        let clearance = (i % 10) + 1; // 1-10

        entities.push(json!({
            "id": user_id,
            "type": "User",
            "attributes": {
                "id": user_id,
                "name": format!("User {}", i),
                "role": role,
                "department": department,
                "clearance": clearance,
                "status": if i % 20 == 0 { "suspended" } else { "active" },
                "suspended": i % 20 == 0,
                "employee_number": i,
                "location": match i % 3 {
                    0 => "us-west",
                    1 => "us-east",
                    _ => "eu-west",
                }
            }
        }));
    }

    // Generate 500 documents/resources
    println!("2️⃣  Generating 500 resources...");
    for i in 0..500 {
        let doc_id = format!("doc_{}", i);
        let owner_id = format!("user_{}", i % 500); // Each doc has an owner
        let department = match i % 5 {
            0 => "engineering",
            1 => "marketing",
            2 => "sales",
            3 => "hr",
            _ => "operations",
        };

        entities.push(json!({
            "id": doc_id,
            "type": "Document",
            "attributes": {
                "owner_id": owner_id,
                "department": department,
                "clearance_required": (i % 10) + 1,
                "classification": match i % 4 {
                    0 => "public",
                    1 => "internal",
                    2 => "confidential",
                    _ => "secret",
                },
                "type": match i % 3 {
                    0 => "report",
                    1 => "document",
                    _ => "spreadsheet",
                },
                "archived": i % 50 == 0,
                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
                "size_bytes": i * 1024,
            }
        }));
    }

    let data = json!({
        "entities": entities
    });

    // Write to file
    println!("3️⃣  Writing to file...");
    let output = serde_json::to_string_pretty(&data)?;
    fs::write("large-test-data.json", output)?;

    println!("✅ Generated 1000 entities (500 users, 500 documents)");
    println!("   📄 File: large-test-data.json");
    println!(
        "   📦 Size: {} bytes",
        fs::metadata("large-test-data.json")?.len()
    );

    Ok(())
}
