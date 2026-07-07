//! API layer for Reaper Management Server
//!
//! Provides RESTful endpoints for managing organizations, policies, and bundles.

pub mod agents;
pub mod auth;
pub mod billing;
pub mod bundles;
pub mod datastore;
pub mod decisions;
pub mod deployments;
pub mod error;
pub mod events;
pub mod health;
pub mod landscape;
pub mod namespaces;
pub mod oauth;
pub mod orgs;
pub mod policies;
pub mod sources;
pub mod teams;
pub mod users;
pub mod webhook_subscriptions;
pub mod webhooks;

use crate::state::AppState;
use axum::Router;
use std::sync::Arc;

/// Build the API router with all routes
pub fn build_api_router() -> Router<Arc<AppState>> {
    // Note: orgs::routes() already includes policies::routes() and teams::routes()
    // via merge(), so we don't add them separately here to avoid route conflicts
    Router::new()
        .merge(health::routes())
        .merge(orgs::routes())
        .merge(auth::routes())
        .merge(users::routes())
        .merge(oauth::routes())
        .merge(agents::routes())
        .merge(events::routes())
        .merge(sources::routes())
        .merge(bundles::routes())
        .merge(webhooks::routes())
        .merge(webhook_subscriptions::routes())
        .merge(namespaces::routes())
        .merge(deployments::routes())
        .merge(decisions::routes())
        .merge(datastore::routes())
        .merge(landscape::routes())
        .merge(billing::routes())
}

/// Build the v1 API routes (kept for backwards compat)
#[allow(dead_code)]
fn api_v1_routes() -> Router<Arc<AppState>> {
    Router::new().merge(orgs::routes())
}
