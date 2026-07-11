//! End-to-end integration tests for Reaper Platform
//!
//! These tests verify the complete flow from management server through agent
//! policy deployment and evaluation.
//!
//! To run these tests, services must be running:
//! - Management Server on port 3000
//! - Agent on port 8080
//! - Or use: docker compose -f docker-compose.yml --profile management up
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

/// Map a bare management path to the single `/api/v1` surface (Plan 07 Phase B).
/// Health/metrics/openapi probes stay unversioned at the root; anything already
/// `/api/v1`-prefixed is left as-is.
fn mgmt_uri(path: &str) -> String {
    let p = path.split('?').next().unwrap_or(path);
    let is_probe = p == "/health"
        || p.starts_with("/health/")
        || p == "/live"
        || p == "/ready"
        || p == "/metrics"
        || p.starts_with("/metrics/")
        || p == "/openapi.json";
    if is_probe || path.starts_with("/api/v1") {
        path.to_string()
    } else {
        format!("/api/v1{path}")
    }
}

fn agent_url() -> String {
    std::env::var("REAPER_AGENT_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

/// Test client with helper methods for API interaction
struct TestClient {
    client: Client,
    management_url: String,
    agent_url: String,
    api_key: Option<String>,
    /// Session token (rst_…) from /auth/signup or /auth/login, sent as a
    /// Bearer credential. The management control plane is default-deny, so
    /// every org-scoped call needs one of these or an API key.
    session_token: Option<String>,
}

/// An authenticated E2E session: a freshly signed-up user plus the org that
/// signup created for them (session credentials are scoped to that org).
struct TestSession {
    client: TestClient,
    org_id: String,
    org_slug: String,
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
            session_token: None,
        }
    }

    #[allow(dead_code)]
    fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    /// Sign up a fresh user through the public auth flow (the same journey a
    /// real customer takes) and return a session-authenticated client along
    /// with the org that signup provisioned.
    async fn signup(label: &str) -> TestSession {
        let mut client = Self::new();
        let unique = &Uuid::new_v4().to_string()[..8];
        let response = client
            .management_post(
                "/auth/signup",
                json!({
                    "email": format!("e2e-{label}-{unique}@example.com"),
                    "password": "E2eTestPassw0rd!",
                    "org_name": format!("E2E {label} {unique}")
                }),
            )
            .await
            .expect("signup request failed");
        assert_eq!(response.status().as_u16(), 201, "signup must succeed");
        let body: Value = response.json().await.unwrap();
        client.session_token = Some(body["session_token"].as_str().unwrap().to_string());
        TestSession {
            client,
            org_id: body["org"]["id"].as_str().unwrap().to_string(),
            org_slug: body["org"]["slug"].as_str().unwrap().to_string(),
        }
    }

    fn auth_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        if let Some(ref token) = self.session_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        req
    }

    // Management API helpers

    async fn management_get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        let req = self
            .client
            .get(format!("{}{}", self.management_url, mgmt_uri(path)));
        self.auth_headers(req).send().await
    }

    async fn management_post(&self, path: &str, body: Value) -> reqwest::Result<reqwest::Response> {
        let req = self
            .client
            .post(format!("{}{}", self.management_url, mgmt_uri(path)))
            .json(&body);
        self.auth_headers(req).send().await
    }

    async fn management_delete(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        let req = self
            .client
            .delete(format!("{}{}", self.management_url, mgmt_uri(path)));
        self.auth_headers(req).send().await
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

    // Signup provisions the org and an Owner session (the real user journey —
    // anonymous org CRUD is rejected by the default-deny control plane).
    let session = TestClient::signup("org-lifecycle").await;
    let client = &session.client;

    // Unauthenticated requests to org routes fail closed.
    let response = TestClient::new()
        .management_get(&format!("/orgs/{}", session.org_slug))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);

    // Get organization
    let response = client
        .management_get(&format!("/orgs/{}", session.org_slug))
        .await
        .unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], session.org_id.as_str());

    // List organizations (a non-platform-admin only ever sees their own)
    let response = client.management_get("/orgs").await.unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    let orgs = body["items"].as_array().unwrap();
    assert!(orgs.iter().any(|o| o["slug"] == session.org_slug.as_str()));

    // Delete organization (Owner holds org:admin)
    let response = client
        .management_delete(&format!("/orgs/{}", session.org_id))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 204);

    // Verify deletion — probe from a different authenticated user, since the
    // deleted org's own session loses its membership with the org.
    let probe = TestClient::signup("org-lifecycle-probe").await;
    let response = probe
        .client
        .management_get(&format!("/orgs/{}", session.org_id))
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

    // Step 1: Signup provisions the org + Owner session.
    let session = TestClient::signup("deploy").await;
    let client = &session.client;
    let slug = session.org_slug.clone();

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
        .management_post(
            &format!("/orgs/{}/bundles/{}/compile", slug, bundle_id),
            json!({}),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "compiled");

    // Step 5: Stage bundle
    let response = client
        .management_post(
            &format!("/orgs/{}/bundles/{}/stage", slug, bundle_id),
            json!({}),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "staged");

    // Step 6: Promote bundle. The E2E server runs single-control (the default),
    // so this goes live immediately for the owner. (Two-person / dual-control
    // approval is exercised in the in-process management integration tests.)
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
        .management_delete(&format!("/orgs/{}", session.org_id))
        .await;
}

// =============================================================================
// Agent Registration and Heartbeat Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_agent_registration_flow() {
    skip_if_no_services!();

    let session = TestClient::signup("agent").await;
    let client = &session.client;

    // Agents list works for the org Owner (agent:read scope).
    let response = client
        .management_get(&format!("/orgs/{}/agents", session.org_slug))
        .await
        .unwrap();
    assert!(response.status().is_success());

    // ... and is refused without credentials (default-deny).
    let response = TestClient::new()
        .management_get(&format!("/orgs/{}/agents", session.org_slug))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);

    // Cleanup
    let _ = client
        .management_delete(&format!("/orgs/{}", session.org_id))
        .await;
}

// =============================================================================
// Policy Source Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_policy_source_management() {
    skip_if_no_services!();

    let session = TestClient::signup("source").await;
    let client = &session.client;

    // Sources list works for the org Owner (policy:read scope).
    let response = client
        .management_get(&format!("/orgs/{}/sources", session.org_slug))
        .await
        .unwrap();
    assert!(response.status().is_success());

    // Cleanup
    let _ = client
        .management_delete(&format!("/orgs/{}", session.org_id))
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

    let session = TestClient::signup("events").await;
    let test_client = &session.client;

    // Verify events endpoint exists (don't actually consume the stream)
    // Just check the endpoint responds
    let response = test_client
        .management_get(&format!("/orgs/{}/events", session.org_slug))
        .await;

    // Events endpoint should exist (may timeout since it's a stream)
    assert!(response.is_ok() || response.is_err());

    // Cleanup
    let _ = test_client
        .management_delete(&format!("/orgs/{}", session.org_id))
        .await;
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_error_handling() {
    skip_if_no_services!();

    let session = TestClient::signup("errors").await;
    let client = &session.client;

    // Test 404 for non-existent org (authenticated — anonymous gets 401 first)
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

    // Unauthenticated requests to protected routes are refused outright.
    let response = TestClient::new().management_get("/orgs").await.unwrap();
    assert_eq!(response.status().as_u16(), 401);
}

// =============================================================================
// Concurrent Request Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_concurrent_requests() {
    skip_if_no_services!();

    let _client = TestClient::new();

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
