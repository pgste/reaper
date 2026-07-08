//! Integration tests for Reaper Management Server
//!
//! Tests the full API workflow from organization creation through bundle promotion.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reaper_management::{
    api::build_api_router,
    auth::api_key::{ApiKeyRepository, CreateApiKey},
    auth::jwks::JwksConfigRepository,
    config::{AuthConfig, Config},
    db::repositories::{AgentRepository, OrganizationRepository},
    db::Database,
    domain::organization::CreateOrganization,
    storage::FilesystemStorage,
    AppState,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

/// Test environment with database and API key support
struct TestEnv {
    #[allow(dead_code)]
    temp_dir: TempDir,
    app: axum::Router,
    db: Arc<Database>,
}

/// Test helper to set up a test environment
async fn setup_test_env() -> TestEnv {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();

    let db_config = reaper_management::db::ephemeral_test_config(temp_dir.path()).await;

    let db = Database::new(&db_config).await.unwrap();
    db.run_migrations().await.unwrap();
    let db = Arc::new(db);

    let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap())
        as Arc<dyn reaper_management::storage::BundleStorage>;

    // Create config with JWT secret for testing
    let config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        ..Config::default()
    };

    let state = AppState::new(db.clone(), config, storage);
    let app = build_api_router().with_state(Arc::new(state));

    TestEnv { temp_dir, app, db }
}

/// Helper to make JSON requests without auth
fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().uri(uri).method(method);

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }

    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());

    builder.body(body).unwrap()
}

/// Helper to make authenticated JSON requests
fn authed_request(method: &str, uri: &str, body: Option<Value>, api_key: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("X-API-Key", api_key);

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }

    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());

    builder.body(body).unwrap()
}

/// Create an API key for testing
async fn create_test_api_key(db: &Database, org_id: Uuid) -> String {
    let api_key_repo = ApiKeyRepository::new(db);
    let created = api_key_repo
        .create(
            org_id,
            CreateApiKey {
                name: "test-key".to_string(),
                scopes: vec![
                    "admin".to_string(),
                    "agent:register".to_string(),
                    "agent:read".to_string(),
                    "agent:write".to_string(),
                    "source:read".to_string(),
                    "source:write".to_string(),
                ],
                expires_at: None,
                created_by: None,
            },
        )
        .await
        .unwrap();
    created.key
}

/// Create an API key with an explicit (non-admin) scope list — for tests that
/// exercise the tenant guard and scope checks, which the platform `admin`
/// scope would bypass.
async fn create_scoped_api_key(db: &Database, org_id: Uuid, scopes: &[&str]) -> String {
    let api_key_repo = ApiKeyRepository::new(db);
    let created = api_key_repo
        .create(
            org_id,
            CreateApiKey {
                // api_keys has UNIQUE(org_id, name); randomize so one org can
                // mint several scoped keys in a test.
                name: format!("scoped-key-{}", Uuid::new_v4()),
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
                expires_at: None,
                created_by: None,
            },
        )
        .await
        .unwrap();
    created.key
}

/// Parse JSON response body
async fn parse_body(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or(json!({}))
}

#[tokio::test]
async fn test_health_endpoint() {
    let env = setup_test_env().await;

    let response = env
        .app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_organization_crud() {
    let env = setup_test_env().await;

    // Create organization
    let create_req = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Test Organization",
            "slug": "test-org"
        })),
    );

    let response = env.app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let org_id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["name"], "Test Organization");
    assert_eq!(body["slug"], "test-org");

    // Org reads/mutations now require authentication (Phase B).
    let key = create_test_api_key(&env.db, org_id.parse().unwrap()).await;

    // Get organization by slug
    let get_req = authed_request("GET", "/orgs/test-org", None, &key);
    let response = env.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["id"], org_id.as_str());

    // List organizations
    let list_req = authed_request("GET", "/orgs", None, &key);
    let response = env.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert!(!body["organizations"].as_array().unwrap().is_empty());

    // Update organization
    let update_req = authed_request(
        "PUT",
        &format!("/orgs/{}", org_id),
        Some(json!({
            "display_name": "Updated Name"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(update_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // A second org's admin key, minted BEFORE the delete: deleting an org
    // cascades away its own API keys, so the probe must come from elsewhere.
    let probe_org = OrganizationRepository::new(&env.db)
        .create(CreateOrganization {
            name: "Probe Org".to_string(),
            slug: "probe-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        })
        .await
        .unwrap();
    let probe_key = create_test_api_key(&env.db, probe_org.id).await;

    // Delete organization
    let delete_req = authed_request("DELETE", &format!("/orgs/{}", org_id), None, &key);
    let response = env.app.clone().oneshot(delete_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify deletion (platform-admin key from the probe org sees 404)
    let get_req = authed_request("GET", &format!("/orgs/{}", org_id), None, &probe_key);
    let response = env.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_policy_lifecycle() {
    let env = setup_test_env().await;

    // Create organization first
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Policy Test Org",
            "slug": "policy-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    // Create policy
    let create_policy = authed_request(
        "POST",
        "/orgs/policy-org/policies",
        Some(json!({
            "name": "test-policy",
            "description": "A test policy",
            "language": "reaper",
            "content": "allow admin to access /admin"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let policy_id = body["id"].as_str().unwrap();
    assert_eq!(body["name"], "test-policy");

    // Get policy
    let get_policy = authed_request(
        "GET",
        &format!("/orgs/policy-org/policies/{}", policy_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(get_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update policy (creates new version)
    let update_policy = authed_request(
        "PUT",
        &format!("/orgs/policy-org/policies/{}", policy_id),
        Some(json!({
            "content": "allow admin to access /admin/*"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(update_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List policies
    let list_policies = authed_request("GET", "/orgs/policy-org/policies", None, &key);
    let response = env.app.clone().oneshot(list_policies).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["policies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_bundle_workflow() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Bundle Test Org",
            "slug": "bundle-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    // Create policy
    let create_policy = authed_request(
        "POST",
        "/orgs/bundle-org/policies",
        Some(json!({
            "name": "bundle-policy",
            "language": "reaper",
            "content": "allow user to read /api"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_body = parse_body(response).await;
    let policy_id = policy_body["id"].as_str().unwrap();

    // Create bundle with policy
    let create_bundle = authed_request(
        "POST",
        "/orgs/bundle-org/bundles",
        Some(json!({
            "name": "test-bundle",
            "description": "Test bundle",
            "policy_ids": [policy_id]
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let bundle_id = body["id"].as_str().unwrap();
    assert_eq!(body["status"], "draft");
    assert_eq!(body["policy_count"], 1);

    // Compile bundle
    let compile = authed_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/compile", bundle_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(compile).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "compiled");
    assert!(body["storage_key"].as_str().is_some());
    assert!(body["checksum"].as_str().is_some());

    // Stage bundle
    let stage = authed_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/stage", bundle_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(stage).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "staged");

    // Promote bundle
    let promote = authed_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/promote", bundle_id),
        Some(json!({
            "notes": "Initial release"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(promote).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "promoted");

    // Get promoted bundle
    let get_promoted = authed_request("GET", "/orgs/bundle-org/bundles/promoted", None, &key);
    let response = env.app.clone().oneshot(get_promoted).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["id"], bundle_id);

    // Download bundle
    let download = authed_request(
        "GET",
        &format!("/orgs/bundle-org/bundles/{}/download", bundle_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(download).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
}

#[tokio::test]
async fn test_agent_registration() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Agent Test Org",
            "slug": "agent-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // Create API key for the org
    let api_key = create_test_api_key(&env.db, org_id).await;

    // Register agent (requires auth)
    let register = authed_request(
        "POST",
        "/orgs/agent-org/agents/register",
        Some(json!({
            "name": "test-agent-1",
            "hostname": "localhost",
            "version": "1.0.0",
            "labels": {}
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(register).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let agent_id = body["agent"]["id"].as_str().unwrap();
    assert_eq!(body["agent"]["name"], "test-agent-1");
    assert_eq!(body["agent"]["status"], "active");

    // Get agent (requires auth)
    let get_agent = authed_request(
        "GET",
        &format!("/orgs/agent-org/agents/{}", agent_id),
        None,
        &api_key,
    );
    let response = env.app.clone().oneshot(get_agent).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Send heartbeat (requires auth)
    let heartbeat = authed_request(
        "POST",
        &format!("/orgs/agent-org/agents/{}/heartbeat", agent_id),
        Some(json!({
            "status": "healthy",
            "metrics": {
                "requests_per_second": 1000,
                "avg_latency_us": 50
            }
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(heartbeat).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List agents (requires auth)
    let list_agents = authed_request("GET", "/orgs/agent-org/agents", None, &api_key);
    let response = env.app.clone().oneshot(list_agents).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_policy_source_crud() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Source Test Org",
            "slug": "source-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // Create API key for the org
    let api_key = create_test_api_key(&env.db, org_id).await;

    // Create Git source (requires auth)
    let create_source = authed_request(
        "POST",
        "/orgs/source-org/sources",
        Some(json!({
            "name": "main-policies",
            "source_type": "git",
            "config": {
                "url": "https://github.com/example/policies.git",
                "branch": "main",
                "path": "policies/"
            },
            "sync_interval_secs": 300
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(create_source).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let source_id = body["id"].as_str().unwrap();
    assert_eq!(body["name"], "main-policies");
    assert_eq!(body["source_type"], "git");

    // Get source (requires auth)
    let get_source = authed_request(
        "GET",
        &format!("/orgs/source-org/sources/{}", source_id),
        None,
        &api_key,
    );
    let response = env.app.clone().oneshot(get_source).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List sources (requires auth)
    let list_sources = authed_request("GET", "/orgs/source-org/sources", None, &api_key);
    let response = env.app.clone().oneshot(list_sources).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["sources"].as_array().unwrap().len(), 1);

    // Delete source (requires auth)
    let delete_source = authed_request(
        "DELETE",
        &format!("/orgs/source-org/sources/{}", source_id),
        None,
        &api_key,
    );
    let response = env.app.clone().oneshot(delete_source).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_invalid_requests() {
    let env = setup_test_env().await;

    // Create org with missing required field
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({
                "name": "Missing Slug"
                // slug is required
            })),
        ))
        .await
        .unwrap();
    assert!(response.status().is_client_error());

    // Create org first (org reads now require auth, so we need a key)
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Error Test Org",
            "slug": "error-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    // Get non-existent organization (authenticated; platform-admin key)
    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", "/orgs/nonexistent", None, &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Get non-existent policy
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/error-org/policies/00000000-0000-0000-0000-000000000000",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Get non-existent bundle
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/error-org/bundles/00000000-0000-0000-0000-000000000000",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_bundle_invalid_transitions() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Transition Test Org",
            "slug": "transition-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    // Create bundle (empty, no policies)
    let create_bundle = authed_request(
        "POST",
        "/orgs/transition-org/bundles",
        Some(json!({
            "name": "empty-bundle"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    let body = parse_body(response).await;
    let bundle_id = body["id"].as_str().unwrap();

    // Try to compile empty bundle - should fail
    let compile = authed_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/compile", bundle_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(compile).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Try to stage draft bundle - should fail
    let stage = authed_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/stage", bundle_id),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(stage).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Try to promote draft bundle - should fail
    let promote = authed_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/promote", bundle_id),
        Some(json!({})),
        &key,
    );
    let response = env.app.clone().oneshot(promote).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// =============================================================================
// Phase 3: New Integration Tests for Metrics and JWKS
// =============================================================================

/// Test that heartbeat stores agent metrics in the database
#[tokio::test]
async fn test_heartbeat_stores_metrics() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Metrics Test Org",
            "slug": "metrics-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // Create API key for the org
    let api_key = create_test_api_key(&env.db, org_id).await;

    // Register agent
    let register = authed_request(
        "POST",
        "/orgs/metrics-org/agents/register",
        Some(json!({
            "name": "metrics-agent",
            "hostname": "localhost",
            "version": "1.0.0"
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(register).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let agent_id: Uuid = body["agent"]["id"].as_str().unwrap().parse().unwrap();

    // Send heartbeat with comprehensive metrics
    let heartbeat = authed_request(
        "POST",
        &format!("/orgs/metrics-org/agents/{}/heartbeat", agent_id),
        Some(json!({
            "status": "healthy",
            "metrics": {
                "requests_total": 50000,
                "requests_per_second": 1250.5,
                "avg_latency_us": 45.0,
                "p50_latency_us": 35.0,
                "p99_latency_us": 150.0,
                "memory_bytes": 52428800,
                "cpu_percent": 25.5,
                "decisions_allow": 48000,
                "decisions_deny": 2000,
                "uptime_seconds": 3600,
                "current_bundle_id": null,
                "current_bundle_version": null
            }
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(heartbeat).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify metrics were stored
    let agent_repo = AgentRepository::new(&env.db);
    let stored_metrics = agent_repo
        .get_metrics(agent_id)
        .await
        .expect("Should query metrics")
        .expect("Metrics should exist");

    assert_eq!(stored_metrics.requests_total, 50000);
    assert!((stored_metrics.requests_per_second - 1250.5).abs() < 0.01);
    assert_eq!(stored_metrics.decisions_allow, 48000);
    assert_eq!(stored_metrics.decisions_deny, 2000);
    assert_eq!(stored_metrics.memory_bytes, 52428800);
}

/// Test that heartbeat without metrics still works
#[tokio::test]
async fn test_heartbeat_without_metrics() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "No Metrics Org",
            "slug": "no-metrics-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let api_key = create_test_api_key(&env.db, org_id).await;

    // Register agent
    let register = authed_request(
        "POST",
        "/orgs/no-metrics-org/agents/register",
        Some(json!({
            "name": "simple-agent",
            "hostname": "localhost"
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(register).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let agent_id = body["agent"]["id"].as_str().unwrap();

    // Send heartbeat without metrics - should still work
    let heartbeat = authed_request(
        "POST",
        &format!("/orgs/no-metrics-org/agents/{}/heartbeat", agent_id),
        Some(json!({
            "status": "healthy"
        })),
        &api_key,
    );
    let response = env.app.clone().oneshot(heartbeat).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["acknowledged"], true);
}

/// Test extract_issuer_from_token function
#[tokio::test]
async fn test_extract_issuer_from_token() {
    use reaper_management::auth::jwks::extract_issuer_from_token;

    // Create a simple JWT with issuer claim (not cryptographically valid, just for parsing)
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(r#"{"iss":"https://auth.example.com","sub":"user123"}"#);
    let signature = "fake_signature";

    let token = format!("{}.{}.{}", header, payload, signature);

    let issuer = extract_issuer_from_token(&token);
    assert_eq!(issuer, Some("https://auth.example.com".to_string()));

    // Test with missing issuer
    let payload_no_iss = URL_SAFE_NO_PAD.encode(r#"{"sub":"user123"}"#);
    let token_no_iss = format!("{}.{}.{}", header, payload_no_iss, signature);
    assert_eq!(extract_issuer_from_token(&token_no_iss), None);

    // Test with invalid token format
    assert_eq!(extract_issuer_from_token("not.a.valid.jwt.token"), None);
    assert_eq!(extract_issuer_from_token(""), None);
}

/// Test JWKS config repository find_by_issuer
#[tokio::test]
async fn test_jwks_find_by_issuer() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "JWKS Test Org",
            "slug": "jwks-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let jwks_repo = JwksConfigRepository::new(&env.db);

    // Create a JWKS config
    let config = jwks_repo
        .create(
            org_id,
            "test-idp",
            "https://idp.example.com/.well-known/jwks.json",
            "https://idp.example.com",
            Some("test-audience"),
        )
        .await
        .expect("Should create JWKS config");

    assert_eq!(config.issuer, "https://idp.example.com");
    assert!(config.is_active);

    // Find by issuer
    let configs = jwks_repo
        .find_by_issuer("https://idp.example.com")
        .await
        .expect("Should find configs");
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].id, config.id);

    // Find by non-existent issuer
    let configs = jwks_repo
        .find_by_issuer("https://other.example.com")
        .await
        .expect("Should return empty");
    assert!(configs.is_empty());

    // Disable config and verify it's not returned
    jwks_repo
        .set_active(config.id, false)
        .await
        .expect("Should disable");
    let configs = jwks_repo
        .find_by_issuer("https://idp.example.com")
        .await
        .expect("Should return empty");
    assert!(configs.is_empty());
}

/// Test that JWKS config can be created and retrieved
#[tokio::test]
async fn test_jwks_config_lifecycle() {
    let env = setup_test_env().await;

    // Create organization
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "JWKS Lifecycle Org",
            "slug": "jwks-lifecycle-org"
        })),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let jwks_repo = JwksConfigRepository::new(&env.db);

    // Create config
    let config = jwks_repo
        .create(
            org_id,
            "auth0-config",
            "https://tenant.auth0.com/.well-known/jwks.json",
            "https://tenant.auth0.com/",
            Some("my-api"),
        )
        .await
        .expect("Should create");

    // Retrieve by ID
    let retrieved = jwks_repo
        .get_by_id(config.id)
        .await
        .expect("Should get")
        .expect("Should exist");
    assert_eq!(retrieved.name, "auth0-config");
    assert_eq!(
        retrieved.jwks_url,
        "https://tenant.auth0.com/.well-known/jwks.json"
    );
    assert_eq!(retrieved.audience, Some("my-api".to_string()));

    // List active for org
    let active = jwks_repo.list_active(org_id).await.expect("Should list");
    assert_eq!(active.len(), 1);

    // List all for org
    let all = jwks_repo.list_all(org_id).await.expect("Should list all");
    assert_eq!(all.len(), 1);

    // Delete
    let deleted = jwks_repo.delete(config.id).await.expect("Should delete");
    assert!(deleted);

    // Verify deletion
    let retrieved = jwks_repo.get_by_id(config.id).await.expect("Should get");
    assert!(retrieved.is_none());
}

// =============================================================================
// Phase 1: SaaS Foundation Integration Tests
// User Authentication, Audit Logging, Rate Limiting
// =============================================================================

/// Helper to make requests with session token
fn session_request(method: &str, uri: &str, body: Option<Value>, token: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("Authorization", format!("Bearer {}", token));

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }

    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());

    builder.body(body).unwrap()
}

/// Test user signup creates user and organization
#[tokio::test]
async fn test_user_signup() {
    let env = setup_test_env().await;

    // Signup new user
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "test@example.com",
            "password": "SecurePass123!",
            "org_name": "Test Company"
        })),
    );

    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    assert!(body["session_token"].as_str().is_some());
    assert!(body["session_token"].as_str().unwrap().starts_with("rst_"));
    assert_eq!(body["user"]["email"], "test@example.com");
    assert!(body["org"]["id"].as_str().is_some());
    assert_eq!(body["org"]["name"], "Test Company");
}

/// Test user signup with existing email fails
#[tokio::test]
async fn test_user_signup_duplicate_email() {
    let env = setup_test_env().await;

    // First signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "duplicate@example.com",
            "password": "SecurePass123!",
            "org_name": "First Company"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Second signup with same email should fail
    let signup_req2 = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "duplicate@example.com",
            "password": "AnotherPass456!",
            "org_name": "Second Company"
        })),
    );
    let response = env.app.clone().oneshot(signup_req2).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// Test user signup with weak password fails
#[tokio::test]
async fn test_user_signup_weak_password() {
    let env = setup_test_env().await;

    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "weak@example.com",
            "password": "short",
            "org_name": "Weak Pass Company"
        })),
    );

    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Test user login with valid credentials
#[tokio::test]
async fn test_user_login() {
    let env = setup_test_env().await;

    // First signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "login@example.com",
            "password": "SecurePass123!",
            "org_name": "Login Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Now login
    let login_req = json_request(
        "POST",
        "/auth/login",
        Some(json!({
            "email": "login@example.com",
            "password": "SecurePass123!"
        })),
    );
    let response = env.app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert!(body["session_token"].as_str().is_some());
    assert!(body["session_token"].as_str().unwrap().starts_with("rst_"));
    assert_eq!(body["user"]["email"], "login@example.com");
    assert!(!body["orgs"].as_array().unwrap().is_empty());
}

/// Test user login with invalid password
#[tokio::test]
async fn test_user_login_invalid_password() {
    let env = setup_test_env().await;

    // First signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "invalid@example.com",
            "password": "SecurePass123!",
            "org_name": "Invalid Test Org"
        })),
    );
    env.app.clone().oneshot(signup_req).await.unwrap();

    // Login with wrong password
    let login_req = json_request(
        "POST",
        "/auth/login",
        Some(json!({
            "email": "invalid@example.com",
            "password": "WrongPassword!"
        })),
    );
    let response = env.app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Test session token authentication for /auth/me
#[tokio::test]
async fn test_session_token_auth() {
    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "session@example.com",
            "password": "SecurePass123!",
            "org_name": "Session Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap();

    // Use session token to access /auth/me
    let me_req = session_request("GET", "/auth/me", None, token);
    let response = env.app.clone().oneshot(me_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["user"]["email"], "session@example.com");
}

/// Test logout invalidates session
#[tokio::test]
async fn test_user_logout() {
    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "logout@example.com",
            "password": "SecurePass123!",
            "org_name": "Logout Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap().to_string();

    // Logout
    let logout_req = session_request("POST", "/auth/logout", None, &token);
    let response = env.app.clone().oneshot(logout_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Try to use the token again - should fail
    let me_req = session_request("GET", "/auth/me", None, &token);
    let response = env.app.clone().oneshot(me_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Test audit logging for signup
#[tokio::test]
async fn test_audit_log_signup() {
    use reaper_management::audit::{AuditQuery, AuditRepository};

    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "audit@example.com",
            "password": "SecurePass123!",
            "org_name": "Audit Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let org_id: Uuid = body["org"]["id"].as_str().unwrap().parse().unwrap();

    // Query audit log
    let audit_repo = AuditRepository::new(&env.db);
    let entries = audit_repo
        .query(&AuditQuery {
            org_id: Some(org_id),
            action: Some("user.signup".to_string()),
            ..Default::default()
        })
        .await
        .expect("Should query audit log");

    assert!(!entries.is_empty());
    assert_eq!(entries[0].action, "user.signup");
}

/// Test audit logging for login
#[tokio::test]
async fn test_audit_log_login() {
    use reaper_management::audit::{AuditQuery, AuditRepository};

    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "auditlogin@example.com",
            "password": "SecurePass123!",
            "org_name": "Audit Login Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let org_id: Uuid = body["org"]["id"].as_str().unwrap().parse().unwrap();

    // Login
    let login_req = json_request(
        "POST",
        "/auth/login",
        Some(json!({
            "email": "auditlogin@example.com",
            "password": "SecurePass123!"
        })),
    );
    env.app.clone().oneshot(login_req).await.unwrap();

    // Query audit log for login
    let audit_repo = AuditRepository::new(&env.db);
    let entries = audit_repo
        .query(&AuditQuery {
            org_id: Some(org_id),
            action: Some("user.login".to_string()),
            ..Default::default()
        })
        .await
        .expect("Should query audit log");

    assert!(!entries.is_empty());
    assert_eq!(entries[0].action, "user.login");
}

/// Test org member management
#[tokio::test]
async fn test_org_member_list() {
    let env = setup_test_env().await;

    // Signup creates owner membership
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "owner@example.com",
            "password": "SecurePass123!",
            "org_name": "Member Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap();
    let org_slug = body["org"]["slug"].as_str().unwrap();

    // List members
    let list_req = session_request("GET", &format!("/orgs/{}/members", org_slug), None, token);
    let response = env.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["role"], "owner");
    assert_eq!(members[0]["user"]["email"], "owner@example.com");
}

/// Test password change
#[tokio::test]
async fn test_password_change() {
    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "pwchange@example.com",
            "password": "OldPassword123!",
            "org_name": "Password Change Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap();

    // Change password
    let change_req = session_request(
        "POST",
        "/auth/password/change",
        Some(json!({
            "current_password": "OldPassword123!",
            "new_password": "NewPassword456!"
        })),
        token,
    );
    let response = env.app.clone().oneshot(change_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Old session should be invalidated, login with new password
    let login_req = json_request(
        "POST",
        "/auth/login",
        Some(json!({
            "email": "pwchange@example.com",
            "password": "NewPassword456!"
        })),
    );
    let response = env.app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Old password should no longer work
    let login_req_old = json_request(
        "POST",
        "/auth/login",
        Some(json!({
            "email": "pwchange@example.com",
            "password": "OldPassword123!"
        })),
    );
    let response = env.app.clone().oneshot(login_req_old).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Test password reset request (doesn't actually send email)
#[tokio::test]
async fn test_password_reset_request() {
    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "reset@example.com",
            "password": "SecurePass123!",
            "org_name": "Reset Test Org"
        })),
    );
    env.app.clone().oneshot(signup_req).await.unwrap();

    // Request password reset
    let reset_req = json_request(
        "POST",
        "/auth/password/reset-request",
        Some(json!({
            "email": "reset@example.com"
        })),
    );
    let response = env.app.clone().oneshot(reset_req).await.unwrap();
    // Returns ACCEPTED to not reveal if email exists
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

/// Test session token doesn't work for API key endpoints
#[tokio::test]
async fn test_session_token_with_api_endpoints() {
    let env = setup_test_env().await;

    // Signup
    let signup_req = json_request(
        "POST",
        "/auth/signup",
        Some(json!({
            "email": "apitest@example.com",
            "password": "SecurePass123!",
            "org_name": "API Test Org"
        })),
    );
    let response = env.app.clone().oneshot(signup_req).await.unwrap();
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap();
    let org_slug = body["org"]["slug"].as_str().unwrap();

    // Use session token to access API endpoints (should work with owner role)
    let list_agents = session_request("GET", &format!("/orgs/{}/agents", org_slug), None, token);
    let response = env.app.clone().oneshot(list_agents).await.unwrap();
    // Owner has admin scope, so this should work
    assert_eq!(response.status(), StatusCode::OK);
}

// ============================================================================
// Data plane (Authorization Data Model) — Phase D1
// ============================================================================

/// The full loop: provision a datastore, manage typed data through the
/// managers' APIs, publish a checksummed version, then load the materialized
/// document into the ACTUAL policy engine and watch a combined
/// RBAC+ABAC+ReBAC policy decide with it. This is the "closes the loop"
/// guarantee: data managed in the control plane drives real decisions.
#[tokio::test]
async fn test_data_plane_end_to_end() {
    use policy_engine::reap::ReaperPolicy;
    use policy_engine::{DataLoader, DataStore, PolicyRequest};
    use std::collections::HashMap;

    let env = setup_test_env().await;

    // --- Org + namespace + key ---
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Data Plane Org", "slug": "dp-org"})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id = Uuid::parse_str(parse_body(response).await["id"].as_str().unwrap()).unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/dp-org/namespaces",
            Some(json!({"slug": "prod", "display_name": "Production"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let base = "/orgs/dp-org/namespaces/prod/datastore";

    // --- Provision from the combined template ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            base,
            Some(json!({"template": "combined"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = parse_body(response).await;
    assert_eq!(body["template"], "combined");
    assert!(body["model"]["roles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == "editor"));

    // Provisioning twice conflicts.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            base,
            Some(json!({"template": "rbac"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // --- Entities: typed validation is enforced at write time ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/entities"),
            Some(json!({
                "entity_id": "alice", "entity_type": "user",
                "attributes": {"mfa": true, "clearance": 5, "department": "eng"}
            })),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // clearance as a STRING must be rejected (type-strict at the source).
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/entities"),
            Some(json!({
                "entity_id": "bob", "entity_type": "user",
                "attributes": {"clearance": "5"}
            })),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // bob, correctly typed, WITHOUT mfa (absence must fail guards downstream).
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/entities"),
            Some(json!({
                "entity_id": "bob", "entity_type": "user",
                "attributes": {"clearance": 2}
            })),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // --- Role bindings (vocabulary-checked) ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/role-bindings"),
            Some(json!({"subject": "alice", "role": "editor"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/role-bindings"),
            Some(json!({"subject": "alice", "role": "warlock"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // --- Tuples (relation vocabulary-checked) ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/tuples"),
            Some(json!({"object": "doc-1", "relation": "owner", "subject": "bob"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/tuples"),
            Some(json!({"object": "doc-1", "relation": "haunts", "subject": "bob"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Group membership (TRAVERSAL relation): carol ∈ team-eng, and team-eng
    // holds viewer on doc-1. Access flows through the graph, not a direct
    // grant — the exact pattern tuple-only stores sell as their headline.
    for tuple in [
        json!({"object": "team-eng", "relation": "member_of", "subject": "carol"}),
        json!({"object": "doc-1", "relation": "viewer", "subject": "team-eng"}),
    ] {
        let response = env
            .app
            .clone()
            .oneshot(authed_request(
                "POST",
                &format!("{base}/tuples"),
                Some(tuple),
                &key,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/entities"),
            Some(json!({"entity_id": "carol", "entity_type": "user", "attributes": {}})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Scoped bindings must be rejected until the materializer represents
    // them — a scoped grant silently going global would be a breach.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/role-bindings"),
            Some(json!({"subject": "alice", "role": "viewer", "scope": "doc-9"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // --- Publish: immutable, checksummed version ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/publish"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let published = parse_body(response).await;
    assert_eq!(published["version"], 1);
    let checksum = published["checksum"].as_str().unwrap().to_string();
    assert!(checksum.starts_with("sha256:"));

    // --- Fetch the version document (what agents load) ---
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("{base}/versions/1"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let version_body = parse_body(response).await;
    let document = version_body["document"].clone();

    // Agent-side integrity contract: recomputing sha256 over the canonical
    // serde_json serialization must reproduce the published checksum.
    {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&document).unwrap();
        let computed = format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()));
        assert_eq!(computed, checksum, "canonical checksum must round-trip");
    }

    // --- THE LOOP: load into the real policy engine and decide ---
    let store = std::sync::Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(&serde_json::to_string(&document).unwrap())
        .unwrap();

    let policy: ReaperPolicy = r#"
        policy data_plane_loop {
            default: deny,
            rule editors_with_mfa {
                allow if {
                    "editor" in user.roles &&
                    user.mfa == true
                }
            }
            rule owners {
                allow if rebac::related(user, "owner", resource)
            }
            rule team_viewers_can_read {
                allow if {
                    context.action == "read" &&
                    rebac::reachable(user, "viewer", resource, "member_of", 3)
                }
            }
        }
    "#
    .parse()
    .unwrap();
    let evaluator = policy.build_ast_evaluator(store);

    let request = |principal: &str, resource: &str| PolicyRequest {
        resource: resource.to_string(),
        action: "write".to_string(),
        context: HashMap::from([("principal".to_string(), principal.to_string())]),
    };

    // alice: editor binding (RBAC) + mfa attribute (ABAC) -> allow
    let d = evaluator.evaluate(&request("alice", "doc-1")).unwrap();
    assert_eq!(format!("{d:?}"), "Allow", "alice via RBAC+ABAC");

    // bob: no role, no mfa — but OWNS doc-1 (ReBAC tuple) -> allow
    let d = evaluator.evaluate(&request("bob", "doc-1")).unwrap();
    assert_eq!(format!("{d:?}"), "Allow", "bob via ReBAC tuple");

    // bob on another resource: nothing applies -> deny (default)
    let d = evaluator.evaluate(&request("bob", "doc-2")).unwrap();
    assert_eq!(format!("{d:?}"), "Deny", "no data, no access");

    // carol: no role, no ownership — reaches doc-1 through team-eng
    // (member_of traversal edge materialized subject→object). This is the
    // direction contract: managed Zanzibar tuples MUST drive
    // rebac::reachable correctly.
    let read = |principal: &str, resource: &str| PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context: HashMap::from([("principal".to_string(), principal.to_string())]),
    };
    let d = evaluator.evaluate(&read("carol", "doc-1")).unwrap();
    assert_eq!(format!("{d:?}"), "Allow", "carol via group-hop ReBAC");
    // …but only for read (the rule gates on action).
    let d = evaluator.evaluate(&request("carol", "doc-1")).unwrap();
    assert_eq!(format!("{d:?}"), "Deny", "group viewers cannot write");

    // Status endpoint reports COUNT(*)-backed numbers.
    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", base, None, &key))
        .await
        .unwrap();
    let status = parse_body(response).await;
    assert_eq!(status["counts"]["entities"], 3);
    assert_eq!(status["counts"]["tuples"], 3);
    assert_eq!(status["counts"]["role_bindings"], 1);
}

/// Time-based change-log retention: pruning aged delta marks must never
/// strand a follower — a replica whose position fell below the pruned
/// floor gets snapshot_required and self-heals via a full snapshot.
#[tokio::test]
async fn test_change_log_retention_prune_forces_snapshot() {
    use reaper_management::db::repositories::DatastoreRepository;

    let env = setup_test_env().await;
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Retention Org", "slug": "retention-org"})),
        ))
        .await
        .unwrap();
    let org_id = Uuid::parse_str(parse_body(response).await["id"].as_str().unwrap()).unwrap();
    let key = create_test_api_key(&env.db, org_id).await;
    let base = "/orgs/retention-org/namespaces/prod/datastore";
    let call = |method: &'static str, path: String, body: Option<Value>| {
        let app = env.app.clone();
        let key = key.clone();
        async move {
            let response = app
                .oneshot(authed_request(method, &path, body, &key))
                .await
                .unwrap();
            let status = response.status();
            let body = parse_body(response).await;
            (status, body)
        }
    };

    let (status, _) = call(
        "POST",
        "/orgs/retention-org/namespaces".to_string(),
        Some(json!({"slug": "prod"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = call(
        "POST",
        base.to_string(),
        Some(json!({"template": "combined"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Churn without publishing — exactly the growth publish-time
    // compaction cannot bound.
    for body in [
        json!({"entity_id": "alice", "entity_type": "user", "attributes": {"mfa": true}}),
        json!({"entity_id": "bob", "entity_type": "user", "attributes": {}}),
    ] {
        let (status, _) = call("POST", format!("{base}/entities"), Some(body)).await;
        assert_eq!(status, StatusCode::OK);
    }

    // A follower at seq 0 sees ordinary deltas before the prune.
    let (status, before) = call("GET", format!("{base}/changes?since=0"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["snapshot_required"], json!(false));
    assert!(before["deltas"].as_array().unwrap().len() >= 2);

    // Prune with a future cutoff (everything is "too old").
    let cutoff = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let pruned = DatastoreRepository::new(&env.db)
        .prune_change_log(&cutoff)
        .await
        .unwrap();
    assert!(pruned >= 2, "expected marks pruned, got {pruned}");

    // The same follower now falls below the floor and is told to
    // full-sync — self-healing, no silent gap.
    let (status, after) = call("GET", format!("{base}/changes?since=0"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        after["snapshot_required"],
        json!(true),
        "pruned follower must be redirected to a snapshot: {after}"
    );
}

/// The durable delta loop, end-to-end against the real APIs: snapshot,
/// then mutations (including a cascade delete), then a /changes pull
/// applied INCREMENTALLY to a live policy-engine store — which must
/// converge to exactly what a fresh publish produces. Repeating the same
/// pull (lost-ack redelivery) must change nothing.
#[tokio::test]
async fn test_data_plane_delta_sync() {
    use policy_engine::{DataLoader, DataStore};
    let _ = tracing_subscriber::fmt()
        .with_env_filter("reaper_management=error")
        .try_init();

    let env = setup_test_env().await;

    // Org, namespace, key, datastore.
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Delta Org", "slug": "delta-org"})),
        ))
        .await
        .unwrap();
    let org_id = Uuid::parse_str(parse_body(response).await["id"].as_str().unwrap()).unwrap();
    let key = create_test_api_key(&env.db, org_id).await;
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/delta-org/namespaces",
            Some(json!({"slug": "prod"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let base = "/orgs/delta-org/namespaces/prod/datastore";
    let call = |method: &'static str, path: String, body: Option<Value>| {
        let app = env.app.clone();
        let key = key.clone();
        async move {
            let response = app
                .oneshot(authed_request(method, &path, body, &key))
                .await
                .unwrap();
            let status = response.status();
            let body = parse_body(response).await;
            (status, body)
        }
    };

    let (status, _) = call(
        "POST",
        base.to_string(),
        Some(json!({"template": "combined"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Seed + snapshot v1.
    for body in [
        json!({"entity_id": "alice", "entity_type": "user", "attributes": {"mfa": true}}),
        json!({"entity_id": "bob", "entity_type": "user", "attributes": {}}),
    ] {
        let (status, _) = call("POST", format!("{base}/entities"), Some(body)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, _) = call(
        "POST",
        format!("{base}/tuples"),
        Some(json!({"object": "doc-1", "relation": "owner", "subject": "bob"})),
    )
    .await;
    let (status, published) = call("POST", format!("{base}/publish"), None).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot_seq = {
        // versions listing exposes the change_seq the snapshot pinned
        let (_, versions) = call("GET", format!("{base}/versions"), None).await;
        versions["versions"][0]["change_seq"].as_i64().unwrap()
    };
    assert_eq!(published["version"], 1);

    // Replica: load snapshot v1.
    let (_, v1) = call("GET", format!("{base}/versions/1"), None).await;
    let replica = std::sync::Arc::new(DataStore::new());
    let loader = DataLoader::new((*replica).clone());
    loader
        .load_json(&serde_json::to_string(&v1["document"]).unwrap())
        .unwrap();

    // Mutations AFTER the snapshot: attribute change, role grant, new
    // tuple, and a cascade delete (bob dies; doc-1's owner edge must go).
    let (status, _) = call(
        "PUT",
        format!("{base}/entities/alice/attributes"),
        Some(json!({"mfa": false, "clearance": 4})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _) = call(
        "POST",
        format!("{base}/role-bindings"),
        Some(json!({"subject": "alice", "role": "admin"})),
    )
    .await;
    let (status, deleted) = call("DELETE", format!("{base}/entities/bob"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        deleted["cascaded"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "doc-1"),
        "cascade must mark doc-1 dirty: {deleted}"
    );

    // Pull the change stream from the snapshot position and apply.
    let apply_changes = |replica: std::sync::Arc<DataStore>, changes: Value| {
        let loader = DataLoader::new((*replica).clone());
        for delta in changes["deltas"].as_array().unwrap() {
            match delta["op"].as_str().unwrap() {
                "upsert" => loader.upsert_entity_doc(&delta["document"]).unwrap(),
                "delete" => loader.delete_entity(delta["entity_id"].as_str().unwrap()),
                other => panic!("unknown op {other}"),
            }
        }
    };
    let (status, changes) = call("GET", format!("{base}/changes?since={snapshot_seq}"), None).await;
    assert_eq!(status, StatusCode::OK, "changes failed: {changes}");
    assert_eq!(changes["snapshot_required"], false);
    assert!(changes["deltas"].as_array().unwrap().len() >= 3);
    apply_changes(replica.clone(), changes.clone());

    // Redelivery (lost ack): applying the SAME pull again must be a no-op.
    apply_changes(replica.clone(), changes);

    // Convergence: a fresh publish's document, loaded into a new store,
    // must be indistinguishable from the incrementally patched replica.
    let (status, _) = call("POST", format!("{base}/publish"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, v2) = call("GET", format!("{base}/versions/2"), None).await;
    let fresh = std::sync::Arc::new(DataStore::new());
    DataLoader::new((*fresh).clone())
        .load_json(&serde_json::to_string(&v2["document"]).unwrap())
        .unwrap();

    let probe = |store: &DataStore| {
        let interner = store.interner();
        let mut out = Vec::new();
        for id in ["alice", "bob", "doc-1"] {
            let eid = interner.intern(id);
            let entity = store.get(eid);
            out.push(format!("{id}:present={}", entity.is_some()));
        }
        let owner = interner.intern("owner");
        let doc1 = interner.intern("doc-1");
        out.push(format!(
            "doc-1.owners={}",
            store.relationships().related(doc1, owner).len()
        ));
        out
    };
    assert_eq!(
        probe(&replica),
        probe(&fresh),
        "delta-patched replica must converge with a fresh snapshot"
    );
    // Attribute maps compared as JSON Values (order-insensitive).
    assert_eq!(
        replica.entity_attributes_json("alice"),
        fresh.entity_attributes_json("alice"),
        "alice attributes must converge"
    );

    // And the specifics: bob gone, doc-1 ownerless, alice demoted+promoted.
    let alice = replica.entity_attributes_json("alice").unwrap();
    assert_eq!(alice["mfa"], false);
    assert_eq!(alice["clearance"], 4);
    assert_eq!(alice["roles"][0], "admin");
}

/// Save-path latency measurement (run explicitly: `--ignored`). Times the
/// FULL stack per write — HTTP router, auth, schema validation, and the
/// transactional mutation+outbox commit (WAL, synchronous=NORMAL) — the
/// number an API caller actually experiences.
#[tokio::test]
#[ignore]
async fn measure_data_plane_save_latency() {
    let env = setup_test_env().await;
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Perf Org", "slug": "perf-org"})),
        ))
        .await
        .unwrap();
    let org_id = Uuid::parse_str(parse_body(response).await["id"].as_str().unwrap()).unwrap();
    let key = create_test_api_key(&env.db, org_id).await;
    env.app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/perf-org/namespaces",
            Some(json!({"slug": "prod"})),
            &key,
        ))
        .await
        .unwrap();
    let base = "/orgs/perf-org/namespaces/prod/datastore";
    env.app
        .clone()
        .oneshot(authed_request(
            "POST",
            base,
            Some(json!({"template": "combined"})),
            &key,
        ))
        .await
        .unwrap();

    let mut timings_us: Vec<u128> = Vec::with_capacity(500);
    for i in 0..500 {
        let body = json!({
            "entity_id": format!("user-{i}"), "entity_type": "user",
            "attributes": {"mfa": i % 2 == 0, "clearance": (i % 7) as i64}
        });
        let request = authed_request("POST", &format!("{base}/entities"), Some(body), &key);
        let start = std::time::Instant::now();
        let response = env.app.clone().oneshot(request).await.unwrap();
        timings_us.push(start.elapsed().as_micros());
        assert_eq!(response.status(), StatusCode::OK);
    }
    timings_us.sort_unstable();
    let pct =
        |p: f64| timings_us[((timings_us.len() as f64 * p) as usize).min(timings_us.len() - 1)];
    println!(
        "entity save (full stack, tx mutation+outbox): p50={}µs p95={}µs p99={}µs max={}µs",
        pct(0.50),
        pct(0.95),
        pct(0.99),
        pct(1.0)
    );

    // Control: authed READ through the same stack — isolates auth+router
    // overhead from the transactional write commit.
    let mut read_us: Vec<u128> = Vec::with_capacity(200);
    for _ in 0..200 {
        let request = authed_request("GET", &format!("{base}/role-bindings"), None, &key);
        let start = std::time::Instant::now();
        let response = env.app.clone().oneshot(request).await.unwrap();
        read_us.push(start.elapsed().as_micros());
        assert_eq!(response.status(), StatusCode::OK);
    }
    read_us.sort_unstable();
    println!(
        "authed read control: p50={}µs p95={}µs",
        read_us[100], read_us[190]
    );

    let start = std::time::Instant::now();
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            &format!("{base}/publish"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    println!(
        "publish (materialize 500 entities + checksum + version row): {}ms",
        start.elapsed().as_millis()
    );
}

// ============================================================================
// Default-deny authentication gateway (Plan 01, Phase A)
// ============================================================================

/// Build a test app WITH the default-deny gateway middleware layered in —
/// mirrors `build_router`'s wiring. The shared `setup_test_env` deliberately
/// omits the gateway (many tests exercise handlers directly), so the gateway
/// gets its own harness here.
async fn setup_gateway_env() -> (axum::Router, Arc<Database>) {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();

    let db_config = reaper_management::db::ephemeral_test_config(temp_dir.path()).await;
    let db = Database::new(&db_config).await.unwrap();
    db.run_migrations().await.unwrap();
    let db = Arc::new(db);

    let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap())
        as Arc<dyn reaper_management::storage::BundleStorage>;

    let config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        ..Config::default()
    };

    let state = Arc::new(AppState::new(db.clone(), config, storage));
    let app = build_api_router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            reaper_management::auth::gateway::require_authentication,
        ))
        .with_state(state);

    // Keep the temp dir alive for the app's lifetime by leaking it: the
    // sqlite file lives under it and is needed for the whole test.
    std::mem::forget(temp_dir);
    (app, db)
}

/// The gateway must fail closed by default: unauthenticated requests to a
/// protected route get 401 at the router layer — INCLUDING routes whose
/// handlers never spelled out `RequireAuth` (the structural bug this fixes).
#[tokio::test]
async fn test_auth_gateway_default_deny() {
    let (app, db) = setup_gateway_env().await;

    // Public route: reachable without authentication.
    let response = app
        .clone()
        .oneshot(json_request("GET", "/health", None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/health must stay public"
    );

    // Seed an org + admin key directly — POST /orgs is gated now.
    let org = OrganizationRepository::new(&db)
        .create(CreateOrganization {
            name: "Gateway Org".to_string(),
            slug: "gateway-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        })
        .await
        .unwrap();
    let key = create_test_api_key(&db, org.id).await;

    // Forgotten-extractor proof: `GET /orgs/{slug}/bundles` (list_bundles)
    // takes NO `RequireAuth`. Before the gateway it served anonymous callers;
    // now it must fail closed.
    let response = app
        .clone()
        .oneshot(json_request("GET", "/orgs/gateway-org/bundles", None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "handler without RequireAuth must still fail closed under the gateway"
    );

    // A route that DOES use RequireAuth is likewise 401 without a credential.
    let response = app
        .clone()
        .oneshot(json_request("GET", "/orgs/gateway-org/agents", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // The `/api/v1` mount is gated on the same terms.
    let response = app
        .clone()
        .oneshot(json_request(
            "GET",
            "/api/v1/orgs/gateway-org/bundles",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "/api/v1 mount must be gated too"
    );

    // With a valid API key the gateway passes the request through to the
    // handler, which serves it (200, not 401).
    let response = app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/gateway-org/agents",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "authenticated request must pass the gateway and reach the handler"
    );
}

// ============================================================================
// Per-handler authZ + tenant scoping (Plan 01, Phase B)
// ============================================================================

/// Phase B defense-in-depth: a fully authenticated caller still can't (a) see
/// or touch another tenant's bundle by guessing its UUID (404, same as "does
/// not exist"), (b) operate on another org's path (403 tenant guard), or
/// (c) act beyond their granted scopes in their OWN org (403).
#[tokio::test]
async fn test_cross_tenant_and_scope_enforcement() {
    let env = setup_test_env().await;

    // Org A with a bundle and a policy, via a platform-admin key.
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Org A", "slug": "org-a"})),
        ))
        .await
        .unwrap();
    let org_a: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key_a = create_test_api_key(&env.db, org_a).await;

    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/org-a/bundles",
            Some(json!({"name": "secret-bundle"})),
            &key_a,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bundle_a = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/org-a/policies",
            Some(json!({
                "name": "secret-policy",
                "language": "reaper",
                "content": "allow admin to access /x"
            })),
            &key_a,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_a = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Org B with NON-admin keys (the platform `admin` scope would bypass the
    // tenant guard, which is exactly what these tests must not do).
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Org B", "slug": "org-b"})),
        ))
        .await
        .unwrap();
    let org_b: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key_b = create_scoped_api_key(
        &env.db,
        org_b,
        &[
            "bundle:read",
            "bundle:write",
            "bundle:promote",
            "policy:read",
        ],
    )
    .await;

    // IDOR: org B addressing org A's bundle through org B's OWN path → 404,
    // indistinguishable from "no such bundle".
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("/orgs/org-b/bundles/{bundle_a}"),
            None,
            &key_b,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "cross-tenant bundle read must 404"
    );

    // ... and mutations through the same trick fail identically.
    for (method, suffix) in [("DELETE", ""), ("POST", "/compile"), ("POST", "/promote")] {
        let body = (suffix == "/promote").then(|| json!({}));
        let response = env
            .app
            .clone()
            .oneshot(authed_request(
                method,
                &format!("/orgs/org-b/bundles/{bundle_a}{suffix}"),
                body,
                &key_b,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "cross-tenant bundle {method}{suffix} must 404"
        );
    }

    // Tenant guard: org B calling org A's path → 403 regardless of scopes.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("/orgs/org-a/bundles/{bundle_a}"),
            None,
            &key_b,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "foreign-org path must 403"
    );

    // Cross-tenant policy read via org B's own path → 404.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("/orgs/org-b/policies/{policy_a}"),
            None,
            &key_b,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "cross-tenant policy read must 404"
    );

    // Scope check: a read-only key cannot create bundles in its OWN org.
    let key_b_ro = create_scoped_api_key(&env.db, org_b, &["bundle:read"]).await;
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/org-b/bundles",
            Some(json!({"name": "nope"})),
            &key_b_ro,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "bundle:read key must not create bundles"
    );

    // Org A's bundle is untouched by all of the above.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("/orgs/org-a/bundles/{bundle_a}"),
            None,
            &key_a,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
