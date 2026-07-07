/// Generate test data for the DSL-functions volume test.
///
/// Users carry the attribute shapes the compiled utility functions operate on:
/// - skills:    list (intersection / difference / count)
/// - profile:   object (has_key / values)
/// - flags:     list with a truthy element (any)
/// - all_flags: list all-truthy (all)
/// - email:     string containing "corp" (find)
/// - csv:       "a,b,c" (find_all)
/// - name:      "temp" (replace -> "perm")
use serde_json::json;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Generating DSL-functions test data...\n");

    let mut entities = Vec::new();
    let num_users = 1000;
    let num_resources = 2000;

    for i in 0..num_users {
        entities.push(json!({
            "id": format!("user_{}", i),
            "type": "User",
            "attributes": {
                "role": if i % 3 == 0 { "admin" } else { "engineer" },
                "skills": ["rust", "python", "go"],
                "profile": {"tier": "gold", "country": "US"},
                "flags": [0, 0, 1],
                "all_flags": [1, 2, 3],
                "email": format!("user_{}@corp.example", i),
                "csv": "a,b,c",
                "name": "temp"
            }
        }));
    }

    for i in 0..num_resources {
        entities.push(json!({
            "id": format!("resource_{}", i),
            "type": "Resource",
            "attributes": {"kind": "document"}
        }));
    }

    let doc = json!({ "entities": entities });
    fs::create_dir_all("test-data")?;
    let path = "test-data/functions-test-data.json";
    fs::write(path, serde_json::to_string_pretty(&doc)?)?;

    println!(
        "✓ Wrote {} entities ({} users, {} resources) to {}",
        num_users + num_resources,
        num_users,
        num_resources,
        path
    );
    Ok(())
}
