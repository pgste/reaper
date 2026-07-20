//! API layer for Reaper Management Server
//!
//! Provides RESTful endpoints for managing organizations, policies, and bundles.

pub mod agents;
pub mod audit;
pub mod auth;
pub mod billing;
pub mod bundles;
pub mod capabilities;
pub mod change_requests;
pub mod connectors;
pub mod datastore;
pub mod decisions;
pub mod deployments;
pub mod environments;
pub mod error;
pub mod events;
pub mod health;
pub mod idempotency;
pub mod landscape;
pub mod namespaces;
pub mod oauth;
pub mod openapi;
pub mod orgs;
pub mod pagination;
pub mod policies;
pub mod preconditions;
pub mod replay;
pub mod revocations;
pub mod scim;
pub mod sources;
pub mod teams;
pub mod users;
pub mod webhook_subscriptions;
pub mod webhooks;
pub mod webhooks_git;

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
pub fn build_openapi_router(enable_billing: bool) -> OpenApiRouter<Arc<AppState>> {
    let router = OpenApiRouter::with_openapi(openapi::ApiDoc::openapi())
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
        .merge(webhooks_git::routes())
        .merge(webhook_subscriptions::routes())
        .merge(namespaces::routes())
        .merge(environments::routes())
        .merge(change_requests::routes())
        .merge(deployments::routes())
        .merge(decisions::routes())
        .merge(audit::routes())
        .merge(connectors::routes())
        .merge(replay::routes())
        .merge(datastore::routes())
        .merge(landscape::routes())
        .merge(revocations::routes())
        .merge(capabilities::routes())
        .merge(scim::routes());
    // Billing is a STUB (fabricated checkout sessions) — mounted only when the
    // operator explicitly opts in (`server.enable_billing` /
    // `REAPER_ENABLE_BILLING`), and excluded from the OpenAPI contract when
    // off, so the published spec never advertises a flow that does not exist
    // (Plan 06 Phase E, R3-04/ADR-5). When on, the operations are tagged
    // `x-experimental` (see `build_openapi`).
    if enable_billing {
        router.merge(billing::routes())
    } else {
        router
    }
}

/// Build the resource-API router with all routes at their bare paths (state
/// deferred). This is the router that gets nested under `/api/v1`; it is also
/// the body of the transitional bare-root alias.
pub fn build_api_router(enable_billing: bool) -> Router<Arc<AppState>> {
    build_openapi_router(enable_billing).split_for_parts().0
}

/// The single versioned surface (Plan 07 Phase B): the resource API is served
/// **only** under `/api/v1`, while the health/metrics/`openapi.json` probes stay
/// unversioned at the root for orchestrators and contract discovery. The binary
/// layers on auth, middleware, and — when `serve_root_alias` is set — the
/// deprecated bare-root alias.
pub fn build_served_router(enable_billing: bool) -> Router<Arc<AppState>> {
    Router::new()
        .nest("/api/v1", build_api_router(enable_billing))
        .merge(probe_routes())
        // Stamp the RFC 9457 `instance` member onto every problem+json
        // response (R2-08). Applied here — over the full versioned surface —
        // so the member carries the real `/api/v1/...` request path.
        .layer(axum::middleware::from_fn(error::problem_instance))
}

/// Unversioned probes + contract discovery served at the root. `/health`,
/// `/health/*`, `/live`, `/ready`, `/metrics`, `/metrics/prometheus` (from the
/// health module) plus `/openapi.json`. These are also reachable under
/// `/api/v1` via the nested router; the root copies exist so orchestrator
/// probes and spec discovery do not have to know the API version.
pub fn probe_routes() -> Router<Arc<AppState>> {
    let (health_router, _) = health::routes().split_for_parts();
    health_router.route("/openapi.json", axum::routing::get(openapi::serve_openapi))
}

/// Generate the assembled OpenAPI 3.1 document for the control plane. With
/// billing enabled, its operations are tagged `x-experimental: true` so the
/// published contract is explicit that the surface is a stub behind an
/// operator opt-in (Plan 06 Phase E, ADR-5).
pub fn build_openapi(enable_billing: bool) -> utoipa::openapi::OpenApi {
    let mut doc = build_openapi_router(enable_billing).into_openapi();
    if enable_billing {
        mark_billing_experimental(&mut doc);
    }
    doc
}

/// Tag every billing operation `x-experimental: true` in the spec.
fn mark_billing_experimental(doc: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::extensions::ExtensionsBuilder;
    for (path, item) in doc.paths.paths.iter_mut() {
        if !path.contains("/billing") && !path.contains("/webhooks/stripe") {
            continue;
        }
        for op in [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.patch.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let ext = ExtensionsBuilder::new()
                .add("x-experimental", serde_json::json!(true))
                .build();
            op.extensions = Some(ext);
        }
    }
}
