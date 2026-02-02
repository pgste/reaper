//! OAuth types and domain models

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// OAuth connection stored in database
#[derive(Debug, Clone, Serialize)]
pub struct OAuthConnection {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub provider: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// OAuth state token for CSRF protection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OAuthState {
    pub org_slug: String,
    pub user_id: String,
    pub nonce: String,
    pub created_at: i64,
}

impl OAuthState {
    pub fn new(org_slug: &str, user_id: &str) -> Self {
        let mut rng = rand::thread_rng();
        let nonce: String = (0..16)
            .map(|_| rng.gen_range(b'a'..=b'z') as char)
            .collect();
        Self {
            org_slug: org_slug.to_string(),
            user_id: user_id.to_string(),
            nonce,
            created_at: Utc::now().timestamp(),
        }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            json.as_bytes(),
        )
    }

    pub fn decode(encoded: &str) -> Option<Self> {
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            encoded,
        )
        .ok()?;
        let json = String::from_utf8(bytes).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn is_valid(&self) -> bool {
        // State tokens expire after 10 minutes
        let now = Utc::now().timestamp();
        now - self.created_at < 600
    }
}

/// Query params for authorize endpoint
#[derive(Debug, Deserialize)]
pub struct AuthorizeParams {
    pub org: String,
}

/// Query params for callback endpoint
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
}

/// GitHub access token response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct GitHubTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

/// GitHub user response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct GitHubUser {
    pub id: i64,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

/// GitHub repository
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub default_branch: String,
    pub description: Option<String>,
}

/// Request to create a source from GitHub repo
#[derive(Debug, Deserialize)]
pub struct CreateSourceFromGitHubRequest {
    pub repo_full_name: String, // e.g., "owner/repo"
    pub branch: Option<String>,
    pub path: Option<String>, // Path within repo to policies
    pub name: Option<String>, // Display name for the source
}

/// List connections response
#[derive(Debug, Serialize)]
pub struct ListConnectionsResponse {
    pub connections: Vec<ConnectionSummary>,
}

/// Connection summary (without sensitive data)
#[derive(Debug, Serialize)]
pub struct ConnectionSummary {
    pub provider: String,
    pub provider_username: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_state_encode_decode() {
        let state = OAuthState::new("acme", "user-123");
        let encoded = state.encode();
        let decoded = OAuthState::decode(&encoded).unwrap();

        assert_eq!(decoded.org_slug, "acme");
        assert_eq!(decoded.user_id, "user-123");
        assert!(decoded.is_valid());
    }
}
