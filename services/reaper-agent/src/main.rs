mod bootstrap;
mod cache;
mod management;

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{Json, Response},
    routing::{get, post},
    Router,
};
use cache::PolicyCache;
use lazy_static::lazy_static;
use opentelemetry::{global, trace::TraceContextExt, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{self as sdktrace, RandomIdGenerator, Sampler},
    Resource,
};
use opentelemetry_semantic_conventions as semconv;
use policy_engine::{
    cache_config::CacheConfig, decision_cache::DecisionCache, EnhancedPolicy, PolicyAction,
    PolicyBundle, PolicyEngine, PolicyRequest, PolicyRule,
};
use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Encoder, Gauge,
    HistogramVec, TextEncoder,
};
use clap::Parser;
use reaper_core::{endpoints, config::ReaperAgentConfig, BUILD_INFO, VERSION};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{error, info, instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "reaper-agent")]
#[command(author, version, about = "Reaper Agent - High-performance policy enforcement", long_about = None)]
struct Args {
    /// Path to configuration file (YAML or JSON)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Port to listen on (overrides config file)
    #[arg(short, long)]
    port: Option<u16>,

    /// Address to bind to (overrides config file)
    #[arg(short, long)]
    bind: Option<String>,

    /// Directory containing bootstrap policies
    #[arg(long)]
    bootstrap_policies: Option<PathBuf>,

    /// File containing bootstrap entity data
    #[arg(long)]
    bootstrap_data: Option<PathBuf>,
}

// ============================================================================
// Prometheus Metrics Registry
// ============================================================================

lazy_static! {
    /// Total decisions by outcome, policy, and service
    static ref DECISIONS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_decisions_total",
        "Total policy decisions made",
        &["decision", "policy_name", "policy_id"]
    )
    .unwrap();

    /// Decision latency histogram (sub-microsecond tracking)
    static ref DECISION_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_decision_duration_seconds",
        "Policy decision latency in seconds",
        &["policy_name"],
        // Buckets: 100ns, 500ns, 1µs, 5µs, 10µs, 50µs, 100µs, 500µs, 1ms
        vec![0.0000001, 0.0000005, 0.000001, 0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001]
    )
    .unwrap();

    /// Total denials (security events)
    static ref DENIALS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_denials_total",
        "Total policy denials",
        &["policy_name", "resource", "action"]
    )
    .unwrap();

    /// Cache performance
    static ref CACHE_HITS: CounterVec = register_counter_vec!(
        "reaper_cache_hits_total",
        "Cache hits",
        &["cache_type"]
    )
    .unwrap();

    static ref CACHE_MISSES: CounterVec = register_counter_vec!(
        "reaper_cache_misses_total",
        "Cache misses",
        &["cache_type"]
    )
    .unwrap();

    /// Active policies loaded
    static ref ACTIVE_POLICIES: Gauge = register_gauge!(
        "reaper_active_policies",
        "Number of active policies loaded"
    )
    .unwrap();

    /// Policy evaluation errors
    static ref ERRORS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_errors_total",
        "Total errors during policy evaluation",
        &["error_type"]
    )
    .unwrap();

    /// Concurrent evaluations gauge
    static ref CONCURRENT_EVALUATIONS: Gauge = register_gauge!(
        "reaper_concurrent_evaluations",
        "Current number of concurrent policy evaluations"
    )
    .unwrap();
}

#[derive(Clone)]
struct AgentState {
    policy_engine: PolicyEngine,
    data_store: Arc<policy_engine::DataStore>, // Shared entity store for compiled evaluators
    stats: Arc<AgentStats>,
    decision_cache: Option<Arc<DecisionCache>>, // Optional decision cache (env configurable)
    cache_config: CacheConfig,                  // Cache configuration for logging
    agent_config: ReaperAgentConfig,            // Full agent configuration
    policy_cache: Option<Arc<PolicyCache>>,     // Optional disk cache for policies
}

// Add Debug manually since PolicyEngine has its own Debug implementation now
impl std::fmt::Debug for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentState")
            .field("policy_engine", &self.policy_engine)
            .field("data_store", &"DataStore")
            .field("stats", &"AgentStats")
            .field("decision_cache", &self.decision_cache.as_ref().map(|_| "DecisionCache"))
            .field("cache_config", &self.cache_config)
            .field("agent_config", &self.agent_config)
            .field("policy_cache", &self.policy_cache.as_ref().map(|_| "PolicyCache"))
            .finish()
    }
}

#[derive(Debug, Default)]
struct AgentStats {
    requests_processed: AtomicU64,
    total_evaluation_time_ns: AtomicU64,
    policy_cache_hits: AtomicU64,
    policy_cache_misses: AtomicU64,
    decision_cache_hits: AtomicU64,
    decision_cache_misses: AtomicU64,
}

/// Policy evaluation request from external services
#[derive(Debug, Deserialize)]
struct EvaluateRequest {
    pub policy_id: Option<String>,
    pub policy_name: Option<String>,
    pub principal: String, // Role: admin, manager, engineer, viewer
    pub resource: String,
    pub action: String,
    pub context: Option<HashMap<String, String>>,
}

/// Policy deployment request from platform
#[derive(Debug, Deserialize)]
struct DeployPolicyRequest {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<DeployPolicyRule>,
}

#[derive(Debug, Deserialize)]
struct DeployPolicyRule {
    pub action: String,
    pub resource: String,
    pub conditions: Option<Vec<String>>,
}

/// Bundle deployment request
#[derive(Debug, Deserialize)]
struct DeployBundleRequest {
    pub bundle: Vec<u8>, // Raw .rbb bytes
    pub version: String, // Expected version
    #[serde(default)]
    pub force: bool, // Override version check
}

/// Bundle deployment response
#[derive(Debug, serde::Serialize)]
struct DeployBundleResponse {
    pub policy_id: String,
    pub version: String,
    pub deployed_at: String,
    pub bundle_hash: String, // Hex-encoded SHA-256
}

impl AgentStats {
    fn record_evaluation(&self, evaluation_time_ns: u64) {
        self.requests_processed.fetch_add(1, Ordering::Relaxed);
        self.total_evaluation_time_ns
            .fetch_add(evaluation_time_ns, Ordering::Relaxed);
    }

    fn record_cache_hit(&self) {
        self.policy_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_miss(&self) {
        self.policy_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    fn record_decision_cache_hit(&self) {
        self.decision_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_decision_cache_miss(&self) {
        self.decision_cache_misses.fetch_add(1, Ordering::Relaxed);
    }
}

/// Initialize observability stack (logs, traces, metrics)
fn init_observability() -> anyhow::Result<()> {
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
        // OTEL enabled - require endpoint configuration
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
            // Structured JSON logs with OTEL
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
            // Pretty logs with OTEL
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info,reaper_agent=info".into()),
                )
                .with(
                    tracing_subscriber::fmt::layer()
                        .pretty()
                        .with_writer(non_blocking.clone()),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }

        info!(
            "OpenTelemetry enabled - exporting traces to {}",
            otel_endpoint
        );
    } else {
        // OTEL disabled - simple logging only
        if use_json {
            // Structured JSON logs without OTEL
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
            // Pretty logs without OTEL
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

    // Keep guard alive for the duration of the program
    std::mem::forget(_guard);

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize observability (logs, traces, metrics)
    init_observability()?;

    info!(
        service = "reaper-agent",
        version = VERSION,
        build_info = BUILD_INFO,
        "Starting Reaper Agent - High-Performance Policy Enforcement"
    );

    // Load configuration
    let mut config = if let Some(ref config_path) = args.config {
        info!("Loading configuration from {:?}", config_path);
        match ReaperAgentConfig::from_file_with_env(config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!("Failed to load config file: {}. Using defaults.", e);
                ReaperAgentConfig::from_env()
            }
        }
    } else {
        info!("No config file specified, using defaults with env overrides");
        ReaperAgentConfig::from_env()
    };

    // Apply CLI argument overrides
    if let Some(port) = args.port {
        config.agent.port = port;
    }
    if let Some(ref bind) = args.bind {
        config.agent.bind_address = bind.clone();
    }
    if let Some(ref bootstrap_policies) = args.bootstrap_policies {
        config.policies.bootstrap_dir = Some(bootstrap_policies.clone());
    }
    if let Some(ref bootstrap_data) = args.bootstrap_data {
        config.data.bootstrap_file = Some(bootstrap_data.clone());
    }

    info!("Configuration: {}", config.summary());

    // Initialize PolicyEngine and DataStore
    let policy_engine = PolicyEngine::new();
    let data_store = Arc::new(policy_engine::DataStore::new());

    // Initialize decision cache from config
    let cache_config = CacheConfig::builder()
        .enabled(config.cache.enabled)
        .capacity(config.cache.capacity)
        .ttl_secs(config.cache.ttl_seconds)
        .build();
    let decision_cache = cache_config.build_cache_arc();

    info!(
        "Decision cache: {}",
        cache_config.summary()
    );

    // Load bootstrap data first (needed for policy compilation)
    if config.data.bootstrap_file.is_some() || config.data.bootstrap_dir.is_some() {
        match bootstrap::load_bootstrap_data(
            data_store.clone(),
            config.data.bootstrap_file.clone(),
            config.data.bootstrap_dir.clone(),
        )
        .await
        {
            Ok(result) => {
                if result.entities_loaded > 0 {
                    info!(
                        "Bootstrap data loaded: {} entities from {} files",
                        result.entities_loaded, result.data_files_loaded
                    );
                }
            }
            Err(e) => {
                warn!("Failed to load bootstrap data: {}", e);
            }
        }
    }

    // Load bootstrap policies
    if config.policies.bootstrap_dir.is_some() {
        match bootstrap::load_bootstrap_policies(
            &policy_engine,
            data_store.clone(),
            config.policies.bootstrap_dir.clone(),
        )
        .await
        {
            Ok(result) => {
                if result.policies_loaded > 0 || result.policies_failed > 0 {
                    info!(
                        "Bootstrap policies: {} loaded, {} failed",
                        result.policies_loaded, result.policies_failed
                    );
                }
            }
            Err(e) => {
                warn!("Failed to load bootstrap policies: {}", e);
            }
        }
    }

    // Initialize policy cache if cache directory is configured
    let policy_cache = if let Some(ref cache_dir) = config.policies.cache_dir {
        match PolicyCache::new(cache_dir.clone()) {
            Ok(cache) => {
                info!("Policy cache enabled: {:?}", cache_dir);
                // Load cached policies on startup
                match cache.load_policies().await {
                    Ok(policies) => {
                        for mut policy in policies {
                            // Build evaluator for cached policy using get_evaluator (builds if needed)
                            if let Err(e) = policy.get_evaluator() {
                                warn!("Failed to build evaluator for cached policy {}: {}", policy.name, e);
                                continue;
                            }
                            if let Err(e) = policy_engine.deploy_policy(policy.clone()) {
                                warn!("Failed to deploy cached policy {}: {}", policy.name, e);
                            } else {
                                info!("Restored cached policy: {} ({})", policy.name, policy.id);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load cached policies: {}", e);
                    }
                }
                Some(Arc::new(cache))
            }
            Err(e) => {
                warn!("Failed to create policy cache: {}", e);
                None
            }
        }
    } else {
        info!("Policy cache disabled (no cache_dir configured)");
        None
    };

    // Initialize management client if enabled
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut management_handle = None;

    if config.management.enabled {
        info!("Management plane enabled - connecting to {}",
            config.management.url.as_deref().unwrap_or("?"));

        match management::ManagementClient::new(
            &config.management,
            config.agent.name.clone(),
            VERSION.to_string(),
        ) {
            Ok(client) => {
                let client = Arc::new(client);
                let policy_engine_for_sync = policy_engine.clone();

                // Create sync service
                let (sync_service, mut update_rx) = management::SyncService::new(
                    client.clone(),
                    config.management.clone(),
                    Arc::new(policy_engine_for_sync),
                    shutdown_rx.clone(),
                );

                // Spawn sync service
                let sync_handle = tokio::spawn(async move {
                    sync_service.run().await;
                });
                management_handle = Some(sync_handle);

                // Spawn bundle update handler
                let policy_engine_for_updates = policy_engine.clone();
                let data_store_for_updates = data_store.clone();
                tokio::spawn(async move {
                    while update_rx.changed().await.is_ok() {
                        if let Some(update) = update_rx.borrow().clone() {
                            info!(
                                bundle_id = %update.bundle_id,
                                checksum = %update.checksum,
                                size = update.data.len(),
                                "Received bundle update, deploying..."
                            );

                            // Parse the management bundle (JSON format)
                            match serde_json::from_slice::<management::ManagementBundle>(&update.data) {
                                Ok(bundle) => {
                                    let mut deployed = 0;
                                    let mut failed = 0;

                                    for policy_entry in bundle.policies {
                                        // Create EnhancedPolicy from bundle entry
                                        let policy_id = Uuid::parse_str(&policy_entry.id)
                                            .unwrap_or_else(|_| Uuid::new_v4());

                                        let mut policy = EnhancedPolicy::new(
                                            policy_entry.id.clone(),
                                            format!("Policy from bundle"),
                                            vec![], // Rules will be set by content
                                        );
                                        policy.id = policy_id;
                                        policy.version = policy_entry.version as u64;
                                        policy.content = policy_entry.content.clone();

                                        // Set the language based on what management server provides
                                        policy.language = match policy_entry.language.as_str() {
                                            "cedar" => policy_engine::PolicyLanguage::Cedar,
                                            "simple" => policy_engine::PolicyLanguage::Simple,
                                            "reaper" | _ => policy_engine::PolicyLanguage::Custom,
                                        };

                                        if let Err(e) = policy_engine_for_updates.deploy_policy(policy) {
                                            warn!(
                                                policy = %policy_entry.id,
                                                error = %e,
                                                "Failed to deploy policy from bundle"
                                            );
                                            failed += 1;
                                        } else {
                                            deployed += 1;
                                        }
                                    }

                                    info!(
                                        bundle_id = %update.bundle_id,
                                        deployed = deployed,
                                        failed = failed,
                                        "Bundle deployment complete"
                                    );
                                }
                                Err(e) => {
                                    error!(error = %e, "Failed to parse management bundle");
                                }
                            }
                        }
                    }
                });

                info!("Management sync service started");
            }
            Err(e) => {
                warn!(error = %e, "Failed to create management client, running in standalone mode");
            }
        }
    } else {
        info!("Running in standalone mode (management plane disabled)");
    }

    info!("Reaper Agent initialized - ready to receive policies and data via API");
    info!("  POST /api/v1/data           - Load entity data (JSON)");
    info!("  POST /api/v1/policies/compile - Deploy compiled .reap policy");

    let state = Arc::new(AgentState {
        policy_engine,
        data_store,
        stats: Arc::new(AgentStats::default()),
        decision_cache,
        cache_config,
        agent_config: config.clone(),
        policy_cache,
    });

    let app = Router::new()
        // Health and metrics
        .route(endpoints::HEALTH, get(health_check))
        .route("/ready", get(readiness_check))
        .route("/live", get(liveness_check))
        .route(endpoints::METRICS, get(metrics))
        // Policy evaluation - the core agent functionality
        .route(endpoints::API_V1_MESSAGES, post(evaluate_policy))
        // Fast path with SIMD JSON parsing (3-5x faster parsing)
        .route("/api/v1/fast-messages", post(fast_evaluate_policy))
        // Batch evaluation endpoint (parallel processing)
        .route("/api/v1/batch-messages", post(batch_evaluate_policy))
        // Data management - load entities
        .route("/api/v1/data", post(load_data_handler))
        .route("/api/v1/data/stream", post(load_data_stream_handler))
        .route("/api/v1/data/sync", post(sync_data))
        // Policy management from platform
        .route("/api/v1/policies/deploy", post(deploy_policy))
        .route("/api/v1/policies/compile", post(deploy_compiled_policy))
        .route("/api/v1/policies", get(list_policies))
        .route("/api/v1/policies/{id}/versions", get(get_policy_versions))
        .route("/api/v1/policies/{id}/version", get(get_policy_current_version))
        // Bundle deployment (hot-reload with versioning)
        .route("/api/v1/bundles/deploy", post(deploy_bundle))
        // Entity CRUD operations (requires eBPF integration)
        .route("/api/v1/entities", post(upsert_entity_handler))
        .route("/api/v1/entities/{type}/{id}", get(get_entity_handler))
        .route(
            "/api/v1/entities/{type}/{id}",
            axum::routing::delete(delete_entity_handler),
        )
        .route("/api/v1/entities/{type}", get(list_entities_handler))
        .route("/api/v1/entities/batch", post(batch_upsert_handler))
        // Debug endpoints
        .route("/debug/datastore", get(debug_datastore))
        .layer(axum::extract::DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit for large datasets
        .with_state(state);

    let bind_addr = format!("{}:{}", config.agent.bind_address, config.agent.port);
    let listener = TcpListener::bind(&bind_addr).await?;
    info!(bind_addr = %bind_addr, "Reaper Agent listening");
    info!("");
    info!("⚡ Policy Evaluation API:");
    info!("  POST /api/v1/messages        - Evaluate policy decision");
    info!("  POST /api/v1/policies/deploy - Deploy policy from platform");
    info!("  GET  /api/v1/policies        - List active policies");
    info!("  GET  /metrics                 - Prometheus metrics");
    info!("  GET  /health                  - Health check");
    info!("");
    info!("📊 Observability:");
    info!("  Logs: Structured JSON (Loki-compatible)");
    info!("  Traces: OpenTelemetry → Tempo");
    info!("  Metrics: Prometheus format");
    info!("");
    info!("🚀 Ready for sub-microsecond policy enforcement!");

    // Run server
    let result = axum::serve(listener, app).await;

    // Signal shutdown to sync service
    let _ = shutdown_tx.send(true);
    if let Some(handle) = management_handle {
        info!("Waiting for management sync service to shutdown...");
        let _ = handle.await;
    }

    // Shutdown telemetry gracefully
    info!("Shutting down telemetry...");
    global::shutdown_tracer_provider();

    result?;
    Ok(())
}

#[instrument]
async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-agent",
        "version": VERSION,
        "capabilities": [
            "policy-evaluation",
            "hot-swapping",
            "sub-microsecond-latency"
        ]
    })))
}

#[instrument]
async fn readiness_check(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    // Check if policy engine has at least one policy loaded
    let engine_stats = state.policy_engine.get_stats();

    if engine_stats.total_policies == 0 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(json!({
        "status": "ready",
        "policies_loaded": engine_stats.total_policies,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

#[instrument]
async fn liveness_check() -> StatusCode {
    // Simple liveness - if we can respond, we're alive
    StatusCode::OK
}

#[instrument]
async fn metrics(State(state): State<Arc<AgentState>>) -> Result<Response, StatusCode> {
    // Update active policies gauge
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    // Encode metrics to Prometheus text format
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();

    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", encoder.format_type())
        .body(buffer.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

#[instrument(
    skip(state, payload),
    fields(
        resource = %payload.resource,
        action = %payload.action,
        policy_name = tracing::field::Empty,
        decision = tracing::field::Empty,
        latency_ns = tracing::field::Empty,
    )
)]
async fn evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<EvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Get current OpenTelemetry span for rich context
    let span = tracing::Span::current();
    let cx = span.context();
    let otel_span = cx.span();
    let span_context = otel_span.span_context();

    // Extract trace ID for logging correlation
    let trace_id = if span_context.is_valid() {
        format!("{:032x}", span_context.trace_id())
    } else {
        "none".to_string()
    };

    // Determine which policy/policies to evaluate
    // Can specify: UUID, policy name, or nothing (evaluate all)
    let policy_ids: Vec<Uuid> = if let Some(id_str) = payload.policy_id {
        // Try to parse as UUID first
        match Uuid::from_str(&id_str) {
            Ok(id) => vec![id],
            Err(_) => {
                // Not a valid UUID - treat as policy name
                match state.policy_engine.get_policy_by_name(&id_str) {
                    Some(policy) => {
                        state.stats.record_cache_hit();
                        CACHE_HITS.with_label_values(&["policy"]).inc();
                        vec![policy.id]
                    }
                    None => {
                        // Policy not found - DENY by default for security
                        ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                        return Ok(Json(json!({
                            "decision": "deny",
                            "policy_id": id_str,
                            "policy_version": 0,
                            "evaluation_time_microseconds": 0.0,
                            "total_time_microseconds": 0.0,
                            "matched_rule": "policy_not_found",
                            "agent_id": "reaper-agent-001"
                        })));
                    }
                }
            }
        }
    } else if let Some(ref name) = payload.policy_name {
        // Look up policy by name
        match state.policy_engine.get_policy_by_name(name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                CACHE_HITS.with_label_values(&["policy"]).inc();
                vec![policy.id]
            }
            None => {
                // Policy not found - DENY by default for security
                state.stats.record_cache_miss();
                CACHE_MISSES.with_label_values(&["policy"]).inc();
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                return Ok(Json(json!({
                    "decision": "deny",
                    "policy_name": name,
                    "policy_version": 0,
                    "evaluation_time_microseconds": 0.0,
                    "total_time_microseconds": 0.0,
                    "matched_rule": "policy_not_found",
                    "agent_id": "reaper-agent-001"
                })));
            }
        }
    } else {
        // No policy specified - evaluate ALL policies (if any deny, return deny)
        let all_policies = state.policy_engine.list_policies();
        if all_policies.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            return Ok(Json(json!({
                "decision": "deny",
                "policy_version": 0,
                "evaluation_time_microseconds": 0.0,
                "total_time_microseconds": 0.0,
                "matched_rule": "no_policies_loaded",
                "agent_id": "reaper-agent-001"
            })));
        }
        all_policies.into_iter().map(|p| p.id).collect()
    };

    // Create policy request
    // The compiled evaluator looks up user entities by ID in the DataStore
    // Use the principal as-is (it's already an entity ID like "user_admin")
    let mut context = payload.context.unwrap_or_default();
    context.insert("principal".to_string(), payload.principal.clone());

    let request = PolicyRequest {
        resource: payload.resource.clone(),
        action: payload.action.clone(),
        context,
    };

    // Check decision cache first (if enabled)
    if let Some(ref cache) = state.decision_cache {
        if let Some(cached_decision) = cache.get(&request) {
            // Cache hit - return cached decision immediately
            state.stats.record_decision_cache_hit();
            CACHE_HITS.with_label_values(&["decision"]).inc();

            let total_time = start_time.elapsed();
            let decision_str = match cached_decision {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            };

            return Ok(Json(json!({
                "decision": decision_str,
                "policy_id": "cached",
                "policy_version": 0,
                "evaluation_time_microseconds": 0.0,
                "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
                "matched_rule": "cached_decision",
                "agent_id": "reaper-agent-001",
                "cache_hit": true
            })));
        }
        state.stats.record_decision_cache_miss();
        CACHE_MISSES.with_label_values(&["decision"]).inc();
    }

    // Evaluate all policies in policy_ids (may be 1 or many)
    // If ANY policy denies, return deny (security first)
    let mut final_decision = PolicyAction::Allow;
    let mut total_eval_time_ns = 0u64;
    let mut matched_policy_id = Uuid::nil();
    let mut matched_policy_name = String::from("unknown");
    let mut matched_policy_version = 0u64;
    let mut matched_rule = String::from("default_allow");

    for policy_id in &policy_ids {
        match state.policy_engine.evaluate(policy_id, &request) {
            Ok(decision) => {
                total_eval_time_ns += decision.evaluation_time_ns;

                // If this policy denies, override the final decision
                if matches!(decision.decision, PolicyAction::Deny) {
                    final_decision = PolicyAction::Deny;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision
                        .matched_rule
                        .map(|idx| format!("rule_{}", idx))
                        .unwrap_or_else(|| "no_rule".to_string());

                    // Get policy name for this denial
                    if let Some(policy) = state.policy_engine.get_policy(policy_id) {
                        matched_policy_name = policy.name.clone();
                    }

                    // Break early on deny (security first - no need to check other policies)
                    break;
                } else if matches!(final_decision, PolicyAction::Allow) {
                    // Only update if we haven't seen a deny yet
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision
                        .matched_rule
                        .map(|idx| format!("rule_{}", idx))
                        .unwrap_or_else(|| "no_rule".to_string());

                    if let Some(policy) = state.policy_engine.get_policy(policy_id) {
                        matched_policy_name = policy.name.clone();
                    }
                }
            }
            Err(e) => {
                // On error, deny for security (fail closed)
                error!("Policy evaluation error for {}: {}", policy_id, e);
                ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
                final_decision = PolicyAction::Deny;
                matched_rule = format!("evaluation_error: {}", e);
                break;
            }
        }
    }

    let total_time = start_time.elapsed();
    state.stats.record_evaluation(total_eval_time_ns);

    let decision_str = match final_decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };

    // Record Prometheus metrics
    DECISIONS_TOTAL
        .with_label_values(&[
            decision_str,
            &matched_policy_name,
            &matched_policy_id.to_string(),
        ])
        .inc();

    // Record latency (convert ns to seconds for Prometheus)
    let latency_seconds = total_eval_time_ns as f64 / 1_000_000_000.0;
    DECISION_DURATION
        .with_label_values(&[&matched_policy_name])
        .observe(latency_seconds);

    // Record span attributes for distributed tracing
    span.record("policy_name", matched_policy_name.as_str());
    span.record("decision", decision_str);
    span.record("latency_ns", total_eval_time_ns);

    // Add OpenTelemetry span attributes
    otel_span.set_attribute(KeyValue::new(
        "reaper.policy.name",
        matched_policy_name.clone(),
    ));
    otel_span.set_attribute(KeyValue::new(
        "reaper.policy.id",
        matched_policy_id.to_string(),
    ));
    otel_span.set_attribute(KeyValue::new("reaper.decision", decision_str));
    otel_span.set_attribute(KeyValue::new(
        "reaper.latency_ns",
        total_eval_time_ns as i64,
    ));
    otel_span.set_attribute(KeyValue::new("reaper.resource", payload.resource.clone()));
    otel_span.set_attribute(KeyValue::new("reaper.action", payload.action.clone()));

    // Log all decisions asynchronously (non-blocking)
    if decision_str == "deny" {
        DENIALS_TOTAL
            .with_label_values(&[&matched_policy_name, &payload.resource, &payload.action])
            .inc();

        // Structured log for denial (security event)
        warn!(
            trace_id = %trace_id,
            decision_id = %format!("dec_{}", uuid::Uuid::new_v4().simple()),
            policy_name = %matched_policy_name,
            policy_id = %matched_policy_id,
            resource = %payload.resource,
            action = %payload.action,
            decision = "deny",
            latency_ns = total_eval_time_ns,
            latency_us = total_eval_time_ns as f64 / 1000.0,
            "Policy decision: DENY"
        );
    } else {
        // Log allow decisions at INFO level (async, non-blocking)
        info!(
            trace_id = %trace_id,
            decision_id = %format!("dec_{}", uuid::Uuid::new_v4().simple()),
            policy_name = %matched_policy_name,
            policy_id = %matched_policy_id,
            resource = %payload.resource,
            action = %payload.action,
            decision = decision_str,
            latency_ns = total_eval_time_ns,
            latency_us = total_eval_time_ns as f64 / 1000.0,
            "Policy decision: ALLOW"
        );
    }

    // Cache the decision for future requests (if caching enabled)
    if let Some(ref cache) = state.decision_cache {
        cache.insert(&request, final_decision.clone());
    }

    Ok(Json(json!({
        "decision": decision_str,
        "policy_id": matched_policy_id.to_string(),
        "policy_version": matched_policy_version,
        "evaluation_time_microseconds": total_eval_time_ns as f64 / 1000.0,
        "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
        "matched_rule": matched_rule,
        "agent_id": "reaper-agent-001",
        "cache_hit": false
    })))
}

/// Fast policy evaluation using SIMD-accelerated JSON parsing (sonic-rs)
///
/// This endpoint provides 3-5x faster JSON parsing compared to the standard endpoint.
/// Use this for latency-critical paths where every microsecond counts.
///
/// Performance characteristics:
/// - JSON parsing: ~2-3µs (vs ~8-10µs with serde_json)
/// - Total request overhead: ~5-10µs less than standard endpoint
#[instrument(skip(state, body))]
async fn fast_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Parse JSON with SIMD-accelerated sonic-rs
    let value: sonic_rs::Value = match sonic_rs::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            ERRORS_TOTAL.with_label_values(&["parse_error"]).inc();
            return Ok(Json(json!({
                "error": format!("JSON parse error: {}", e),
                "decision": "deny"
            })));
        }
    };

    // Extract fields efficiently
    let principal = value.get("principal").and_then(|v| v.as_str()).unwrap_or("");
    let resource = value.get("resource").and_then(|v| v.as_str()).unwrap_or("");
    let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let policy_id_opt = value.get("policy_id").and_then(|v| v.as_str());
    let policy_name_opt = value.get("policy_name").and_then(|v| v.as_str());

    // Build context from JSON
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    if let Some(ctx) = value.get("context") {
        if let Some(obj) = ctx.as_object() {
            for (k, v) in obj.iter() {
                if let Some(s) = v.as_str() {
                    context.insert(k.to_string(), s.to_string());
                } else if v.is_i64() {
                    context.insert(k.to_string(), v.as_i64().unwrap().to_string());
                } else if let Some(b) = v.as_bool() {
                    context.insert(k.to_string(), b.to_string());
                }
            }
        }
    }

    // Determine policy to evaluate
    let policy_ids: Vec<Uuid> = if let Some(id_str) = policy_id_opt {
        match Uuid::from_str(id_str) {
            Ok(id) => vec![id],
            Err(_) => {
                // Try as policy name
                match state.policy_engine.get_policy_by_name(id_str) {
                    Some(policy) => {
                        state.stats.record_cache_hit();
                        CACHE_HITS.with_label_values(&["policy"]).inc();
                        vec![policy.id]
                    }
                    None => {
                        ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                        return Ok(Json(json!({
                            "decision": "deny",
                            "error": "policy_not_found"
                        })));
                    }
                }
            }
        }
    } else if let Some(name) = policy_name_opt {
        match state.policy_engine.get_policy_by_name(name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                CACHE_HITS.with_label_values(&["policy"]).inc();
                vec![policy.id]
            }
            None => {
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                return Ok(Json(json!({
                    "decision": "deny",
                    "error": "policy_not_found"
                })));
            }
        }
    } else {
        // Evaluate all policies
        state.policy_engine.list_policies().iter().map(|p| p.id).collect()
    };

    // Build request
    let request = PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
    };

    // Evaluate policies
    let mut final_decision = PolicyAction::Deny;
    let mut matched_policy_id = Uuid::nil();
    let mut matched_policy_version = 0u64;
    let mut matched_rule: Option<usize> = None;
    let mut total_eval_time_ns = 0u64;

    for policy_id in &policy_ids {
        let eval_start = std::time::Instant::now();

        match state.policy_engine.evaluate(policy_id, &request) {
            Ok(decision) => {
                let eval_time_ns = eval_start.elapsed().as_nanos() as u64;
                total_eval_time_ns += eval_time_ns;

                if decision.decision == PolicyAction::Allow {
                    final_decision = PolicyAction::Allow;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision.matched_rule;
                    break;
                } else if decision.decision == PolicyAction::Deny {
                    final_decision = PolicyAction::Deny;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision.matched_rule;
                }
            }
            Err(_) => {
                ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
            }
        }
    }

    let total_time = start_time.elapsed();
    let decision_str = match final_decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };

    // Record metrics
    if let Some(policy) = state.policy_engine.get_policy(&matched_policy_id) {
        DECISIONS_TOTAL
            .with_label_values(&[decision_str, &policy.name, &matched_policy_id.to_string()])
            .inc();
        DECISION_DURATION
            .with_label_values(&[&policy.name])
            .observe(total_time.as_secs_f64());
    }

    Ok(Json(json!({
        "decision": decision_str,
        "policy_id": matched_policy_id.to_string(),
        "policy_version": matched_policy_version,
        "evaluation_time_microseconds": total_eval_time_ns as f64 / 1000.0,
        "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
        "matched_rule": matched_rule,
        "agent_id": "reaper-agent-001",
        "fast_path": true
    })))
}

/// Batch request structure
#[derive(Debug, Deserialize)]
struct BatchEvaluateRequest {
    pub policy_name: Option<String>,
    pub requests: Vec<BatchRequestItem>,
}

#[derive(Debug, Deserialize)]
struct BatchRequestItem {
    pub principal: String,
    pub resource: String,
    pub action: String,
    pub context: Option<HashMap<String, String>>,
}

/// Batch policy evaluation using parallel processing
///
/// This endpoint evaluates multiple policy requests in parallel using rayon.
/// Ideal for bulk authorization checks where latency is less critical than throughput.
///
/// Performance characteristics:
/// - Parallel evaluation across all CPU cores
/// - Optional decision cache integration
/// - Returns results in same order as input requests
#[instrument(skip(state, payload))]
async fn batch_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<BatchEvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    let start_time = std::time::Instant::now();

    // Find the policy to evaluate
    let policy = if let Some(ref name) = payload.policy_name {
        match state.policy_engine.get_policy_by_name(name) {
            Some(p) => p,
            None => {
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                return Ok(Json(json!({
                    "error": "Policy not found",
                    "policy_name": name
                })));
            }
        }
    } else {
        // Use first policy if none specified
        let policies = state.policy_engine.list_policies();
        if policies.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            return Ok(Json(json!({
                "error": "No policies loaded"
            })));
        }
        policies.into_iter().next().unwrap()
    };

    // Convert batch requests to PolicyRequests
    let requests: Vec<PolicyRequest> = payload
        .requests
        .iter()
        .map(|r| {
            let mut context = r.context.clone().unwrap_or_default();
            context.insert("principal".to_string(), r.principal.clone());
            PolicyRequest {
                resource: r.resource.clone(),
                action: r.action.clone(),
                context,
            }
        })
        .collect();

    let request_count = requests.len();

    // Evaluate requests with decision cache support
    let results: Vec<Value> = requests
        .iter()
        .enumerate()
        .map(|(i, req)| {
            let eval_start = std::time::Instant::now();

            // Check decision cache first
            let (decision, cache_hit) = if let Some(ref cache) = state.decision_cache {
                if let Some(cached) = cache.get(req) {
                    state.stats.record_decision_cache_hit();
                    CACHE_HITS.with_label_values(&["decision"]).inc();
                    (cached, true)
                } else {
                    state.stats.record_decision_cache_miss();
                    CACHE_MISSES.with_label_values(&["decision"]).inc();

                    // Evaluate and cache
                    let result = state.policy_engine.evaluate(&policy.id, req);
                    let decision = match result {
                        Ok(d) => d.decision,
                        Err(_) => PolicyAction::Deny,
                    };
                    cache.insert(req, decision.clone());
                    (decision, false)
                }
            } else {
                // No cache - evaluate directly
                let result = state.policy_engine.evaluate(&policy.id, req);
                let decision = match result {
                    Ok(d) => d.decision,
                    Err(_) => PolicyAction::Deny,
                };
                (decision, false)
            };

            let duration = eval_start.elapsed();
            let decision_str = match decision {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            };

            // Record metrics
            DECISIONS_TOTAL
                .with_label_values(&[decision_str, &policy.name, &policy.id.to_string()])
                .inc();

            json!({
                "index": i,
                "decision": decision_str,
                "evaluation_time_microseconds": duration.as_nanos() as f64 / 1000.0,
                "cache_hit": cache_hit
            })
        })
        .collect();

    let total_time = start_time.elapsed();

    // Count decisions
    let allow_count = results
        .iter()
        .filter(|r| r.get("decision").and_then(|d| d.as_str()) == Some("allow"))
        .count();
    let deny_count = results.len() - allow_count;

    info!(
        policy_name = %policy.name,
        request_count = request_count,
        allow_count = allow_count,
        deny_count = deny_count,
        total_time_ms = total_time.as_millis(),
        "Batch evaluation completed"
    );

    Ok(Json(json!({
        "policy_name": policy.name,
        "policy_id": policy.id.to_string(),
        "request_count": request_count,
        "results": results,
        "summary": {
            "allowed": allow_count,
            "denied": deny_count,
            "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
            "avg_time_microseconds": total_time.as_nanos() as f64 / 1000.0 / request_count as f64
        },
        "agent_id": "reaper-agent-001"
    })))
}

#[instrument(skip(state, payload))]
async fn deploy_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployPolicyRequest>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&payload.policy_id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format"
            })))
        }
    };

    // Convert rules
    let rules: Result<Vec<PolicyRule>, String> = payload
        .rules
        .into_iter()
        .map(|rule| {
            let action = match rule.action.as_str() {
                "allow" => Ok(PolicyAction::Allow),
                "deny" => Ok(PolicyAction::Deny),
                "log" => Ok(PolicyAction::Log),
                _ => Err(format!("Invalid action: {}", rule.action)),
            }?;

            Ok(PolicyRule {
                action,
                resource: rule.resource,
                conditions: rule.conditions.unwrap_or_default(),
            })
        })
        .collect();

    let rules = match rules {
        Ok(rules) => rules,
        Err(e) => {
            return Ok(Json(json!({
                "error": e
            })))
        }
    };

    // Create policy with the specified ID
    let mut policy = EnhancedPolicy::new(payload.name, payload.description, rules);

    // Override the generated ID with the one from the request
    policy.id = policy_id;

    // Hot-swap deploy the policy
    match state.policy_engine.deploy_policy(policy.clone()) {
        Ok(()) => {
            // Update active policies gauge
            let engine_stats = state.policy_engine.get_stats();
            ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

            // Save to policy cache if enabled
            if let Some(ref cache) = state.policy_cache {
                if let Err(e) = cache.save_policy(&policy).await {
                    warn!("Failed to cache policy {}: {}", policy_id, e);
                }
            }

            info!("Policy {} hot-swapped successfully", policy_id);
            Ok(Json(json!({
                "status": "deployed",
                "policy_id": policy.id.to_string(),
                "policy_name": policy.name,
                "version": policy.version,
                "deployment_time": chrono::Utc::now(),
                "message": "Policy hot-swapped successfully with zero downtime"
            })))
        }
        Err(e) => {
            ERRORS_TOTAL
                .with_label_values(&["policy_deployment_failed"])
                .inc();
            error!("Failed to deploy policy: {}", e);
            Ok(Json(json!({
                "error": format!("Failed to deploy policy: {}", e)
            })))
        }
    }
}

#[instrument(skip(state))]
async fn list_policies(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    let policies = state.policy_engine.list_policies();

    let policy_list: Vec<Value> = policies
        .into_iter()
        .map(|policy| {
            json!({
                "id": policy.id.to_string(),
                "name": policy.name,
                "version": policy.version,
                "rules_count": policy.rules.len(),
                "created_at": policy.created_at,
                "updated_at": policy.updated_at
            })
        })
        .collect();

    Ok(Json(json!({
        "policies": policy_list,
        "total": policy_list.len()
    })))
}

/// Get version history for a policy
#[instrument(skip(state))]
async fn get_policy_versions(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path(policy_id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let policy_uuid = Uuid::from_str(&policy_id).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid policy ID: {}", e),
        )
    })?;

    let versions = state.policy_engine.list_versions(&policy_uuid);

    let version_list: Vec<Value> = versions
        .into_iter()
        .map(|v| {
            json!({
                "version": v.version,
                "deployed_at": chrono::DateTime::<chrono::Utc>::from(v.deployed_at).to_rfc3339(),
                "bundle_hash": v.bundle_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(),
                "policy_id": v.policy_id
            })
        })
        .collect();

    Ok(Json(json!({
        "policy_id": policy_id,
        "versions": version_list,
        "total": version_list.len()
    })))
}

/// Get current version of a policy
#[instrument(skip(state))]
async fn get_policy_current_version(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path(policy_id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let policy_uuid = Uuid::from_str(&policy_id).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid policy ID: {}", e),
        )
    })?;

    let version = state.policy_engine.get_version(&policy_uuid).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("No version found for policy {}", policy_id),
        )
    })?;

    Ok(Json(json!({
        "policy_id": policy_id,
        "version": version.version,
        "deployed_at": chrono::DateTime::<chrono::Utc>::from(version.deployed_at).to_rfc3339(),
        "bundle_hash": version.bundle_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    })))
}

/// Load entity data (JSON) into the agent's DataStore
#[derive(Debug, Deserialize)]
struct LoadDataRequest {
    pub data: String, // Raw JSON string with entities
}

#[instrument(skip(state, payload))]
async fn load_data_handler(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<LoadDataRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!("Loading entity data into DataStore");

    use policy_engine::DataLoader;

    // DataStore uses Arc internally, so cloning is cheap and shares data
    let loader = DataLoader::new((*state.data_store).clone());
    let entity_count = loader.load_json(&payload.data).map_err(|e| {
        error!("Failed to load entity data: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to load entity data: {}", e),
        )
    })?;

    info!("✓ Loaded {} entities into DataStore", entity_count);

    Ok(Json(json!({
        "status": "success",
        "entities_loaded": entity_count,
        "message": format!("Loaded {} entities successfully", entity_count)
    })))
}

/// Load entity data using streaming for memory efficiency
/// Accepts file content as raw bytes in request body
#[instrument(skip(state, body))]
async fn load_data_stream_handler(
    State(state): State<Arc<AgentState>>,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!("Loading entity data using streaming (memory-efficient)");

    use policy_engine::{DataLoader, StreamingLoader};
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Write incoming data to temp file
    let mut temp_file = NamedTempFile::new().map_err(|e| {
        error!("Failed to create temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create temp file: {}", e),
        )
    })?;

    temp_file.write_all(&body).map_err(|e| {
        error!("Failed to write to temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write to temp file: {}", e),
        )
    })?;

    temp_file.flush().map_err(|e| {
        error!("Failed to flush temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to flush temp file: {}", e),
        )
    })?;

    let temp_path = temp_file.path();

    // Use streaming loader with 10K chunk size
    let loader = DataLoader::new((*state.data_store).clone());
    let streaming_loader = StreamingLoader::new(loader, 10_000);

    let stats = streaming_loader.stream_and_load(temp_path).map_err(|e| {
        error!("Failed to stream entity data: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to stream entity data: {}", e),
        )
    })?;

    info!(
        "✓ Streamed {} entities in {} chunks ({:.2}s)",
        stats.total,
        stats.chunks_processed,
        stats.duration.as_secs_f64()
    );

    Ok(Json(json!({
        "status": "success",
        "entities_loaded": stats.total,
        "chunks_processed": stats.chunks_processed,
        "duration_ms": stats.duration.as_millis(),
        "message": format!("Streamed {} entities in {} chunks", stats.total, stats.chunks_processed)
    })))
}

// ============================================================================
// Data Synchronization Endpoint (Phase 1.3)
// ============================================================================

/// Request to synchronize entity data from a management server
#[derive(Debug, Deserialize)]
struct SyncDataRequest {
    /// List of entities to sync
    pub entities: Vec<SyncEntityData>,
    /// If true, clear all existing entities before inserting
    #[serde(default)]
    pub replace_all: bool,
    /// Optional source metadata for tracking
    pub source: Option<SyncSource>,
}

/// Entity data for sync endpoint
#[derive(Debug, Deserialize)]
struct SyncEntityData {
    /// Unique entity identifier
    pub id: String,
    /// Entity type (e.g., "User", "Resource", "Group")
    pub entity_type: String,
    /// Entity attributes as key-value pairs
    pub attributes: serde_json::Map<String, serde_json::Value>,
    /// Optional parent entity ID (for hierarchies)
    pub parent: Option<String>,
}

/// Source information for tracking where the sync came from
#[derive(Debug, Deserialize)]
struct SyncSource {
    /// Source type: "sync-client", "api", "file"
    #[serde(rename = "type")]
    pub source_type: String,
    /// Server URL if from sync client
    pub server_url: Option<String>,
    /// Server version for tracking
    pub server_version: Option<String>,
    /// Team/namespace if applicable
    pub team: Option<String>,
}

/// Response from sync operation
#[derive(Debug, serde::Serialize)]
struct SyncDataResponse {
    pub status: String,
    pub inserted: usize,
    pub failed: usize,
    pub replaced: bool,
    pub total_entities: usize,
}

/// Synchronize entity data from external source (sync client or management server)
///
/// POST /api/v1/data/sync
///
/// This endpoint supports bulk entity synchronization with optional replace-all semantics.
#[instrument(skip(state, payload))]
async fn sync_data(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<SyncDataRequest>,
) -> Result<Json<SyncDataResponse>, (StatusCode, String)> {
    let source_info = payload.source.as_ref().map(|s| format!("{} ({})", s.source_type, s.server_url.as_deref().unwrap_or("local"))).unwrap_or_else(|| "api".to_string());

    info!(
        "Syncing entity data: {} entities, replace_all={}, source={}",
        payload.entities.len(),
        payload.replace_all,
        source_info
    );

    // Clear existing data if replace_all is true
    if payload.replace_all {
        info!("Clearing existing entity data (replace_all=true)");
        state.data_store.clear();
    }

    // Get the string interner from the data store
    let interner = state.data_store.interner();

    // Convert and insert entities
    let mut inserted = 0;
    let mut failed = 0;

    for entity_data in &payload.entities {
        match convert_sync_entity(entity_data, interner) {
            Ok(entity) => {
                state.data_store.insert(entity);
                inserted += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to convert entity {}/{}: {}",
                    entity_data.entity_type, entity_data.id, e
                );
                failed += 1;
            }
        }
    }

    let total = state.data_store.all().len();

    info!(
        "✓ Sync complete: inserted={}, failed={}, total_entities={}",
        inserted, failed, total
    );

    Ok(Json(SyncDataResponse {
        status: if failed == 0 { "success".to_string() } else { "partial".to_string() },
        inserted,
        failed,
        replaced: payload.replace_all,
        total_entities: total,
    }))
}

/// Convert JSON entity data to the internal Entity format
fn convert_sync_entity(
    data: &SyncEntityData,
    interner: &policy_engine::StringInterner,
) -> Result<policy_engine::Entity, String> {
    use policy_engine::EntityBuilder;

    // Intern the entity ID and type
    let entity_id = interner.intern(&data.id);
    let entity_type = interner.intern(&data.entity_type);

    // Build entity using EntityBuilder
    let mut builder = EntityBuilder::new(entity_id, entity_type);

    // Convert attributes from JSON to AttributeValue
    for (key, value) in &data.attributes {
        let attr_key = interner.intern(key);
        let attr_value = json_to_attribute_value(value, interner)?;
        builder = builder.with_attribute(attr_key, attr_value);
    }

    // Add parent if specified
    if let Some(parent_id) = &data.parent {
        let parent_interned = interner.intern(parent_id);
        builder = builder.with_parent(parent_interned);
    }

    Ok(builder.build())
}

/// Convert a JSON value to an AttributeValue
fn json_to_attribute_value(
    value: &serde_json::Value,
    interner: &policy_engine::StringInterner,
) -> Result<policy_engine::AttributeValue, String> {
    use policy_engine::AttributeValue;

    match value {
        serde_json::Value::Null => Ok(AttributeValue::Null),
        serde_json::Value::Bool(b) => Ok(AttributeValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(AttributeValue::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(AttributeValue::Float(f))
            } else {
                Err(format!("Unsupported number format: {}", n))
            }
        }
        serde_json::Value::String(s) => {
            Ok(AttributeValue::String(interner.intern(s)))
        }
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr
                .iter()
                .map(|v| json_to_attribute_value(v, interner))
                .collect();
            Ok(AttributeValue::List(items?))
        }
        serde_json::Value::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                let key = interner.intern(k);
                let val = json_to_attribute_value(v, interner)?;
                map.insert(key, val);
            }
            Ok(AttributeValue::Object(map))
        }
    }
}

/// Deploy and compile a .reap policy file with the agent's DataStore
#[derive(Debug, Deserialize)]
struct DeployCompiledPolicyRequest {
    pub policy_content: String, // Raw .reap policy content
    pub policy_name: String,
}

#[instrument(skip(state, payload))]
async fn deploy_compiled_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployCompiledPolicyRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!(
        "Deploying and compiling .reap policy: {}",
        payload.policy_name
    );

    use policy_engine::ReaperPolicy;
    use std::str::FromStr;

    // Parse the .reap policy content
    let policy = ReaperPolicy::from_str(&payload.policy_content).map_err(|e| {
        error!("Failed to parse .reap policy: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to parse .reap policy: {}", e),
        )
    })?;

    // Compile with the agent's DataStore
    let evaluator = policy.build(state.data_store.clone()).map_err(|e| {
        error!("Failed to compile policy: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to compile policy: {}", e),
        )
    })?;

    info!("✓ Policy compiled successfully");

    // Create EnhancedPolicy with the compiled evaluator
    let mut enhanced_policy = EnhancedPolicy {
        id: uuid::Uuid::new_v4(),
        version: 1,
        name: payload.policy_name.clone(),
        description: "Compiled .reap policy".to_string(),
        language: policy_engine::PolicyLanguage::Custom,
        content: payload.policy_content.clone(),
        rules: vec![],
        metadata: std::collections::HashMap::new(),
        priority: 100,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        evaluator: Some(Arc::new(evaluator)),
        source_metadata: None,
    };

    // Set API source metadata
    enhanced_policy.set_api_source(None, Some("platform".to_string()));

    let policy_id = enhanced_policy.id;

    // Deploy to PolicyEngine
    state
        .policy_engine
        .deploy_policy(enhanced_policy.clone())
        .map_err(|e| {
            error!("Failed to deploy policy: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to deploy policy: {}", e),
            )
        })?;

    // Update metrics
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    // Save to policy cache if enabled
    if let Some(ref cache) = state.policy_cache {
        if let Err(e) = cache.save_policy(&enhanced_policy).await {
            warn!("Failed to cache compiled policy {}: {}", policy_id, e);
        }
    }

    info!("✓ Policy deployed successfully: {}", policy_id);

    Ok(Json(json!({
        "status": "deployed",
        "policy_id": policy_id.to_string(),
        "policy_name": payload.policy_name,
        "version": 1,
        "deployment_time": chrono::Utc::now(),
        "message": "Policy compiled and deployed successfully"
    })))
}

/// Deploy a policy bundle (.rbb file) with version tracking
///
/// This endpoint deploys bundles using the full ReaperDSL compiler,
/// preserving all complex conditions, functions, and rule logic.
#[instrument(skip(state, payload))]
async fn deploy_bundle(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployBundleRequest>,
) -> Result<Json<DeployBundleResponse>, (StatusCode, String)> {
    info!(
        "Received bundle deployment request (version: {}, force: {})",
        payload.version, payload.force
    );

    // 1. Parse .rbb bundle
    let bundle = PolicyBundle::from_bytes(&payload.bundle).map_err(|e| {
        ERRORS_TOTAL.with_label_values(&["invalid_bundle"]).inc();
        error!("Failed to parse bundle: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid bundle format: {}", e),
        )
    })?;

    info!(
        "Bundle parsed successfully: {} (version: {}, rules: {})",
        bundle.metadata.policy_name,
        bundle
            .metadata
            .policy_version
            .as_deref()
            .unwrap_or("unknown"),
        bundle.policy.rules.len()
    );

    // 2. Deploy to PolicyEngine with compiled evaluator using the agent's DataStore
    let policy_version = state
        .policy_engine
        .deploy_bundle_with_store(bundle, state.data_store.clone(), payload.force)
        .map_err(|e| {
            ERRORS_TOTAL
                .with_label_values(&["bundle_deployment_failed"])
                .inc();
            error!("Failed to deploy bundle: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Bundle deployment failed: {}", e),
            )
        })?;

    // 3. Update metrics
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    info!(
        "Bundle deployed successfully: policy_id={}, version={}",
        policy_version.policy_id, policy_version.version
    );

    // 4. Convert bundle_hash to hex string
    let bundle_hash_hex = policy_version
        .bundle_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    // 5. Return response
    let response = DeployBundleResponse {
        policy_id: policy_version.policy_id,
        version: policy_version.version,
        deployed_at: chrono::DateTime::<chrono::Utc>::from(policy_version.deployed_at).to_rfc3339(),
        bundle_hash: bundle_hash_hex,
    };

    Ok(Json(response))
}

// ===== Entity CRUD Operations (Stub Implementation) =====
//
// NOTE: These endpoints define the API contract for entity management.
// Full implementation requires eBPF integration with entity maps.
// Currently returns stub responses for API compatibility.

#[derive(Debug, Deserialize)]
struct UpsertEntityRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub string_attrs: HashMap<String, String>,
    pub numeric_attrs: HashMap<String, i64>,
    pub relationships: Vec<RelationshipRequest>,
    pub flags: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct RelationshipRequest {
    pub rel_type: String,
    pub target: String,
}

#[derive(Debug, serde::Serialize)]
struct EntityResponse {
    pub entity_id: String,
    pub entity_type: String,
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub string_attrs: HashMap<String, String>,
    pub numeric_attrs: HashMap<String, i64>,
    pub relationships: Vec<RelationshipResponse>,
    pub flags: HashMap<String, bool>,
}

#[derive(Debug, serde::Serialize)]
struct RelationshipResponse {
    pub rel_type: String,
    pub target: String,
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, serde::Serialize)]
struct ListEntitiesResponse {
    pub entities: Vec<EntityResponse>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
struct BatchUpsertRequest {
    pub entities: Vec<UpsertEntityRequest>,
}

#[derive(Debug, serde::Serialize)]
struct BatchUpsertResponse {
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<(String, String)>, // (entity_id, error)
}

/// POST /api/v1/entities - Create or update entity
#[instrument(skip(state))]
async fn upsert_entity_handler(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<UpsertEntityRequest>,
) -> Result<Json<EntityResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity upsert request (stub): type={}, id={}",
        req.entity_type, req.entity_id
    );

    // Return stub response
    let response = EntityResponse {
        entity_id: req.entity_id.clone(),
        entity_type: req.entity_type.clone(),
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        string_attrs: req.string_attrs.clone(),
        numeric_attrs: req.numeric_attrs.clone(),
        relationships: req
            .relationships
            .iter()
            .map(|r| RelationshipResponse {
                rel_type: r.rel_type.clone(),
                target: r.target.clone(),
            })
            .collect(),
        flags: req.flags.clone(),
    };

    Ok(Json(response))
}

/// GET /api/v1/entities/:type/:id - Get entity
#[instrument(skip(state))]
async fn get_entity_handler(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path((entity_type, entity_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<EntityResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity get request (stub): type={}, id={}",
        entity_type, entity_id
    );

    // Return stub response
    let response = EntityResponse {
        entity_id: entity_id.clone(),
        entity_type: entity_type.clone(),
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        string_attrs: HashMap::new(),
        numeric_attrs: HashMap::new(),
        relationships: vec![],
        flags: HashMap::new(),
    };

    Ok(Json(response))
}

/// DELETE /api/v1/entities/:type/:id - Delete entity
#[instrument(skip(state))]
async fn delete_entity_handler(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path((entity_type, entity_id)): axum::extract::Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity delete request (stub): type={}, id={}",
        entity_type, entity_id
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/entities/:type - List entities of type
#[instrument(skip(state))]
async fn list_entities_handler(
    State(state): State<Arc<AgentState>>,
    axum::extract::Path(entity_type): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<ListParams>,
) -> Result<Json<ListEntitiesResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity list request (stub): type={}, limit={}",
        entity_type, params.limit
    );

    // Return stub response
    let response = ListEntitiesResponse {
        entities: vec![],
        total: 0,
    };

    Ok(Json(response))
}

/// POST /api/v1/entities/batch - Batch upsert
#[instrument(skip(state))]
async fn batch_upsert_handler(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<BatchUpsertRequest>,
) -> Result<Json<BatchUpsertResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Batch upsert request (stub): {} entities",
        req.entities.len()
    );

    // Return stub response
    let response = BatchUpsertResponse {
        succeeded: req.entities.len(),
        failed: 0,
        errors: vec![],
    };

    Ok(Json(response))
}

// Debug endpoint to check DataStore stats
#[instrument(skip(state))]
async fn debug_datastore(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    let stats = state.data_store.stats();
    Ok(Json(json!({
        "total_entities": stats.total_entities,
        "unique_types": stats.unique_types,
        "indexed_attributes": stats.indexed_attributes
    })))
}
