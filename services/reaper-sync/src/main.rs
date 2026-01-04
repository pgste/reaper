//! Reaper Sync Client
//!
//! The sync client polls a management server for policy updates and deploys
//! them to a Reaper Agent. It can run continuously or in one-shot mode.
//!
//! # Usage
//!
//! ```bash
//! # Run with config file
//! reaper-sync --config /etc/reaper/sync.yaml
//!
//! # Run once and exit
//! reaper-sync --config /etc/reaper/sync.yaml --once
//!
//! # Run with environment variables
//! REAPER_SERVER_URL=http://platform:8081 \
//! REAPER_AGENT_URL=http://agent:8080 \
//! reaper-sync
//! ```

mod agent_client;
mod config;
mod server_client;
mod sync_engine;

use anyhow::Result;
use clap::Parser;
use config::SyncConfig;
use std::path::PathBuf;
use sync_engine::SyncEngine;
use tracing::{error, info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Reaper Sync Client - Policy synchronization from management server to agent
#[derive(Parser, Debug)]
#[command(name = "reaper-sync")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file (YAML)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Run once and exit (don't run continuously)
    #[arg(long)]
    once: bool,

    /// Also sync entity data
    #[arg(long)]
    sync_entities: bool,

    /// Replace all entities when syncing (instead of merge)
    #[arg(long)]
    replace_entities: bool,

    /// Wait for agent to become available before starting
    #[arg(long, default_value = "true")]
    wait_for_agent: bool,

    /// Maximum attempts to wait for agent
    #[arg(long, default_value = "10")]
    wait_attempts: u32,

    /// Delay between wait attempts in seconds
    #[arg(long, default_value = "3")]
    wait_delay: u64,

    /// Override server URL
    #[arg(long, env = "REAPER_SERVER_URL")]
    server_url: Option<String>,

    /// Override agent URL
    #[arg(long, env = "REAPER_AGENT_URL")]
    agent_url: Option<String>,

    /// Override poll interval in seconds
    #[arg(long, env = "REAPER_POLL_INTERVAL")]
    poll_interval: Option<u64>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "REAPER_LOG_LEVEL")]
    log_level: String,

    /// Output logs in JSON format
    #[arg(long, env = "REAPER_LOG_JSON")]
    log_json: bool,
}

fn init_logging(level: &str, json: bool) {
    let level = match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let filter = EnvFilter::from_default_env().add_directive(level.into());

    if json {
        tracing_subscriber::registry()
            .with(fmt::layer().json())
            .with(filter)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(filter)
            .init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    init_logging(&args.log_level, args.log_json);

    info!("Reaper Sync Client starting");

    // Load configuration
    let mut config = if let Some(ref config_path) = args.config {
        if config_path.exists() {
            info!("Loading configuration from {:?}", config_path);
            SyncConfig::from_file(config_path)?
        } else {
            warn!(
                "Config file {:?} not found, using environment/defaults",
                config_path
            );
            SyncConfig::from_env()?
        }
    } else {
        info!("No config file specified, using environment/defaults");
        SyncConfig::from_env()?
    };

    // Apply command-line overrides
    if let Some(ref server_url) = args.server_url {
        config.sync.server.url = server_url.clone();
    }
    if let Some(ref agent_url) = args.agent_url {
        config.sync.agent.url = agent_url.clone();
    }
    if let Some(poll_interval) = args.poll_interval {
        config.sync.behavior.poll_interval_seconds = poll_interval;
    }

    info!("Configuration: {}", config.summary());

    // Create sync engine
    let mut engine = SyncEngine::new(config.clone())?;

    // Wait for agent if requested
    if args.wait_for_agent {
        info!(
            "Waiting for agent at {} (max {} attempts)",
            config.sync.agent.url, args.wait_attempts
        );

        let agent_client = agent_client::AgentClient::new(&config)?;
        if !agent_client
            .wait_for_agent(args.wait_attempts, args.wait_delay)
            .await
        {
            error!("Agent not available, exiting");
            std::process::exit(1);
        }
    }

    // Run sync
    if args.once {
        // One-shot mode
        info!("Running in one-shot mode");

        // Sync policies
        let result = engine.sync_once().await;

        if result.success {
            info!(
                "Sync complete: deployed={}, skipped={}, failed={}",
                result.deployed, result.skipped, result.failed
            );
        } else {
            error!("Sync failed: {:?}", result.error);
        }

        // Optionally sync entities
        if args.sync_entities {
            info!("Syncing entity data (replace_all={})", args.replace_entities);
            let entity_result = engine.sync_entities(args.replace_entities).await;

            if entity_result.success {
                info!("Entity sync complete: {} entities", entity_result.entities_synced);
            } else {
                error!("Entity sync failed: {:?}", entity_result.error);
            }
        }

        // Print final stats
        let stats = engine.stats();
        info!(
            "Final stats: total_syncs={}, total_deployed={}, tracked={}",
            stats.total_syncs, stats.total_policies_deployed, stats.tracked_policies
        );

        if !result.success {
            std::process::exit(1);
        }
    } else {
        // Continuous mode
        info!("Running in continuous mode");

        // Optionally sync entities on start
        if args.sync_entities {
            info!(
                "Initial entity sync (replace_all={})",
                args.replace_entities
            );
            let entity_result = engine.sync_entities(args.replace_entities).await;
            if !entity_result.success {
                warn!("Initial entity sync failed: {:?}", entity_result.error);
            }
        }

        // Run continuous sync loop
        engine.run_continuous().await?;
    }

    Ok(())
}
