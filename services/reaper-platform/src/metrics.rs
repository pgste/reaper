//! Prometheus metrics for the platform.

use lazy_static::lazy_static;
use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Gauge, HistogramVec,
};

lazy_static! {
    /// Total API requests by endpoint and status
    pub static ref API_REQUESTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_platform_api_requests_total",
        "Total API requests",
        &["endpoint", "method", "status"]
    )
    .unwrap();

    /// API request duration histogram
    pub static ref API_REQUEST_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_platform_api_request_duration_seconds",
        "API request duration in seconds",
        &["endpoint"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
    )
    .unwrap();

    /// Total policies managed
    pub static ref POLICIES_TOTAL: Gauge = register_gauge!(
        "reaper_platform_policies_total",
        "Total policies managed"
    )
    .unwrap();

    /// Total deployments
    pub static ref DEPLOYMENTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_platform_deployments_total",
        "Total policy deployments",
        &["result"]
    )
    .unwrap();

    /// Registered agents
    pub static ref AGENTS_TOTAL: Gauge = register_gauge!(
        "reaper_platform_agents_total",
        "Total registered agents"
    )
    .unwrap();

    /// Bundles stored
    pub static ref BUNDLES_TOTAL: Gauge = register_gauge!(
        "reaper_platform_bundles_total",
        "Total bundles stored"
    )
    .unwrap();
}
