mod auth;
mod bootstrap;
mod cache;
mod handlers;
mod management;
mod metrics_cache;
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
    routing::{get, post},
    Router,
};
use cache::PolicyCache;
use clap::Parser;
use policy_engine::{
    cache_config::CacheConfig, create_shared_buffer, DecisionLogConfig, EnhancedPolicy,
    PolicyEngine,
};
use reaper_core::{config::ReaperAgentConfig, endpoints, BUILD_INFO, VERSION};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// Import from extracted modules
use handlers::{
    // Data handlers
    apply_data_deltas,
    // Evaluation handlers
    batch_evaluate_policy,
    // Entity handlers
    batch_upsert_handler,
    check_document,
    confirm_data_version,
    debug_datastore,
    delete_entity_handler,
    // Policy management handlers
    deploy_bundle,
    deploy_compiled_policy,
    deploy_data_version,
    deploy_policy,
    evaluate_policy,
    // Decision handlers
    export_decisions,
    fast_evaluate_policy,
    get_decision_by_id,
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
    load_bundles_atomic,
    load_data_handler,
    load_data_stream_handler,
    metrics,
    readiness_check,
    sync_data,
    upsert_entity_handler,
};
use observability::init_observability;
use opentelemetry::global;
use state::{AgentState, AgentStats, DataSyncState};

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
#[allow(dead_code)] // wire-format mirror; serde populates all fields
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

    // Fail-closed exposure check: a non-loopback bind without inbound auth,
    // agent-terminated mTLS, or the explicit opt-out never gets to serve.
    if let Err(reason) = config
        .auth
        .validate_exposure(&config.agent.bind_address, config.tls.require_client_cert)
    {
        error!("{reason}");
        anyhow::bail!(reason);
    }
    if config.auth.enabled {
        info!(
            "🔐 Inbound authentication ENABLED (mode: {:?})",
            config.auth.mode
        );
    } else if config.auth.allow_unauthenticated
        && !reaper_core::config::is_loopback_bind(&config.agent.bind_address)
    {
        warn!(
            "Inbound authentication DISABLED on a non-loopback bind — explicitly opted out via \
             allow_unauthenticated. Anyone who can reach {}:{} can query and mutate this agent.",
            config.agent.bind_address, config.agent.port
        );
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
    // Data-plane sync state: shared by the heartbeat reporter (two-way
    // visibility) and the data handlers/staleness guard.
    let data_sync = Arc::new(DataSyncState::from_env());

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

                // Create sync service with stats and start time for metrics
                let (sync_service, mut update_rx) = management::SyncService::new(
                    client.clone(),
                    config.management.clone(),
                    data_store.clone(),
                    stats.clone(),
                    started_at,
                    shutdown_rx.clone(),
                    data_sync.clone(),
                );

                // Spawn sync service
                let sync_handle = tokio::spawn(async move {
                    sync_service.run().await;
                });
                management_handle = Some(sync_handle);

                // Spawn bundle update handler
                let policy_engine_for_updates = policy_engine.clone();
                let _data_store_for_updates = data_store.clone();
                let client_for_updates = client.clone();
                tokio::spawn(async move {
                    while update_rx.changed().await.is_ok() {
                        // Clone out of the watch guard immediately so it is not
                        // held across the awaits below (deploy report).
                        let maybe_update = update_rx.borrow().clone();
                        if let Some(update) = maybe_update {
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

                                    // Confirm the applied version to the control
                                    // plane so rollouts converge on real state.
                                    let success = failed == 0;
                                    let err = (!success)
                                        .then(|| format!("{failed} policy(ies) failed to deploy"));
                                    if let Err(e) = client_for_updates
                                        .report_deployment(
                                            update.bundle_id,
                                            &update.checksum,
                                            success,
                                            err.as_deref(),
                                        )
                                        .await
                                    {
                                        warn!(error = %e,
                                            "Failed to report deployment status to management");
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, "Failed to parse management bundle");
                                    // Report the failure so the rollout doesn't
                                    // wait on this agent indefinitely.
                                    if let Err(re) = client_for_updates
                                        .report_deployment(
                                            update.bundle_id,
                                            &update.checksum,
                                            false,
                                            Some("failed to parse management bundle"),
                                        )
                                        .await
                                    {
                                        warn!(error = %re,
                                            "Failed to report deployment failure to management");
                                    }
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
        decision_metrics: Arc::new(metrics_cache::DecisionMetrics::new()),
        data_sync: data_sync.clone(),
        bundle_verifier: Arc::new(management::verify::BundleVerifier::from_config(
            &config.management,
        )),
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
        .route("/api/v1/check", post(check_document))
        // Data management - load entities
        .route("/api/v1/data", post(load_data_handler))
        .route("/api/v1/data/stream", post(load_data_stream_handler))
        .route("/api/v1/data/sync", post(sync_data))
        .route("/api/v1/data/deploy-version", post(deploy_data_version))
        .route("/api/v1/data/confirm-version", post(confirm_data_version))
        .route("/api/v1/data/apply-deltas", post(apply_data_deltas))
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
        .route("/api/v1/bundles/load", post(load_bundles_atomic))
        // Entity CRUD operations (requires eBPF integration)
        .route("/api/v1/entities", post(upsert_entity_handler))
        .route("/api/v1/entities/{type}/{id}", get(get_entity_handler))
        .route(
            "/api/v1/entities/{type}/{id}",
            axum::routing::delete(delete_entity_handler),
        )
        .route("/api/v1/entities/{type}", get(list_entities_handler))
        .route("/api/v1/entities/batch", post(batch_upsert_handler))
        // Decision log endpoints (OPA-style audit logging)
        .route("/api/v1/decisions", get(get_decisions))
        .route("/api/v1/decisions/stats", get(get_decision_stats))
        .route("/api/v1/decisions/export", post(export_decisions))
        .route("/api/v1/decisions/{decision_id}", get(get_decision_by_id));

    // Debug endpoints: compiled out of release builds unless explicitly
    // re-enabled — /debug/datastore dumps the full entity store, which is
    // tenant data, not something a production agent should ever serve.
    let debug_endpoints = cfg!(debug_assertions)
        || std::env::var("REAPER_DEBUG_ENDPOINTS")
            .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
            .unwrap_or(false);
    let app = if debug_endpoints {
        app.route("/debug/datastore", get(debug_datastore))
    } else {
        app
    };

    // Inbound authentication (Plan 01 Phase C): default-deny over every
    // non-health route. Configuration-driven and zero-cost when disabled —
    // the layer is only mounted at all when auth.enabled, and the verifier
    // pre-computes keys/digests so the enabled path is ~one hash per request.
    let app = match auth::AgentAuthVerifier::from_config(&config) {
        Some(verifier) => app.layer(axum::middleware::from_fn_with_state(
            verifier,
            auth::require_agent_auth,
        )),
        None => app,
    };

    let app = app
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024)) // 256MB: bulk data loads (100k+ entity benchmark datasets) exceed 100MB
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

    // Spawn UDS listener(s) if enabled — shared (one socket) or sharded
    // (thread-per-core, N sockets) per config.uds.shards.
    if let Some(uds_app) = uds_app {
        uds::spawn_uds_listeners(&config.uds, uds_app);
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
