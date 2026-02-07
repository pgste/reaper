//! Integration tests for Unix Domain Socket transport.
//!
//! These tests spin up a minimal axum server on a temporary Unix socket
//! and verify the SDK's UDS client can communicate with it.

use axum::{routing::get, routing::post, Json, Router};
use reaper_sdk::{Decision, PolicyRequest, PolicyResponse, ReaperClient, Source};
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::net::UnixListener;

/// Create a minimal mock agent router for testing.
fn mock_agent_router() -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/v1/messages", post(evaluate_handler))
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn evaluate_handler(Json(_request): Json<PolicyRequest>) -> Json<PolicyResponse> {
    Json(PolicyResponse {
        decision: Decision::Allow,
        latency_ns: 42,
        source: Source::Userspace,
    })
}

#[tokio::test]
async fn test_uds_health_check() {
    let tmp_dir = TempDir::new().unwrap();
    let socket_path = tmp_dir.path().join("test-agent.sock");

    let uds_listener = UnixListener::bind(&socket_path).unwrap();
    let app = mock_agent_router();

    // Spawn the mock server
    tokio::spawn(async move {
        axum::serve(uds_listener, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Create UDS client and check health
    let client = ReaperClient::unix(&socket_path).unwrap();
    let result = client.health_check().await;
    assert!(result.is_ok(), "Health check failed: {:?}", result.err());
}

#[tokio::test]
async fn test_uds_evaluate() {
    let tmp_dir = TempDir::new().unwrap();
    let socket_path = tmp_dir.path().join("test-agent.sock");

    let uds_listener = UnixListener::bind(&socket_path).unwrap();
    let app = mock_agent_router();

    tokio::spawn(async move {
        axum::serve(uds_listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = ReaperClient::unix(&socket_path).unwrap();

    let request = PolicyRequest {
        policy_id: "test-policy".to_string(),
        principal: "user:alice".to_string(),
        action: "read".to_string(),
        resource: "/api/data".to_string(),
        context: HashMap::new(),
    };

    let response = client.evaluate(request).await.unwrap();
    assert_eq!(response.decision, Decision::Allow);
    assert_eq!(response.latency_ns, 42);
    assert_eq!(response.source, Source::Userspace);
}

#[tokio::test]
async fn test_uds_connection_failure() {
    // Attempting to use a non-existent socket should fail on request, not on creation
    let client = ReaperClient::unix("/tmp/nonexistent-reaper-socket-12345.sock").unwrap();
    let result = client.health_check().await;
    assert!(result.is_err(), "Expected error for non-existent socket");
}

#[tokio::test]
async fn test_uds_post_json_generic() {
    let tmp_dir = TempDir::new().unwrap();
    let socket_path = tmp_dir.path().join("test-agent.sock");

    let uds_listener = UnixListener::bind(&socket_path).unwrap();
    let app = mock_agent_router();

    tokio::spawn(async move {
        axum::serve(uds_listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = ReaperClient::unix(&socket_path).unwrap();

    // Use post_json with the SDK's own types to verify the generic method
    let request = PolicyRequest {
        policy_id: "test-policy".to_string(),
        principal: "user:alice".to_string(),
        action: "read".to_string(),
        resource: "/api/data".to_string(),
        context: HashMap::new(),
    };

    let response: PolicyResponse = client
        .post_json("/api/v1/messages", &request)
        .await
        .unwrap();
    assert_eq!(response.decision, Decision::Allow);
}

#[tokio::test]
async fn test_uds_multiple_requests() {
    let tmp_dir = TempDir::new().unwrap();
    let socket_path = tmp_dir.path().join("test-agent.sock");

    let uds_listener = UnixListener::bind(&socket_path).unwrap();
    let app = mock_agent_router();

    tokio::spawn(async move {
        axum::serve(uds_listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = ReaperClient::unix(&socket_path).unwrap();

    // Send multiple requests to verify connection pooling works
    for i in 0..10 {
        let request = PolicyRequest {
            policy_id: format!("policy-{}", i),
            principal: "user:alice".to_string(),
            action: "read".to_string(),
            resource: "/api/data".to_string(),
            context: HashMap::new(),
        };

        let response = client.evaluate(request).await.unwrap();
        assert_eq!(response.decision, Decision::Allow);
    }
}
