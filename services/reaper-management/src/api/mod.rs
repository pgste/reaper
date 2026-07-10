//! API layer for Reaper Management Server
//!
//! Provides RESTful endpoints for managing organizations, policies, and bundles.

pub mod agents;
pub mod audit;
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
pub mod openapi;
pub mod orgs;
pub mod policies;
pub mod replay;
pub mod revocations;
pub mod scim;
pub mod sources;
pub mod teams;
pub mod users;
pub mod webhook_subscriptions;
pub mod webhooks;

use crate::state::AppState;
use axum::Router;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

/// Assemble the full control-plane router together with its OpenAPI 3.1
/// contract (Plan 07, Phase A).
///
/// Every module's `routes()` returns an [`OpenApiRouter`] that carries both the
/// axum routes and the `#[utoipa::path]`-derived operations, so merging them
/// yields the served router and the published spec from one tree
/// ([`split_for_parts`](OpenApiRouter::split_for_parts) /
/// [`into_openapi`](OpenApiRouter::into_openapi)). The contract-parity test
/// (`tests/api_contract.rs`) guards against a raw `.route(..)` sneaking a route
/// past the contract.
///
/// Note: `orgs::routes()` already includes `policies::routes()` and
/// `teams::routes()` via merge(), so they are not added separately here (avoids
/// route conflicts).
pub fn build_openapi_router() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::with_openapi(openapi::ApiDoc::openapi())
        // The contract endpoint itself is a plain route (not part of the
        // documented surface); the parity gate allowlists it.
        .route("/openapi.json", axum::routing::get(openapi::serve_openapi))
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
        .merge(audit::routes())
        .merge(replay::routes())
        .merge(datastore::routes())
        .merge(landscape::routes())
        .merge(billing::routes())
        .merge(revocations::routes())
        .merge(scim::routes())
}

/// Build the API router with all routes (state deferred — applied by the
/// caller). Backwards-compatible entry point for existing callers/tests.
pub fn build_api_router() -> Router<Arc<AppState>> {
    build_openapi_router().split_for_parts().0
}

/// Generate the assembled OpenAPI 3.1 document for the control plane.
pub fn build_openapi() -> utoipa::openapi::OpenApi {
    build_openapi_router().into_openapi()
}
