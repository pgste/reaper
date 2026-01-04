//! Integration tests for Reaper Management Server
//!
//! Tests the full API workflow from organization creation through bundle promotion.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use reaper_management::{
    api::build_api_router,
    auth::api_key::{ApiKeyRepository, CreateApiKey},
    config::{AuthConfig, Config, DatabaseConfig},
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
    let mut config = Config::default();
    config.auth = AuthConfig {
        jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
        ..AuthConfig::default()
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
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
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
    assert!(body["organizations"].as_array().unwrap().len() >= 1);

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
