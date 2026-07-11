//! `Idempotency-Key` support for propagation-triggering POSTs (Plan 07,
//! Phase D — promote/rollback, rollout-create, org-create).
//!
//! Automation retrying a timed-out POST must not double-apply the side effect.
//! A client sends `Idempotency-Key: <opaque>` with the request; the first
//! execution claims the key (a `pending` row under a DB unique constraint —
//! the database arbitrates concurrent duplicates), runs the operation, and
//! stores the response. A replay of the same key within the retention window
//! returns the stored response verbatim (marked with
//! `Idempotency-Replayed: true`) and triggers nothing.
//!
//! ADR-6: the key is bound to a fingerprint of the request identity — the same
//! key with a *different* request is a 422 (client bug), never a replay. A
//! request without the header runs exactly as before; keys are opt-in.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::future::Future;

use crate::db::repositories::idempotency::{ClaimOutcome, IdempotencyRepository};
use crate::db::Database;

use super::error::{ApiError, ApiResult};

/// How long a completed key replays before it ages out (the plan's 24–72 h
/// band). Env override: `REAPER_IDEMPOTENCY_RETENTION_SECS` (read by the
/// sweeper in `main.rs` as well, so both ends stay in step).
pub const DEFAULT_RETENTION_SECS: i64 = 48 * 3600;

pub fn retention() -> chrono::Duration {
    let secs = std::env::var("REAPER_IDEMPOTENCY_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_SECS);
    chrono::Duration::seconds(secs.max(60))
}

/// Fingerprint the request identity (operation + path parts + body) so a key
/// cannot be replayed against a different request (ADR-6).
pub fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update([0x1f]); // unit separator: "ab"+"c" ≠ "a"+"bc"
    }
    hex::encode(hasher.finalize())
}

/// Read the `Idempotency-Key` header, rejecting unusable values.
fn key_from(headers: &HeaderMap) -> ApiResult<Option<String>> {
    let Some(raw) = headers.get("Idempotency-Key") else {
        return Ok(None);
    };
    let key = raw
        .to_str()
        .map_err(|_| ApiError::BadRequest("Idempotency-Key must be visible ASCII".to_string()))?
        .trim();
    if key.is_empty() || key.len() > 255 {
        return Err(ApiError::BadRequest(
            "Idempotency-Key must be 1–255 characters".to_string(),
        ));
    }
    Ok(Some(key.to_string()))
}

/// Run `op` under idempotency-key semantics.
///
/// - No `Idempotency-Key` header: `op` runs exactly as before, nothing stored.
/// - Fresh key: `op` runs; its `(status, JSON body)` is stored for replay and
///   returned with `Idempotency-Replayed: false`. If `op` fails, the claim is
///   released so the client may retry the same key.
/// - Replayed key, same fingerprint: the stored response is returned verbatim
///   with `Idempotency-Replayed: true`; `op` does NOT run.
/// - Replayed key, different fingerprint: 422 (ADR-6).
/// - Key still `pending` (the first request is in flight): 409.
pub async fn run<F, Fut>(
    db: &Database,
    headers: &HeaderMap,
    scope: &str,
    scope_id: &str,
    request_fingerprint: &str,
    op: F,
) -> ApiResult<Response>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ApiResult<(StatusCode, serde_json::Value)>>,
{
    let Some(key) = key_from(headers)? else {
        let (status, body) = op().await?;
        return Ok((status, axum::Json(body)).into_response());
    };

    let repo = IdempotencyRepository::new(db);
    match repo
        .try_claim(scope, scope_id, &key, request_fingerprint, retention())
        .await?
    {
        ClaimOutcome::Claimed => {
            let outcome = op().await;
            match outcome {
                Ok((status, body)) => {
                    let body_str = body.to_string();
                    repo.complete(scope, scope_id, &key, status.as_u16(), &body_str)
                        .await?;
                    Ok((
                        status,
                        [("Idempotency-Replayed", "false")],
                        axum::Json(body),
                    )
                        .into_response())
                }
                Err(e) => {
                    // Failed operations are not memoized: release the claim so
                    // the client can retry with the same key.
                    let _ = repo.release(scope, scope_id, &key).await;
                    Err(e)
                }
            }
        }
        ClaimOutcome::Existing(record) => {
            if record.request_hash != request_fingerprint {
                return Err(ApiError::Validation(format!(
                    "Idempotency-Key {key:?} was already used for a different request; \
                     use a fresh key per distinct operation"
                )));
            }
            if record.status != "completed" {
                return Err(ApiError::Conflict(format!(
                    "the original request for Idempotency-Key {key:?} is still in flight; \
                     retry shortly"
                )));
            }
            let status = StatusCode::from_u16(record.response_status.unwrap_or(200) as u16)
                .unwrap_or(StatusCode::OK);
            let body: serde_json::Value = record
                .response_body
                .as_deref()
                .and_then(|b| serde_json::from_str(b).ok())
                .unwrap_or(serde_json::Value::Null);
            Ok((status, [("Idempotency-Replayed", "true")], axum::Json(body)).into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_separator_safe() {
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
    }

    #[test]
    fn key_extraction_rules() {
        let mut h = HeaderMap::new();
        assert_eq!(key_from(&h).unwrap(), None);

        h.insert("Idempotency-Key", " retry-123 ".parse().unwrap());
        assert_eq!(key_from(&h).unwrap().as_deref(), Some("retry-123"));

        h.insert("Idempotency-Key", "".parse().unwrap());
        assert!(key_from(&h).is_err());

        h.insert("Idempotency-Key", "x".repeat(256).parse().unwrap());
        assert!(key_from(&h).is_err());
    }
}
