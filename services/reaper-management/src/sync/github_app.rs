//! GitHub App authentication (Plan 09 Step 6).
//!
//! Replaces the personal-OAuth-token-in-clone-URL shape (Product F3): instead
//! of embedding a long-lived user PAT in the stored source config, an org
//! installs the Reaper GitHub App on the repos it wants synced. At sync time
//! the control plane mints a **short-lived installation access token** from
//! the App's private key, scoped to that installation, and uses it for that
//! one clone. Nothing durable holds a credential, and revoking the connecting
//! user's PAT no longer orphans the source.
//!
//! Flow:
//!  1. Sign a ~9-minute App JWT (RS256) with the App private key. `iss` is the
//!     App id; GitHub authenticates the App from the JWT's signature.
//!  2. `POST /app/installations/{id}/access_tokens` with that JWT → a token
//!     valid ~1h, scoped to the installation.
//!
//! The token is used immediately and never persisted.

use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitHubAppError {
    #[error("GitHub App is not configured (need app_id + app_private_key)")]
    NotConfigured,
    #[error("Invalid App private key: {0}")]
    InvalidKey(String),
    #[error("Failed to sign App JWT: {0}")]
    Jwt(String),
    #[error("GitHub API error: {0}")]
    Api(String),
}

/// App JWT claims. GitHub requires `iat` slightly in the past (clock skew) and
/// `exp` ≤ 10 minutes out; we use 9.
#[derive(Debug, Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// A minted installation token and its expiry (never persisted).
#[derive(Debug, Clone)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct InstallationTokenResponse {
    token: String,
    #[serde(default)]
    expires_at: Option<String>,
}

/// Minimal GitHub App client: holds the App id + private key and mints
/// per-installation tokens on demand.
pub struct GitHubAppClient {
    app_id: String,
    encoding_key: EncodingKey,
    api_base: String,
}

impl GitHubAppClient {
    /// Build from config. Returns `NotConfigured` if either the app id or the
    /// private key is absent, so callers can cleanly fall back to OAuth.
    pub fn from_config(
        app_id: Option<&str>,
        app_private_key: Option<&str>,
    ) -> Result<Self, GitHubAppError> {
        let app_id = app_id.ok_or(GitHubAppError::NotConfigured)?;
        let pem = app_private_key.ok_or(GitHubAppError::NotConfigured)?;
        if app_id.is_empty() || pem.is_empty() {
            return Err(GitHubAppError::NotConfigured);
        }
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| GitHubAppError::InvalidKey(e.to_string()))?;
        Ok(Self {
            app_id: app_id.to_string(),
            encoding_key,
            api_base: "https://api.github.com".to_string(),
        })
    }

    /// Override the API base (for pointing at a mock in integration tests).
    #[allow(dead_code)]
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    /// Sign a short-lived App JWT (RS256). `now_ts` is injected for testability;
    /// production passes `Utc::now().timestamp()`.
    fn app_jwt_at(&self, now_ts: i64) -> Result<String, GitHubAppError> {
        let claims = AppClaims {
            // 60s in the past to tolerate clock skew (GitHub's guidance).
            iat: now_ts - 60,
            exp: now_ts + 9 * 60,
            iss: self.app_id.clone(),
        };
        encode(&Header::new(Algorithm::RS256), &claims, &self.encoding_key)
            .map_err(|e| GitHubAppError::Jwt(e.to_string()))
    }

    /// Mint a short-lived installation access token for `installation_id`.
    pub async fn installation_token(
        &self,
        installation_id: &str,
    ) -> Result<InstallationToken, GitHubAppError> {
        let jwt = self.app_jwt_at(Utc::now().timestamp())?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.api_base, installation_id
        );

        let resp = crate::http::http_client_default()
            .map_err(|e| GitHubAppError::Api(format!("HTTP client build failed: {e}")))?
            .post(&url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "Reaper-Management")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| GitHubAppError::Api(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubAppError::Api(format!(
                "installation token request failed ({status}): {body}"
            )));
        }

        let parsed: InstallationTokenResponse = resp
            .json()
            .await
            .map_err(|e| GitHubAppError::Api(format!("parse token response: {e}")))?;

        Ok(InstallationToken {
            token: parsed.token,
            expires_at: parsed
                .expires_at
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway 2048-bit RSA key in PKCS#8 PEM, for signing tests only.
    const TEST_RSA_PEM: &str = include_str!("testdata/test_rsa_pkcs8.pem");

    #[test]
    fn not_configured_without_id_or_key() {
        assert!(matches!(
            GitHubAppClient::from_config(None, Some(TEST_RSA_PEM)),
            Err(GitHubAppError::NotConfigured)
        ));
        assert!(matches!(
            GitHubAppClient::from_config(Some("123"), None),
            Err(GitHubAppError::NotConfigured)
        ));
        assert!(matches!(
            GitHubAppClient::from_config(Some(""), Some(TEST_RSA_PEM)),
            Err(GitHubAppError::NotConfigured)
        ));
    }

    #[test]
    fn invalid_key_is_rejected() {
        assert!(matches!(
            GitHubAppClient::from_config(Some("123"), Some("not a pem")),
            Err(GitHubAppError::InvalidKey(_))
        ));
    }

    #[test]
    fn signs_a_verifiable_app_jwt_with_expected_claims() {
        use jsonwebtoken::{decode, DecodingKey, Validation};

        let client = GitHubAppClient::from_config(Some("42"), Some(TEST_RSA_PEM)).unwrap();
        let now = 1_700_000_000;
        let token = client.app_jwt_at(now).unwrap();

        // Verify with the matching public key (extracted from the same PEM).
        let pub_pem = include_str!("testdata/test_rsa_public.pem");
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = false;
        validation.set_required_spec_claims(&["exp"]);
        let decoded = decode::<serde_json::Value>(
            &token,
            &DecodingKey::from_rsa_pem(pub_pem.as_bytes()).unwrap(),
            &validation,
        )
        .expect("App JWT must verify against the App public key");

        assert_eq!(decoded.claims["iss"], "42");
        assert_eq!(decoded.claims["iat"], now - 60);
        assert_eq!(decoded.claims["exp"], now + 540);
    }
}
