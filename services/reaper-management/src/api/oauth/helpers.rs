//! OAuth helper functions

use axum::http::header::HeaderMap;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    auth::users::SessionRepository,
    state::AppState,
};

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
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;

    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT access_token_encrypted, token_expires_at FROM oauth_connections WHERE org_id = ? AND provider = 'github'",
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
    );

    Ok(token)
}

/// Simple token encryption (in production, use a proper encryption library)
pub(super) fn encrypt_token(token: &str, key: &str) -> String {
    // This is a simple obfuscation - in production use proper encryption like AES-GCM
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let key_hash = hasher.finalize();

    let encrypted: Vec<u8> = token
        .bytes()
        .zip(key_hash.iter().cycle())
        .map(|(t, k)| t ^ k)
        .collect();

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &encrypted)
}

/// Simple token decryption
pub(super) fn decrypt_token(encrypted: &str, key: &str) -> String {
    let encrypted_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encrypted)
            .unwrap_or_default();

    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let key_hash = hasher.finalize();

    let decrypted: Vec<u8> = encrypted_bytes
        .iter()
        .zip(key_hash.iter().cycle())
        .map(|(e, k)| e ^ k)
        .collect();

    String::from_utf8(decrypted).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_encryption_roundtrip() {
        let key = "test-secret-key";
        let token = "ghp_abcdefghijklmnop";

        let encrypted = encrypt_token(token, key);
        let decrypted = decrypt_token(&encrypted, key);

        assert_eq!(decrypted, token);
        assert_ne!(encrypted, token);
    }
}
