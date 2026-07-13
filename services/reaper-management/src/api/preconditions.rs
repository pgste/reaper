//! ETag / `If-Match` optimistic-concurrency preconditions (Plan 07, Phase C).
//!
//! Policies derive their ETag from the current version's `content_hash`
//! joined with the `row_version` counter that EVERY write — content or
//! metadata — bumps (R2-03: two representations must never share one tag,
//! RFC 9110 §8.8.1); bundles from their `updated_at` timestamp (bumped by
//! every bundle write path). A `PUT` must echo the ETag it read via
//! `If-Match`; the repository then re-checks the version/timestamp **inside
//! the UPDATE's WHERE clause**, so the precondition check here is a fast fail
//! and the SQL guard is the atomic arbiter — two writers racing past this
//! check still resolve to exactly one winner.
//!
//! Enforcement is ON by default (`server.require_if_match = true` since
//! R2-02; the ADR-3 warn-only transition release has shipped): a `PUT`
//! without `If-Match` is rejected with 428. Operators migrating automation
//! can opt back down for one release with `REAPER_REQUIRE_IF_MATCH=false`,
//! in which mode the write proceeds unguarded and logs a deprecation
//! warning. A *stale* `If-Match`, when sent, always fails with 412
//! regardless of the flag — honoring an explicit precondition is never a
//! compatibility break.

use axum::http::{header, HeaderMap};

use super::error::{ApiError, ApiResult};

/// A parsed `If-Match` request header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfMatch {
    /// `If-Match: *` — "the resource exists"; matches any current state.
    Any,
    /// A single entity-tag, unquoted (weak `W/` prefixes are tolerated and
    /// compared by value).
    Value(String),
}

/// Parse the `If-Match` header, if present. Quotes and a `W/` weakness prefix
/// are stripped; list syntax takes the first tag (we mint single strong tags,
/// so a well-behaved client never sends a list).
pub fn if_match(headers: &HeaderMap) -> Option<IfMatch> {
    let raw = headers.get(header::IF_MATCH)?.to_str().ok()?.trim();
    if raw == "*" {
        return Some(IfMatch::Any);
    }
    let first = raw.split(',').next().unwrap_or(raw).trim();
    let v = first.strip_prefix("W/").unwrap_or(first).trim();
    let v = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(v);
    Some(IfMatch::Value(v.to_string()))
}

/// Quote a bare value as a strong entity-tag for the `ETag` response header.
pub fn etag(value: &str) -> String {
    format!("\"{value}\"")
}

/// Evaluate the `If-Match` precondition for a guarded write.
///
/// Returns `Ok(true)` when the write must run **guarded** (the caller passes
/// its expected version/timestamp through to the repository's SQL guard), and
/// `Ok(false)` when it may run unguarded (transitional warn-only mode, no
/// header sent).
pub fn check_precondition(
    headers: &HeaderMap,
    current_tag: &str,
    require_if_match: bool,
    resource: &str,
) -> ApiResult<bool> {
    match if_match(headers) {
        None => {
            if require_if_match {
                Err(ApiError::PreconditionRequired(format!(
                    "PUT on {resource} requires If-Match; GET the resource and echo its ETag"
                )))
            } else {
                tracing::warn!(
                    resource = %resource,
                    "PUT without If-Match — running unguarded (deprecated; \
                     this deployment opted down from the enforced default \
                     via server.require_if_match=false / REAPER_REQUIRE_IF_MATCH=false)"
                );
                Ok(false)
            }
        }
        Some(IfMatch::Any) => Ok(true),
        Some(IfMatch::Value(v)) if v == current_tag => Ok(true),
        Some(IfMatch::Value(_)) => Err(ApiError::PreconditionFailed(format!(
            "{resource} was modified since it was read; GET it again for the current ETag"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn parses_quoted_star_and_weak_tags() {
        assert_eq!(if_match(&HeaderMap::new()), None);
        assert_eq!(if_match(&headers_with("*")), Some(IfMatch::Any));
        assert_eq!(
            if_match(&headers_with("\"abc123\"")),
            Some(IfMatch::Value("abc123".into()))
        );
        assert_eq!(
            if_match(&headers_with("W/\"abc123\"")),
            Some(IfMatch::Value("abc123".into()))
        );
        assert_eq!(
            if_match(&headers_with("abc123")),
            Some(IfMatch::Value("abc123".into()))
        );
    }

    #[test]
    fn precondition_matrix() {
        let h_match = headers_with("\"tag\"");
        let h_stale = headers_with("\"old\"");
        let h_any = headers_with("*");
        let none = HeaderMap::new();

        // Matching tag → guarded write, both modes.
        assert!(check_precondition(&h_match, "tag", true, "policy x").unwrap());
        assert!(check_precondition(&h_match, "tag", false, "policy x").unwrap());
        // `*` → guarded write.
        assert!(check_precondition(&h_any, "tag", true, "policy x").unwrap());
        // Stale tag → 412 regardless of the enforcement flag.
        assert!(matches!(
            check_precondition(&h_stale, "tag", false, "policy x").unwrap_err(),
            ApiError::PreconditionFailed(_)
        ));
        // Missing header: warn-only mode → unguarded; enforcing → 428.
        assert!(!check_precondition(&none, "tag", false, "policy x").unwrap());
        assert!(matches!(
            check_precondition(&none, "tag", true, "policy x").unwrap_err(),
            ApiError::PreconditionRequired(_)
        ));
    }
}
