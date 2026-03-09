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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}
