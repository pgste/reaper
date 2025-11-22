/// Generate test data for ReBAC (Relationship-Based Access Control) testing
///
/// Creates users and resources with various relationships
///
/// Relationships:
/// - Ownership (owner_id)
/// - Team membership (team_id, team_role)
/// - Sharing (shared_with_user)
/// - Parent-child (parent_owner_id)
/// - Organization hierarchy (manager_level, department)
/// - Collaboration (collaborator_id)
/// - Group membership (group_id)

use serde_json::{json, Value};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Generating ReBAC test data...\n");

    let mut entities = Vec::new();
    let num_users = 1000;
    let num_resources = 2000;

    let departments = ["engineering", "sales", "hr", "finance", "operations"];
    let teams = ["alpha", "beta", "gamma", "delta", "epsilon"];
    let groups = ["public", "restricted", "team-only", "private"];

    // Generate users with relationship attributes
    println!("👥 Generating {} users with relationship attributes...", num_users);
    for i in 0..num_users {
        let department = departments[i % departments.len()];
        let team_id = format!("team_{}", teams[i % teams.len()]);
        let group_id = format!("group_{}", groups[i % groups.len()]);

        // Team roles
        let team_role = match i % 10 {
            0 => "lead",
            1..=2 => "senior",
            3..=7 => "member",
            _ => "pending",
        };

        // Manager levels (1-5, higher = more authority)
        let role = match i % 10 {
            0 => "manager",
            1..=3 => "senior",
            _ => "staff",
        };

        let manager_level = match role {
            "manager" => 3 + (i % 3),  // 3-5
            "senior" => 2,
            _ => 1,
        };

        // Senior manager flag (level >= 4)
        let is_senior_manager = manager_level >= 4;

        // Group member flag (all users are group members except pending)
        let group_member = team_role != "pending";

        entities.push(json!({
            "id": format!("user_{}", i),
            "type": "User",
            "attributes": {
                "id": format!("user_{}", i),
                "name": format!("User {}", i),
                "role": role,
                "department": department,
                "team_id": team_id,
                "team_role": team_role,
                "group_id": group_id,
                "manager_level": manager_level,
                "is_senior_manager": is_senior_manager,
                "group_member": group_member,
                "email": format!("user{}@{}.example.com", i, department),
            }
        }));
    }

    // Generate resources with relationship attributes
    println!("📄 Generating {} resources with relationship attributes...", num_resources);
    for i in 0..num_resources {
        let owner_id = format!("user_{}", i % num_users);
        let team_id = format!("team_{}", teams[i % teams.len()]);
        let group_id = format!("group_{}", groups[i % groups.len()]);

        // Some resources have sharing relationships
        let shared_with_user = if i % 3 == 0 {
            format!("user_{}", (i + 100) % num_users)
        } else {
            "".to_string()
        };

        // Some resources have parent relationships
        let (parent_owner_id, inherit_permissions) = if i % 5 == 0 {
            (format!("user_{}", (i / 2) % num_users), true)
        } else {
            ("".to_string(), false)
        };

        // Some resources have collaborators
        let (collaborator_id, collaboration_status) = if i % 4 == 0 {
            (format!("user_{}", (i + 50) % num_users), "active")
        } else {
            ("".to_string(), "none")
        };

        // Group access level
        let group_access_level = match i % 4 {
            0 => "owner",
            1 => "admin",
            2 => "member",
            _ => "viewer",
        };

        // Department and level of owner (for hierarchy checks)
        let owner_department = departments[(i / 2) % departments.len()];
        let owner_level = 1 + (i % 3);  // 1-3

        entities.push(json!({
            "id": format!("resource_{}", i),
            "type": "Resource",
            "attributes": {
                "id": format!("resource_{}", i),
                "name": format!("Resource {}", i),
                "type": "project",
                "owner_id": owner_id,
                "team_id": team_id,
                "group_id": group_id,
                "shared_with_user": shared_with_user,
                "parent_owner_id": parent_owner_id,
                "inherit_permissions": inherit_permissions,
                "collaborator_id": collaborator_id,
                "collaboration_status": collaboration_status,
                "group_access_level": group_access_level,
                "owner_department": owner_department,
                "owner_level": owner_level,
                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
            }
        }));
    }

    // Write to file
    let output = json!({
        "entities": entities
    });

    let filename = "rebac-test-data.json";
    fs::write(filename, serde_json::to_string_pretty(&output)?)?;

    println!("\n✅ Generated ReBAC test data:");
    println!("   Users:      {}", num_users);
    println!("   Resources:  {}", num_resources);
    println!("   Total:      {}", num_users + num_resources);
    println!("   File:       {}", filename);
    println!("\n🔗 Relationship Types:");
    println!("   Ownership:     100% (all resources have owners)");
    println!("   Team members:  100% (all users in teams)");
    println!("   Shared:        ~33% (shared with specific users)");
    println!("   Parent-child:  ~20% (inherit permissions)");
    println!("   Collaborators: ~25% (active collaborations)");
    println!("   Groups:        100% (all users in groups)");
    println!("\n👥 Team Role Distribution:");
    println!("   Leads:    {} (10%)", num_users / 10);
    println!("   Seniors:  {} (20%)", num_users / 5);
    println!("   Members:  {} (50%)", num_users / 2);
    println!("   Pending:  {} (20%)", num_users / 5);

    Ok(())
}
