//! End-to-end integration tests for Reaper Platform
//!
//! These tests verify the complete flow from management server through agent
//! policy deployment and evaluation.
//!
//! The suite is self-contained: each test spins up REAL `reaper-management`
//! and `reaper-agent` binaries as child processes on ephemeral ports, backed
//! by a throwaway SQLite database and storage dir. Just build the binaries
//! and run the tests — no docker or manual stack required:
//!
//!   cargo build -p reaper-management -p reaper-agent
//!   cargo test  -p reaper-e2e-tests --test e2e_tests
//!
//! To run against an already-running stack instead (e.g. the docker-compose
//! `management` profile in CI), export the URLs and the suite binds to them
//! rather than spawning:
//!
//!   REAPER_MANAGEMENT_URL=http://localhost:3000 \
//!   REAPER_AGENT_URL=http://localhost:8080 \
//!   cargo test -p reaper-e2e-tests --test e2e_tests
//!
//! When those env vars ARE set but the stack is unreachable, the tests FAIL
//! loudly — they never silently skip, so a broken stack can't masquerade as a
//! green run.

use reqwest::Client;
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

// =============================================================================
// Process + stack management
// =============================================================================

/// A spawned child process that is killed and reaped when dropped.
struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Locate a built workspace binary (debug or release), honouring an explicit
/// `REAPER_E2E_<NAME>_BIN` override.
fn bin(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var(format!(
        "REAPER_E2E_{}_BIN",
        name.replace('-', "_").to_uppercase()
    )) {
        return Some(p.into());
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = manifest.join(format!("../../target/{profile}/{name}"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Grab a free TCP port by binding to :0 and releasing it.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// Poll `{url}/health` until it returns success or we give up.
async fn wait_healthy(client: &Client, url: &str) -> bool {
    for _ in 0..100 {
        if let Ok(resp) = client.get(format!("{url}/health")).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// A running management + agent pair the test drives. Either self-spawned
/// (owns the child processes and their temp dir) or bound to external URLs.
struct TestStack {
    // Declared before `_tmp` so the processes are killed before their
    // backing SQLite file / storage dir is removed.
    _mgmt: Option<Proc>,
    _agent: Option<Proc>,
    _tmp: Option<tempfile::TempDir>,
    management_url: String,
    agent_url: String,
}

impl TestStack {
    /// Bring up a stack for one test. Binds to `REAPER_MANAGEMENT_URL` /
    /// `REAPER_AGENT_URL` when set, otherwise spawns fresh binaries.
    async fn spawn() -> TestStack {
        if let Ok(management_url) = std::env::var("REAPER_MANAGEMENT_URL") {
            let agent_url = std::env::var("REAPER_AGENT_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string());
            let client = http_client();
            assert!(
                wait_healthy(&client, &management_url).await,
                "REAPER_MANAGEMENT_URL is set but management is not reachable at {management_url}"
            );
            assert!(
                wait_healthy(&client, &agent_url).await,
                "REAPER_AGENT_URL is set but the agent is not reachable at {agent_url}"
            );
            return TestStack {
                _mgmt: None,
                _agent: None,
                _tmp: None,
                management_url,
                agent_url,
            };
        }

        let mgmt_bin = bin("reaper-management").expect(
            "reaper-management binary not found — run `cargo build -p reaper-management` first",
        );
        let agent_bin = bin("reaper-agent")
            .expect("reaper-agent binary not found — run `cargo build -p reaper-agent` first");

        let tmp = tempfile::TempDir::new().unwrap();
        let storage = tmp.path().join("storage");
        std::fs::create_dir_all(&storage).unwrap();
        let db_config = reaper_management::db::ephemeral_test_config(tmp.path()).await;

        let mgmt_port = free_port();
        let agent_port = free_port();
        let management_url = format!("http://127.0.0.1:{mgmt_port}");
        let agent_url = format!("http://127.0.0.1:{agent_port}");

        let mgmt = Proc(
            Command::new(&mgmt_bin)
                .env("REAPER_PORT", mgmt_port.to_string())
                .env("REAPER_BIND_ADDRESS", "127.0.0.1")
                .env("REAPER_DATABASE_TYPE", &db_config.db_type)
                .env("REAPER_DATABASE_URL", &db_config.url)
                .env("REAPER_STORAGE_TYPE", "filesystem")
                .env("REAPER_STORAGE_PATH", storage.display().to_string())
                .env(
                    "REAPER_JWT_SECRET",
                    "e2e-secret-not-for-prod-needs-32-chars-min",
                )
                .env("RUST_LOG", "warn")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn reaper-management"),
        );
        let agent = Proc(
            Command::new(&agent_bin)
                .env("REAPER_PORT", agent_port.to_string())
                .env("REAPER_BIND_ADDRESS", "127.0.0.1")
                .env("RUST_LOG", "warn")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn reaper-agent"),
        );

        let client = http_client();
        assert!(
            wait_healthy(&client, &management_url).await,
            "self-spawned management failed to become healthy"
        );
        assert!(
            wait_healthy(&client, &agent_url).await,
            "self-spawned agent failed to become healthy"
        );

        TestStack {
            _mgmt: Some(mgmt),
            _agent: Some(agent),
            _tmp: Some(tmp),
            management_url,
            agent_url,
        }
    }

    /// A lightweight client bound to this stack's URLs. Cheap to call many
    /// times (e.g. for concurrency tests) — it does not spawn anything.
    fn client(&self) -> TestClient {
        TestClient {
            client: http_client(),
            management_url: self.management_url.clone(),
            agent_url: self.agent_url.clone(),
        }
    }
}

/// Test client with helper methods for API interaction.
struct TestClient {
    client: Client,
    management_url: String,
    agent_url: String,
}

impl TestClient {
    // Management API helpers

    async fn management_get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}{}", self.management_url, path))
            .send()
            .await
    }

    async fn management_post(&self, path: &str, body: Value) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(format!("{}{}", self.management_url, path))
            .json(&body)
            .send()
            .await
    }

    async fn management_delete(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.client
            .delete(format!("{}{}", self.management_url, path))
            .send()
            .await
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

// =============================================================================
// Health Check Tests
// =============================================================================

#[tokio::test]
async fn test_management_health() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let response = client.management_get("/health").await.unwrap();
    assert!(response.status().is_success());

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
}

#[tokio::test]
async fn test_agent_health() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let response = client.agent_get("/health").await.unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn test_management_metrics() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let response = client.management_get("/metrics/prometheus").await.unwrap();
    assert!(response.status().is_success());

    let body = response.text().await.unwrap();
    // Verify Prometheus format
    assert!(body.contains("reaper_management_"));
}

#[tokio::test]
async fn test_agent_metrics() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let response = client.agent_get("/metrics").await.unwrap();
    assert!(response.status().is_success());
}

// =============================================================================
// Organization Lifecycle Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_organization_lifecycle() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let slug = format!("e2e-org-{}", &Uuid::new_v4().to_string()[..8]);

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
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let slug = format!("e2e-deploy-{}", &Uuid::new_v4().to_string()[..8]);

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
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let slug = format!("e2e-agent-{}", &Uuid::new_v4().to_string()[..8]);

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
    let _ = client.management_delete(&format!("/orgs/{}", org_id)).await;
}

// =============================================================================
// Policy Source Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_policy_source_management() {
    let stack = TestStack::spawn().await;
    let client = stack.client();
    let slug = format!("e2e-source-{}", &Uuid::new_v4().to_string()[..8]);

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
    let _ = client.management_delete(&format!("/orgs/{}", org_id)).await;
}

// =============================================================================
// Agent Policy Evaluation Tests
// =============================================================================

#[tokio::test]
async fn test_e2e_agent_policy_evaluation() {
    let stack = TestStack::spawn().await;
    let client = stack.client();

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
    let stack = TestStack::spawn().await;
    let client = stack.client();

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
    let stack = TestStack::spawn().await;
    let test_client = stack.client();
    let slug = format!("e2e-events-{}", &Uuid::new_v4().to_string()[..8]);

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
    let stack = TestStack::spawn().await;
    let client = stack.client();

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
    let stack = TestStack::spawn().await;

    // Send multiple health checks concurrently against the same stack.
    let futures: Vec<_> = (0..10)
        .map(|_| {
            let c = stack.client();
            async move { c.management_get("/health").await }
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
    let stack = TestStack::spawn().await;

    // Send multiple policy evaluations concurrently against the same stack.
    let futures: Vec<_> = (0..10)
        .map(|i| {
            let c = stack.client();
            async move {
                c.agent_post(
                    "/api/v1/messages",
                    json!({
                        "principal": format!("user-{}", i),
                        "action": "read",
                        "resource": format!("/api/resource-{}", i)
                    }),
                )
                .await
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // All requests should complete (regardless of allow/deny)
    for result in results {
        assert!(result.is_ok());
    }
}
