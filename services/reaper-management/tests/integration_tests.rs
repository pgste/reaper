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
    auth::jwks::{JwksConfig, JwksConfigRepository},
    config::{AuthConfig, Config, DatabaseConfig},
    db::repositories::AgentRepository,
    db::Database,
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
    let db_path = temp_dir.path().join("test.db");
    let storage_path = temp_dir.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();

    let db_config = DatabaseConfig {
        db_type: "sqlite".to_string(),
        url: format!("sqlite:{}", db_path.display()),
        max_connections: 5,
    };

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
    let org_id = body["id"].as_str().unwrap();
    assert_eq!(body["name"], "Test Organization");
    assert_eq!(body["slug"], "test-org");

    // Get organization by slug
    let get_req = json_request("GET", "/orgs/test-org", None);
    let response = env.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["id"], org_id);

    // List organizations
    let list_req = json_request("GET", "/orgs", None);
    let response = env.app.clone().oneshot(list_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert!(!body["organizations"].as_array().unwrap().is_empty());

    // Update organization
    let update_req = json_request(
        "PUT",
        &format!("/orgs/{}", org_id),
        Some(json!({
            "display_name": "Updated Name"
        })),
    );
    let response = env.app.clone().oneshot(update_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Delete organization
    let delete_req = json_request("DELETE", &format!("/orgs/{}", org_id), None);
    let response = env.app.clone().oneshot(delete_req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify deletion
    let get_req = json_request("GET", &format!("/orgs/{}", org_id), None);
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

    // Create policy
    let create_policy = json_request(
        "POST",
        "/orgs/policy-org/policies",
        Some(json!({
            "name": "test-policy",
            "description": "A test policy",
            "language": "reaper",
            "content": "allow admin to access /admin"
        })),
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let policy_id = body["id"].as_str().unwrap();
    assert_eq!(body["name"], "test-policy");

    // Get policy
    let get_policy = json_request(
        "GET",
        &format!("/orgs/policy-org/policies/{}", policy_id),
        None,
    );
    let response = env.app.clone().oneshot(get_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update policy (creates new version)
    let update_policy = json_request(
        "PUT",
        &format!("/orgs/policy-org/policies/{}", policy_id),
        Some(json!({
            "content": "allow admin to access /admin/*"
        })),
    );
    let response = env.app.clone().oneshot(update_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List policies
    let list_policies = json_request("GET", "/orgs/policy-org/policies", None);
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

    // Create policy
    let create_policy = json_request(
        "POST",
        "/orgs/bundle-org/policies",
        Some(json!({
            "name": "bundle-policy",
            "language": "reaper",
            "content": "allow user to read /api"
        })),
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_body = parse_body(response).await;
    let policy_id = policy_body["id"].as_str().unwrap();

    // Create bundle with policy
    let create_bundle = json_request(
        "POST",
        "/orgs/bundle-org/bundles",
        Some(json!({
            "name": "test-bundle",
            "description": "Test bundle",
            "policy_ids": [policy_id]
        })),
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = parse_body(response).await;
    let bundle_id = body["id"].as_str().unwrap();
    assert_eq!(body["status"], "draft");
    assert_eq!(body["policy_count"], 1);

    // Compile bundle
    let compile = json_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/compile", bundle_id),
        None,
    );
    let response = env.app.clone().oneshot(compile).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "compiled");
    assert!(body["storage_key"].as_str().is_some());
    assert!(body["checksum"].as_str().is_some());

    // Stage bundle
    let stage = json_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/stage", bundle_id),
        None,
    );
    let response = env.app.clone().oneshot(stage).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "staged");

    // Promote bundle
    let promote = json_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/promote", bundle_id),
        Some(json!({
            "notes": "Initial release"
        })),
    );
    let response = env.app.clone().oneshot(promote).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["status"], "promoted");

    // Get promoted bundle
    let get_promoted = json_request("GET", "/orgs/bundle-org/bundles/promoted", None);
    let response = env.app.clone().oneshot(get_promoted).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = parse_body(response).await;
    assert_eq!(body["id"], bundle_id);

    // Download bundle
    let download = json_request(
        "GET",
        &format!("/orgs/bundle-org/bundles/{}/download", bundle_id),
        None,
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

    // Get non-existent organization
    let response = env
        .app
        .clone()
        .oneshot(json_request("GET", "/orgs/nonexistent", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

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

    // Create org first for further tests
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({
            "name": "Error Test Org",
            "slug": "error-org"
        })),
    );
    env.app.clone().oneshot(create_org).await.unwrap();

    // Get non-existent policy
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "GET",
            "/orgs/error-org/policies/00000000-0000-0000-0000-000000000000",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Get non-existent bundle
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "GET",
            "/orgs/error-org/bundles/00000000-0000-0000-0000-000000000000",
            None,
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
    env.app.clone().oneshot(create_org).await.unwrap();

    // Create bundle (empty, no policies)
    let create_bundle = json_request(
        "POST",
        "/orgs/transition-org/bundles",
        Some(json!({
            "name": "empty-bundle"
        })),
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    let body = parse_body(response).await;
    let bundle_id = body["id"].as_str().unwrap();

    // Try to compile empty bundle - should fail
    let compile = json_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/compile", bundle_id),
        None,
    );
    let response = env.app.clone().oneshot(compile).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Try to stage draft bundle - should fail
    let stage = json_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/stage", bundle_id),
        None,
    );
    let response = env.app.clone().oneshot(stage).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Try to promote draft bundle - should fail
    let promote = json_request(
        "POST",
        &format!("/orgs/transition-org/bundles/{}/promote", bundle_id),
        Some(json!({})),
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
    assert_eq!(retrieved.jwks_url, "https://tenant.auth0.com/.well-known/jwks.json");
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
    let list_req = session_request(
        "GET",
        &format!("/orgs/{}/members", org_slug),
        None,
        token,
    );
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
    let list_agents = session_request(
        "GET",
        &format!("/orgs/{}/agents", org_slug),
        None,
        token,
    );
    let response = env.app.clone().oneshot(list_agents).await.unwrap();
    // Owner has admin scope, so this should work
    assert_eq!(response.status(), StatusCode::OK);
}
