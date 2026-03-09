//! Prometheus metrics for Reaper Management Server
//!
//! Provides metrics for monitoring management server operations.

use lazy_static::lazy_static;
use prometheus::{
    register_counter_vec, register_gauge, register_gauge_vec, register_histogram_vec, CounterVec,
    Encoder, Gauge, GaugeVec, HistogramVec, TextEncoder,
};

lazy_static! {
    // === API Request Metrics ===

    /// Total API requests by endpoint and status
    pub static ref API_REQUESTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_api_requests_total",
        "Total API requests",
        &["endpoint", "method", "status"]
    )
    .unwrap();

    /// API request duration histogram
    pub static ref API_REQUEST_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_management_api_request_duration_seconds",
        "API request duration in seconds",
        &["endpoint", "method"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
    )
    .unwrap();

    // === Organization Metrics ===

    /// Active organizations count
    pub static ref ORGANIZATIONS_TOTAL: Gauge = register_gauge!(
        "reaper_management_organizations_total",
        "Total number of organizations"
    )
    .unwrap();

    // === Policy Metrics ===

    /// Total policies by organization
    pub static ref POLICIES_TOTAL: GaugeVec = register_gauge_vec!(
        "reaper_management_policies_total",
        "Total policies per organization",
        &["org_id"]
    )
    .unwrap();

    /// Policy operations (create, update, delete)
    pub static ref POLICY_OPERATIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_policy_operations_total",
        "Policy operations",
        &["operation", "org_id"]
    )
    .unwrap();

    // === Bundle Metrics ===

    /// Total bundles by organization and status
    pub static ref BUNDLES_TOTAL: GaugeVec = register_gauge_vec!(
        "reaper_management_bundles_total",
        "Total bundles per organization",
        &["org_id", "status"]
    )
    .unwrap();

    /// Bundle operations (create, promote, deprecate)
    pub static ref BUNDLE_OPERATIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_bundle_operations_total",
        "Bundle operations",
        &["operation", "org_id"]
    )
    .unwrap();

    /// Bundle compilation duration
    pub static ref BUNDLE_COMPILE_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_management_bundle_compile_duration_seconds",
        "Bundle compilation duration in seconds",
        &["org_id"],
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .unwrap();

    // === Agent Metrics ===

    /// Registered agents by organization and status
    pub static ref AGENTS_TOTAL: GaugeVec = register_gauge_vec!(
        "reaper_management_agents_total",
        "Total registered agents",
        &["org_id", "status"]
    )
    .unwrap();

    /// Agent heartbeats received
    pub static ref AGENT_HEARTBEATS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_agent_heartbeats_total",
        "Total agent heartbeats received",
        &["org_id", "agent_id"]
    )
    .unwrap();

    /// Agent registrations
    pub static ref AGENT_REGISTRATIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_agent_registrations_total",
        "Total agent registrations",
        &["org_id", "result"]
    )
    .unwrap();

    // === Policy Source Metrics ===

    /// Policy sources by type and status
    pub static ref SOURCES_TOTAL: GaugeVec = register_gauge_vec!(
        "reaper_management_sources_total",
        "Total policy sources",
        &["org_id", "source_type", "status"]
    )
    .unwrap();

    /// Source sync operations
    pub static ref SOURCE_SYNCS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_source_syncs_total",
        "Total source sync operations",
        &["org_id", "source_type", "result"]
    )
    .unwrap();

    // === Storage Metrics ===

    /// Storage operations
    pub static ref STORAGE_OPERATIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_storage_operations_total",
        "Total storage operations",
        &["operation", "backend", "result"]
    )
    .unwrap();

    /// Storage operation duration
    pub static ref STORAGE_OPERATION_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_management_storage_operation_duration_seconds",
        "Storage operation duration in seconds",
        &["operation", "backend"],
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5]
    )
    .unwrap();

    // === Database Metrics ===

    /// Database query count by operation
    pub static ref DB_QUERIES_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_db_queries_total",
        "Total database queries",
        &["operation", "table"]
    )
    .unwrap();

    /// Database query duration
    pub static ref DB_QUERY_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_management_db_query_duration_seconds",
        "Database query duration in seconds",
        &["operation"],
        vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]
    )
    .unwrap();

    // === Event Metrics ===

    /// SSE events published
    pub static ref EVENTS_PUBLISHED_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_events_published_total",
        "Total SSE events published",
        &["event_type", "org_id"]
    )
    .unwrap();

    /// Current SSE subscribers
    pub static ref SSE_SUBSCRIBERS: Gauge = register_gauge!(
        "reaper_management_sse_subscribers",
        "Current number of SSE subscribers"
    )
    .unwrap();

    // === Authentication Metrics ===

    /// Authentication attempts
    pub static ref AUTH_ATTEMPTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_auth_attempts_total",
        "Total authentication attempts",
        &["method", "result"]
    )
    .unwrap();

    /// Active JWT tokens (estimated)
    pub static ref ACTIVE_TOKENS: Gauge = register_gauge!(
        "reaper_management_active_tokens",
        "Estimated active JWT tokens"
    )
    .unwrap();

    // === Database Pool Metrics ===

    /// Database pool size (configured max connections)
    pub static ref DB_POOL_SIZE: Gauge = register_gauge!(
        "reaper_management_db_pool_size",
        "Database connection pool size (max connections)"
    )
    .unwrap();

    /// Database pool connections (current active + idle)
    pub static ref DB_POOL_CONNECTIONS: Gauge = register_gauge!(
        "reaper_management_db_pool_connections",
        "Current database pool connections"
    )
    .unwrap();

    /// Database pool idle connections
    pub static ref DB_POOL_IDLE: Gauge = register_gauge!(
        "reaper_management_db_pool_idle",
        "Current idle database pool connections"
    )
    .unwrap();

    // === Health Check Metrics ===

    /// Health check status by component (1 = healthy, 0 = unhealthy)
    pub static ref HEALTH_CHECK_STATUS: GaugeVec = register_gauge_vec!(
        "reaper_management_health_check_status",
        "Health check status by component (1=healthy, 0=unhealthy)",
        &["component"]
    )
    .unwrap();

    /// Health check latency by component
    pub static ref HEALTH_CHECK_LATENCY: HistogramVec = register_histogram_vec!(
        "reaper_management_health_check_latency_seconds",
        "Health check latency in seconds",
        &["component"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
    )
    .unwrap();

    // === Error Metrics ===

    /// Total errors by type
    pub static ref ERRORS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_management_errors_total",
        "Total errors by type",
        &["error_type", "endpoint"]
    )
    .unwrap();

    // === Request Size Metrics ===

    /// HTTP request body size
    pub static ref REQUEST_SIZE_BYTES: HistogramVec = register_histogram_vec!(
        "reaper_management_http_request_size_bytes",
        "HTTP request body size in bytes",
        &["endpoint", "method"],
        vec![100.0, 1000.0, 10000.0, 100000.0, 1000000.0, 10000000.0]
    )
    .unwrap();

    /// HTTP response body size
    pub static ref RESPONSE_SIZE_BYTES: HistogramVec = register_histogram_vec!(
        "reaper_management_http_response_size_bytes",
        "HTTP response body size in bytes",
        &["endpoint", "method"],
        vec![100.0, 1000.0, 10000.0, 100000.0, 1000000.0, 10000000.0]
    )
    .unwrap();

    // === Connection Metrics ===

    /// Active HTTP connections
    pub static ref ACTIVE_CONNECTIONS: Gauge = register_gauge!(
        "reaper_management_active_connections",
        "Current active HTTP connections"
    )
    .unwrap();
}

/// Encode all metrics to Prometheus text format
/// Returns an error if encoding fails instead of panicking
pub fn encode_metrics() -> Result<String, MetricsError> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| MetricsError::Encode(e.to_string()))?;
    String::from_utf8(buffer).map_err(|e| MetricsError::Utf8(e.to_string()))
}

/// Metrics encoding error
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("Failed to encode metrics: {0}")]
    Encode(String),
    #[error("Invalid UTF-8 in metrics output: {0}")]
    Utf8(String),
}

// Convenience aliases for commonly used metrics
pub use API_REQUESTS_TOTAL as API_REQUESTS;
pub use API_REQUEST_DURATION as API_LATENCY;
pub use STORAGE_OPERATIONS_TOTAL as STORAGE_OPERATIONS;

/// Initialize metrics (call once at startup)
pub fn init_metrics() {
    // Force lazy_static initialization
    lazy_static::initialize(&API_REQUESTS_TOTAL);
    lazy_static::initialize(&API_REQUEST_DURATION);
    lazy_static::initialize(&ORGANIZATIONS_TOTAL);
    lazy_static::initialize(&POLICIES_TOTAL);
    lazy_static::initialize(&BUNDLES_TOTAL);
    lazy_static::initialize(&AGENTS_TOTAL);
    lazy_static::initialize(&SOURCES_TOTAL);
    lazy_static::initialize(&SSE_SUBSCRIBERS);
    lazy_static::initialize(&DB_POOL_SIZE);
    lazy_static::initialize(&DB_POOL_CONNECTIONS);
    lazy_static::initialize(&DB_POOL_IDLE);
    lazy_static::initialize(&HEALTH_CHECK_STATUS);
    lazy_static::initialize(&ERRORS_TOTAL);
    lazy_static::initialize(&ACTIVE_CONNECTIONS);

    tracing::info!("Prometheus metrics initialized");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_encode() {
        init_metrics();

        // Record some test metrics
        API_REQUESTS_TOTAL
            .with_label_values(&["/health", "GET", "200"])
            .inc();
        ORGANIZATIONS_TOTAL.set(5.0);

        let output = encode_metrics().expect("encoding should succeed");
        assert!(output.contains("reaper_management_api_requests_total"));
        assert!(output.contains("reaper_management_organizations_total"));
    }
}
