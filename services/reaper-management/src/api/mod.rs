//! API layer for Reaper Management Server
//!
//! Provides RESTful endpoints for managing organizations, policies, and bundles.

pub mod agents;
pub mod auth;
pub mod bundles;
pub mod error;
pub mod events;
pub mod health;
pub mod orgs;
pub mod policies;
pub mod sources;
pub mod teams;

use axum::Router;
use std::sync::Arc;
use crate::state::AppState;

/// Build the API router with all routes
pub fn build_api_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(health::routes())
        .merge(orgs::routes())
        .merge(auth::routes())
        .merge(agents::routes())
        .merge(events::routes())
        .merge(sources::routes())
        .merge(bundles::routes())
}

/// Build the v1 API routes (kept for backwards compat)
#[allow(dead_code)]
fn api_v1_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(orgs::routes())
}
