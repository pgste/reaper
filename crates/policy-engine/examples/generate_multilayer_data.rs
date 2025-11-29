/// Generate test data for Multilayer (RBAC + ABAC + ReBAC) testing
///
/// Creates users and resources with ALL attributes needed for multilayer policy:
/// - RBAC: roles, status
/// - ABAC: clearances, departments, classifications
/// - ReBAC: ownership, teams, sharing, collaboration
///
/// Generates varied scenarios to test different rule combinations
use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Generating Multilayer test data...\n");

    let mut entities = Vec::new();
    let num_users = 1000;
    let num_resources = 2000;

    let departments = ["engineering", "sales", "hr", "finance", "operations"];
    let teams = ["alpha", "beta", "gamma", "delta", "epsilon"];
    let roles = [
        "admin",
        "executive",
        "manager",
        "senior",
        "analyst",
        "staff",
        "intern",
    ];
    let classifications = ["public", "internal", "confidential", "secret"];

    // Generate users with all attributes (RBAC + ABAC + ReBAC)
    println!(
        "👥 Generating {} users with multilayer attributes...",
        num_users
    );
    for i in 0..num_users {
        let department = departments[i % departments.len()];
        let team_id = format!("team_{}", teams[i % teams.len()]);

        // RBAC attributes
        let role = roles[i % roles.len()];
        // 5% suspended users (but keep user_0-19 active for tests, user_20 IS suspended)
        let status = if i >= 20 && i % 20 == 0 {
            "suspended"
        } else {
            "active"
        };
        let suspended = status == "suspended";

        // Team roles (ReBAC)
        let team_role = match i % 10 {
            0 => "lead",
            1..=2 => "senior",
            3..=7 => "member",
            _ => "pending",
        };

        // ABAC attributes - clearance based on role
        let clearance = match role {
            "admin" => 10,
            "executive" => 8 + (i % 3), // 8-10
            "manager" => 5 + (i % 3),   // 5-7
            "senior" => 4 + (i % 2),    // 4-5
            "analyst" => 3 + (i % 2),   // 3-4
            "staff" => 1 + (i % 3),     // 1-3
            "intern" => 1,
            _ => 2,
        };

        // Derived flags
        let high_clearance = clearance >= 5;
        let clearance_match = clearance >= 3;

        // Manager level (ReBAC hierarchy)
        let manager_level = match role {
            "admin" => 5,
            "executive" => 5,
            "manager" => 3 + (i % 3), // 3-5
            "senior" => 2,
            _ => 1,
        };
        let is_senior_manager = manager_level >= 4;

        // Group membership
        let group_id = format!("group_{}", i % 4);
        let group_member = team_role != "pending";

        entities.push(json!({
            "id": format!("user_{}", i),
            "type": "User",
            "attributes": {
                // Identity
                "id": format!("user_{}", i),
                "name": format!("User {}", i),
                "email": format!("user{}@{}.example.com", i, department),

                // RBAC
                "role": role,
                "status": status,
                "suspended": suspended,

                // ABAC
                "department": department,
                "clearance": clearance,
                "high_clearance": high_clearance,
                "clearance_match": clearance_match,

                // ReBAC
                "team_id": team_id,
                "team_role": team_role,
                "group_id": group_id,
                "manager_level": manager_level,
                "is_senior_manager": is_senior_manager,
                "group_member": group_member,
            }
        }));
    }

    // Generate resources with all attributes
    println!(
        "📄 Generating {} resources with multilayer attributes...",
        num_resources
    );
    for i in 0..num_resources {
        let department = departments[i % departments.len()];
        let team_id = format!("team_{}", teams[i % teams.len()]);
        let classification = classifications[i % classifications.len()];

        // Clearance required based on classification
        let clearance_required = match classification {
            "public" => 1,
            "internal" => 3,
            "confidential" => 5,
            "secret" => 8,
            _ => 1,
        };

        // Ownership (ReBAC)
        let owner_id = format!("user_{}", i % num_users);

        // Status flags
        let archived = i % 10 == 0; // 10% archived
        let public_in_dept = i % 3 == 0; // 33% public within department

        // Sharing relationships (ReBAC) - 33% shared
        let shared_with_user = if i % 3 == 0 {
            format!("user_{}", (i + 100) % num_users)
        } else {
            "".to_string()
        };

        // Collaboration (ReBAC) - 25% have collaborators
        let (collaborator_id, collaboration_status) = if i % 4 == 0 {
            (format!("user_{}", (i + 50) % num_users), "active")
        } else {
            ("".to_string(), "none")
        };

        // Parent-child relationships - 20%
        let (parent_owner_id, inherit_permissions) = if i % 5 == 0 {
            (format!("user_{}", (i / 2) % num_users), true)
        } else {
            ("".to_string(), false)
        };

        // Owner attributes for hierarchy checks
        let owner_department = department;
        let owner_level = 1 + (i % 3);

        // Group
        let group_id = format!("group_{}", i % 4);
        let group_access_level = match i % 4 {
            0 => "owner",
            1 => "admin",
            2 => "member",
            _ => "viewer",
        };

        entities.push(json!({
            "id": format!("resource_{}", i),
            "type": "Resource",
            "attributes": {
                // Identity
                "id": format!("resource_{}", i),
                "name": format!("Resource {}", i),
                "type": "document",

                // ABAC
                "department": department,
                "classification": classification,
                "clearance_required": clearance_required,
                "archived": archived,
                "public_in_dept": public_in_dept,

                // ReBAC
                "owner_id": owner_id,
                "team_id": team_id,
                "group_id": group_id,
                "shared_with_user": shared_with_user,
                "collaborator_id": collaborator_id,
                "collaboration_status": collaboration_status,
                "parent_owner_id": parent_owner_id,
                "inherit_permissions": inherit_permissions,
                "owner_department": owner_department,
                "owner_level": owner_level,
                "group_access_level": group_access_level,

                "created_at": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
            }
        }));
    }

    // Write to file
    let output = json!({
        "entities": entities
    });

    let filename = "test-data/multilayer-test-data.json";
    fs::write(filename, serde_json::to_string_pretty(&output)?)?;

    println!("\n✅ Generated Multilayer test data:");
    println!("   Users:      {}", num_users);
    println!("   Resources:  {}", num_resources);
    println!("   Total:      {}", num_users + num_resources);
    println!("   File:       {}", filename);

    println!("\n📊 RBAC Distribution:");
    let admin_count = num_users / roles.len();
    println!(
        "   Admins:      {} ({:.1}%)",
        admin_count,
        (admin_count as f64 / num_users as f64) * 100.0
    );
    println!(
        "   Executives:  {} ({:.1}%)",
        admin_count,
        (admin_count as f64 / num_users as f64) * 100.0
    );
    println!(
        "   Managers:    {} ({:.1}%)",
        admin_count,
        (admin_count as f64 / num_users as f64) * 100.0
    );
    println!(
        "   Staff:       {} ({:.1}%)",
        admin_count * 3,
        (admin_count as f64 * 3.0 / num_users as f64) * 100.0
    );
    println!("   Suspended:   {} (5%)", num_users / 20);

    println!("\n🔐 ABAC Distribution:");
    println!("   Departments:  5 (engineering, sales, hr, finance, operations)");
    println!("   Clearances:   1-10 (role-based)");
    println!("   High clear:   ~40% (clearance >= 5)");

    println!("\n📄 Resource Distribution:");
    println!("   Classifications:");
    println!("     Public:       {} (25%)", num_resources / 4);
    println!("     Internal:     {} (25%)", num_resources / 4);
    println!("     Confidential: {} (25%)", num_resources / 4);
    println!("     Secret:       {} (25%)", num_resources / 4);
    println!("   Archived:       {} (10%)", num_resources / 10);
    println!("   Public in dept: {} (33%)", num_resources / 3);

    println!("\n🔗 ReBAC Distribution:");
    println!("   Ownership:      100% (all have owners)");
    println!("   Team members:   100% (all in teams)");
    println!("   Shared:         ~33% (667 resources)");
    println!("   Collaborators:  ~25% (500 resources)");
    println!("   Hierarchical:   ~20% (400 resources)");

    println!("\n💡 Multilayer Characteristics:");
    println!("   This dataset enables testing of:");
    println!("   ✓ Pure RBAC (admin access)");
    println!("   ✓ Pure ABAC (department + clearance)");
    println!("   ✓ Pure ReBAC (ownership, sharing)");
    println!("   ✓ RBAC + ABAC (role + clearance)");
    println!("   ✓ RBAC + ReBAC (role + ownership)");
    println!("   ✓ ABAC + ReBAC (clearance + team)");
    println!("   ✓ All three combined (executive + team lead + clearance)");

    Ok(())
}
