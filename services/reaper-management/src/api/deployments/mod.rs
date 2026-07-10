//! Deployment API endpoints
//!
//! Provides REST endpoints for managing deployment strategies, rollouts,
//! rollbacks, and version pins.

mod pins;
mod rollback_config;
mod rollouts;
mod status;
mod strategies;
pub mod types;

use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

pub use types::*;

/// Build deployment routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Deployment strategies
        .routes(routes!(
            strategies::list_strategies,
            strategies::create_strategy
        ))
        .routes(routes!(
            strategies::get_strategy,
            strategies::delete_strategy
        ))
        // Rollouts
        .routes(routes!(rollouts::start_rollout))
        .routes(routes!(rollouts::list_rollouts))
        .routes(routes!(rollouts::get_rollout))
        .routes(routes!(rollouts::approve_wave))
        .routes(routes!(rollouts::cancel_rollout))
        // Rollback
        .routes(routes!(rollouts::rollback_namespace))
        .routes(routes!(rollouts::rollback_org))
        // Version pins
        .routes(routes!(pins::create_pin, pins::get_pin, pins::delete_pin))
        .routes(routes!(pins::list_pins))
        // Deployment status tracking
        .routes(routes!(status::get_rollout_deployments))
        .routes(routes!(status::get_deployment_summary))
        .routes(routes!(status::acknowledge_deployment))
        .routes(routes!(status::get_agent_deployment))
        // Auto-rollback configuration
        .routes(routes!(
            rollback_config::get_rollback_config,
            rollback_config::update_rollback_config
        ))
        .routes(routes!(
            rollback_config::get_namespace_rollback_config,
            rollback_config::update_namespace_rollback_config
        ))
        .routes(routes!(rollback_config::check_rollback_trigger))
}
