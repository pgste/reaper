//! OAuth integration for Git providers
//!
//! Provides OAuth flows for connecting to GitHub, GitLab, and Bitbucket
//! to enable seamless repository access for policy sources.

mod connections;
mod github;
mod helpers;
mod types;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::state::AppState;

// Re-export types for external use
pub use types::{
    AuthorizeParams, CallbackParams, ConnectionSummary, CreateSourceFromGitHubRequest, GitHubRepo,
    ListConnectionsResponse, OAuthConnection,
};

/// Build OAuth routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // GitHub OAuth
        .route("/auth/github/authorize", get(github::github_authorize))
        .route("/auth/github/callback", get(github::github_callback))
        // OAuth connections management
        .route(
            "/orgs/{org}/oauth/connections",
            get(connections::list_connections).post(connections::create_connection),
        )
        .route(
            "/orgs/{org}/oauth/connections/{provider}",
            get(connections::get_connection).delete(connections::delete_connection),
        )
        // GitHub repo listing
        .route("/orgs/{org}/github/repos", get(github::list_github_repos))
        .route(
            "/orgs/{org}/sources/github",
            post(github::create_source_from_github),
        )
}
