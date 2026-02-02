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

use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::state::AppState;

pub use types::*;

/// Build deployment routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Deployment strategies
        .route("/orgs/{org}/deployment-strategies", get(strategies::list_strategies))
        .route("/orgs/{org}/deployment-strategies", post(strategies::create_strategy))
        .route(
            "/orgs/{org}/deployment-strategies/{strategy_id}",
            get(strategies::get_strategy),
        )
        .route(
            "/orgs/{org}/deployment-strategies/{strategy_id}",
            delete(strategies::delete_strategy),
        )
        // Rollouts
        .route("/orgs/{org}/bundles/{bundle_id}/rollout", post(rollouts::start_rollout))
        .route("/orgs/{org}/rollouts", get(rollouts::list_rollouts))
        .route("/orgs/{org}/rollouts/{rollout_id}", get(rollouts::get_rollout))
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/approve",
            post(rollouts::approve_wave),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/cancel",
            post(rollouts::cancel_rollout),
        )
        // Rollback
        .route(
            "/orgs/{org}/namespaces/{namespace}/rollback",
            post(rollouts::rollback_namespace),
        )
        .route("/orgs/{org}/rollback", post(rollouts::rollback_org))
        // Version pins
        .route("/orgs/{org}/agents/{agent_id}/pin", post(pins::create_pin))
        .route("/orgs/{org}/agents/{agent_id}/pin", get(pins::get_pin))
        .route("/orgs/{org}/agents/{agent_id}/pin", delete(pins::delete_pin))
        .route("/orgs/{org}/pins", get(pins::list_pins))
        // Deployment status tracking
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/deployments",
            get(status::get_rollout_deployments),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/summary",
            get(status::get_deployment_summary),
        )
        .route(
            "/orgs/{org}/agents/{agent_id}/deployment/acknowledge",
            post(status::acknowledge_deployment),
        )
        .route(
            "/orgs/{org}/agents/{agent_id}/deployment",
            get(status::get_agent_deployment),
        )
        // Auto-rollback configuration
        .route("/orgs/{org}/auto-rollback", get(rollback_config::get_rollback_config))
        .route("/orgs/{org}/auto-rollback", post(rollback_config::update_rollback_config))
        .route(
            "/orgs/{org}/namespaces/{namespace}/auto-rollback",
            get(rollback_config::get_namespace_rollback_config),
        )
        .route(
            "/orgs/{org}/namespaces/{namespace}/auto-rollback",
            post(rollback_config::update_namespace_rollback_config),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/check-rollback",
            post(rollback_config::check_rollback_trigger),
        )
}
