//! Reaper Management Server
//!
//! Multi-tenant policy management server providing:
//! - Organization-scoped policy management
//! - Multiple policy sources (Git repositories, External APIs)
//! - Bundle compilation and promotion workflow
//! - Agent registration and SSE notifications
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

use axum::Router;
use clap::Parser;
use reaper_management::{api, config::Config, db, AppState};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tracing::info;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reaper_management=info,tower_http=info".into()),
        )
        .init();

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

    info!("Configuration: {}", config.summary());

    // Initialize database
    info!("Initializing database...");
    let db = db::init_database(&config.database).await?;

    // Create application state
    let state = AppState::new(db, config.clone());

    // Build router
    let app = build_router(state);

    // Start server
    let addr = SocketAddr::new(
        config.server.bind_address.parse()?,
        config.server.port,
    );

    info!("Starting Reaper Management Server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Build the Axum router with all routes
fn build_router(state: AppState) -> Router {
    api::build_api_router().with_state(Arc::new(state))
}
