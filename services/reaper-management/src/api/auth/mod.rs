//! Authentication API endpoints
//!
//! Provides endpoints for API key management, JWKS configuration, and client certificates.

mod api_keys;
mod certificates;
mod jwks;
mod sso;
pub mod types;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::state::AppState;

pub use types::*;

/// Build auth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Token operations
        .route("/auth/token/refresh", post(api_keys::refresh_token))
        // API key management (requires auth)
        .route(
            "/orgs/{org}/api-keys",
            get(api_keys::list_api_keys).post(api_keys::create_api_key),
        )
        .route(
            "/orgs/{org}/api-keys/{key_id}",
            get(api_keys::get_api_key).delete(api_keys::delete_api_key),
        )
        .route(
            "/orgs/{org}/api-keys/{key_id}/revoke",
            post(api_keys::revoke_api_key),
        )
        // JWKS configuration management
        .route(
            "/orgs/{org}/auth/jwks",
            get(jwks::list_jwks_configs).post(jwks::create_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}",
            get(jwks::get_jwks_config).delete(jwks::delete_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}/activate",
            post(jwks::activate_jwks_config),
        )
        .route(
            "/orgs/{org}/auth/jwks/{config_id}/deactivate",
            post(jwks::deactivate_jwks_config),
        )
        // Client certificate management (mTLS)
        .route(
            "/orgs/{org}/auth/certificates",
            get(certificates::list_certificates).post(certificates::register_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}",
            get(certificates::get_certificate).delete(certificates::delete_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/revoke",
            post(certificates::revoke_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/bind",
            post(certificates::bind_certificate),
        )
        .route(
            "/orgs/{org}/auth/certificates/{cert_id}/unbind",
            post(certificates::unbind_certificate),
        )
        // Enterprise SSO (OIDC login + per-org IdP config)
        .merge(sso::routes())
}
