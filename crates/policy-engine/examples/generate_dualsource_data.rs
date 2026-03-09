/// Generate test data from TWO separate sources for multi-source policy testing
///
/// This demonstrates a realistic scenario where:
/// 1. Role mappings come from an identity provider (user_id -> roles)
/// 2. User attributes come from a directory service (user_id -> attributes)
///
/// The policy engine must:
/// - Load both data sources
/// - Join them on user_id
/// - Evaluate policies that require data from BOTH sources
///
/// This tests memory efficiency and performance at scale.
use serde_json::json;
use std::env;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get scale from command line (default 100)
    let args: Vec<String> = env::args().collect();
    let scale = if args.len() > 1 {
        args[1].parse::<usize>().unwrap_or(100)
    } else {
        100
    };

    let num_users = scale;
    let num_resources = scale * 2; // 2x resources

    println!(
        "🔐 Generating Dual-Source test data (scale: {})...\n",
        scale
    );
    println!("   Users:     {}", num_users);
    println!("   Resources: {}", num_resources);
    println!();

    // ============================================================
    // DATA SOURCE 1: Role Mappings (from identity provider)
    // ============================================================
    println!("👥 Generating SOURCE 1: Role mappings...");
    let mut role_entities = Vec::new();

    let roles_list = ["analyst", "admin", "viewer", "editor", "auditor"];

    for i in 0..num_users {
        let user_id = format!("user_{}", i);

        // Assign roles based on user ID pattern
        let primary_role = roles_list[i % roles_list.len()];
        let mut user_roles = vec![primary_role.to_string()];

        // 20% of users get a secondary role
        if i % 5 == 0 {
            let secondary_role = roles_list[(i + 1) % roles_list.len()];
            if secondary_role != primary_role {
                user_roles.push(secondary_role.to_string());
            }
        }

        // 10% of admins get superadmin
        if primary_role == "admin" && i % 10 == 0 {
            user_roles.push("superadmin".to_string());
        }

        role_entities.push(json!({
            "id": user_id.clone(),
            "type": "User",
            "attributes": {
                "id": user_id,
                "roles": user_roles.clone(),
                "primary_role": primary_role,
            }
        }));
    }

    let roles_output = json!({
        "entities": role_entities
    });

    let roles_filename = if scale <= 1000 {
        "test-data/dualsource-roles-small.json"
    } else {
        "test-data/dualsource-roles-large.json"
    };

    // Use non-pretty format for large files to save memory
    let roles_str = if scale <= 1000 {
        serde_json::to_string_pretty(&roles_output)?
    } else {
        serde_json::to_string(&roles_output)?
    };
    fs::write(roles_filename, roles_str)?;
    println!(
        "   ✓ Wrote {} role mappings to {}",
        num_users, roles_filename
    );

    // ============================================================
    // DATA SOURCE 2: User Attributes (from directory service)
    // ============================================================
    println!("📋 Generating SOURCE 2: User attributes...");
    let mut attribute_entities = Vec::new();

    let departments = ["engineering", "security", "finance", "hr", "operations"];
    let locations = ["us-west", "us-east", "eu-west", "ap-south", "ap-east"];

    for i in 0..num_users {
        let user_id = format!("user_{}", i);
        let department = departments[i % departments.len()];
        let location = locations[i % locations.len()];

        // Clearance level 1-5
        let clearance = 1 + (i % 5);

        // High clearance flag (clearance >= 3)
        let high_clearance = clearance >= 3;

        // 5% suspended users
        let is_active = i % 20 != 0;

        // 10% contractors
        let is_contractor = i % 10 == 0;

        attribute_entities.push(json!({
            "id": user_id.clone(),
            "type": "User",
            "attributes": {
                "id": user_id,
                "name": format!("User {}", i),
                "email": format!("user{}@{}.example.com", i, department),
                "department": department,
                "location": location,
                "clearance": clearance,
                "high_clearance": high_clearance,
                "is_active": is_active,
                "is_contractor": is_contractor,
                "cost_center": format!("CC-{:04}", i % 100),
            }
        }));
    }

    let attributes_output = json!({
        "entities": attribute_entities
    });

    let attributes_filename = if scale <= 1000 {
        "test-data/dualsource-attributes-small.json"
    } else {
        "test-data/dualsource-attributes-large.json"
    };

    let attributes_str = if scale <= 1000 {
        serde_json::to_string_pretty(&attributes_output)?
    } else {
        serde_json::to_string(&attributes_output)?
    };
    fs::write(attributes_filename, attributes_str)?;
    println!(
        "   ✓ Wrote {} attribute records to {}",
        num_users, attributes_filename
    );

    // ============================================================
    // DATA SOURCE 3: Resources (shared)
    // ============================================================
    println!("📄 Generating resources...");
    let mut resource_entities = Vec::new();

    let classifications = ["public", "internal", "confidential", "secret"];

    for i in 0..num_resources {
        let resource_id = format!("doc_{}", i);
        let department = departments[i % departments.len()];
        let classification = classifications[i % classifications.len()];

        // Clearance required based on classification
        let clearance_required = match classification {
            "public" => 1,
            "internal" => 2,
            "confidential" => 3,
            "secret" => 5,
            _ => 1,
        };

        // Owner from same department (every 10th user)
        let owner_id = format!("user_{}", (i * 10) % num_users);

        // 10% archived
        let is_archived = i % 10 == 0;

        resource_entities.push(json!({
            "id": resource_id.clone(),
            "type": "Resource",
            "attributes": {
                "id": resource_id,
                "name": format!("Document {}", i),
                "type": "document",
                "department": department,
                "classification": classification,
                "clearance_required": clearance_required,
                "owner_id": owner_id,
                "is_archived": is_archived,
            }
        }));
    }

    let resources_output = json!({
        "entities": resource_entities
    });

    let resources_filename = if scale <= 1000 {
        "test-data/dualsource-resources-small.json"
    } else {
        "test-data/dualsource-resources-large.json"
    };

    let resources_str = if scale <= 1000 {
        serde_json::to_string_pretty(&resources_output)?
    } else {
        serde_json::to_string(&resources_output)?
    };
    fs::write(resources_filename, resources_str)?;
    println!(
        "   ✓ Wrote {} resources to {}",
        num_resources, resources_filename
    );

    // ============================================================
    // Summary
    // ============================================================
    println!("\n✅ Generated Dual-Source test data:");
    println!("   Scale:       {}", scale);
    println!("   Users:       {}", num_users);
    println!("   Resources:   {}", num_resources);
    println!("   Total:       {}", num_users + num_resources);
    println!("\n📊 Data Distribution:");
    println!("   Active users:     {} (95%)", num_users * 95 / 100);
    println!("   Suspended:        {} (5%)", num_users * 5 / 100);
    println!("   Contractors:      {} (10%)", num_users * 10 / 100);
    println!("   Multi-role users: {} (20%)", num_users * 20 / 100);
    println!("\n🔐 Clearance Distribution:");
    println!("   Level 1-2 (low):  {} (40%)", num_users * 40 / 100);
    println!("   Level 3-4 (med):  {} (40%)", num_users * 40 / 100);
    println!("   Level 5 (high):   {} (20%)", num_users * 20 / 100);
    println!("\n📄 Resource Classification:");
    println!("   Public:        {} (25%)", num_resources / 4);
    println!("   Internal:      {} (25%)", num_resources / 4);
    println!("   Confidential:  {} (25%)", num_resources / 4);
    println!("   Secret:        {} (25%)", num_resources / 4);
    println!("   Archived:      {} (10%)", num_resources / 10);

    println!("\n📦 Output Files:");
    println!(
        "   1. {} - Role mappings (identity provider)",
        roles_filename
    );
    println!(
        "   2. {} - User attributes (directory service)",
        attributes_filename
    );
    println!("   3. {} - Resources", resources_filename);

    println!("\n💡 Usage:");
    println!("   cargo run --example generate_dualsource_data 100       # 100 users");
    println!("   cargo run --example generate_dualsource_data 1000000   # 1M users");

    Ok(())
}
