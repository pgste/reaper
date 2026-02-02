//! User authentication API endpoints
//!
//! Provides endpoints for user signup, login, logout, and password management.

mod auth;
mod helpers;
mod members;
pub mod types;
mod verification;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::state::AppState;

pub use types::*;

/// Build user auth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Public endpoints (no auth required)
        .route("/auth/signup", post(auth::signup))
        .route("/auth/login", post(auth::login))
        .route("/auth/password/reset-request", post(auth::request_password_reset))
        .route("/auth/password/reset", post(auth::reset_password))
        .route("/auth/email/verify", post(verification::verify_email))
        // Authenticated endpoints
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/password/change", post(auth::change_password))
        .route("/auth/email/resend", post(verification::resend_verification))
        // Org member management
        .route(
            "/orgs/{org}/members",
            get(members::list_org_members).post(members::invite_member),
        )
        .route(
            "/orgs/{org}/members/{user_id}",
            get(members::get_member).delete(members::remove_member),
        )
        .route("/orgs/{org}/members/{user_id}/role", post(members::update_member_role))
}
