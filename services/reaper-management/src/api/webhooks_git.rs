//! Git provider push-webhook endpoints (Plan 09 Step 7).
//!
//! `POST /webhooks/git/{provider}` is the low-latency path that turns a push
//! into a sync (the reconciliation loop remains the fallback for missed
//! webhooks). These routes are **public** — authenticated by the provider's
//! signature over the request body, NOT by `RequireAuth` — so verification is
//! mandatory and fail-closed:
//!
//! - GitHub: `X-Hub-Signature-256: sha256=<hmac>` over the raw body, keyed by
//!   the configured `webhook_secret` (constant-time compare).
//! - GitLab: `X-Gitlab-Token` equals the configured `webhook_secret`
//!   (constant-time compare).
//!
//! A missing/invalid signature returns 401 and does NOT sync.
//!
//! The signed body is parsed only enough to identify the repository; the
//! target source is resolved by `repo_full_name`, and each match is synced via
//! the shared `SyncService::trigger_sync` (idempotent per SHA), so a webhook
//! and a poll landing on the same commit never double-apply.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::{info, warn};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{db::repositories::PolicySourceRepository, state::AppState};

type HmacSha256 = Hmac<Sha256>;

/// Build git-webhook routes (public, signature-authenticated).
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new().routes(routes!(git_webhook))
}

/// Verify GitHub's `X-Hub-Signature-256` over `body` with `secret`.
fn github_signature_valid(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    let Some(header) = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(provided) = hex::decode(hex_sig) else {
        return false;
    };

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    // `verify_slice` is constant-time and length-checked.
    expected.ct_eq(provided.as_slice()).into()
}

/// Verify GitLab's `X-Gitlab-Token` equals `secret` (constant-time).
fn gitlab_token_valid(headers: &HeaderMap, secret: &str) -> bool {
    let Some(token) = headers.get("X-Gitlab-Token").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    token.as_bytes().ct_eq(secret.as_bytes()).into()
}

/// Extract "owner/repo" from a push payload for the given provider.
fn extract_repo_full_name(provider: &str, body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    match provider {
        "github" => v
            .get("repository")
            .and_then(|r| r.get("full_name"))
            .and_then(|s| s.as_str())
            .map(String::from),
        "gitlab" => v
            .get("project")
            .and_then(|p| p.get("path_with_namespace"))
            .and_then(|s| s.as_str())
            .map(String::from),
        _ => None,
    }
}

/// Receive a git provider push webhook and trigger a sync (Plan 09 Step 7).
///
/// Public endpoint authenticated by the provider signature over the request
/// body (not bearer auth). A missing/invalid signature returns 401.
#[utoipa::path(
    post,
    path = "/webhooks/git/{provider}",
    tag = "webhooks",
    params(("provider" = String, Path, description = "Git provider: 'github' or 'gitlab'")),
    responses(
        (status = 200, description = "Webhook accepted (sync triggered or no-op)"),
        (status = 401, description = "Missing or invalid signature"),
        (status = 404, description = "Unknown provider")
    )
)]
async fn git_webhook(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let provider = provider.to_lowercase();

    // Resolve the configured webhook secret for this provider. A provider with
    // no secret configured cannot be verified, so it is rejected (fail closed)
    // rather than accepting unauthenticated pushes.
    let secret = match provider.as_str() {
        "github" => state
            .config
            .oauth
            .github
            .as_ref()
            .and_then(|c| c.webhook_secret.clone()),
        "gitlab" => state
            .config
            .oauth
            .gitlab
            .as_ref()
            .and_then(|c| c.webhook_secret.clone()),
        other => {
            warn!(provider = %other, "git webhook for unknown provider");
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown provider"})),
            )
                .into_response();
        }
    };

    let Some(secret) = secret else {
        warn!(provider = %provider, "git webhook received but no webhook_secret configured");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "webhook not configured"})),
        )
            .into_response();
    };

    // Mandatory signature verification — no valid signature, no sync.
    let verified = match provider.as_str() {
        "github" => github_signature_valid(&headers, &body, &secret),
        "gitlab" => gitlab_token_valid(&headers, &secret),
        _ => false,
    };
    if !verified {
        warn!(provider = %provider, "git webhook signature verification FAILED");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid signature"})),
        )
            .into_response();
    }

    // Identify the repo and resolve target sources.
    let Some(repo_full_name) = extract_repo_full_name(&provider, &body) else {
        // Signature was valid but the payload isn't a push we can route (e.g. a
        // ping event) — acknowledge without syncing.
        return (StatusCode::OK, Json(json!({"status": "ignored"}))).into_response();
    };

    let source_repo = PolicySourceRepository::new(&state.db);
    let sources = match source_repo
        .find_git_sources_by_repo(Some(&provider), &repo_full_name)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "webhook source lookup failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "lookup failed"})),
            )
                .into_response();
        }
    };

    if sources.is_empty() {
        info!(repo = %repo_full_name, provider = %provider, "webhook: no matching source");
        return (
            StatusCode::OK,
            Json(json!({"status": "no matching source"})),
        )
            .into_response();
    }

    // Trigger each matching source. Sync is idempotent per SHA, so a webhook
    // racing the reconciliation loop is safe.
    let mut triggered = 0;
    for source in &sources {
        match state.sync_service.trigger_sync(source.id).await {
            Ok(_) => triggered += 1,
            Err(e) => warn!(source_id = %source.id, error = %e, "webhook-triggered sync failed"),
        }
    }

    info!(
        repo = %repo_full_name,
        provider = %provider,
        matched = sources.len(),
        triggered,
        "git webhook processed"
    );
    (
        StatusCode::OK,
        Json(json!({"status": "ok", "sources_triggered": triggered})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    fn github_sig(body: &[u8], secret: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn github_signature_roundtrip() {
        let body = br#"{"repository":{"full_name":"acme/policies"}}"#;
        let secret = "shhh";
        let sig = github_sig(body, secret);
        assert!(github_signature_valid(
            &hdrs(&[("X-Hub-Signature-256", &sig)]),
            body,
            secret
        ));
    }

    #[test]
    fn github_signature_rejects_wrong_secret_and_tamper() {
        let body = br#"{"repository":{"full_name":"acme/policies"}}"#;
        let sig = github_sig(body, "right");
        // Wrong secret.
        assert!(!github_signature_valid(
            &hdrs(&[("X-Hub-Signature-256", &sig)]),
            body,
            "wrong"
        ));
        // Tampered body.
        assert!(!github_signature_valid(
            &hdrs(&[("X-Hub-Signature-256", &sig)]),
            br#"{"repository":{"full_name":"evil/repo"}}"#,
            "right"
        ));
        // Missing header.
        assert!(!github_signature_valid(&hdrs(&[]), body, "right"));
        // Malformed header (no sha256= prefix).
        assert!(!github_signature_valid(
            &hdrs(&[("X-Hub-Signature-256", "deadbeef")]),
            body,
            "right"
        ));
    }

    #[test]
    fn gitlab_token_constant_time_compare() {
        assert!(gitlab_token_valid(
            &hdrs(&[("X-Gitlab-Token", "secret")]),
            "secret"
        ));
        assert!(!gitlab_token_valid(
            &hdrs(&[("X-Gitlab-Token", "nope")]),
            "secret"
        ));
        assert!(!gitlab_token_valid(&hdrs(&[]), "secret"));
    }

    #[test]
    fn repo_extraction_per_provider() {
        assert_eq!(
            extract_repo_full_name("github", br#"{"repository":{"full_name":"a/b"}}"#),
            Some("a/b".to_string())
        );
        assert_eq!(
            extract_repo_full_name("gitlab", br#"{"project":{"path_with_namespace":"g/p"}}"#),
            Some("g/p".to_string())
        );
        assert_eq!(extract_repo_full_name("github", b"not json"), None);
        assert_eq!(extract_repo_full_name("github", br#"{"ping":true}"#), None);
    }
}
