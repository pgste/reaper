//! Reaper Platform - Policy Management & Agent Orchestration Service
//!
//! This service provides:
//! - Policy CRUD operations and versioning
//! - Bundle compilation and distribution
//! - Agent management (placeholder)
//!
//! ## Module Structure
//!
//! - `handlers`: HTTP request handlers organized by domain
//! - `metrics`: Prometheus metrics definitions
//! - `state`: Shared platform state
//! - `types`: Request/response type definitions

mod handlers;
mod metrics;
mod state;
mod types;

use axum::{
    routing::{get, post},
    Router,
};
use parking_lot::RwLock;
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule};
use reaper_core::{endpoints, BUILD_INFO};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use handlers::{
    create_bundle, create_policy, delete_policy, deploy_bundle_to_agents, deploy_policy_to_agents,
    get_agent, get_bundle, get_policy, health_check, list_agents, list_policies, metrics,
    prometheus_metrics, update_policy,
};
use state::{DeploymentStats, PlatformState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!(
        "Starting Reaper Platform {} - Policy Management & Agent Orchestration",
        BUILD_INFO
    );

    let policy_engine = PolicyEngine::new();

    // Create a default "allow-all" policy for demo purposes
    let default_policy = EnhancedPolicy::new(
        "default-allow-all".to_string(),
        "Default policy that allows all requests".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    info!("Deploying default allow-all policy");
    policy_engine.deploy_policy(default_policy)?;

    // Initialize agents with default localhost agent for testing
    let mut agents = HashMap::new();
    agents.insert("agent-001".to_string(), "http://localhost:8080".to_string());

    let state = Arc::new(PlatformState {
        policy_engine,
        deployment_stats: Arc::new(RwLock::new(DeploymentStats::default())),
        bundle_storage: Arc::new(RwLock::new(HashMap::new())),
        agents: Arc::new(RwLock::new(agents)),
    });

    let app = Router::new()
        // Health and metrics
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        .route("/metrics/prometheus", get(prometheus_metrics))
        // Policy management
        .route(
            endpoints::API_V1_POLICIES,
            get(list_policies).post(create_policy),
        )
        .route(
            "/api/v1/policies/{id}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        .route(
            "/api/v1/policies/{id}/deploy",
            post(deploy_policy_to_agents),
        )
        // Bundle management
        .route("/api/v1/bundles", post(create_bundle))
        .route("/api/v1/bundles/{id}", get(get_bundle))
        .route("/api/v1/bundles/deploy", post(deploy_bundle_to_agents))
        // Agent management (placeholder for now)
        .route(endpoints::API_V1_AGENTS, get(list_agents))
        .route("/api/v1/agents/{id}", get(get_agent))
        .with_state(state);

    // REAPER_PLATFORM_PORT / REAPER_PORT / REAPER_BIND_ADDR (see resolve_bind)
    let (bind, port) = reaper_core::resolve_bind("REAPER_PLATFORM", "0.0.0.0", 8081);
    let listener = TcpListener::bind(format!("{bind}:{port}")).await?;
    info!("🎯 Reaper Platform listening on {}", listener.local_addr()?);
    info!("");
    info!("📋 Policy Management API:");
    info!("  GET    /api/v1/policies        - List all policies");
    info!("  POST   /api/v1/policies        - Create new policy");
    info!("  GET    /api/v1/policies/{{id}} - Get policy details");
    info!("  PUT    /api/v1/policies/{{id}} - Update policy");
    info!("  DELETE /api/v1/policies/{{id}} - Delete policy");
    info!("  POST   /api/v1/policies/{{id}}/deploy - Deploy to agents");
    info!("");

    axum::serve(listener, app).await?;

    Ok(())
}
