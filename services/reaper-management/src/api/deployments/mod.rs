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

/// Authorize a MUTATING deployment action (rollout / rollback / approve-wave /
/// cancel / pin / strategy / rollback-config write) — SEC R2-1.
///
/// Fleet propagation is a privileged operation, so org membership alone is not
/// enough: the caller must hold a deploy scope (`deployment:write`, or
/// `bundle:promote` — a promote-capable pipeline token can also roll out what
/// it promoted) with `org:admin` as the human-role fallback, mirroring
/// `change_requests::authorize`. Then the usual tenancy check: the caller's
/// org must match, with the global `admin` scope as the platform-operator
/// escape hatch.
///
/// Read endpoints (status, lists, trigger checks) and the agent-facing
/// deployment acknowledgement deliberately keep the plain membership gate.
pub(crate) async fn authorize_deploy(
    state: &crate::state::AppState,
    user: &crate::auth::middleware::AuthenticatedUser,
    org: &str,
    action: &str,
) -> Result<crate::domain::organization::Organization, crate::api::error::ApiError> {
    use crate::auth::scopes::Scope;

    if !user.has_any_permission(&[
        Scope::DeploymentWrite,
        Scope::BundlePromote,
        Scope::OrgAdmin,
    ]) {
        return Err(crate::api::error::ApiError::Forbidden(format!(
            "Missing {} scope (or {} / {}): required to {}",
            Scope::DeploymentWrite.as_str(),
            Scope::BundlePromote.as_str(),
            Scope::OrgAdmin.as_str(),
            action,
        )));
    }

    let organization = crate::api::orgs::resolve_org(
        &crate::db::repositories::OrganizationRepository::new(&state.db),
        org,
    )
    .await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(crate::api::error::ApiError::Forbidden(format!(
            "Cannot {} for other organizations",
            action
        )));
    }
    Ok(organization)
}
