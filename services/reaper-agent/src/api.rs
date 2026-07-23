//! OpenAPI 3.1 contract for the Reaper Agent data plane (Plan 07, Phase A).
//!
//! This module assembles the agent's OpenAPI document **for documentation
//! only** — it is never used to serve traffic. The live enforcement router is
//! built in `main.rs` and is intentionally left untouched so the hot
//! evaluation path (routing, the 16 MB eval body-limit, auth, panic guard) is
//! byte-for-byte what it was. The `#[utoipa::path]` attributes on the handlers
//! are compile-time metadata (utoipa generates separate `__path_*` items); they
//! do not wrap or alter the handler functions.
//!
//! Drift between this document and the served routes is caught by
//! `tests/api_contract.rs`, which asserts every handler routed in `main.rs` has
//! a documented operation and vice-versa.

use std::sync::Arc;
use std::sync::OnceLock;

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::handlers;
use crate::state::AgentState;

/// Document-level metadata for the agent contract.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Reaper Agent API",
        version = "v1",
        description = "Sub-microsecond policy enforcement data plane. Evaluation, \
                       policy/bundle deployment, managed entity data, and decision \
                       audit. All routes require inbound agent auth except the \
                       health/metrics probes.",
        license(name = "Apache-2.0")
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "Health, readiness, liveness and metrics probes (public)"),
        (name = "evaluation", description = "Policy evaluation — the enforcement hot path"),
        (name = "policies", description = "Policy and bundle deployment (from the platform)"),
        (name = "data", description = "Managed entity data load and synchronization"),
        (name = "entities", description = "Entity CRUD"),
        (name = "decisions", description = "OPA-style decision audit log")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_jwt",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

/// Assemble the agent's OpenAPI document. The `OpenApiRouter` here mirrors the
/// route table in `main.rs` purely to collect the `#[utoipa::path]` operations;
/// its axum side is discarded (`into_openapi`). Keep this list in lockstep with
/// `main.rs` — the contract-parity test fails if they diverge.
fn openapi_router() -> OpenApiRouter<Arc<AgentState>> {
    // Handlers are referenced through their defining submodule so the
    // `routes!` macro can find the sibling `__path_*` item the
    // `#[utoipa::path]` attribute generates there (the `handlers::*`
    // re-exports carry the fn but not that generated item).
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        // Health / metrics probes
        .routes(routes!(handlers::health::health_check))
        .routes(routes!(handlers::health::readiness_check))
        .routes(routes!(handlers::health::liveness_check))
        .routes(routes!(handlers::health::metrics))
        // Evaluation (hot path)
        .routes(routes!(handlers::evaluate::evaluate_policy))
        .routes(routes!(handlers::evaluate::fast_evaluate_policy))
        .routes(routes!(handlers::evaluate::batch_evaluate_policy))
        .routes(routes!(handlers::check::check_document))
        .routes(routes!(handlers::admission::admission_review))
        // Managed data
        .routes(routes!(handlers::data::load_data_handler))
        .routes(routes!(handlers::data::load_data_stream_handler))
        .routes(routes!(handlers::data::sync_data))
        .routes(routes!(handlers::data::deploy_data_version))
        .routes(routes!(handlers::data::confirm_data_version))
        .routes(routes!(handlers::data::apply_data_deltas))
        // Policy / bundle deployment
        .routes(routes!(handlers::policies::deploy_policy))
        .routes(routes!(handlers::policies::deploy_compiled_policy))
        .routes(routes!(handlers::policies::list_policies))
        .routes(routes!(handlers::policies::get_policy_versions))
        .routes(routes!(handlers::policies::get_policy_current_version))
        .routes(routes!(handlers::policies::deploy_bundle))
        .routes(routes!(handlers::policies::load_bundles_atomic))
        // Entity CRUD (GET + DELETE share the {type}/{id} path)
        .routes(routes!(handlers::entities::upsert_entity_handler))
        .routes(routes!(
            handlers::entities::get_entity_handler,
            handlers::entities::delete_entity_handler
        ))
        .routes(routes!(handlers::entities::list_entities_handler))
        .routes(routes!(handlers::entities::batch_upsert_handler))
        // Decision audit
        .routes(routes!(handlers::decisions::get_decisions))
        .routes(routes!(handlers::decisions::get_decision_stats))
        .routes(routes!(handlers::decisions::export_decisions))
        .routes(routes!(handlers::decisions::get_decision_by_id))
        // Debug (served only when debug endpoints are enabled)
        .routes(routes!(handlers::entities::debug_datastore))
}

/// Generate the assembled OpenAPI 3.1 document for the agent.
pub fn build_openapi() -> utoipa::openapi::OpenApi {
    openapi_router().into_openapi()
}

/// The document is stable for the process lifetime, so build it once.
static SPEC: OnceLock<utoipa::openapi::OpenApi> = OnceLock::new();

/// `GET /openapi.json` — serve the generated agent contract. Public; the
/// document describes the surface, not data. This is the only route the
/// contract work adds to the served router, and it is not on the hot path.
pub async fn serve_openapi() -> axum::Json<&'static utoipa::openapi::OpenApi> {
    axum::Json(SPEC.get_or_init(build_openapi))
}
