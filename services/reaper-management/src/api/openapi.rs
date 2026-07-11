//! OpenAPI 3.1 contract assembly (Plan 07, Phase A).
//!
//! The control-plane spec is generated from the handlers themselves via
//! `utoipa` + `utoipa-axum`: each module's `routes()` returns an
//! [`OpenApiRouter`](utoipa_axum::router::OpenApiRouter) that carries both the
//! axum route table and the `#[utoipa::path]`-derived operations, so the
//! served routes and the published contract share a single source of truth.
//! The contract-parity test (`tests/api_contract.rs`) enforces that they never
//! drift.
//!
//! [`ApiDoc`] supplies the document-level metadata (info, security scheme, tag
//! descriptions); the per-module routers supply the paths and component
//! schemas.

use std::sync::OnceLock;

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

/// Document-level OpenAPI metadata. Paths and schemas are contributed by the
/// per-module `OpenApiRouter`s assembled in [`super::build_openapi_router`].
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Reaper Management API",
        version = "v1",
        description = "Multi-tenant policy management control plane. All routes are \
                       served under the `/api/v1` prefix and require a bearer JWT \
                       unless documented as a public probe.",
        license(name = "Apache-2.0")
    ),
    servers(
        (url = "/api/v1", description = "Versioned control-plane surface")
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "Health, readiness and metrics probes (public)"),
        (name = "orgs", description = "Organization lifecycle"),
        (name = "policies", description = "Policy CRUD and versioning"),
        (name = "bundles", description = "Signed policy bundles and promotion"),
        (name = "agents", description = "Agent registration and fleet"),
        (name = "sources", description = "Git / API / S3 policy sources"),
        (name = "namespaces", description = "Namespaces"),
        (name = "teams", description = "Teams and membership"),
        (name = "users", description = "User management"),
        (name = "auth", description = "Authentication and sessions"),
        (name = "oauth", description = "OAuth / OIDC integration"),
        (name = "sso", description = "Single sign-on"),
        (name = "scim", description = "SCIM 2.0 provisioning"),
        (name = "deployments", description = "Deployments and rollouts"),
        (name = "decisions", description = "Decision audit query and export"),
        (name = "audit", description = "Management-action audit log"),
        (name = "replay", description = "Decision replay and counterfactuals"),
        (name = "datastore", description = "Entity datastore for ABAC/ReBAC"),
        (name = "landscape", description = "Fleet landscape and metrics"),
        (name = "billing", description = "Billing and usage"),
        (name = "revocations", description = "Bundle revocations"),
        (name = "webhooks", description = "Inbound webhooks and subscriptions"),
        (name = "events", description = "Server-sent event streams")
    )
)]
pub struct ApiDoc;

/// The assembled contract is stable for the process lifetime, so build it once.
static SPEC: OnceLock<utoipa::openapi::OpenApi> = OnceLock::new();

/// `GET /openapi.json` — serve the generated control-plane contract. Public
/// (see the auth gateway allowlist); the document describes the surface only.
pub async fn serve_openapi() -> axum::Json<&'static utoipa::openapi::OpenApi> {
    axum::Json(SPEC.get_or_init(super::build_openapi))
}

/// Registers the `bearer_jwt` security scheme referenced by protected
/// operations.
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
