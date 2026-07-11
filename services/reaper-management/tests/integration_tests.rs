//! Integration tests for Reaper Management Server
//!
//! Tests the full API workflow from organization creation through bundle promotion.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reaper_management::{
    api::build_served_router,
    auth::api_key::{ApiKeyRepository, CreateApiKey},
    auth::jwks::JwksConfigRepository,
    config::{AuthConfig, Config, PromotionApproval},
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

/// Test helper to set up a test environment (single-control promotion, the
/// default).
async fn setup_test_env() -> TestEnv {
    setup_env_with(|_| {}).await
}

/// Set up a test environment, letting the caller tweak the config (e.g. to turn
/// on dual-control promotion).
async fn setup_env_with(customize: impl FnOnce(&mut Config)) -> TestEnv {
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
    let mut config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        ..Config::default()
    };
    customize(&mut config);

    let state = AppState::new(db.clone(), config, storage);
    let app = build_served_router().with_state(Arc::new(state));

    TestEnv { temp_dir, app, db }
}

/// Map a bare resource path to the single `/api/v1` surface (Plan 07 Phase B).
/// Probes (`/health*`, `/live`, `/ready`, `/metrics*`, `/openapi.json`) stay
/// unversioned; anything already `/api/v1`-prefixed is left as-is. Lets the
/// existing bare-path test call sites exercise the versioned router unchanged.
fn v1_uri(uri: &str) -> String {
    let path = uri.split('?').next().unwrap_or(uri);
    let is_probe = path == "/health"
        || path.starts_with("/health/")
        || path == "/live"
        || path == "/ready"
        || path == "/metrics"
        || path.starts_with("/metrics/")
        || path == "/openapi.json";
    if is_probe || uri.starts_with("/api/v1") {
        uri.to_string()
    } else {
        format!("/api/v1{uri}")
    }
}

/// Helper to make JSON requests without auth
fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().uri(v1_uri(uri)).method(method);

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
        .uri(v1_uri(uri))
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
    create_named_api_key(db, org_id, "test-key").await
}

/// Create a named API key. Keys are unique per (org, name), so a second key in
/// the same org (e.g. a distinct approver for two-person controls) needs a
/// distinct name.
async fn create_named_api_key(db: &Database, org_id: Uuid, name: &str) -> String {
    let api_key_repo = ApiKeyRepository::new(db);
    let created = api_key_repo
        .create(
            org_id,
            CreateApiKey {
                name: name.to_string(),
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

    // Single-control (default): promoting goes live immediately for a caller
    // holding bundle:promote. (Dual-control / two-person approval is covered by
    // the dedicated governance tests below.)
    let promote = authed_request(
        "POST",
        &format!("/orgs/bundle-org/bundles/{}/promote", bundle_id),
        Some(json!({ "notes": "Initial release" })),
        &key,
    );
    let response = env.app.clone().oneshot(promote).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = parse_body(response).await;
    assert_eq!(body["status"], "promoted");
    assert_eq!(body["id"], bundle_id);

    // A change record is still written even under single-control.
    let list = authed_request("GET", "/orgs/bundle-org/change-requests", None, &key);
    let response = env.app.clone().oneshot(list).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let crs = parse_body(response).await;
    assert_eq!(crs.as_array().unwrap().len(), 1);
    assert_eq!(crs[0]["status"], "executed");
    assert_eq!(crs[0]["kind"], "promote");

    // Get promoted bundle returns it.
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

    // Try to promote draft bundle - should fail (it isn't staged, so a promote
    // change request can't even be opened).
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
        .uri(v1_uri(uri))
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
    let app = build_served_router()
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

/// Owner-on-create + multi-org sessions (Plan 01 follow-up from PR #10):
/// a signed-up user creates a SECOND org through `POST /orgs` and can
/// immediately manage it — the creator is recorded as Owner, and session
/// resolution picks the membership matching the org the request path
/// addresses instead of blindly using the first membership.
#[tokio::test]
async fn test_org_create_grants_owner_and_multi_org_sessions_work() {
    let env = setup_test_env().await;

    // Signup → first org + session token.
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/auth/signup",
            Some(json!({
                "email": "multi-org@example.com",
                "password": "SecurePass123!",
                "org_name": "First Org"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = parse_body(response).await;
    let token = body["session_token"].as_str().unwrap().to_string();
    let first_slug = body["org"]["slug"].as_str().unwrap().to_string();

    // Create a SECOND org with the same session.
    let response = env
        .app
        .clone()
        .oneshot(session_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Second Org", "slug": "second-org"})),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Immediately manageable: read, mutate (Owner holds org:write/org:admin),
    // and create org-scoped resources — all with the SAME session.
    let response = env
        .app
        .clone()
        .oneshot(session_request("GET", "/orgs/second-org", None, &token))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "creator must be able to read the org they just created"
    );

    let response = env
        .app
        .clone()
        .oneshot(session_request(
            "PUT",
            "/orgs/second-org",
            Some(json!({"display_name": "Renamed"})),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "creator must be Owner of the org they created"
    );

    let response = env
        .app
        .clone()
        .oneshot(session_request(
            "POST",
            "/orgs/second-org/policies",
            Some(json!({
                "name": "p1",
                "language": "reaper",
                "content": "allow admin to access /x"
            })),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // The FIRST org keeps working with the same session (path-aware
    // membership selection, not last-created).
    let response = env
        .app
        .clone()
        .oneshot(session_request(
            "GET",
            &format!("/orgs/{first_slug}"),
            None,
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // A stranger's org stays out of reach for this session: no membership →
    // the tenant guard rejects, path-awareness notwithstanding.
    OrganizationRepository::new(&env.db)
        .create(CreateOrganization {
            name: "Stranger Org".to_string(),
            slug: "stranger-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        })
        .await
        .unwrap();
    let response = env
        .app
        .clone()
        .oneshot(session_request("GET", "/orgs/stranger-org", None, &token))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "no membership in the addressed org must be rejected"
    );
}

/// Revocation endpoints (Plan 02 Phase B step 4): an org admin revokes a
/// bundle hash and a key id; the served list is signed with the control
/// plane's bundle key, carries both entries, and its serial advances.
#[tokio::test]
async fn test_revocation_list_served_signed_and_updates() {
    use reaper_core::bundle_signing::{SigAlgorithm, SigningKey, VerifyingKey};
    use reaper_core::revocation::SignedRevocationList;

    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();
    let db_config = reaper_management::db::ephemeral_test_config(temp_dir.path()).await;
    let db = Arc::new(Database::new(&db_config).await.unwrap());
    db.run_migrations().await.unwrap();
    let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap())
        as Arc<dyn reaper_management::storage::BundleStorage>;

    // Signing key the control plane will sign the revocation list with.
    let signing_key = SigningKey::generate(SigAlgorithm::Ed25519Sha256);
    let pub_hex = signing_key.public_key_hex();

    let config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        bundles: reaper_management::config::BundlesConfig {
            signing_key: Some(signing_key.private_key_hex()),
            signing_key_id: "k1".to_string(),
            signing_algorithm: reaper_core::bundle_signing::ALG_ED25519.to_string(),
            ..Default::default()
        },
        ..Config::default()
    };
    let state = AppState::new(db.clone(), config, storage);
    let app = build_served_router().with_state(Arc::new(state));

    // Org + admin key.
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Rev Org", "slug": "rev-org"})),
        ))
        .await
        .unwrap();
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&db, org_id).await;

    // Empty list first: signed, serial 0.
    let response = app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/rev-org/revocations",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let signed: SignedRevocationList = serde_json::from_value(parse_body(response).await).unwrap();
    let vk = VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, &pub_hex).unwrap();
    let list = signed
        .verify(&vk, Some("k1"))
        .expect("list signature verifies");
    assert_eq!(list.serial, 0);
    assert!(list.revoked_bundle_hashes.is_empty());

    // Revoke a hash and a key id.
    for body in [
        json!({"kind": "hash", "value": "deadbeef", "reason": "bad bundle"}),
        json!({"kind": "key_id", "value": "leaked-key"}),
    ] {
        let response = app
            .clone()
            .oneshot(authed_request(
                "POST",
                "/orgs/rev-org/revocations",
                Some(body),
                &key,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // The served list now reflects both, still signed, with a higher serial.
    let response = app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/rev-org/revocations",
            None,
            &key,
        ))
        .await
        .unwrap();
    let signed: SignedRevocationList = serde_json::from_value(parse_body(response).await).unwrap();
    let list = signed.verify(&vk, Some("k1")).expect("still verifies");
    assert!(list.serial >= 2, "serial advanced: {}", list.serial);
    assert!(list.is_revoked("deadbeef", "other"));
    assert!(list.is_revoked("aaaa", "leaked-key"));

    // A different org's admin key cannot read this org's list (tenant guard).
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Other", "slug": "other-org"})),
        ))
        .await
        .unwrap();
    let other_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let other_key = create_scoped_api_key(&db, other_id, &["agent:read"]).await;
    let response = app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/rev-org/revocations",
            None,
            &other_key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// =============================================================================
// Governed promotion (two-person control) — Plan 02, Phase B step 5
// =============================================================================

/// Create an org (with slug) and an admin API key; return (org_id, key).
async fn org_with_key(env: &TestEnv, name: &str, slug: &str) -> (Uuid, String) {
    let create_org = json_request("POST", "/orgs", Some(json!({ "name": name, "slug": slug })));
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&env.db, org_id).await;
    (org_id, key)
}

/// Create a one-policy bundle, compile and stage it; return the bundle id.
async fn staged_bundle(env: &TestEnv, slug: &str, key: &str, name: &str) -> String {
    let policy = authed_request(
        "POST",
        &format!("/orgs/{slug}/policies"),
        Some(json!({
            "name": format!("{name}-pol"),
            "language": "reaper",
            "content": "allow user to read /api"
        })),
        key,
    );
    let resp = env.app.clone().oneshot(policy).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let policy_id = parse_body(resp).await["id"].as_str().unwrap().to_string();

    let create = authed_request(
        "POST",
        &format!("/orgs/{slug}/bundles"),
        Some(json!({ "name": name, "policy_ids": [policy_id] })),
        key,
    );
    let resp = env.app.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bundle_id = parse_body(resp).await["id"].as_str().unwrap().to_string();

    for step in ["compile", "stage"] {
        let req = authed_request(
            "POST",
            &format!("/orgs/{slug}/bundles/{bundle_id}/{step}"),
            None,
            key,
        );
        let resp = env.app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{step} step failed");
    }
    bundle_id
}

/// POST a promote/rollback (`path` = e.g. `bundles/{id}/promote`) and return
/// the opened change request id.
async fn open_change_request(env: &TestEnv, slug: &str, key: &str, path: &str) -> String {
    let req = authed_request(
        "POST",
        &format!("/orgs/{slug}/{path}"),
        Some(json!({})),
        key,
    );
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    parse_body(resp).await["id"].as_str().unwrap().to_string()
}

/// Approve a change request as `approver` and assert it executed, promoting
/// `expect_bundle`.
async fn approve_ok(env: &TestEnv, slug: &str, approver: &str, cr_id: &str, expect_bundle: &str) {
    let req = authed_request(
        "POST",
        &format!("/orgs/{slug}/change-requests/{cr_id}/approve"),
        None,
        approver,
    );
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["status"], "promoted");
    assert_eq!(body["id"], expect_bundle);
}

/// A test env with two-person (dual-control) promotion turned on.
async fn dual_control_env() -> TestEnv {
    setup_env_with(|c| c.bundles.promotion_approval = PromotionApproval::DualControl).await
}

/// Under dual-control, a promote opens a pending change request; the requester
/// cannot self-approve, but a distinct principal can, which executes it.
#[tokio::test]
async fn test_dual_control_requires_distinct_approver() {
    let env = dual_control_env().await;
    let (org_id, key) = org_with_key(&env, "Dual Org", "dual-org").await;
    let bundle = staged_bundle(&env, "dual-org", &key, "dual-bundle").await;

    // Promote only opens a pending request — nothing is live yet.
    let promote = authed_request(
        "POST",
        &format!("/orgs/dual-org/bundles/{bundle}/promote"),
        Some(json!({ "notes": "please review" })),
        &key,
    );
    let resp = env.app.clone().oneshot(promote).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = parse_body(resp).await;
    assert_eq!(body["status"], "pending");
    let cr = body["id"].as_str().unwrap().to_string();

    let getp = authed_request("GET", "/orgs/dual-org/bundles/promoted", None, &key);
    let resp = env.app.clone().oneshot(getp).await.unwrap();
    assert!(parse_body(resp).await.is_null());

    // The requester cannot approve their own request.
    let self_approve = authed_request(
        "POST",
        &format!("/orgs/dual-org/change-requests/{cr}/approve"),
        None,
        &key,
    );
    let resp = env.app.clone().oneshot(self_approve).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // A distinct principal approves — now it's live.
    let approver = create_named_api_key(&env.db, org_id, "approver-key").await;
    approve_ok(&env, "dual-org", &approver, &cr, &bundle).await;
}

/// With `allow_self_approval`, a single principal (e.g. an automated pipeline's
/// service account) may both open and approve — dual-control still records the
/// change, but the four-eyes constraint is intentionally relaxed.
#[tokio::test]
async fn test_dual_control_allow_self_approval() {
    let env = setup_env_with(|c| {
        c.bundles.promotion_approval = PromotionApproval::DualControl;
        c.bundles.allow_self_approval = true;
    })
    .await;
    let (_org, key) = org_with_key(&env, "SelfOK Org", "selfok-org").await;
    let bundle = staged_bundle(&env, "selfok-org", &key, "selfok-bundle").await;

    let cr = open_change_request(
        &env,
        "selfok-org",
        &key,
        &format!("bundles/{bundle}/promote"),
    )
    .await;

    // Same principal approves its own request — allowed here.
    approve_ok(&env, "selfok-org", &key, &cr, &bundle).await;
}

/// Opening a promotion change request requires authentication.
#[tokio::test]
async fn test_promote_change_request_requires_auth() {
    let env = setup_test_env().await;
    let _ = org_with_key(&env, "Auth CR Org", "authcr-org").await;

    // No API key → 401, before any change request could be recorded.
    let req = json_request(
        "POST",
        &format!("/orgs/authcr-org/bundles/{}/promote", Uuid::new_v4()),
        Some(json!({})),
    );
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// A change request opened in one org is invisible and un-approvable from
/// another org (tenant isolation: 404, not another org's decision).
#[tokio::test]
async fn test_change_request_cross_org_isolation() {
    let env = dual_control_env().await;
    let (_a, a_key) = org_with_key(&env, "CR Org A", "cr-org-a").await;
    let (_b, b_key) = org_with_key(&env, "CR Org B", "cr-org-b").await;

    let bundle = staged_bundle(&env, "cr-org-a", &a_key, "iso-bundle").await;
    let cr = open_change_request(
        &env,
        "cr-org-a",
        &a_key,
        &format!("bundles/{bundle}/promote"),
    )
    .await;

    // Org B cannot read org A's change request through its own org scope.
    let get = authed_request(
        "GET",
        &format!("/orgs/cr-org-b/change-requests/{cr}"),
        None,
        &b_key,
    );
    let resp = env.app.clone().oneshot(get).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ...nor approve it through its own org.
    let approve = authed_request(
        "POST",
        &format!("/orgs/cr-org-b/change-requests/{cr}/approve"),
        None,
        &b_key,
    );
    let resp = env.app.clone().oneshot(approve).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // The request is still pending in org A (org B's probes changed nothing).
    let get = authed_request(
        "GET",
        &format!("/orgs/cr-org-a/change-requests/{cr}"),
        None,
        &a_key,
    );
    let resp = env.app.clone().oneshot(get).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(parse_body(resp).await["status"], "pending");
}

/// End-to-end rollback: promote A, promote B (A deprecated), then roll back to
/// A through a second change request. Rollback is recorded with its own kind
/// and executed only by a distinct approver.
#[tokio::test]
async fn test_rollback_change_request_flow() {
    let env = dual_control_env().await;
    let (org_id, key) = org_with_key(&env, "Rollback Org", "rollback-org").await;
    let approver = create_named_api_key(&env.db, org_id, "approver-key").await;

    let a = staged_bundle(&env, "rollback-org", &key, "bundle-a").await;
    let b = staged_bundle(&env, "rollback-org", &key, "bundle-b").await;

    // Promote A, then B — B is now live and A is deprecated.
    let cr_a =
        open_change_request(&env, "rollback-org", &key, &format!("bundles/{a}/promote")).await;
    approve_ok(&env, "rollback-org", &approver, &cr_a, &a).await;
    let cr_b =
        open_change_request(&env, "rollback-org", &key, &format!("bundles/{b}/promote")).await;
    approve_ok(&env, "rollback-org", &approver, &cr_b, &b).await;

    // Open a rollback change request targeting A.
    let roll = authed_request(
        "POST",
        &format!("/orgs/rollback-org/bundles/{a}/rollback"),
        Some(json!({ "notes": "B misbehaved" })),
        &key,
    );
    let resp = env.app.clone().oneshot(roll).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = parse_body(resp).await;
    assert_eq!(body["kind"], "rollback");
    assert_eq!(body["status"], "pending");
    let cr_roll = body["id"].as_str().unwrap().to_string();

    // A distinct principal approves the rollback — A is live again.
    approve_ok(&env, "rollback-org", &approver, &cr_roll, &a).await;

    let getp = authed_request("GET", "/orgs/rollback-org/bundles/promoted", None, &key);
    let resp = env.app.clone().oneshot(getp).await.unwrap();
    assert_eq!(parse_body(resp).await["id"], a);

    // The executed rollback record names a distinct approver.
    let getcr = authed_request(
        "GET",
        &format!("/orgs/rollback-org/change-requests/{cr_roll}"),
        None,
        &key,
    );
    let resp = env.app.clone().oneshot(getcr).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["status"], "executed");
    assert!(body["approver_id"].is_string());
    assert_ne!(body["approver_id"], body["requester_id"]);
}

/// Compliance / separation-of-duties: `bundle:approve` is a distinct authority
/// from `bundle:promote`, so an org can grant "can request a promotion" and
/// "can approve one" to different principals. A promote-only principal cannot
/// approve, and an approve-only principal cannot originate a promotion.
#[tokio::test]
async fn test_approve_scope_separated_from_promote() {
    let env = dual_control_env().await;
    let (org_id, _owner) = org_with_key(&env, "SoD Org", "sod-org").await;

    // Requester: authors/stages and OPENS a promotion — but cannot approve.
    let requester = create_scoped_api_key(
        &env.db,
        org_id,
        &[
            "policy:read",
            "policy:write",
            "bundle:read",
            "bundle:write",
            "bundle:promote",
        ],
    )
    .await;
    // Approver: can ONLY approve — no authority to originate a promotion.
    let approver = create_scoped_api_key(&env.db, org_id, &["bundle:read", "bundle:approve"]).await;

    let bundle = staged_bundle(&env, "sod-org", &requester, "sod-bundle").await;

    // The approve-only principal cannot OPEN a promote request (no promote).
    let open_as_approver = authed_request(
        "POST",
        &format!("/orgs/sod-org/bundles/{bundle}/promote"),
        Some(json!({})),
        &approver,
    );
    let resp = env.app.clone().oneshot(open_as_approver).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The requester opens the promote request.
    let cr = open_change_request(
        &env,
        "sod-org",
        &requester,
        &format!("bundles/{bundle}/promote"),
    )
    .await;

    // The requester cannot approve it — they lack bundle:approve. This is a
    // scope failure, independent of the distinct-principal rule.
    let approve_as_requester = authed_request(
        "POST",
        &format!("/orgs/sod-org/change-requests/{cr}/approve"),
        None,
        &requester,
    );
    let resp = env.app.clone().oneshot(approve_as_requester).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The dedicated approver (bundle:approve only) approves — and it executes.
    approve_ok(&env, "sod-org", &approver, &cr, &bundle).await;
}

// =============================================================================
// Enterprise SSO — broker session (Plan 03, Phase 1)
// =============================================================================

/// A session minted by the SSO broker is a normal `rst_` session that
/// `RequireAuth` accepts, the IdP group maps to the org role, and a repeat
/// login for the same IdP subject reuses the same user (no duplicate).
#[tokio::test]
async fn test_sso_broker_session_is_accepted_and_stable() {
    use reaper_management::auth::sso::broker::{establish_session, ExternalIdentity, LoginContext};
    use reaper_management::auth::sso::{SsoConfig, SsoProtocol};

    let env = setup_test_env().await;
    let (org_id, _owner) = org_with_key(&env, "SSO Org", "sso-org").await;

    let now = chrono::Utc::now();
    let config = SsoConfig {
        id: Uuid::new_v4(),
        org_id,
        protocol: SsoProtocol::Oidc,
        enabled: true,
        issuer: "https://idp.example.com".into(),
        client_id: "reaper".into(),
        client_secret_encrypted: None,
        discovery_url: None,
        jwks_url: None,
        attr_map_json: Some(r#"{"group_map":{"reaper-admins":"owner"}}"#.into()),
        allowed_domains_json: None,
        default_role: "viewer".into(),
        created_at: now,
        updated_at: now,
    };
    let identity = ExternalIdentity {
        issuer: "https://idp.example.com".into(),
        subject: "idp-subject-123".into(),
        email: "alice@example.com".into(),
        email_verified: true,
        groups: vec!["reaper-admins".into()],
        display_name: Some("Alice".into()),
    };

    let est = establish_session(
        &env.db,
        org_id,
        &identity,
        &config,
        &LoginContext::default(),
    )
    .await
    .expect("broker should mint a session");
    assert!(est.token.starts_with("rst_"));

    // The minted token authenticates against a RequireAuth + scope-gated route.
    // The "reaper-admins" group mapped to Owner, which carries bundle:read.
    let req = authed_request_bearer("GET", "/orgs/sso-org/bundles", &est.token);
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // A second login for the same (issuer, subject) reuses the same user row.
    let est2 = establish_session(
        &env.db,
        org_id,
        &identity,
        &config,
        &LoginContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(est.user_id, est2.user_id);
}

/// Build a Bearer-authenticated request (session token, not an API key).
fn authed_request_bearer(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(v1_uri(uri))
        .method(method)
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

// =============================================================================
// Enterprise SCIM — provisioning + deprovision-revokes-sessions (Plan 03, Ph 2)
// =============================================================================

/// A Bearer-authenticated request with an optional JSON body (SCIM token or
/// session token).
fn bearer_request(method: &str, uri: &str, body: Option<Value>, token: &str) -> Request<Body> {
    let mut b = Request::builder()
        .uri(v1_uri(uri))
        .method(method)
        .header("Authorization", format!("Bearer {token}"));
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());
    b.body(body).unwrap()
}

/// Mint a SCIM token for an org via the admin endpoint; return its plaintext.
async fn mint_scim_token(env: &TestEnv, slug: &str, admin_key: &str) -> String {
    let req = authed_request(
        "POST",
        &format!("/orgs/{slug}/scim/tokens"),
        Some(json!({ "name": "test-idp" })),
        admin_key,
    );
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    parse_body(resp).await["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// SCIM provisioning makes a user an org member, and deprovisioning
/// (`active=false`) revokes their live sessions within one request.
#[tokio::test]
async fn test_scim_provision_and_deprovision_revokes_sessions() {
    use reaper_management::auth::{Session, SessionRepository};

    let env = setup_test_env().await;
    let (_org, owner_key) = org_with_key(&env, "SCIM Org", "scim-org").await;
    let scim_token = mint_scim_token(&env, "scim-org", &owner_key).await;

    // Provision a user via SCIM.
    let create = bearer_request(
        "POST",
        "/scim/v2/Users",
        Some(json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "alice@example.com",
            "active": true
        })),
        &scim_token,
    );
    let resp = env.app.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = parse_body(resp).await;
    assert_eq!(body["active"], true);
    let user_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // The user shows up in a filtered SCIM list.
    let list = bearer_request(
        "GET",
        "/scim/v2/Users?filter=userName%20eq%20%22alice@example.com%22",
        None,
        &scim_token,
    );
    let resp = env.app.clone().oneshot(list).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(parse_body(resp).await["totalResults"], 1);

    // Give the user a live session; as a Viewer they can read bundles.
    let (session, session_token) = Session::new(user_id, None, None, 24);
    SessionRepository::new(&env.db)
        .create(&session)
        .await
        .unwrap();
    let req = bearer_request("GET", "/orgs/scim-org/bundles", None, &session_token);
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Deprovision via SCIM PATCH active=false.
    let patch = bearer_request(
        "PATCH",
        &format!("/scim/v2/Users/{user_id}"),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{ "op": "replace", "path": "active", "value": false }]
        })),
        &scim_token,
    );
    let resp = env.app.clone().oneshot(patch).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The previously-valid session is now revoked — terminated user is denied.
    let req = bearer_request("GET", "/orgs/scim-org/bundles", None, &session_token);
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// An unknown SCIM token is rejected, and a token only ever acts on its own org.
#[tokio::test]
async fn test_scim_token_tenant_isolation() {
    let env = setup_test_env().await;
    let (_a, a_key) = org_with_key(&env, "SCIM A", "scim-iso-a").await;
    let (_b, _b_key) = org_with_key(&env, "SCIM B", "scim-iso-b").await;
    let token_a = mint_scim_token(&env, "scim-iso-a", &a_key).await;

    // Unknown token → 401.
    let req = bearer_request("GET", "/scim/v2/Users", None, "scim_deadbeefdeadbeef");
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Provision a user with A's token; it lands in org A only.
    let create = bearer_request(
        "POST",
        "/scim/v2/Users",
        Some(json!({ "userName": "bob@a.example.com", "active": true })),
        &token_a,
    );
    let resp = env.app.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // A's token lists exactly its own org's members (bob), and org B sees none.
    let list = bearer_request("GET", "/scim/v2/Users", None, &token_a);
    let resp = env.app.clone().oneshot(list).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["totalResults"], 1);
    assert_eq!(body["Resources"][0]["userName"], "bob@a.example.com");
}

// ============================================================================
// Audit governance: retention windows + legal holds (Plan 04, step 6)
// ============================================================================

/// Create an org directly through the repository (the governance tests don't
/// exercise org CRUD).
async fn seed_org(env: &TestEnv, name: &str, slug: &str) -> Uuid {
    OrganizationRepository::new(&env.db)
        .create(CreateOrganization {
            name: name.to_string(),
            slug: slug.to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        })
        .await
        .unwrap()
        .id
}

/// Count audit records for (org, action) — proves governance changes are audited.
async fn audit_count(env: &TestEnv, org_id: Uuid, action: &str) -> usize {
    reaper_management::audit::AuditRepository::new(&env.db)
        .query(&reaper_management::audit::AuditQuery {
            org_id: Some(org_id),
            action: Some(action.to_string()),
            ..Default::default()
        })
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn test_audit_retention_lifecycle_and_validation() {
    let env = setup_test_env().await;
    let org_id = seed_org(&env, "Retention Org", "retention-org").await;
    let key = create_scoped_api_key(&env.db, org_id, &["org:admin"]).await;

    // Unset → the default window, marked as such.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/retention-org/audit/retention",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["source"], "default");
    assert_eq!(
        body["days"], 90,
        "default window matches the old schema TTL"
    );

    // Set an explicit window.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "PUT",
            "/orgs/retention-org/audit/retention",
            Some(json!({"days": 30})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["days"], 30);
    assert_eq!(body["source"], "explicit");

    // Read-back is the explicit setting.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/retention-org/audit/retention",
            None,
            &key,
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["days"], 30);
    assert_eq!(body["source"], "explicit");

    // Out-of-range windows are rejected: a typo must not configure
    // instant-delete or near-infinite retention.
    for bad in [0, -5, 4000] {
        let resp = env
            .app
            .clone()
            .oneshot(authed_request(
                "PUT",
                "/orgs/retention-org/audit/retention",
                Some(json!({"days": bad})),
                &key,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "days={bad}");
    }

    // The change was audited.
    assert_eq!(audit_count(&env, org_id, "audit.retention_update").await, 1);
}

#[tokio::test]
async fn test_audit_legal_hold_lifecycle() {
    let env = setup_test_env().await;
    let org_id = seed_org(&env, "Hold Org", "hold-org").await;
    let key = create_scoped_api_key(&env.db, org_id, &["org:admin"]).await;

    // A hold requires a reason — it is itself a compliance record.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/hold-org/audit/legal-holds",
            Some(json!({"reason": "   "})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Place a scoped hold.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/hold-org/audit/legal-holds",
            Some(json!({
                "reason": "Litigation #2026-114",
                "filter": {"principal": "alice", "decision": "deny"}
            })),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let hold = parse_body(resp).await;
    let hold_id = hold["id"].as_str().unwrap().to_string();
    assert_eq!(hold["reason"], "Litigation #2026-114");
    assert_eq!(hold["filter"]["principal"], "alice");
    assert!(hold["released_at"].is_null(), "created active");

    // Listed, active.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/hold-org/audit/legal-holds",
            None,
            &key,
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["count"], 1);
    assert_eq!(body["active"], 1);

    // Fetchable by id.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("/orgs/hold-org/audit/legal-holds/{hold_id}"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Release: 204; the record survives as released (compliance history),
    // and a second release is a visible 404, not a silent no-op.
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "DELETE",
            &format!("/orgs/hold-org/audit/legal-holds/{hold_id}"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "DELETE",
            &format!("/orgs/hold-org/audit/legal-holds/{hold_id}"),
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "double release surfaces"
    );

    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/hold-org/audit/legal-holds",
            None,
            &key,
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["count"], 1, "released hold stays in the record");
    assert_eq!(body["active"], 0);
    assert!(!body["holds"][0]["released_at"].is_null());

    // Both lifecycle events were audited.
    assert_eq!(
        audit_count(&env, org_id, "audit.legal_hold_create").await,
        1
    );
    assert_eq!(
        audit_count(&env, org_id, "audit.legal_hold_release").await,
        1
    );
}

#[tokio::test]
async fn test_audit_governance_tenant_isolation_and_scopes() {
    let env = setup_test_env().await;
    let org_a = seed_org(&env, "Gov Org A", "gov-org-a").await;
    let org_b = seed_org(&env, "Gov Org B", "gov-org-b").await;
    let key_a = create_scoped_api_key(&env.db, org_a, &["org:admin"]).await;
    let reader_a = create_scoped_api_key(&env.db, org_a, &["agent:read"]).await;

    // Org A's admin cannot read or mutate org B's governance.
    for (method, uri, body) in [
        ("GET", "/orgs/gov-org-b/audit/retention", None),
        (
            "PUT",
            "/orgs/gov-org-b/audit/retention",
            Some(json!({"days": 7})),
        ),
        ("GET", "/orgs/gov-org-b/audit/legal-holds", None),
        (
            "POST",
            "/orgs/gov-org-b/audit/legal-holds",
            Some(json!({"reason": "cross-tenant probe"})),
        ),
        ("POST", "/orgs/gov-org-b/audit/purge", None),
    ] {
        let resp = env
            .app
            .clone()
            .oneshot(authed_request(method, uri, body, &key_a))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} must be tenant-isolated"
        );
    }

    // A read-only (agent:read) credential is rejected even on its OWN org:
    // governance is admin surface (holds reveal litigation posture).
    for (method, uri, body) in [
        ("GET", "/orgs/gov-org-a/audit/retention", None),
        (
            "PUT",
            "/orgs/gov-org-a/audit/retention",
            Some(json!({"days": 7})),
        ),
        ("GET", "/orgs/gov-org-a/audit/legal-holds", None),
    ] {
        let resp = env
            .app
            .clone()
            .oneshot(authed_request(method, uri, body, &reader_a))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} must require org:admin"
        );
    }

    // And org B remains untouched by the probes: no holds, default retention.
    let key_b = create_scoped_api_key(&env.db, org_b, &["org:admin"]).await;
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/gov-org-b/audit/legal-holds",
            None,
            &key_b,
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["count"], 0);
    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/gov-org-b/audit/retention",
            None,
            &key_b,
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["source"], "default");
}

#[tokio::test]
async fn test_audit_purge_unavailable_without_decision_store() {
    // The test env has no REAPER_CLICKHOUSE_URL → the manual purge answers 503
    // with setup guidance instead of pretending to have purged anything.
    let env = setup_test_env().await;
    let org_id = seed_org(&env, "Purge Org", "purge-org").await;
    let key = create_scoped_api_key(&env.db, org_id, &["org:admin"]).await;

    let resp = env
        .app
        .clone()
        .oneshot(authed_request(
            "POST",
            "/orgs/purge-org/audit/purge",
            None,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ============================================================================
// Counterfactual replay (Plan 04, step 8)
// ============================================================================

/// Full bundle→headless-engine load (the replay engine's policy path), plus
/// the API's error posture. The row-scan side is covered by unit tests over
/// `replay_row` (flip counting, reproduction sanity, encryption) — this test
/// proves a REAL compiled bundle loads into a REAL engine.
#[tokio::test]
async fn test_replay_engine_loads_real_bundle_and_api_guards() {
    let env = setup_test_env().await;

    // Org + compiled bundle via the same API journey operators take.
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({"name": "Replay Org", "slug": "replay-org"})),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    let create_policy = authed_request(
        "POST",
        "/orgs/replay-org/policies",
        Some(json!({
            "name": "replay-policy",
            "language": "reaper",
            "content": "policy replaytest {\n    default: deny,\n    rule allow_user_read {\n        allow if {\n            context.action == \"read\" && context.principal == \"user\"\n        }\n    }\n}"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_id = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let create_bundle = authed_request(
        "POST",
        "/orgs/replay-org/bundles",
        Some(json!({"name": "replay-bundle", "policy_ids": [policy_id]})),
        &key,
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bundle_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let compile = authed_request(
        "POST",
        &format!("/orgs/replay-org/bundles/{bundle_id}/compile"),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(compile).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Namespace + datastore + published snapshot: the DSL evaluator resolves
    // principals as entities (like a production agent with synced data), so
    // replay pins the same data_version the decisions recorded.
    for (uri, body) in [
        ("/orgs/replay-org/namespaces", json!({"slug": "prod"})),
        (
            "/orgs/replay-org/namespaces/prod/datastore",
            json!({"template": "combined"}),
        ),
        (
            "/orgs/replay-org/namespaces/prod/datastore/entities",
            json!({"entity_id": "user", "entity_type": "user", "attributes": {}}),
        ),
    ] {
        let response = env
            .app
            .clone()
            .oneshot(authed_request("POST", uri, Some(body), &key))
            .await
            .unwrap();
        assert!(response.status().is_success(), "{uri}");
    }
    let publish = authed_request(
        "POST",
        "/orgs/replay-org/namespaces/prod/datastore/publish",
        None,
        &key,
    );
    let response = env.app.clone().oneshot(publish).await.unwrap();
    assert!(response.status().is_success());
    let published_version = parse_body(response).await["version"].as_i64().unwrap();
    assert_eq!(published_version, 1);

    // A second AppState over the SAME db + storage dir stands in for the
    // running server (TestEnv only exposes the router).
    let storage = Arc::new(FilesystemStorage::new(&env.temp_dir.path().join("storage")).unwrap())
        as Arc<dyn reaper_management::storage::BundleStorage>;
    let config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        ..Config::default()
    };
    let state = AppState::new(env.db.clone(), config, storage);

    let request = |bundle_id: Uuid| reaper_management::replay::ReplayRequest {
        bundle_id,
        from: None,
        to: None,
        filter: Default::default(),
        namespace: Some("prod".to_string()),
        data_version: Some(1),
        decryption_key: None,
        max_rows: None,
    };

    // The compiled artifact loads into a headless engine, policies deployed.
    let (engine, policy_ids, data_version) =
        reaper_management::replay::build_headless_engine(&state, org_id, &request(bundle_id))
            .await
            .expect("compiled bundle must load");
    assert_eq!(policy_ids.len(), 1);
    assert_eq!(data_version, Some(1), "pinned snapshot loaded");
    assert_eq!(engine.get_stats().total_policies, 1);

    // And the loaded engine actually DECIDES — a true headless evaluation of
    // the real compiled artifact, with the production set semantics.
    let eval = |principal: &str, action: &str| {
        let mut context = std::collections::HashMap::new();
        context.insert("principal".to_string(), principal.to_string());
        engine.evaluate_set(
            &policy_ids,
            &policy_engine::PolicyRequest {
                resource: "/api".to_string(),
                action: action.to_string(),
                context,
            },
        )
    };
    assert_eq!(
        eval("user", "read").decision,
        policy_engine::PolicyAction::Allow,
        "the pinned snapshot resolves the principal"
    );
    // A principal absent from the snapshot fails closed — same as production.
    assert_eq!(
        eval("intruder", "read").decision,
        policy_engine::PolicyAction::Deny
    );
    assert_eq!(
        eval("user", "write").decision,
        policy_engine::PolicyAction::Deny
    );

    // Unknown bundle → clear not-found, tenant-scoped.
    let err =
        reaper_management::replay::build_headless_engine(&state, org_id, &request(Uuid::new_v4()))
            .await
            .unwrap_err();
    assert!(err.contains("not found"), "{err}");

    // API posture: no ClickHouse in the test env → POST replay is 503 with
    // setup guidance, never a fake success.
    let post = authed_request(
        "POST",
        "/orgs/replay-org/replay",
        Some(json!({"bundle_id": bundle_id})),
        &key,
    );
    let response = env.app.clone().oneshot(post).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Non-admin credential → 403 (replay reads audit data wholesale).
    let reader = create_scoped_api_key(&env.db, org_id, &["agent:read"]).await;
    let post = authed_request(
        "POST",
        "/orgs/replay-org/replay",
        Some(json!({"bundle_id": bundle_id})),
        &reader,
    );
    let response = env.app.clone().oneshot(post).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Unknown job id → 404.
    let get = authed_request(
        "GET",
        &format!("/orgs/replay-org/replay/{}", Uuid::new_v4()),
        None,
        &key,
    );
    let response = env.app.clone().oneshot(get).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// =============================================================================
// Single /api/v1 surface + deprecation alias (Plan 07 Phase B)
// =============================================================================

/// A raw GET that bypasses the `v1_uri` helper — used to probe the exact path.
fn raw_get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn resource_api_is_only_under_api_v1() {
    let env = setup_test_env().await;

    // Bare-root resource path is not served → 404 (the pre-Plan-07 dual mount
    // is gone).
    let resp = env.app.clone().oneshot(raw_get("/orgs")).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "bare /orgs must 404 on the single /api/v1 surface"
    );

    // The versioned path exists — unauthenticated, so 401/403, but NOT 404.
    let resp = env
        .app
        .clone()
        .oneshot(raw_get("/api/v1/orgs"))
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "/api/v1/orgs must be routed"
    );

    // Probes stay unversioned at the root.
    let resp = env.app.clone().oneshot(raw_get("/health")).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "/health stays at root"
    );
    let resp = env
        .app
        .clone()
        .oneshot(raw_get("/openapi.json"))
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "/openapi.json stays at root, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn deprecation_headers_marker_on_alias() {
    use axum::{routing::get, Router};
    // The alias applies this layer to the bare-root routes; assert it tags the
    // response per RFC 8594 regardless of the handler outcome.
    let app =
        Router::new()
            .route("/orgs", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(
                reaper_management::middleware::deprecation_headers,
            ));
    let resp = app.oneshot(raw_get("/orgs")).await.unwrap();
    assert_eq!(resp.headers().get("Deprecation").unwrap(), "true");
    assert!(resp
        .headers()
        .get("Link")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("successor-version"));
    assert!(resp.headers().get("Warning").is_some());
}

// =============================================================================
// Optimistic concurrency: ETag / If-Match on policy + bundle PUTs (Plan 07,
// Phase C). The DoD two-writer test: both read the same ETag, exactly one
// write succeeds, the loser gets 412 and its content never lands.
// =============================================================================

/// `authed_request` plus an `If-Match` precondition header.
fn authed_request_if_match(
    method: &str,
    uri: &str,
    body: Option<Value>,
    api_key: &str,
    if_match: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(v1_uri(uri))
        .method(method)
        .header("X-API-Key", api_key)
        .header("If-Match", if_match);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());
    builder.body(body).unwrap()
}

/// Bootstrap an org + api key + one policy; returns (key, policy path).
async fn seed_policy(env: &TestEnv, slug: &str) -> (String, String) {
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({"name": format!("{slug} org"), "slug": slug})),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    let create_policy = authed_request(
        "POST",
        &format!("/orgs/{slug}/policies"),
        Some(json!({
            "name": "cc-policy",
            "language": "reaper",
            "content": "allow admin to access /admin"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_id = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    (key, format!("/orgs/{slug}/policies/{policy_id}"))
}

fn etag_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get("ETag")
        .expect("response carries an ETag")
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn policy_get_returns_etag_and_put_rotates_it() {
    let env = setup_test_env().await;
    let (key, path) = seed_policy(&env, "etag-org").await;

    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", &path, None, &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let tag = etag_of(&response);
    assert!(
        tag.starts_with('"') && tag.ends_with('"'),
        "strong quoted ETag: {tag}"
    );

    // Guarded content update with the correct ETag succeeds and rotates it.
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"content": "allow admin to access /admin/*"})),
            &key,
            &tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let new_tag = etag_of(&response);
    assert_ne!(new_tag, tag, "content update must rotate the ETag");
}

#[tokio::test]
async fn policy_lost_update_is_prevented() {
    let env = setup_test_env().await;
    let (key, path) = seed_policy(&env, "race-org").await;

    // Both writers read the same state.
    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", &path, None, &key))
        .await
        .unwrap();
    let shared_tag = etag_of(&response);

    // Writer A wins.
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"content": "allow writer-a to access /a"})),
            &key,
            &shared_tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Writer B, holding the now-stale tag, must get 412 — not a silent clobber.
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"content": "allow writer-b to access /b"})),
            &key,
            &shared_tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);

    // Writer A's version (2) is current; writer B's content never landed.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            &format!("{path}/versions"),
            None,
            &key,
        ))
        .await
        .unwrap();
    let body = parse_body(response).await;
    let versions = body["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2, "exactly one successful content update");
}

#[tokio::test]
async fn policy_put_without_if_match_modes() {
    // Transitional default (warn-only): unguarded PUT still succeeds.
    let env = setup_test_env().await;
    let (key, path) = seed_policy(&env, "warn-org").await;
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "PUT",
            &path,
            Some(json!({"description": "no precondition"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Enforcing mode: missing If-Match → 428 Precondition Required (ADR-3).
    let env = setup_env_with(|c| c.server.require_if_match = true).await;
    let (key, path) = seed_policy(&env, "enforce-org").await;
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "PUT",
            &path,
            Some(json!({"description": "no precondition"})),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);

    // But a correct If-Match still succeeds under enforcement.
    let get = env
        .app
        .clone()
        .oneshot(authed_request("GET", &path, None, &key))
        .await
        .unwrap();
    let tag = etag_of(&get);
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"description": "guarded"})),
            &key,
            &tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn bundle_lost_update_is_prevented() {
    let env = setup_test_env().await;

    // Org + key + bundle.
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({"name": "Bundle CC Org", "slug": "bundle-cc-org"})),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    let create_bundle = authed_request(
        "POST",
        "/orgs/bundle-cc-org/bundles",
        Some(json!({"name": "cc-bundle", "description": "before"})),
        &key,
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bundle_id = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let path = format!("/orgs/bundle-cc-org/bundles/{bundle_id}");

    // Both writers read the same ETag.
    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", &path, None, &key))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let shared_tag = etag_of(&response);

    // Writer A wins (and the ETag rotates: updated_at bumped).
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"name": "writer-a"})),
            &key,
            &shared_tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_ne!(etag_of(&response), shared_tag);

    // Writer B with the stale tag → 412; its rename never lands.
    let response = env
        .app
        .clone()
        .oneshot(authed_request_if_match(
            "PUT",
            &path,
            Some(json!({"name": "writer-b"})),
            &key,
            &shared_tag,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);

    let response = env
        .app
        .clone()
        .oneshot(authed_request("GET", &path, None, &key))
        .await
        .unwrap();
    assert_eq!(parse_body(response).await["name"], "writer-a");
}

// =============================================================================
// Idempotency keys on propagation POSTs (Plan 07, Phase D). DoD: replaying the
// same key returns the original result (same status + body) and does NOT
// re-trigger the side effect; the same key with a different body is a 422.
// =============================================================================

/// Request builder with an `Idempotency-Key` (and optional API key).
fn idem_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    api_key: Option<&str>,
    idem_key: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(v1_uri(uri))
        .method(method)
        .header("Idempotency-Key", idem_key);
    if let Some(key) = api_key {
        builder = builder.header("X-API-Key", key);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());
    builder.body(body).unwrap()
}

fn replayed_header(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get("Idempotency-Replayed")
        .map(|v| v.to_str().unwrap().to_string())
}

#[tokio::test]
async fn org_create_idempotency_replay() {
    let env = setup_test_env().await;
    let body = json!({"name": "Idem Org", "slug": "idem-org"});

    // First execution: created, not a replay.
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            "/orgs",
            Some(body.clone()),
            None,
            "key-1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(replayed_header(&response).as_deref(), Some("false"));
    let first = parse_body(response).await;
    let org_id = first["id"].as_str().unwrap().to_string();

    // Replay: identical status + body, marked as a replay, and no second org.
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            "/orgs",
            Some(body.clone()),
            None,
            "key-1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(replayed_header(&response).as_deref(), Some("true"));
    let second = parse_body(response).await;
    assert_eq!(
        first, second,
        "replay must return the stored response verbatim"
    );

    // Without idempotency the same slug would 409; the replay bypassed the
    // handler entirely. Exactly one org with this slug exists.
    let response = env
        .app
        .clone()
        .oneshot(json_request("GET", &format!("/orgs/{org_id}"), None))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::NOT_FOUND);

    // Same key, DIFFERENT body → 422 (ADR-6), never a silent replay.
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Other Org", "slug": "other-org"})),
            None,
            "key-1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A request WITHOUT a key still behaves exactly as before (duplicate slug
    // reaches the handler and conflicts).
    let response = env
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/orgs",
            Some(json!({"name": "Idem Org", "slug": "idem-org"})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn promote_idempotency_replay_single_side_effect() {
    let env = setup_test_env().await;

    // Org + key + policy + compiled/staged bundle.
    let create_org = json_request(
        "POST",
        "/orgs",
        Some(json!({"name": "Idem Promote Org", "slug": "idem-promote-org"})),
    );
    let response = env.app.clone().oneshot(create_org).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let org_id: Uuid = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = create_test_api_key(&env.db, org_id).await;

    let create_policy = authed_request(
        "POST",
        "/orgs/idem-promote-org/policies",
        Some(json!({
            "name": "idem-policy",
            "language": "reaper",
            "content": "allow user to read /api"
        })),
        &key,
    );
    let response = env.app.clone().oneshot(create_policy).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let policy_id = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let create_bundle = authed_request(
        "POST",
        "/orgs/idem-promote-org/bundles",
        Some(json!({"name": "idem-bundle", "policy_ids": [policy_id]})),
        &key,
    );
    let response = env.app.clone().oneshot(create_bundle).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bundle_id = parse_body(response).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    for step in ["compile", "stage"] {
        let req = authed_request(
            "POST",
            &format!("/orgs/idem-promote-org/bundles/{bundle_id}/{step}"),
            None,
            &key,
        );
        let response = env.app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{step} must succeed");
    }

    // Promote with an idempotency key (single-control default → 200).
    let promote_uri = format!("/orgs/idem-promote-org/bundles/{bundle_id}/promote");
    let promote_body = json!({"notes": "release"});
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            &promote_uri,
            Some(promote_body.clone()),
            Some(&key),
            "promote-1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(replayed_header(&response).as_deref(), Some("false"));
    let first = parse_body(response).await;
    assert_eq!(first["status"], "promoted");

    // Replay: same response, no second promotion attempt. (Without the key
    // this request would now 400 — the bundle is no longer in a promotable
    // state — so a 200 here proves the handler did not run again.)
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            &promote_uri,
            Some(promote_body.clone()),
            Some(&key),
            "promote-1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(replayed_header(&response).as_deref(), Some("true"));
    assert_eq!(parse_body(response).await, first);

    // Exactly ONE change record exists — the side effect happened once.
    let response = env
        .app
        .clone()
        .oneshot(authed_request(
            "GET",
            "/orgs/idem-promote-org/change-requests",
            None,
            &key,
        ))
        .await
        .unwrap();
    let body = parse_body(response).await;
    let crs = body.as_array().cloned().unwrap_or_else(|| {
        body["change_requests"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    });
    assert_eq!(crs.len(), 1, "replay must not open a second change request");

    // A FRESH key reaches the handler, which now correctly rejects (the bundle
    // is already promoted) — and the failed attempt is NOT memoized.
    let response = env
        .app
        .clone()
        .oneshot(idem_request(
            "POST",
            &promote_uri,
            Some(promote_body),
            Some(&key),
            "promote-2",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
