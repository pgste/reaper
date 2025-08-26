//! # Reaper Agent
//!
//! High-performance policy enforcement service

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use reaper_core::{endpoints, BUILD_INFO};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, instrument};

#[derive(Clone)]
struct AgentState {
    // Agent state will be added here
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::init();

    info!("Starting Reaper Agent {}", BUILD_INFO);

    let state = Arc::new(AgentState {});

    let app = Router::new()
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        .route(endpoints::API_V1_MESSAGES, post(evaluate_policy))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("Reaper Agent listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}

#[instrument]
async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-agent",
        "version": reaper_core::VERSION
    })))
}

#[instrument]
async fn metrics(State(_state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "service": "reaper-agent",
        "uptime_seconds": 0,
        "policies_evaluated": 0,
        "average_response_time_microseconds": 0
    })))
}

#[instrument]
async fn evaluate_policy(
    State(_state): State<Arc<AgentState>>,
    Json(_payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    // Policy evaluation implementation will go here
    Ok(Json(json!({
        "decision": "allow",
        "policy_id": "default",
        "evaluation_time_microseconds": 1
    })))
}
