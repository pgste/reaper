//! Integration tests for Reaper Sync Client
//!
//! These tests use wiremock to simulate the server and agent endpoints.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod test_config {
    pub fn sync_config(server_url: &str, agent_url: &str) -> reaper_sync::config::SyncConfig {
        reaper_sync::config::SyncConfig {
            sync: reaper_sync::config::SyncSettings {
                server: reaper_sync::config::ServerConfig {
                    url: server_url.to_string(),
                    api_version: "v1".to_string(),
                    timeout_seconds: 30,
                },
                auth: reaper_sync::config::AuthConfig {
                    auth_type: "none".to_string(),
                    token: None,
                    token_file: None,
                    cert_file: None,
                    key_file: None,
                    ca_file: None,
                },
                scope: reaper_sync::config::ScopeConfig {
                    teams: vec![],
                    environments: vec![],
                    regions: vec![],
                    policy_ids: vec![],
                },
                behavior: reaper_sync::config::BehaviorConfig {
                    mode: "active".to_string(),
                    poll_interval_seconds: 1,
                    batch_size: 100,
                    retry_max_attempts: 3,
                    retry_backoff_seconds: 1,
                    sync_on_start: true,
                },
                agent: reaper_sync::config::AgentConfig {
                    url: agent_url.to_string(),
                    health_check_interval_seconds: 10,
                    timeout_seconds: 10,
                },
                cache: reaper_sync::config::CacheConfig::default(),
                metrics: reaper_sync::config::MetricsConfig::default(),
            },
        }
    }
}

#[tokio::test]
async fn test_server_client_list_policies() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Configure mock response
    Mock::given(method("GET"))
        .and(path("/api/v1/policies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "policies": [
                {
                    "id": "policy-1",
                    "name": "test-policy",
                    "version": 1,
                    "checksum": "abc123",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "language": "simple"
                }
            ],
            "total": 1,
            "page": 1
        })))
        .mount(&mock_server)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), "http://localhost:8080");
    let client = reaper_sync::server_client::ServerClient::new(config).unwrap();

    let response = client.list_policies().await.unwrap();
    assert_eq!(response.policies.len(), 1);
    assert_eq!(response.policies[0].name, "test-policy");
}

#[tokio::test]
async fn test_server_client_get_policy() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/policies/policy-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "policy-1",
            "name": "test-policy",
            "description": "A test policy",
            "version": 1,
            "language": "simple",
            "content": "allow if principal == \"admin\"",
            "rules": [],
            "checksum": "abc123",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        })))
        .mount(&mock_server)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), "http://localhost:8080");
    let client = reaper_sync::server_client::ServerClient::new(config).unwrap();

    let response = client.get_policy("policy-1").await.unwrap();
    assert_eq!(response.id, "policy-1");
    assert_eq!(response.name, "test-policy");
    assert_eq!(response.version, 1);
}

#[tokio::test]
async fn test_server_client_get_entities() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/entities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entities": [
                {
                    "type": "User",
                    "id": "user-1",
                    "attributes": {
                        "name": "Admin User",
                        "role": "admin"
                    }
                }
            ],
            "total": 1
        })))
        .mount(&mock_server)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), "http://localhost:8080");
    let client = reaper_sync::server_client::ServerClient::new(config).unwrap();

    let response = client.get_entities().await.unwrap();
    assert_eq!(response.entities.len(), 1);
}

#[tokio::test]
async fn test_agent_client_health_check() {
    let mock_agent = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "healthy"
        })))
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config("http://localhost:8081", &mock_agent.uri());
    let client = reaper_sync::agent_client::AgentClient::new(&config).unwrap();

    let healthy = client.health_check().await.unwrap();
    assert!(healthy);
}

#[tokio::test]
async fn test_agent_client_deploy_policy() {
    let mock_agent = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/policies/deploy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "deployed",
            "policy_id": "policy-1",
            "policy_name": "test-policy",
            "version": 1,
            "message": "Policy deployed successfully"
        })))
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config("http://localhost:8081", &mock_agent.uri());
    let client = reaper_sync::agent_client::AgentClient::new(&config).unwrap();

    let policy = reaper_sync::server_client::PolicyDetail {
        id: "policy-1".to_string(),
        name: "test-policy".to_string(),
        description: "Test policy".to_string(),
        version: 1,
        language: "simple".to_string(),
        content: "allow if true".to_string(),
        rules: Some(vec![]),
        metadata: None,
        checksum: None,
        created_at: None,
        updated_at: None,
    };

    let response = client.deploy_policy(&policy).await.unwrap();
    assert_eq!(response.policy_id, "policy-1");
}

#[tokio::test]
async fn test_agent_client_sync_data() {
    let mock_agent = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/data/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "synced",
            "inserted": 5,
            "failed": 0,
            "replaced": false,
            "total_entities": 5
        })))
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config("http://localhost:8081", &mock_agent.uri());
    let client = reaper_sync::agent_client::AgentClient::new(&config).unwrap();

    let entities = vec![
        json!({ "type": "User", "id": "user-1" }),
        json!({ "type": "User", "id": "user-2" }),
    ];

    let response = client.sync_data(entities, false).await.unwrap();
    assert_eq!(response.inserted, 5);
    assert_eq!(response.failed, 0);
}

#[tokio::test]
async fn test_sync_engine_sync_once() {
    // Start mock servers
    let mock_server = MockServer::start().await;
    let mock_agent = MockServer::start().await;

    // Mock server policy list
    Mock::given(method("GET"))
        .and(path("/api/v1/policies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "policies": [
                {
                    "id": "policy-1",
                    "name": "test-policy",
                    "version": 1,
                    "checksum": "abc123",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "language": "simple"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    // Mock server policy details
    Mock::given(method("GET"))
        .and(path("/api/v1/policies/policy-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "policy-1",
            "name": "test-policy",
            "description": "Test policy",
            "version": 1,
            "language": "simple",
            "content": "allow if true",
            "rules": []
        })))
        .mount(&mock_server)
        .await;

    // Mock agent health check
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_agent)
        .await;

    // Mock agent deploy
    Mock::given(method("POST"))
        .and(path("/api/v1/policies/deploy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "deployed",
            "policy_id": "policy-1"
        })))
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), &mock_agent.uri());
    let mut engine = reaper_sync::sync_engine::SyncEngine::new(config).unwrap();

    let result = engine.sync_once().await;

    assert!(result.success);
    assert_eq!(result.deployed, 1);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.failed, 0);
}

#[tokio::test]
async fn test_sync_engine_skips_unchanged_policies() {
    let mock_server = MockServer::start().await;
    let mock_agent = MockServer::start().await;

    // Mock server policy list
    Mock::given(method("GET"))
        .and(path("/api/v1/policies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "policies": [
                {
                    "id": "policy-1",
                    "name": "test-policy",
                    "version": 1,
                    "checksum": "abc123",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "language": "simple"
                }
            ]
        })))
        .expect(2)
        .mount(&mock_server)
        .await;

    // Mock server policy details (only called once)
    Mock::given(method("GET"))
        .and(path("/api/v1/policies/policy-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "policy-1",
            "name": "test-policy",
            "description": "Test policy",
            "version": 1,
            "language": "simple",
            "content": "allow if true",
            "rules": []
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // Mock agent health check
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(2)
        .mount(&mock_agent)
        .await;

    // Mock agent deploy (only called once)
    Mock::given(method("POST"))
        .and(path("/api/v1/policies/deploy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "deployed",
            "policy_id": "policy-1"
        })))
        .expect(1)
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), &mock_agent.uri());
    let mut engine = reaper_sync::sync_engine::SyncEngine::new(config).unwrap();

    // First sync - should deploy
    let result1 = engine.sync_once().await;
    assert!(result1.success);
    assert_eq!(result1.deployed, 1);

    // Second sync - should skip (unchanged)
    let result2 = engine.sync_once().await;
    assert!(result2.success);
    assert_eq!(result2.deployed, 0);
    assert_eq!(result2.skipped, 1);
}

#[tokio::test]
async fn test_sync_engine_handles_agent_unavailable() {
    let mock_server = MockServer::start().await;

    // Agent endpoint that returns 503
    let mock_agent = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_agent)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), &mock_agent.uri());
    let mut engine = reaper_sync::sync_engine::SyncEngine::new(config).unwrap();

    let result = engine.sync_once().await;

    assert!(!result.success);
    assert!(result.error.is_some());
}

#[tokio::test]
async fn test_sync_engine_handles_server_error() {
    let mock_server = MockServer::start().await;
    let mock_agent = MockServer::start().await;

    // Mock agent health check passes
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_agent)
        .await;

    // Mock server returns error
    Mock::given(method("GET"))
        .and(path("/api/v1/policies"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let config = test_config::sync_config(&mock_server.uri(), &mock_agent.uri());
    let mut engine = reaper_sync::sync_engine::SyncEngine::new(config).unwrap();

    let result = engine.sync_once().await;

    assert!(!result.success);
    assert!(result.error.is_some());
}
