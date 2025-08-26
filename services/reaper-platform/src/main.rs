use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, put},
    Router,
};
use reaper_core::{endpoints, BUILD_INFO, VERSION};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, instrument};

#[derive(Clone, Debug)]
struct PlatformState {
    // Platform state will be added here
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!("Starting Reaper Platform {}", BUILD_INFO);

    let state = Arc::new(PlatformState {});

    let app = Router::new()
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        .route(
            endpoints::API_V1_POLICIES,
            get(list_policies).post(create_policy),
        )
        .route("/api/v1/policies/:id", put(update_policy))
        .route(endpoints::API_V1_AGENTS, get(list_agents))
        .route("/api/v1/agents/:id", get(get_agent))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8081").await?;
    info!("Reaper Platform listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}

#[instrument]
async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-platform",
        "version": VERSION
    })))
}

#[instrument]
async fn metrics(State(_state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "service": "reaper-platform",
        "active_agents": 0,
        "total_policies": 0,
        "policy_deployments_today": 0
    })))
}

#[instrument]
async fn list_policies(
    State(_state): State<Arc<PlatformState>>,
) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "policies": [],
        "total": 0
    })))
}

#[instrument]
async fn create_policy(
    State(_state): State<Arc<PlatformState>>,
    Json(_payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "id": "policy-001",
        "status": "created",
        "deployed_to_agents": 0
    })))
}

#[instrument]
async fn update_policy(
    State(_state): State<Arc<PlatformState>>,
    Json(_payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "updated",
        "rollout_progress": "0%"
    })))
}

#[instrument]
async fn list_agents(State(_state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "agents": [],
        "total": 0
    })))
}

#[instrument]
async fn get_agent(State(_state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "id": "agent-001",
        "status": "healthy",
        "last_seen": "2025-08-20T10:00:00Z"
    })))
}
