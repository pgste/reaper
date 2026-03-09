// ! Generate 100k entity dataset for stress testing

use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🏗️  Generating HUGE test dataset (100k entities)...\n");

    let mut entities = Vec::new();

    // Generate 50,000 users with varying attributes
    println!("1️⃣  Generating 50,000 users...");
    for i in 0..50_000 {
        let user_id = format!("user_{}", i);
        let department = match i % 10 {
            0 => "engineering",
            1 => "marketing",
            2 => "sales",
            3 => "hr",
            4 => "operations",
            5 => "finance",
            6 => "legal",
            7 => "support",
            8 => "product",
            _ => "research",
        };
        let role = match i % 20 {
            0 => "admin",
            1..=4 => "manager",
            5..=8 => "senior",
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
                "status": if i % 100 == 0 { "suspended" } else { "active" },
                "suspended": i % 100 == 0,
                "employee_number": i,
                "location": match i % 5 {
                    0 => "us-west-1",
                    1 => "us-west-2",
                    2 => "us-east-1",
                    3 => "eu-west-1",
                    _ => "ap-southeast-1",
                },
                "team": format!("team_{}", i % 200),
                "manager_id": if i > 0 { format!("user_{}", i / 10) } else { "user_0".to_string() },
            }
        }));

        if i % 10000 == 0 && i > 0 {
            println!("   Progress: {}/50000 users generated", i);
        }
    }

    // Generate 50,000 documents/resources
    println!("2️⃣  Generating 50,000 resources...");
    for i in 0..50_000 {
        let doc_id = format!("doc_{}", i);
        let owner_id = format!("user_{}", i % 50000); // Each doc has an owner
        let department = match i % 10 {
            0 => "engineering",
            1 => "marketing",
            2 => "sales",
            3 => "hr",
            4 => "operations",
            5 => "finance",
            6 => "legal",
            7 => "support",
            8 => "product",
            _ => "research",
        };

        entities.push(json!({
            "id": doc_id,
            "type": "Document",
            "attributes": {
                "owner_id": owner_id,
                "department": department,
                "clearance_required": (i % 10) + 1,
                "classification": match i % 5 {
                    0 => "public",
                    1 => "internal",
                    2 => "confidential",
                    3 => "secret",
                    _ => "top-secret",
                },
                "type": match i % 4 {
                    0 => "report",
                    1 => "document",
                    2 => "spreadsheet",
                    _ => "presentation",
                },
                "archived": i % 500 == 0,
                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
                "size_bytes": (i * 1024) % 10_000_000,
                "project_id": format!("proj_{}", i % 1000),
            }
        }));

        if i % 10000 == 0 && i > 0 {
            println!("   Progress: {}/50000 documents generated", i);
        }
    }

    let data = json!({
        "entities": entities
    });

    // Write to file
    println!("3️⃣  Writing to file...");
    let output = serde_json::to_string_pretty(&data)?;
    fs::write("test-data/huge-test-data.json", output)?;

    let file_size = fs::metadata("test-data/huge-test-data.json")?.len();

    println!();
    println!("✅ Generated 100,000 entities (50,000 users, 50,000 documents)");
    println!("   📄 File: huge-test-data.json");
    println!("   📦 Size: {:.2} MB", file_size as f64 / 1_048_576.0);
    println!();
    println!("⚠️  Note: This is a large dataset for stress testing.");
    println!("   Expected memory usage: ~50-100 MB in DataStore");
    println!("   (Thanks to string interning, much less than raw JSON!)");

    Ok(())
}
