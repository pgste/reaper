//! OAuth types and domain models

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::ToSchema;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

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

    /// Encode as `base64(payload).base64(HMAC-SHA256(payload))`.
    ///
    /// The HMAC binds the state to the server secret so a client cannot forge a
    /// state naming another user/org — the callback only trusts a state whose
    /// signature verifies (CSRF / connection-hijack protection).
    pub fn encode(&self, secret: &[u8]) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            json.as_bytes(),
        );

        let sig = Self::sign(&payload, secret);
        format!("{payload}.{sig}")
    }

    /// Decode and verify the HMAC. Returns `None` if the signature is missing,
    /// malformed, or does not verify — i.e. the state was tampered with or forged.
    pub fn decode(encoded: &str, secret: &[u8]) -> Option<Self> {
        let (payload, sig) = encoded.split_once('.')?;

        // Constant-time verification via HMAC verify.
        let mut mac = HmacSha256::new_from_slice(secret).ok()?;
        mac.update(payload.as_bytes());
        let expected =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, sig).ok()?;
        mac.verify_slice(&expected).ok()?;

        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
                .ok()?;
        let json = String::from_utf8(bytes).ok()?;
        serde_json::from_str(&json).ok()
    }

    fn sign(payload: &str, secret: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            mac.finalize().into_bytes(),
        )
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSourceFromGitHubRequest {
    pub repo_full_name: String, // e.g., "owner/repo"
    pub branch: Option<String>,
    pub path: Option<String>, // Path within repo to policies
    pub name: Option<String>, // Display name for the source
}

/// List connections response
#[derive(Debug, Serialize, ToSchema)]
pub struct ListConnectionsResponse {
    pub connections: Vec<ConnectionSummary>,
}

/// Connection summary (without sensitive data)
#[derive(Debug, Serialize, ToSchema)]
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
        let secret = b"server-secret";
        let state = OAuthState::new("acme", "user-123");
        let encoded = state.encode(secret);
        let decoded = OAuthState::decode(&encoded, secret).unwrap();

        assert_eq!(decoded.org_slug, "acme");
        assert_eq!(decoded.user_id, "user-123");
        assert!(decoded.is_valid());
    }

    #[test]
    fn test_oauth_state_rejects_forged_or_tampered() {
        let secret = b"server-secret";
        let encoded = OAuthState::new("acme", "user-123").encode(secret);

        // Wrong secret (attacker does not know it) must fail.
        assert!(OAuthState::decode(&encoded, b"wrong-secret").is_none());

        // Tampering with the payload to name a different user must fail.
        let (_payload, sig) = encoded.split_once('.').unwrap();
        let forged_payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"org_slug":"victim","user_id":"attacker","nonce":"x","created_at":0}"#,
        );
        let forged = format!("{forged_payload}.{sig}");
        assert!(OAuthState::decode(&forged, secret).is_none());

        // Missing signature must fail.
        assert!(OAuthState::decode("justpayload", secret).is_none());
    }
}
