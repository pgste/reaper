//! Reaper Management Server
//!
//! Multi-tenant policy management server providing:
//! - Organization-scoped policy management
//! - Multiple policy sources (Git repositories, External APIs)
//! - Bundle compilation and promotion workflow
//! - Agent registration and SSE notifications
//!
//! # Production Features
//!
//! - Graceful shutdown with in-flight request draining
//! - Security headers (XSS, clickjacking protection)
//! - Request correlation IDs for distributed tracing
//! - Configuration validation on startup
//! - Comprehensive health checks (/health, /ready, /live)
//!
//! # Usage
//!
//! ```bash
//! # With default config
//! reaper-management
//!
//! # With config file
//! reaper-management --config /etc/reaper/management.yaml
//!
//! # With environment overrides
//! REAPER_PORT=8081 REAPER_DATABASE_URL=sqlite:///var/lib/reaper/mgmt.db reaper-management
//! ```

use axum::{middleware, Router};
use clap::Parser;
use reaper_management::{
    api, auth, config::Config, db, graceful, metrics, middleware as app_middleware, rate_limit,
    storage, AppState,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

/// Reaper Management Server CLI
#[derive(Parser, Debug)]
#[command(name = "reaper-management")]
#[command(about = "Multi-tenant policy management server")]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Override bind address
    #[arg(long)]
    bind: Option<String>,

    /// Override port
    #[arg(short, long)]
    port: Option<u16>,

    /// Skip configuration validation (not recommended for production)
    #[arg(long, default_value = "false")]
    skip_validation: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reaper_management=info,tower_http=info".into()),
        )
        .json()
        .init();

    // Initialize Prometheus metrics
    metrics::init_metrics();

    let cli = Cli::parse();

    // Load configuration
    let mut config = if let Some(config_path) = &cli.config {
        info!("Loading configuration from {:?}", config_path);
        Config::from_file(config_path)?
    } else {
        info!("Using default configuration with environment overrides");
        Config::from_env()?
    };

    // Apply CLI overrides
    if let Some(bind) = cli.bind {
        config.server.bind_address = bind;
    }
    if let Some(port) = cli.port {
        config.server.port = port;
    }

    // Validate configuration
    if !cli.skip_validation {
        info!("Validating configuration...");
        if let Err(e) = config.validate() {
            error!("Configuration validation failed: {}", e);
            return Err(e.into());
        }
        info!("Configuration validated successfully");
    } else {
        warn!("Skipping configuration validation (not recommended for production)");
    }

    info!("Configuration: {}", config.summary());

    // Prepare directories
    if let Err(e) = config.prepare_directories() {
        warn!("Failed to prepare directories: {}", e);
        // Continue anyway, individual operations will fail if needed
    }

    // Initialize database
    info!("Initializing database...");
    let db = db::init_database(&config.database).await?;

    // Initialize storage
    info!("Initializing storage backend...");
    let storage = storage::create_storage(&config.storage).await?;
    info!("Using storage backend: {}", storage.backend_name());

    // Create application state
    let state = Arc::new(AppState::new(db, config.clone(), storage));

    // Cross-instance eventing: on PostgreSQL, LISTEN for sibling
    // instances' publish notifications and re-broadcast them locally so
    // agents connected to THIS instance wake up too.
    if state.db.db_type() == "postgres" {
        reaper_management::events_pg::spawn_pg_event_bridge(
            state.clone(),
            config.database.url.clone(),
        );
    }

    // Change-log retention sweeper: publish-time compaction never bounds a
    // datastore that churns without publishing, so age out old delta marks
    // on a schedule. Pruning is always safe — a replica below the floor
    // gets snapshot_required and self-heals — but retention should stay
    // far above agent sync intervals so healthy followers never hit it.
    // REAPER_CHANGE_LOG_RETENTION_SECS=0 disables (default 30 days).
    {
        let retention_secs: u64 = std::env::var("REAPER_CHANGE_LOG_RETENTION_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30 * 24 * 3600);
        let sweep_secs: u64 = std::env::var("REAPER_CHANGE_LOG_SWEEP_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
        if retention_secs > 0 {
            let db = state.db.clone();
            tokio::spawn(async move {
                let mut tick =
                    tokio::time::interval(std::time::Duration::from_secs(sweep_secs.max(60)));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    let cutoff = (chrono::Utc::now()
                        - chrono::Duration::seconds(retention_secs as i64))
                    .to_rfc3339();
                    let repo = reaper_management::db::repositories::DatastoreRepository::new(&db);
                    match repo.prune_change_log(&cutoff).await {
                        Ok(0) => {}
                        Ok(n) => info!(pruned = n, "change-log retention: aged out delta marks"),
                        Err(e) => warn!("change-log retention sweep failed: {e}"),
                    }
                }
            });
        }
    }

    // Create rate limiter
    let rate_limiter = rate_limit::create_rate_limiter(&config.rate_limit);

    // Build router
    let app = build_router(state.clone(), rate_limiter);

    // Start server
    let addr = SocketAddr::new(config.server.bind_address.parse()?, config.server.port);

    info!("Starting Reaper Management Server on {}", addr);
    info!("Health check: http://{}/health", addr);
    info!("Metrics: http://{}/metrics/prometheus", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Run server with graceful shutdown. ConnectInfo is REQUIRED: the
    // rate-limit middleware extracts the peer address, and without
    // into_make_service_with_connect_info every request 500s — a bug the
    // router-level tests could never see (they bypass serve()); caught by
    // the process-level data-plane E2E.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(state.clone()))
    .await?;

    info!("Server shutdown complete");
    Ok(())
}

/// Build the Axum router with all routes and middleware
fn build_router(
    state: Arc<AppState>,
    rate_limiter: Option<Arc<rate_limit::ApiRateLimiter>>,
) -> Router {
    // Serve the API at BOTH the root (existing consumers/tests) and the
    // documented /api/v1 prefix (reaper-sync and external clients build
    // URLs against /api/v1 — caught by the process-level data-plane E2E).
    let api_router =
        api::build_api_router().merge(Router::new().nest("/api/v1", api::build_api_router()));

    // Default-deny authentication gateway: authenticate every non-public request
    // at the router layer so a handler that forgets `RequireAuth` still fails
    // closed. Runs innermost (right before handlers), after body-size/rate-limit.
    let api_router = api_router.layer(middleware::from_fn_with_state(
        state.clone(),
        auth::gateway::require_authentication,
    ));

    // Build the router with middleware
    // Middleware is applied in reverse order (last added runs first)
    let router = api_router
        .with_state(state.clone())
        // Apply security headers to all responses
        .layer(middleware::from_fn(app_middleware::security_headers))
        // Apply correlation ID middleware
        .layer(middleware::from_fn(app_middleware::correlation_id))
        // Apply request metrics middleware
        .layer(middleware::from_fn(app_middleware::request_metrics))
        // Apply body size limit
        .layer(middleware::from_fn(app_middleware::body_size_limit))
        // Apply access logging
        .layer(middleware::from_fn(app_middleware::access_log))
        // Apply tracing
        .layer(TraceLayer::new_for_http());

    // Apply rate limiting if enabled
    if let Some(limiter) = rate_limiter {
        router.layer(middleware::from_fn_with_state(
            limiter,
            rate_limit::rate_limit_middleware,
        ))
    } else {
        router
    }
}

/// Wait for shutdown signal and coordinate graceful shutdown
async fn shutdown_signal(state: Arc<AppState>) {
    // Wait for OS shutdown signal
    graceful::wait_for_shutdown_signal().await;

    // Initiate shutdown
    state.initiate_shutdown();

    // Wait for in-flight requests with timeout
    let shutdown_config = graceful::ShutdownConfig {
        timeout: Duration::from_secs(30),
        force_after_timeout: true,
    };

    graceful::graceful_shutdown(state.shutdown_signal(), &shutdown_config).await;
}
