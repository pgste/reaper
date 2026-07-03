mod bootstrap;
mod cache;
mod handlers;
mod management;
mod observability;
mod state;
mod tls;
mod types;
mod uds;

// Fast allocator: policy evaluation is allocation-heavy on the request path
// (request maps, response buffers). mimalloc is faster and has less
// fragmentation than the system allocator under this concurrent load.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Json, Response},
    routing::{delete, get, post},
    Router,
};
use cache::PolicyCache;
use clap::Parser;
use policy_engine::{
    cache_config::CacheConfig, create_shared_buffer, DecisionFilter, DecisionLogConfig,
    DecisionLogEntry, EnhancedPolicy, PolicyAction, PolicyBundle, PolicyEngine, PolicyRequest,
    PolicyRule,
};
use prometheus::{Encoder, TextEncoder};
use reaper_core::{config::ReaperAgentConfig, endpoints, BUILD_INFO, VERSION};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

// Import from extracted modules
use handlers::{
    // Evaluation handlers
    batch_evaluate_policy,
    // Entity handlers
    batch_upsert_handler,
    debug_datastore,
    delete_entity_handler,
    // Policy management handlers
    deploy_bundle,
    deploy_compiled_policy,
    deploy_policy,
    evaluate_policy,
    // Decision handlers
    export_decisions,
    fast_evaluate_policy,
    get_decision_stats,
    get_decisions,
    get_entity_handler,
    get_policy_current_version,
    get_policy_versions,
    // Health handlers
    health_check,
    list_entities_handler,
    list_policies,
    liveness_check,
    // Data handlers
    load_data_handler,
    load_data_stream_handler,
    metrics,
    readiness_check,
    sync_data,
    upsert_entity_handler,
};
use observability::{
    init_observability, record_decision, record_denial, set_active_policies, ACTIVE_POLICIES,
    CACHE_HITS, CACHE_MISSES, CONCURRENT_EVALUATIONS, DECISIONS_TOTAL, DECISION_DURATION,
    DECISION_LOG_BUFFER_SIZE, DECISION_LOG_ENTRIES, DECISION_LOG_FLUSHES, DENIALS_TOTAL,
    ERRORS_TOTAL,
};
use opentelemetry::{global, trace::TraceContextExt, KeyValue};
use state::{AgentState, AgentStats};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use types::{
    BatchEvaluateRequest, BatchRequestItem, BatchResponseItem, DecisionQuery, DeployBundleRequest,
    DeployBundleResponse, DeployCompiledRequest, DeployPolicyRequest, EvaluateRequest,
    EvaluateResponse, ExportDecisionsRequest, PackageEvaluateRequest,
};

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

// Types moved to their respective modules:
// - Prometheus metrics: observability.rs
// - AgentState, AgentStats: state.rs
// - Request/Response types: types.rs

// Keep DeployPolicyRule here as it's used internally by deploy_policy handler
#[derive(Debug, Deserialize)]
struct DeployPolicyRule {
    pub action: String,
    pub resource: String,
    pub conditions: Option<Vec<String>>,
}

// AgentStats methods are now in state.rs
// init_observability is now in observability.rs

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

    info!("Decision cache: {}", cache_config.summary());

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
                            // Rebuild the evaluator for the cached policy (the
                            // evaluator itself is not serialized). Pass the
                            // populated DataStore so Reaper-DSL policies that
                            // read entity attributes are restored correctly and
                            // survive the restart.
                            if let Err(e) =
                                policy.build_evaluator_with_data(Some(data_store.clone()))
                            {
                                warn!(
                                    "Failed to build evaluator for cached policy {}: {}",
                                    policy.name, e
                                );
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

    // Create shared stats for both management sync and request handling
    // Enhanced metrics (histogram, CPU, memory) are controlled by config
    let stats = Arc::new(AgentStats::new(
        config.observability.enable_enhanced_metrics,
    ));
    let started_at = std::time::Instant::now();

    if config.observability.enable_enhanced_metrics {
        info!("Enhanced metrics enabled (REAPER_ENHANCED_METRICS=true)");
    } else {
        debug!("Enhanced metrics disabled (set REAPER_ENHANCED_METRICS=true to enable)");
    }

    // Initialize management client if enabled
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut management_handle = None;

    if config.management.enabled {
        info!(
            "Management plane enabled - connecting to {}",
            config.management.url.as_deref().unwrap_or("?")
        );

        match management::ManagementClient::new(
            &config.management,
            config.agent.name.clone(),
            VERSION.to_string(),
        ) {
            Ok(client) => {
                let client = Arc::new(client);
                let policy_engine_for_sync = policy_engine.clone();

                // Create sync service with stats and start time for metrics
                let (sync_service, mut update_rx) = management::SyncService::new(
                    client.clone(),
                    config.management.clone(),
                    Arc::new(policy_engine_for_sync),
                    data_store.clone(),
                    stats.clone(),
                    started_at,
                    shutdown_rx.clone(),
                );

                // Spawn sync service
                let sync_handle = tokio::spawn(async move {
                    sync_service.run().await;
                });
                management_handle = Some(sync_handle);

                // Spawn bundle update handler
                let policy_engine_for_updates = policy_engine.clone();
                let _data_store_for_updates = data_store.clone();
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
                            match serde_json::from_slice::<management::ManagementBundle>(
                                &update.data,
                            ) {
                                Ok(bundle) => {
                                    let mut deployed = 0;
                                    let mut failed = 0;

                                    for policy_entry in bundle.policies {
                                        // Create EnhancedPolicy from bundle entry
                                        let policy_id = Uuid::parse_str(&policy_entry.id)
                                            .unwrap_or_else(|_| Uuid::new_v4());

                                        let mut policy = EnhancedPolicy::new(
                                            policy_entry.id.clone(),
                                            "Policy from bundle".to_string(),
                                            vec![], // Rules will be set by content
                                        );
                                        policy.id = policy_id;
                                        policy.version = policy_entry.version as u64;
                                        policy.content = policy_entry.content.clone();

                                        // Set the language based on what management server provides
                                        policy.language = match policy_entry.language.as_str() {
                                            "cedar" => policy_engine::PolicyLanguage::Cedar,
                                            "simple" => policy_engine::PolicyLanguage::Simple,
                                            _ => policy_engine::PolicyLanguage::ReaperDsl,
                                        };

                                        if let Err(e) =
                                            policy_engine_for_updates.deploy_policy(policy)
                                        {
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

    // Initialize decision logging buffer from environment config
    let decision_log_config = DecisionLogConfig::from_env();
    let decision_buffer = if decision_log_config.enabled {
        match create_shared_buffer(decision_log_config) {
            Ok(buffer) => {
                info!("Decision logging enabled");
                Some(buffer)
            }
            Err(e) => {
                warn!(error = %e, "Failed to create decision buffer, decision logging disabled");
                None
            }
        }
    } else {
        None
    };

    // Generate or use configured agent ID
    let agent_id = std::env::var("REAPER_AGENT_ID").unwrap_or_else(|_| {
        format!(
            "agent-{}",
            Uuid::new_v4().to_string().split('-').next().unwrap()
        )
    });

    let state = Arc::new(AgentState {
        policy_engine,
        data_store,
        stats, // Use shared stats for consistency with management sync
        decision_cache,
        cache_config,
        agent_config: config.clone(),
        policy_cache,
        decision_buffer,
        agent_id,
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
        .route(
            "/api/v1/policies/{id}/version",
            get(get_policy_current_version),
        )
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
        // Decision log endpoints (OPA-style audit logging)
        .route("/api/v1/decisions", get(get_decisions))
        .route("/api/v1/decisions/stats", get(get_decision_stats))
        .route("/api/v1/decisions/export", post(export_decisions))
        .layer(axum::extract::DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit for large datasets
        .with_state(state);

    // Clone router for UDS listener before the TCP server consumes it
    let uds_app = if config.uds.enabled {
        Some(app.clone())
    } else {
        None
    };

    let bind_addr = format!("{}:{}", config.agent.bind_address, config.agent.port);

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

    // Spawn UDS listener if enabled
    if let Some(uds_app) = uds_app {
        let socket_path = config.uds.socket_path.clone();
        let socket_permissions = config.uds.socket_permissions;
        info!(
            path = %socket_path.display(),
            "Starting UDS listener"
        );
        tokio::spawn(async move {
            if let Err(e) = uds::serve_uds(socket_path, socket_permissions, uds_app).await {
                error!("UDS server error: {}", e);
            }
        });
    }

    // Run server with TLS if configured
    let result = if config.tls.enabled {
        info!("🔒 TLS enabled - secure mode");
        if config.tls.require_client_cert {
            info!("   mTLS: Client certificates REQUIRED");
        }

        // Validate TLS settings
        tls::validate_tls_settings(&config.tls)?;

        // Create TLS config
        let tls_config = tls::create_tls_config(&config.tls).await?;

        let addr: std::net::SocketAddr = bind_addr.parse()?;
        info!("🚀 Ready for sub-microsecond policy enforcement (HTTPS)!");

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|e| anyhow::anyhow!("TLS server error: {}", e))
    } else {
        info!("🚀 Ready for sub-microsecond policy enforcement!");
        let listener = TcpListener::bind(&bind_addr).await?;
        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {}", e))
    };

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
