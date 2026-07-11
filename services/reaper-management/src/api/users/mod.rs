//! User authentication API endpoints
//!
//! Provides endpoints for user signup, login, logout, and password management.

mod auth;
mod helpers;
mod members;
pub mod types;
mod verification;

use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

pub use types::*;

/// Build user auth routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Public endpoints (no auth required)
        .routes(routes!(auth::signup))
        .routes(routes!(auth::login))
        .routes(routes!(auth::request_password_reset))
        .routes(routes!(auth::reset_password))
        .routes(routes!(verification::verify_email))
        // Authenticated endpoints
        .routes(routes!(auth::logout))
        .routes(routes!(auth::get_current_user))
        .routes(routes!(auth::change_password))
        .routes(routes!(verification::resend_verification))
        // Org member management
        .routes(routes!(members::list_org_members, members::invite_member))
        .routes(routes!(members::get_member, members::remove_member))
        .routes(routes!(members::update_member_role))
}
