//! Authentication API endpoints
//!
//! Provides endpoints for API key management, JWKS configuration, and client certificates.

mod api_keys;
mod certificates;
mod jwks;
mod sso;
pub mod types;

use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

pub use types::*;

/// Build auth routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Token operations
        .routes(routes!(api_keys::refresh_token))
        // API key management (requires auth)
        .routes(routes!(api_keys::list_api_keys, api_keys::create_api_key))
        .routes(routes!(api_keys::get_api_key, api_keys::delete_api_key))
        .routes(routes!(api_keys::revoke_api_key))
        // JWKS configuration management
        .routes(routes!(jwks::list_jwks_configs, jwks::create_jwks_config))
        .routes(routes!(jwks::get_jwks_config, jwks::delete_jwks_config))
        .routes(routes!(jwks::activate_jwks_config))
        .routes(routes!(jwks::deactivate_jwks_config))
        // Client certificate management (mTLS)
        .routes(routes!(
            certificates::list_certificates,
            certificates::register_certificate
        ))
        .routes(routes!(
            certificates::get_certificate,
            certificates::delete_certificate
        ))
        .routes(routes!(certificates::revoke_certificate))
        .routes(routes!(certificates::bind_certificate))
        .routes(routes!(certificates::unbind_certificate))
        // Enterprise SSO (OIDC login + per-org IdP config)
        .merge(sso::routes())
}
