/// Generate test data for RBAC (Role-Based Access Control) testing
///
/// Creates users with roles and resources with types/ownership
///
/// Roles:
/// - admin (10%)
/// - manager (20%)
/// - user (70%)
///
/// Resource types:
/// - report
/// - document
/// - project
/// - file
use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Generating RBAC test data...\n");

    let mut entities = Vec::new();
    let num_users = 1000;
    let num_resources = 2000;

    // Generate users with roles
    println!("👥 Generating {} users with roles...", num_users);
    for i in 0..num_users {
        let role = match i % 10 {
            0 => "admin",          // 10% admins
            1..=2 => "manager",    // 20% managers
            _ => "user",           // 70% regular users
        };

        entities.push(json!({
            "id": format!("user_{}", i),
            "type": "User",
            "attributes": {
                "id": format!("user_{}", i),
                "name": format!("User {}", i),
                "role": role,
                "email": format!("user{}@example.com", i),
                "active": true,
            }
        }));
    }

    // Generate resources with types and owners
    println!("📄 Generating {} resources with types and ownership...", num_resources);
    for i in 0..num_resources {
        let resource_type = match i % 4 {
            0 => "report",
            1 => "document",
            2 => "project",
            _ => "file",
        };

        let owner_id = format!("user_{}", i % num_users);

        entities.push(json!({
            "id": format!("resource_{}", i),
            "type": "Resource",
            "attributes": {
                "id": format!("resource_{}", i),
                "type": resource_type,
                "name": format!("{} {}",
                    match resource_type {
                        "report" => "Report",
                        "document" => "Document",
                        "project" => "Project",
                        _ => "File",
                    },
                    i
                ),
                "owner_id": owner_id,
                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
                "status": "active",
            }
        }));
    }

    // Write to file
    let output = json!({
        "entities": entities
    });

    let filename = "rbac-test-data.json";
    fs::write(filename, serde_json::to_string_pretty(&output)?)?;

    println!("\n✅ Generated RBAC test data:");
    println!("   Users:      {}", num_users);
    println!("   Resources:  {}", num_resources);
    println!("   Total:      {}", num_users + num_resources);
    println!("   File:       {}", filename);
    println!("\n📊 Role Distribution:");
    println!("   Admins:     {} (10%)", num_users / 10);
    println!("   Managers:   {} (20%)", num_users / 5);
    println!("   Users:      {} (70%)", num_users * 7 / 10);
    println!("\n📄 Resource Distribution:");
    println!("   Reports:    {} (25%)", num_resources / 4);
    println!("   Documents:  {} (25%)", num_resources / 4);
    println!("   Projects:   {} (25%)", num_resources / 4);
    println!("   Files:      {} (25%)", num_resources / 4);

    Ok(())
}
