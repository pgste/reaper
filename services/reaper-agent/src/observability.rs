//! Observability infrastructure for the Reaper Agent.
//!
//! This module provides:
//! - Prometheus metrics for policy decisions, latency, and cache performance
//! - OpenTelemetry integration for distributed tracing
//! - Structured logging setup

use lazy_static::lazy_static;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{self as sdktrace, RandomIdGenerator, Sampler},
    Resource,
};
use opentelemetry_semantic_conventions as semconv;
use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Encoder, Gauge,
    HistogramVec, TextEncoder,
};
use reaper_core::VERSION;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// ============================================================================
// Prometheus Metrics Registry
// ============================================================================

lazy_static! {
    /// Total decisions by outcome, policy name, and policy ID.
    pub static ref DECISIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_decisions_total",
        "Total policy decisions made",
        &["decision", "policy_name", "policy_id"]
    )
    .expect("Failed to register DECISIONS_TOTAL metric");

    /// Decision latency histogram (sub-microsecond tracking).
    /// Buckets: 100ns, 500ns, 1µs, 5µs, 10µs, 50µs, 100µs, 500µs, 1ms
    pub static ref DECISION_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_decision_duration_seconds",
        "Policy decision latency in seconds",
        &["policy_name"],
        vec![0.0000001, 0.0000005, 0.000001, 0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001]
    )
    .expect("Failed to register DECISION_DURATION metric");

    /// Total denials (security events).
    pub static ref DENIALS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_denials_total",
        "Total policy denials",
        &["policy_name", "resource", "action"]
    )
    .expect("Failed to register DENIALS_TOTAL metric");

    /// Cache hit counter by cache type.
    pub static ref CACHE_HITS: CounterVec = register_counter_vec!(
        "reaper_cache_hits_total",
        "Cache hits",
        &["cache_type"]
    )
    .expect("Failed to register CACHE_HITS metric");

    /// Cache miss counter by cache type.
    pub static ref CACHE_MISSES: CounterVec = register_counter_vec!(
        "reaper_cache_misses_total",
        "Cache misses",
        &["cache_type"]
    )
    .expect("Failed to register CACHE_MISSES metric");

    /// Number of active policies loaded.
    pub static ref ACTIVE_POLICIES: Gauge = register_gauge!(
        "reaper_active_policies",
        "Number of active policies loaded"
    )
    .expect("Failed to register ACTIVE_POLICIES metric");

    /// Error counter by type.
    pub static ref ERRORS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_errors_total",
        "Total errors during policy evaluation",
        &["error_type"]
    )
    .expect("Failed to register ERRORS_TOTAL metric");

    /// Current number of concurrent evaluations.
    pub static ref CONCURRENT_EVALUATIONS: Gauge = register_gauge!(
        "reaper_concurrent_evaluations",
        "Current number of concurrent policy evaluations"
    )
    .expect("Failed to register CONCURRENT_EVALUATIONS metric");

    /// Total decision log entries recorded.
    pub static ref DECISION_LOG_ENTRIES: Gauge = register_gauge!(
        "reaper_decision_log_entries_total",
        "Total decision log entries recorded"
    )
    .expect("Failed to register DECISION_LOG_ENTRIES metric");

    /// Current decision log buffer size.
    pub static ref DECISION_LOG_BUFFER_SIZE: Gauge = register_gauge!(
        "reaper_decision_log_buffer_size",
        "Current decision log buffer size"
    )
    .expect("Failed to register DECISION_LOG_BUFFER_SIZE metric");

    /// Total decision log file flushes.
    pub static ref DECISION_LOG_FLUSHES: Gauge = register_gauge!(
        "reaper_decision_log_flushes_total",
        "Total decision log file flushes"
    )
    .expect("Failed to register DECISION_LOG_FLUSHES metric");
}

/// Record a policy decision in Prometheus metrics.
pub fn record_decision(decision: &str, policy_name: &str, policy_id: &str, duration_secs: f64) {
    DECISIONS_TOTAL
        .with_label_values(&[decision, policy_name, policy_id])
        .inc();
    DECISION_DURATION
        .with_label_values(&[policy_name])
        .observe(duration_secs);
}

/// Record a denial with resource and action context.
pub fn record_denial(policy_name: &str, resource: &str, action: &str) {
    DENIALS_TOTAL
        .with_label_values(&[policy_name, resource, action])
        .inc();
}

/// Record a cache hit.
pub fn record_cache_hit(cache_type: &str) {
    CACHE_HITS.with_label_values(&[cache_type]).inc();
}

/// Record a cache miss.
pub fn record_cache_miss(cache_type: &str) {
    CACHE_MISSES.with_label_values(&[cache_type]).inc();
}

/// Record an error.
pub fn record_error(error_type: &str) {
    ERRORS_TOTAL.with_label_values(&[error_type]).inc();
}

/// Set the number of active policies.
pub fn set_active_policies(count: usize) {
    ACTIVE_POLICIES.set(count as f64);
}

/// Increment concurrent evaluations.
pub fn inc_concurrent_evaluations() {
    CONCURRENT_EVALUATIONS.inc();
}

/// Decrement concurrent evaluations.
pub fn dec_concurrent_evaluations() {
    CONCURRENT_EVALUATIONS.dec();
}

/// Gather all metrics and encode as Prometheus text format.
pub fn gather_metrics() -> Result<String, String> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| format!("Failed to encode metrics: {}", e))?;
    String::from_utf8(buffer).map_err(|e| format!("Failed to convert metrics to string: {}", e))
}

// ============================================================================
// Observability Initialization
// ============================================================================

/// Initialize the observability stack (logs, traces, metrics).
///
/// Configuration is read from environment variables:
/// - `OTEL_ENABLED`: Enable OpenTelemetry tracing (default: false)
/// - `OTEL_ENDPOINT`: OpenTelemetry collector endpoint (required if OTEL_ENABLED=true)
/// - `REAPER_LOG_FORMAT`: Log format, "json" or "pretty" (default: json)
pub fn init_observability() -> anyhow::Result<()> {
    // Check if OpenTelemetry is enabled
    let otel_enabled = std::env::var("OTEL_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";

    // Determine output format from environment
    let use_json =
        std::env::var("REAPER_LOG_FORMAT").unwrap_or_else(|_| "json".to_string()) == "json";

    // Create async non-blocking writer for high-performance logging
    let (non_blocking, _guard) = tracing_appender::non_blocking(std::io::stdout());

    if otel_enabled {
        init_with_otel(use_json, non_blocking)?;
    } else {
        init_without_otel(use_json, non_blocking);
    }

    // Keep guard alive for the duration of the program
    std::mem::forget(_guard);

    Ok(())
}

/// Initialize logging with OpenTelemetry tracing.
fn init_with_otel(
    use_json: bool,
    non_blocking: tracing_appender::non_blocking::NonBlocking,
) -> anyhow::Result<()> {
    let otel_endpoint = std::env::var("OTEL_ENDPOINT").map_err(|_| {
        anyhow::anyhow!(
            "OTEL_ENABLED=true requires OTEL_ENDPOINT to be set (e.g., http://tempo:4317)"
        )
    })?;

    // Initialize OpenTelemetry tracer with configured endpoint
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(otel_endpoint.clone()),
        )
        .with_trace_config(
            sdktrace::config()
                .with_sampler(Sampler::AlwaysOn)
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(Resource::new(vec![
                    KeyValue::new(semconv::resource::SERVICE_NAME, "reaper-agent"),
                    KeyValue::new(semconv::resource::SERVICE_VERSION, VERSION),
                    KeyValue::new("reaper.component", "policy-engine"),
                ])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    // Build subscriber with telemetry layer
    if use_json {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn,reaper_agent=info".into()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking),
            )
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info,reaper_agent=info".into()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_writer(non_blocking),
            )
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .init();
    }

    info!(
        "OpenTelemetry enabled - exporting traces to {}",
        otel_endpoint
    );

    Ok(())
}

/// Initialize logging without OpenTelemetry.
fn init_without_otel(use_json: bool, non_blocking: tracing_appender::non_blocking::NonBlocking) {
    if use_json {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn,reaper_agent=info".into()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info,reaper_agent=info".into()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_writer(non_blocking),
            )
            .init();
    }

    info!("OpenTelemetry disabled - logs only (set OTEL_ENABLED=true to enable tracing)");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: We can't fully test metric registration since lazy_static runs once.
    // These tests verify the helper functions work correctly.

    #[test]
    fn test_record_decision() {
        // This should not panic
        record_decision("allow", "test-policy", "test-id", 0.000001);
        record_decision("deny", "test-policy", "test-id", 0.000002);
    }

    #[test]
    fn test_record_denial() {
        record_denial("test-policy", "/api/admin", "write");
    }

    #[test]
    fn test_record_cache_operations() {
        record_cache_hit("policy");
        record_cache_hit("decision");
        record_cache_miss("policy");
    }

    #[test]
    fn test_record_error() {
        record_error("evaluation");
        record_error("parse");
    }

    #[test]
    fn test_set_active_policies() {
        set_active_policies(10);
        set_active_policies(5);
    }

    #[test]
    fn test_concurrent_evaluations() {
        inc_concurrent_evaluations();
        inc_concurrent_evaluations();
        dec_concurrent_evaluations();
    }

    #[test]
    fn test_gather_metrics() {
        // Record some metrics first
        record_decision("allow", "gather-test", "id-1", 0.000001);

        let result = gather_metrics();
        assert!(result.is_ok());

        let metrics_text = result.unwrap();
        assert!(metrics_text.contains("reaper_decisions_total"));
    }

    #[test]
    fn test_decision_duration_buckets() {
        // Test that sub-microsecond buckets work
        let durations = [
            0.0000001, // 100ns
            0.0000005, // 500ns
            0.000001,  // 1µs
            0.00001,   // 10µs
        ];

        for duration in durations {
            record_decision("allow", "bucket-test", "id-1", duration);
        }
    }

    #[test]
    fn test_metric_labels() {
        // Test various label combinations
        record_decision("allow", "policy-a", "id-a", 0.000001);
        record_decision("deny", "policy-b", "id-b", 0.000001);
        record_denial("policy-a", "/resource/1", "read");
        record_denial("policy-b", "/resource/2", "write");
    }
}
