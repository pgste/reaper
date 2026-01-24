//! Test scenario generation
//!
//! Generates test requests with varied data for benchmarking.
//! Produces approximately 60% allow and 40% deny decisions.

use std::collections::HashMap;

/// A test request scenario
#[derive(Debug, Clone)]
pub struct TestRequest {
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub context: Option<HashMap<String, String>>,
}

/// Generate a batch of test requests
///
/// Distribution:
/// - 30% admin users (always allowed)
/// - 30% engineers (allowed for read/write/update)
/// - 20% viewers (allowed for read only)
/// - 20% guests (usually denied)
///
/// Uses entity IDs that match the loaded benchmark_entities.json
pub fn generate_requests(count: usize) -> Vec<TestRequest> {
    (0..count)
        .map(|i| {
            let scenario = i % 10;
            match scenario {
                // Admin users - always allowed (30%)
                0..=2 => TestRequest {
                    principal: format!("user_admin_{}", i % 5),
                    action: pick_action(i),
                    resource: pick_resource(i),
                    context: Some(create_context("admin", "engineering")),
                },
                // Engineers - allowed for most operations (30%)
                3..=5 => TestRequest {
                    principal: format!("user_engineer_{}", i % 5),
                    action: pick_engineer_action(i),
                    resource: pick_resource(i),
                    context: Some(create_context("engineer", "engineering")),
                },
                // Viewers - only read allowed (20%)
                6..=7 => TestRequest {
                    principal: format!("user_viewer_{}", i % 5),
                    action: if i % 3 == 0 { "write" } else { "read" }.to_string(),
                    resource: pick_resource(i),
                    context: Some(create_context("viewer", "marketing")),
                },
                // Guests - usually denied (20%)
                _ => TestRequest {
                    principal: format!("guest_{}", i % 5),
                    action: pick_restricted_action(i),
                    resource: pick_admin_resource(i),
                    context: Some(create_context("guest", "external")),
                },
            }
        })
        .collect()
}

fn pick_action(i: usize) -> String {
    match i % 4 {
        0 => "read",
        1 => "write",
        2 => "update",
        _ => "delete",
    }
    .to_string()
}

fn pick_engineer_action(i: usize) -> String {
    match i % 5 {
        0 => "read",
        1 => "write",
        2 => "update",
        3 => "read",  // More reads
        _ => "write", // No delete
    }
    .to_string()
}

fn pick_restricted_action(i: usize) -> String {
    match i % 3 {
        0 => "delete",
        1 => "admin",
        _ => "execute",
    }
    .to_string()
}

fn pick_resource(i: usize) -> String {
    match i % 6 {
        0 => "/api/v1/data",
        1 => "/api/v1/users",
        2 => "/api/v1/reports",
        3 => "/api/v1/analytics",
        4 => "/api/v1/config",
        _ => "/api/v1/metrics",
    }
    .to_string()
}

fn pick_admin_resource(i: usize) -> String {
    match i % 3 {
        0 => "/api/admin/settings",
        1 => "/api/admin/users",
        _ => "/api/admin/secrets",
    }
    .to_string()
}

fn create_context(role: &str, department: &str) -> HashMap<String, String> {
    let mut context = HashMap::new();
    context.insert("role".to_string(), role.to_string());
    context.insert("department".to_string(), department.to_string());
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_requests() {
        let requests = generate_requests(100);
        assert_eq!(requests.len(), 100);

        // Check distribution
        let admin_count = requests
            .iter()
            .filter(|r| r.context.as_ref().map(|c| c.get("role") == Some(&"admin".to_string())).unwrap_or(false))
            .count();
        let guest_count = requests
            .iter()
            .filter(|r| r.context.as_ref().map(|c| c.get("role") == Some(&"guest".to_string())).unwrap_or(false))
            .count();

        // Approximately 30% admin, 20% guest
        assert!(admin_count >= 20 && admin_count <= 40);
        assert!(guest_count >= 10 && guest_count <= 30);
    }
}
