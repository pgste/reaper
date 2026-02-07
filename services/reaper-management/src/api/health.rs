//! Health check endpoints
//!
//! Provides health, readiness, and liveness endpoints for monitoring.
//!
//! Endpoints:
//! - `/health` - Comprehensive health check (database connectivity)
//! - `/health/live` - Liveness probe (process is running)
//! - `/health/ready` - Readiness probe (can accept traffic)
//! - `/health/deep` - Deep health check with all component status
//! - `/metrics` - JSON metrics
//! - `/metrics/prometheus` - Prometheus text format metrics

use axum::{
    extract::State,
    http::StatusCode,
    response::{Json, Response},
    routing::get,
    Router,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics;
use crate::state::AppState;

/// Health response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: i64,
    pub database: ComponentHealth,
}

/// Component health status
#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Health status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Liveness response (minimal)
#[derive(Debug, Serialize)]
pub struct LivenessResponse {
    pub status: String,
}

/// Readiness response
#[derive(Debug, Serialize)]
pub struct ReadinessResponse {
    pub status: String,
    pub checks: ReadinessChecks,
}

#[derive(Debug, Serialize)]
pub struct ReadinessChecks {
    pub database: bool,
    pub storage: bool,
}

/// Deep health response with all components
#[derive(Debug, Serialize)]
pub struct DeepHealthResponse {
    pub status: HealthStatus,
    pub version: String,
    pub uptime_seconds: i64,
    pub components: ComponentsHealth,
    pub checks_duration_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct ComponentsHealth {
    pub database: ComponentHealth,
    pub storage: ComponentHealth,
    pub event_broadcaster: ComponentHealth,
}

/// Metrics response (JSON format)
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
        .route("/health/live", get(liveness_handler))
        .route("/health/ready", get(readiness_handler))
        .route("/health/deep", get(deep_health_handler))
        .route("/live", get(liveness_handler))
        .route("/ready", get(readiness_handler))
        .route("/metrics", get(metrics_handler))
        .route("/metrics/prometheus", get(prometheus_metrics_handler))
}

/// Health check handler (standard health check)
async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthResponse>, StatusCode> {
    let db_health = check_database(&state).await;

    if db_health.status == HealthStatus::Unhealthy {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(HealthResponse {
        status: "healthy".to_string(),
        version: crate::VERSION.to_string(),
        uptime_seconds: state.uptime_seconds(),
        database: db_health,
    }))
}

/// Liveness probe handler
/// Returns 200 if the process is running, regardless of dependency status
async fn liveness_handler() -> Json<LivenessResponse> {
    Json(LivenessResponse {
        status: "alive".to_string(),
    })
}

/// Readiness probe handler
/// Returns 200 only if the service can accept traffic
async fn readiness_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReadinessResponse>, StatusCode> {
    // Check if shutting down
    if state.is_shutting_down() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // Quick database check
    let db_ok = quick_check_database(&state).await;

    // Quick storage check
    let storage_ok = state.storage.is_available().await;

    if !db_ok || !storage_ok {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(ReadinessResponse {
        status: "ready".to_string(),
        checks: ReadinessChecks {
            database: db_ok,
            storage: storage_ok,
        },
    }))
}

/// Deep health check with all components
async fn deep_health_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DeepHealthResponse>, StatusCode> {
    let start = Instant::now();

    // Check all components
    let db_health = check_database(&state).await;
    let storage_health = check_storage(&state).await;
    let broadcaster_health = check_event_broadcaster(&state);

    let checks_duration = start.elapsed();

    // Determine overall status
    let overall_status = if db_health.status == HealthStatus::Unhealthy
        || storage_health.status == HealthStatus::Unhealthy
    {
        HealthStatus::Unhealthy
    } else if db_health.status == HealthStatus::Degraded
        || storage_health.status == HealthStatus::Degraded
        || broadcaster_health.status == HealthStatus::Degraded
    {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    // Update health metrics
    update_health_metrics(&db_health, &storage_health, &broadcaster_health);

    let response = DeepHealthResponse {
        status: overall_status,
        version: crate::VERSION.to_string(),
        uptime_seconds: state.uptime_seconds(),
        components: ComponentsHealth {
            database: db_health,
            storage: storage_health,
            event_broadcaster: broadcaster_health,
        },
        checks_duration_ms: checks_duration.as_millis() as u64,
    };

    // Return 503 if unhealthy
    if overall_status == HealthStatus::Unhealthy {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(response))
}

/// Check database connectivity with latency measurement
async fn check_database(state: &AppState) -> ComponentHealth {
    let start = Instant::now();

    match state.db.sqlite_pool() {
        Some(pool) => {
            match tokio::time::timeout(
                Duration::from_secs(5),
                sqlx::query("SELECT 1").fetch_one(pool),
            )
            .await
            {
                Ok(Ok(_)) => {
                    let latency = start.elapsed();
                    let status = if latency > Duration::from_secs(1) {
                        HealthStatus::Degraded
                    } else {
                        HealthStatus::Healthy
                    };
                    ComponentHealth {
                        status,
                        message: None,
                        latency_ms: Some(latency.as_millis() as u64),
                        details: Some(serde_json::json!({
                            "type": state.db.db_type(),
                        })),
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!("Database health check failed: {}", e);
                    ComponentHealth {
                        status: HealthStatus::Unhealthy,
                        message: Some("Database query failed".to_string()),
                        latency_ms: Some(start.elapsed().as_millis() as u64),
                        details: None,
                    }
                }
                Err(_) => {
                    tracing::error!("Database health check timed out");
                    ComponentHealth {
                        status: HealthStatus::Unhealthy,
                        message: Some("Database timeout".to_string()),
                        latency_ms: Some(5000),
                        details: None,
                    }
                }
            }
        }
        None => ComponentHealth {
            status: HealthStatus::Unhealthy,
            message: Some("No database pool configured".to_string()),
            latency_ms: None,
            details: None,
        },
    }
}

/// Quick database check (no latency measurement)
async fn quick_check_database(state: &AppState) -> bool {
    match state.db.sqlite_pool() {
        Some(pool) => tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query("SELECT 1").fetch_one(pool),
        )
        .await
        .is_ok(),
        None => false,
    }
}

/// Check storage backend
async fn check_storage(state: &AppState) -> ComponentHealth {
    let start = Instant::now();

    if state.storage.is_available().await {
        let latency = start.elapsed();
        ComponentHealth {
            status: HealthStatus::Healthy,
            message: None,
            latency_ms: Some(latency.as_millis() as u64),
            details: Some(serde_json::json!({
                "backend": state.storage.backend_name(),
            })),
        }
    } else {
        ComponentHealth {
            status: HealthStatus::Unhealthy,
            message: Some("Storage backend unavailable".to_string()),
            latency_ms: Some(start.elapsed().as_millis() as u64),
            details: Some(serde_json::json!({
                "backend": state.storage.backend_name(),
            })),
        }
    }
}

/// Check event broadcaster
fn check_event_broadcaster(state: &AppState) -> ComponentHealth {
    let subscriber_count = state.event_tx.receiver_count();

    // Consider degraded if too many subscribers (potential memory issue)
    let status = if subscriber_count > 10000 {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    ComponentHealth {
        status,
        message: None,
        latency_ms: None,
        details: Some(serde_json::json!({
            "subscribers": subscriber_count,
        })),
    }
}

/// Update health-related metrics
fn update_health_metrics(
    db: &ComponentHealth,
    storage: &ComponentHealth,
    _broadcaster: &ComponentHealth,
) {
    // Update database latency metric if available
    if let Some(latency) = db.latency_ms {
        metrics::DB_QUERY_DURATION
            .with_label_values(&["health_check"])
            .observe(latency as f64 / 1000.0);
    }

    // Update storage operations metric
    if storage.latency_ms.is_some() {
        metrics::STORAGE_OPERATIONS
            .with_label_values(&["health_check", "success"])
            .inc();
    }
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
async fn prometheus_metrics_handler(State(state): State<Arc<AppState>>) -> Response<String> {
    // Update gauge metrics before encoding
    metrics::SSE_SUBSCRIBERS.set(state.event_tx.receiver_count() as f64);

    match metrics::encode_metrics() {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            .body(body)
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body("Failed to build response".to_string())
                    .unwrap()
            }),
        Err(e) => {
            tracing::error!("Failed to encode metrics: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(format!("Failed to encode metrics: {}", e))
                .unwrap()
        }
    }
}
