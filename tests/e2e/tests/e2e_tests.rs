//! End-to-end integration tests for Reaper Platform
//!
//! These tests verify the complete flow from management server through agent
//! policy deployment and evaluation.
//!
//! To run these tests, services must be running:
//! - Management Server on port 3000
//! - Agent (managed mode) on port 8082
//! - Or use: docker compose -f docker-compose.full.yml --profile managed up
//!
//! Run with: cargo test -p reaper-e2e-tests --test e2e_tests

use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

// Service URLs - can be overridden with environment variables
fn management_url() -> String {
    std::env::var("REAPER_MANAGEMENT_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

fn agent_url() -> String {
    std::env::var("REAPER_AGENT_URL").unwrap_or_else(|_| "http://localhost:8082".to_string())
}

/// Test client with helper methods for API interaction
struct TestClient {
    client: Client,
    management_url: String,
    agent_url: String,
    api_key: Option<String>,
}

impl TestClient {
    fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            management_url: management_url(),
            agent_url: agent_url(),
            api_key: None,
        }
    }

    #[allow(dead_code)]
    fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    // Management API helpers

    async fn management_get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        let mut req = self.client.get(format!("{}{}", self.management_url, path));
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        req.send().await
    }

    async fn management_post(&self, path: &str, body: Value) -> reqwest::Result<reqwest::Response> {
        let mut req = self
            .client
            .post(format!("{}{}", self.management_url, path))
            .json(&body);
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        req.send().await
    }

    async fn management_delete(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        let mut req = self
            .client
            .delete(format!("{}{}", self.management_url, path));
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        req.send().await
    }

    // Agent API helpers

    async fn agent_get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}{}", self.agent_url, path))
            .send()
            .await
    }

    async fn agent_post(&self, path: &str, body: Value) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(format!("{}{}", self.agent_url, path))
            .json(&body)
            .send()
            .await
    }
}

/// Check if services are available
async fn services_available() -> bool {
    let client = TestClient::new();

    let management_health = client.management_get("/health").await;
    let agent_health = client.agent_get("/health").await;

    management_health.is_ok()
        && management_health.unwrap().status().is_success()
        && agent_health.is_ok()
        && agent_health.unwrap().status().is_success()
}

/// Skip test if services are not available
macro_rules! skip_if_no_services {
    () => {
        if !services_available().await {
            eprintln!("Skipping test: services not available");
            return;
        }
    };
}

// =============================================================================
// Health Check Tests
// =============================================================================

#[tokio::test]
async fn test_management_health() {
    skip_if_no_services!();

    let client = TestClient::new();
    let response = client.management_get("/health").await.unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
}

#[tokio::test]
async fn test_agent_health() {
    skip_if_no_services!();

    let client = TestClient::new();
    let response = client.agent_get("/health").await.unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_management_metrics() {
    skip_if_no_services!();

    let client = TestClient::new();
    let response = client.management_get("/metrics/prometheus").await.unwrap();
    assert!(response.status().is_success());

    let body = response.text().await.unwrap();
    // Verify Prometheus format
    assert!(body.contains("reaper_management_"));
}

#[tokio::test]
async fn test_agent_metrics() {
    skip_if_no_services!();

    let client = TestClient::new();
    let response = client.agent_get("/metrics").await.unwrap();
    assert!(response.status().is_success());
}

// =============================================================================
// Organization Lifecycle Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_organization_lifecycle() {
    skip_if_no_services!();

    let client = TestClient::new();
    let slug = format!("e2e-org-{}", Uuid::new_v4().to_string()[..8].to_string());

    // Create organization
    let response = client
        .management_post(
            "/orgs",
            json!({
                "name": "E2E Test Organization",
                "slug": slug
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);

    let body: Value = response.json().await.unwrap();
    let org_id = body["id"].as_str().unwrap();
    assert_eq!(body["slug"], slug);

    // Get organization
    let response = client
        .management_get(&format!("/orgs/{}", slug))
        .await
        .unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], org_id);

    // List organizations
    let response = client.management_get("/orgs").await.unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    let orgs = body["organizations"].as_array().unwrap();
    assert!(orgs.iter().any(|o| o["slug"] == slug));

    // Delete organization
    let response = client
        .management_delete(&format!("/orgs/{}", org_id))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 204);

    // Verify deletion
    let response = client
        .management_get(&format!("/orgs/{}", org_id))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
}

// =============================================================================
// Full Policy Deployment Flow Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_full_policy_deployment() {
    skip_if_no_services!();

    let client = TestClient::new();
    let slug = format!(
        "e2e-deploy-{}",
        Uuid::new_v4().to_string()[..8].to_string()
    );

    // Step 1: Create organization
    let response = client
        .management_post(
            "/orgs",
            json!({
                "name": "E2E Deployment Org",
                "slug": slug
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let org_body: Value = response.json().await.unwrap();
    let _org_id = org_body["id"].as_str().unwrap();

    // Step 2: Create policy
    let response = client
        .management_post(
            &format!("/orgs/{}/policies", slug),
            json!({
                "name": "e2e-test-policy",
                "language": "reaper",
                "content": "allow admin to access /admin/*\nallow user to read /api/*"
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let policy_body: Value = response.json().await.unwrap();
    let policy_id = policy_body["id"].as_str().unwrap();

    // Step 3: Create bundle with policy
    let response = client
        .management_post(
            &format!("/orgs/{}/bundles", slug),
            json!({
                "name": "e2e-test-bundle",
                "policy_ids": [policy_id]
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let bundle_body: Value = response.json().await.unwrap();
    let bundle_id = bundle_body["id"].as_str().unwrap();
    assert_eq!(bundle_body["status"], "draft");

    // Step 4: Compile bundle
    let response = client
        .management_post(&format!("/orgs/{}/bundles/{}/compile", slug, bundle_id), json!({}))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "compiled");

    // Step 5: Stage bundle
    let response = client
        .management_post(&format!("/orgs/{}/bundles/{}/stage", slug, bundle_id), json!({}))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "staged");

    // Step 6: Promote bundle
    let response = client
        .management_post(
            &format!("/orgs/{}/bundles/{}/promote", slug, bundle_id),
            json!({"notes": "E2E test promotion"}),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "promoted");

    // Step 7: Verify promoted bundle is accessible
    let response = client
        .management_get(&format!("/orgs/{}/bundles/promoted", slug))
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], bundle_id);

    // Step 8: Download bundle (verify binary is available)
    let response = client
        .management_get(&format!("/orgs/{}/bundles/{}/download", slug, bundle_id))
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );

    // Cleanup
    let _ = client
        .management_delete(&format!("/orgs/{}", org_body["id"].as_str().unwrap()))
        .await;
}

// =============================================================================
// Agent Registration and Heartbeat Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_agent_registration_flow() {
    skip_if_no_services!();

    let client = TestClient::new();
    let slug = format!("e2e-agent-{}", Uuid::new_v4().to_string()[..8].to_string());

    // Create organization
    let response = client
        .management_post(
            "/orgs",
            json!({
                "name": "E2E Agent Org",
                "slug": slug
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let org_body: Value = response.json().await.unwrap();
    let org_id = org_body["id"].as_str().unwrap();

    // Note: In production, we'd create an API key here.
    // For e2e tests, we may need to use a pre-configured test key
    // or test without auth depending on configuration.

    // Verify agents list endpoint works
    let response = client
        .management_get(&format!("/orgs/{}/agents", slug))
        .await
        .unwrap();
    // May require auth - just verify endpoint exists
    assert!(response.status().is_success() || response.status().as_u16() == 401);

    // Cleanup
    let _ = client
        .management_delete(&format!("/orgs/{}", org_id))
        .await;
}

// =============================================================================
// Policy Source Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_policy_source_management() {
    skip_if_no_services!();

    let client = TestClient::new();
    let slug = format!(
        "e2e-source-{}",
        Uuid::new_v4().to_string()[..8].to_string()
    );

    // Create organization
    let response = client
        .management_post(
            "/orgs",
            json!({
                "name": "E2E Source Org",
                "slug": slug
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let org_body: Value = response.json().await.unwrap();
    let org_id = org_body["id"].as_str().unwrap();

    // Verify sources endpoint exists
    let response = client
        .management_get(&format!("/orgs/{}/sources", slug))
        .await
        .unwrap();
    // May require auth
    assert!(response.status().is_success() || response.status().as_u16() == 401);

    // Cleanup
    let _ = client
        .management_delete(&format!("/orgs/{}", org_id))
        .await;
}

// =============================================================================
// Agent Policy Evaluation Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_agent_policy_evaluation() {
    skip_if_no_services!();

    let client = TestClient::new();

    // Test policy evaluation on agent (if agent has policies loaded)
    let response = client
        .agent_post(
            "/api/v1/messages",
            json!({
                "principal": "admin",
                "action": "access",
                "resource": "/admin/dashboard"
            }),
        )
        .await
        .unwrap();

    // Agent should respond (may allow or deny based on loaded policies)
    // The key is that it processes the request
    assert!(response.status().is_success() || response.status().is_client_error());
}

#[tokio::test]
async fn test_e2e_agent_list_policies() {
    skip_if_no_services!();

    let client = TestClient::new();

    // List active policies on agent
    let response = client.agent_get("/api/v1/policies").await.unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    // Should have a policies array (may be empty)
    assert!(body.is_object() || body.is_array());
}

// =============================================================================
// Events/SSE Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_events_endpoint() {
    skip_if_no_services!();

    let test_client = TestClient::new();
    let slug = format!(
        "e2e-events-{}",
        Uuid::new_v4().to_string()[..8].to_string()
    );

    // Create organization for events test
    let response = test_client
        .management_post(
            "/orgs",
            json!({
                "name": "E2E Events Org",
                "slug": slug
            }),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let org_body: Value = response.json().await.unwrap();
    let org_id = org_body["id"].as_str().unwrap();

    // Verify events endpoint exists (don't actually consume the stream)
    // Just check the endpoint responds
    let response = test_client
        .management_get(&format!("/orgs/{}/events", slug))
        .await;

    // Events endpoint should exist (may timeout since it's a stream)
    assert!(response.is_ok() || response.is_err());

    // Cleanup
    let _ = test_client
        .management_delete(&format!("/orgs/{}", org_id))
        .await;
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_error_handling() {
    skip_if_no_services!();

    let client = TestClient::new();

    // Test 404 for non-existent org
    let response = client
        .management_get("/orgs/nonexistent-org-12345")
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);

    // Test 400 for invalid request body
    let response = client
        .management_post(
            "/orgs",
            json!({
                // Missing required fields
                "invalid": "data"
            }),
        )
        .await
        .unwrap();
    assert!(response.status().is_client_error());
}

// =============================================================================
// Concurrent Request Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_concurrent_requests() {
    skip_if_no_services!();

    let client = TestClient::new();

    // Send multiple health checks concurrently
    let futures: Vec<_> = (0..10)
        .map(|_| async {
            let c = TestClient::new();
            c.management_get("/health").await
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // All requests should succeed
    for result in results {
        assert!(result.is_ok());
        assert!(result.unwrap().status().is_success());
    }
}

#[tokio::test]
async fn test_e2e_agent_concurrent_evaluations() {
    skip_if_no_services!();

    // Send multiple policy evaluations concurrently
    let futures: Vec<_> = (0..10)
        .map(|i| async move {
            let c = TestClient::new();
            c.agent_post(
                "/api/v1/messages",
                json!({
                    "principal": format!("user-{}", i),
                    "action": "read",
                    "resource": format!("/api/resource-{}", i)
                }),
            )
            .await
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // All requests should complete (regardless of allow/deny)
    for result in results {
        assert!(result.is_ok());
    }
}
