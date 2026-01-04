//! Health check endpoints
//!
//! Provides health and metrics endpoints for monitoring.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Json, Response},
    routing::get,
    Router,
};
use serde::Serialize;
use std::sync::Arc;

use crate::metrics;
use crate::state::AppState;

/// Health response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: i64,
    pub database: DatabaseHealth,
}

#[derive(Debug, Serialize)]
pub struct DatabaseHealth {
    pub status: String,
    #[serde(rename = "type")]
    pub db_type: String,
}

/// Metrics response
#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub uptime_seconds: i64,
    pub database_type: String,
    pub event_subscribers: usize,
}

/// Build health routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/metrics/prometheus", get(prometheus_metrics_handler))
}

/// Health check handler
async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthResponse>, StatusCode> {
    // Check database connectivity
    let db_status = match state.db.sqlite_pool() {
        Some(pool) => match sqlx::query("SELECT 1").fetch_one(pool).await {
            Ok(_) => "connected",
            Err(e) => {
                tracing::error!("Database health check failed: {}", e);
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
        },
        None => "not_configured",
    };

    Ok(Json(HealthResponse {
        status: "healthy".to_string(),
        version: crate::VERSION.to_string(),
        uptime_seconds: state.uptime_seconds(),
        database: DatabaseHealth {
            status: db_status.to_string(),
            db_type: state.db.db_type().to_string(),
        },
    }))
}

/// Metrics handler (JSON format)
async fn metrics_handler(State(state): State<Arc<AppState>>) -> Json<MetricsResponse> {
    // Update SSE subscribers metric
    metrics::SSE_SUBSCRIBERS.set(state.event_tx.receiver_count() as f64);

    Json(MetricsResponse {
        uptime_seconds: state.uptime_seconds(),
        database_type: state.db.db_type().to_string(),
        event_subscribers: state.event_tx.receiver_count(),
    })
}

/// Prometheus metrics handler (text format)
async fn prometheus_metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    // Update gauge metrics before encoding
    metrics::SSE_SUBSCRIBERS.set(state.event_tx.receiver_count() as f64);

    let body = metrics::encode_metrics();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(body.into())
        .unwrap()
}
