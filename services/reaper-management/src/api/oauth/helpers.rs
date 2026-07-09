//! OAuth helper functions

use axum::http::header::HeaderMap;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use chrono::{DateTime, Utc};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    auth::users::SessionRepository,
    state::AppState,
};

/// Domain-separation label so the token-encryption key derived from the master
/// secret is independent of the JWT signing key derived from the same secret.
const TOKEN_KEY_INFO: &[u8] = b"reaper:oauth-token-encryption:v1";
/// XChaCha20-Poly1305 nonce length (extended nonce — safe to pick at random).
const NONCE_LEN: usize = 24;

/// Get user ID from session token in headers
pub(super) async fn get_user_id_from_session(
    state: &AppState,
    headers: &HeaderMap,
) -> ApiResult<Uuid> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| {
            ApiError::Unauthorized("Missing or invalid Authorization header".to_string())
        })?;

    if !token.starts_with("rst_") {
        return Err(ApiError::Unauthorized(
            "Invalid session token format".to_string(),
        ));
    }

    let session_repo = SessionRepository::new(&state.db);
    let session = session_repo
        .find_by_token(token)
        .await
        .map_err(|_| ApiError::Unauthorized("Invalid session".to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("Invalid session".to_string()))?;

    Ok(session.user_id)
}

/// Get GitHub access token for an org
pub(super) async fn get_github_token(state: &AppState, org_id: Uuid) -> ApiResult<String> {
    let pool = state.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT access_token_encrypted, token_expires_at FROM oauth_connections WHERE org_id = $1 AND provider = 'github'",
    )
    .bind(org_id.to_string())
    .fetch_optional(pool)
    .await?;

    let (encrypted_token, expires_at) = row.ok_or_else(|| {
        ApiError::BadRequest(
            "GitHub not connected. Please authorize GitHub access first.".to_string(),
        )
    })?;

    // Check if token is expired
    if let Some(expires_at) = expires_at {
        if let Ok(expires) = DateTime::parse_from_rfc3339(&expires_at) {
            if expires.with_timezone(&Utc) < Utc::now() {
                return Err(ApiError::BadRequest(
                    "GitHub token expired. Please reconnect GitHub.".to_string(),
                ));
            }
        }
    }

    // Decrypt token
    let token = decrypt_token(
        &encrypted_token,
        &state.config.auth.jwt_secret.clone().unwrap_or_default(),
    )?;

    Ok(token)
}

/// Derive a 32-byte AEAD key from the master secret with domain separation.
///
/// Returns an error if the master secret is empty — we must never fall back to
/// encrypting tokens under a known/empty key.
fn derive_key(master_secret: &str) -> ApiResult<[u8; 32]> {
    if master_secret.is_empty() {
        return Err(ApiError::Internal(
            "Token encryption key is not configured (auth.jwt_secret is empty)".to_string(),
        ));
    }
    let hk = Hkdf::<Sha256>::new(None, master_secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(TOKEN_KEY_INFO, &mut key)
        .map_err(|_| ApiError::Internal("Failed to derive token encryption key".to_string()))?;
    Ok(key)
}

/// Encrypt an OAuth token with XChaCha20-Poly1305 (authenticated encryption).
///
/// Output is base64(nonce ‖ ciphertext‖tag). A fresh random 24-byte nonce is
/// used per call, so encrypting the same token twice yields different output and
/// there is no reusable keystream (unlike the previous XOR obfuscation).
pub(crate) fn encrypt_token(token: &str, master_secret: &str) -> ApiResult<String> {
    let key = derive_key(master_secret)?;
    let cipher = XChaCha20Poly1305::new((&key).into());

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, token.as_bytes())
        .map_err(|_| ApiError::Internal("Token encryption failed".to_string()))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &out,
    ))
}

/// Decrypt a token produced by [`encrypt_token`].
///
/// Fails (rather than silently returning an empty string) on any tampering,
/// wrong key, or legacy-format value — callers surface this as "reconnect".
pub(crate) fn decrypt_token(encrypted: &str, master_secret: &str) -> ApiResult<String> {
    let key = derive_key(master_secret)?;
    let cipher = XChaCha20Poly1305::new((&key).into());

    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encrypted)
        .map_err(|_| ApiError::BadRequest("Malformed stored token".to_string()))?;
    if data.len() <= NONCE_LEN {
        return Err(ApiError::BadRequest("Malformed stored token".to_string()));
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| {
            ApiError::BadRequest(
                "Stored token could not be decrypted. Please reconnect GitHub.".to_string(),
            )
        })?;

    String::from_utf8(plaintext)
        .map_err(|_| ApiError::Internal("Decrypted token was not valid UTF-8".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_encryption_roundtrip() {
        let key = "test-secret-key-that-is-not-empty";
        let token = "ghp_abcdefghijklmnop";

        let encrypted = encrypt_token(token, key).unwrap();
        let decrypted = decrypt_token(&encrypted, key).unwrap();

        assert_eq!(decrypted, token);
        assert_ne!(encrypted, token);
    }

    #[test]
    fn test_nonce_randomization() {
        // Same token + key must produce different ciphertext each time (no
        // reusable keystream).
        let key = "another-secret-key";
        let token = "ghp_secretvalue";
        assert_ne!(
            encrypt_token(token, key).unwrap(),
            encrypt_token(token, key).unwrap()
        );
    }

    #[test]
    fn test_empty_key_rejected() {
        assert!(encrypt_token("tok", "").is_err());
    }

    #[test]
    fn test_tamper_detection() {
        let key = "secret-key-for-tamper-test";
        let mut enc = encrypt_token("ghp_x", key).unwrap();
        // Flip a character in the base64 to simulate tampering.
        enc.insert(0, 'A');
        assert!(decrypt_token(&enc, key).is_err());
    }

    #[test]
    fn test_wrong_key_fails() {
        let enc = encrypt_token("ghp_x", "key-one-original").unwrap();
        assert!(decrypt_token(&enc, "key-two-different").is_err());
    }
}
