//! OAuth configuration for Git providers

use serde::{Deserialize, Serialize};

/// OAuth configuration for Git providers
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OAuthConfig {
    pub github: Option<GitHubOAuthConfig>,
    pub gitlab: Option<GitLabOAuthConfig>,
    pub bitbucket: Option<BitbucketOAuthConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "default_github_redirect_uri")]
    pub redirect_uri: String,

    // --- GitHub App (Plan 09 Step 6) -------------------------------------
    // The App replaces the personal-OAuth-token-in-clone-URL shape: cloning
    // authenticates with a short-lived installation token minted at sync
    // time from the App's private key, scoped to the repos the org admin
    // approved at install time. The OAuth fields above remain for user
    // login / repo listing only.
    /// GitHub App id (the numeric id from the App settings page).
    #[serde(default)]
    pub app_id: Option<String>,
    /// GitHub App public slug — builds the install URL
    /// (`https://github.com/apps/{slug}/installations/new`).
    #[serde(default)]
    pub app_slug: Option<String>,
    /// GitHub App private key, PEM (RS256). Keep in a secret store; never
    /// logged. Used only to mint short-lived installation tokens.
    #[serde(default)]
    pub app_private_key: Option<String>,
    /// Webhook secret shared with GitHub for `X-Hub-Signature-256`
    /// verification on `POST /webhooks/git/github` (Plan 09 Step 7).
    #[serde(default)]
    pub webhook_secret: Option<String>,
}

fn default_github_redirect_uri() -> String {
    "http://localhost:8081/auth/github/callback".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitLabOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub base_url: Option<String>, // For self-hosted GitLab
    /// Secret token GitLab echoes verbatim in `X-Gitlab-Token` on webhooks
    /// (Plan 09 Step 7).
    #[serde(default)]
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}
