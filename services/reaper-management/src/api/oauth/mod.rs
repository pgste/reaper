//! OAuth integration for Git providers
//!
//! Provides OAuth flows for connecting to GitHub, GitLab, and Bitbucket
//! to enable seamless repository access for policy sources.

mod connections;
mod github;
pub(crate) mod helpers;
mod types;

use std::sync::Arc;

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

// Re-export types for external use
pub use types::{
    AuthorizeParams, CallbackParams, ConnectionSummary, CreateSourceFromGitHubRequest, GitHubRepo,
    ListConnectionsResponse, OAuthConnection,
};

/// Build OAuth routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // GitHub OAuth
        .routes(routes!(github::github_authorize))
        .routes(routes!(github::github_callback))
        // GitHub App install (Plan 09 Step 6)
        .routes(routes!(github::github_app_install))
        .routes(routes!(github::github_app_setup_callback))
        // OAuth connections management
        .routes(routes!(
            connections::list_connections,
            connections::create_connection
        ))
        .routes(routes!(
            connections::get_connection,
            connections::delete_connection
        ))
        // GitHub repo listing
        .routes(routes!(github::list_github_repos))
        .routes(routes!(github::create_source_from_github))
}
