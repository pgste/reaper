//! Default-deny authentication gateway.
//!
//! A router-level middleware that authenticates every request to a non-public
//! route *before* it reaches a handler, so a handler that forgets the
//! [`RequireAuth`] extractor still fails closed. This is the structural fix for
//! the "authorization is opt-in and fails open" finding: whole control-plane
//! route groups (bundles/policies/orgs/teams/billing) previously omitted
//! `RequireAuth` and served anonymous callers.
//!
//! On success the resolved `AuthenticatedUser` is stashed in request
//! extensions; the handler's `RequireAuth` reads it back (fast path) without
//! re-validating. The gateway authenticates only — per-handler scope and
//! tenant checks remain the handlers' responsibility (a layer can't know which
//! org a given route addresses).

use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Request, State},
    middleware::Next,
    response::Response,
};

use super::middleware::RequireAuth;
use crate::config::GatewayMode;
use crate::state::AppState;

/// Is this a route that must be reachable without authentication?
///
/// Matched after stripping an optional `/api/v1` prefix, because the API is
/// mounted at both the bare root and `/api/v1`. Everything not listed here is
/// deny-by-default.
fn is_public_path(path: &str) -> bool {
    // Normalize the dual mount: treat `/api/v1/x` the same as `/x`.
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let p = if p.is_empty() { "/" } else { p };

    // Liveness/readiness/metrics — orchestrators scrape these unauthenticated.
    // `/openapi.json` is the public API contract (Plan 07) — the spec describes
    // the surface, it does not expose data.
    if p == "/health"
        || p.starts_with("/health/")
        || p == "/live"
        || p == "/ready"
        || p == "/metrics"
        || p.starts_with("/metrics/")
        || p == "/openapi.json"
    {
        return true;
    }

    // The genuinely public auth endpoints. NOTE: /auth/me, /auth/logout, and
    // /auth/password/change require a session and are intentionally absent.
    if matches!(
        p,
        "/auth/login"
            | "/auth/signup"
            | "/auth/token/refresh"
            | "/auth/password/reset"
            | "/auth/email/verify"
            | "/auth/github/authorize"
            | "/auth/github/callback"
    ) {
        return true;
    }

    // Webhook ingest endpoints authenticate via their own source signatures,
    // not a session: bundle-update (source-signed) and Stripe (verified
    // against the stripe-signature header in the handler). Webhook
    // *subscription management* lives under /orgs/{org}/webhooks and stays
    // authenticated.
    p == "/webhooks/bundle-update"
        || p.starts_with("/webhooks/bundle-update/")
        || p == "/webhooks/stripe"
}

/// Router-level default-deny authentication middleware.
pub async fn require_authentication(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // Legacy escape hatch: per-handler `RequireAuth` only.
    if state.config.auth.gateway_mode == GatewayMode::Disabled {
        return next.run(request).await;
    }

    if is_public_path(request.uri().path()) {
        return next.run(request).await;
    }

    let (mut parts, body) = request.into_parts();
    match RequireAuth::from_request_parts(&mut parts, &state).await {
        Ok(RequireAuth(user)) => {
            // Stash for the handler's RequireAuth fast path.
            parts.extensions.insert(user);
            next.run(Request::from_parts(parts, body)).await
        }
        Err(rejection) => match state.config.auth.gateway_mode {
            GatewayMode::LogOnly => {
                tracing::warn!(
                    path = %parts.uri.path(),
                    "auth gateway (log_only): unauthenticated request would be rejected under enforcing mode"
                );
                next.run(Request::from_parts(parts, body)).await
            }
            // Enforcing (Disabled handled above): return the 401 from RequireAuth.
            _ => rejection,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::is_public_path;

    #[test]
    fn public_paths_are_public_at_root_and_v1() {
        let public = [
            "/health",
            "/health/ready",
            "/health/deep",
            "/live",
            "/ready",
            "/metrics",
            "/metrics/prometheus",
            "/auth/login",
            "/auth/signup",
            "/auth/token/refresh",
            "/auth/password/reset",
            "/auth/email/verify",
            "/auth/github/authorize",
            "/auth/github/callback",
            "/webhooks/bundle-update",
            "/webhooks/bundle-update/3f00",
            "/webhooks/stripe",
        ];
        for base in ["", "/api/v1"] {
            for p in public {
                let full = format!("{base}{p}");
                assert!(is_public_path(&full), "{full} should be public");
            }
        }
    }

    #[test]
    fn protected_paths_require_auth() {
        let protected = [
            "/auth/me",
            "/auth/logout",
            "/auth/password/change",
            "/orgs",
            "/orgs/acme/bundles",
            "/orgs/acme/bundles/123",
            "/orgs/acme/webhooks",
            "/orgs/acme/policies",
            "/api/v1/orgs/acme/policies",
            "/api/v1/orgs/acme/bundles/123",
            "/debug/datastore",
        ];
        for p in protected {
            assert!(!is_public_path(p), "{p} must require authentication");
        }
    }
}
