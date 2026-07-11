//! Inbound authentication for the agent HTTP API (Plan 01, Phase C).
//!
//! Configuration-driven and engineered to stay off the hot path:
//!
//! - **Disabled (default):** the middleware layer is *not mounted at all*
//!   (see `main.rs`) — evaluation requests pay zero instructions for auth.
//! - **Enabled:** everything derivable from config is computed once at
//!   startup into [`AgentAuthVerifier`] (header names parsed, secret/token
//!   digests hashed, the JWT decoding key and validation rules built), so a
//!   request pays one path match plus one SHA-256 over its credential:
//!   - static bearer token / proxy fingerprint → digest compare (~100ns);
//!   - management-minted JWT → full HMAC verification **once**, then a
//!     digest-keyed cache hit until the token expires (~100ns steady-state).
//!
//! Two credential kinds, selected by `auth.mode`:
//! - **mTLS** — TLS terminated at the agent with `tls.require_client_cert`
//!   (the handshake is the authentication), or a trusted reverse proxy's
//!   verified-fingerprint header (optionally allowlisted).
//! - **Bearer** — the management-minted agent JWT (HS256 against the shared
//!   `jwt_secret`, issuer/audience pinned) or a static `bearer_token`.
//!
//! Health/readiness/liveness/metrics stay open: orchestrators probe them
//! unauthenticated and they expose no policy data.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderName, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use sha2::{Digest, Sha256};

use reaper_core::config::ReaperAgentConfig;

/// Upper bound on cached validated-JWT digests. Only *valid* tokens are ever
/// cached, so this is the number of distinct live credentials, not something
/// an attacker can grow; the bound is a safety net, not a working limit.
const JWT_CACHE_MAX: usize = 4096;

/// Routes that must stay reachable without credentials (orchestrator probes).
fn is_exempt(path: &str) -> bool {
    matches!(
        path,
        "/health" | "/ready" | "/live" | "/metrics" | "/openapi.json"
    ) || path.starts_with("/metrics/")
}

/// The read-only data-plane hot path: policy evaluation. Optionally left open
/// (sidecar posture) so a co-located caller pays no auth on the hot path while
/// the management plane stays gated. These endpoints do not mutate policy or
/// data — they only decide against the already-loaded set.
fn is_data_plane(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/messages" | "/api/v1/fast-messages" | "/api/v1/batch-messages" | "/api/v1/check"
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// All auth state pre-computed at startup — nothing on the request path
/// allocates, parses config, or derives keys.
pub struct AgentAuthVerifier {
    accepts_mtls: bool,
    accepts_bearer: bool,
    /// TLS terminates at the agent and the handshake already required a
    /// client cert — every connection that reaches the router is authed.
    tls_handshake_is_auth: bool,
    /// Trusted-proxy fingerprint header, pre-parsed.
    fingerprint_header: Option<HeaderName>,
    /// Digests of allowlisted fingerprints (empty = any non-empty value).
    allowed_fingerprint_digests: Vec<[u8; 32]>,
    /// Digest of the static bearer token, plus its length as a cheap
    /// pre-filter so long JWTs don't pay a SHA-256 just to fail this compare
    /// (token length is not a secret; constant-time compares leak it anyway).
    bearer_token_digest: Option<(usize, [u8; 32])>,
    /// Pre-built JWT decoding key + pinned validation rules.
    jwt: Option<(jsonwebtoken::DecodingKey, jsonwebtoken::Validation)>,
    /// Sidecar posture: leave the evaluation hot path unauthenticated while
    /// the management plane stays gated (see [`is_data_plane`]).
    open_data_plane: bool,
    /// Per-process random SipHash key for the validated-JWT cache: an
    /// attacker can't precompute colliding tokens without it, and a chance
    /// collision only costs a re-verify (entries confirm the exact token).
    jwt_cache_hasher: std::hash::RandomState,
    /// keyed-hash(token) → (token, exp). A hit compares the stored token
    /// byte-for-byte before trusting the entry, so correctness never rests
    /// on the hash. Steady-state JWT auth is one ~ns-scale keyed hash + one
    /// lock-free map hit + one memcmp instead of an HMAC verify per request.
    /// (Prefix-timing on the memcmp is not a usable oracle: every miss falls
    /// through to full HMAC verification whose µs-scale cost and variance
    /// swamp it, and the compared value is a high-entropy signature.)
    validated_jwts: DashMap<u64, (Box<str>, i64)>,
}

impl AgentAuthVerifier {
    /// Build the verifier from config. Returns `None` when inbound auth is
    /// disabled — the caller then skips mounting the middleware entirely.
    pub fn from_config(config: &ReaperAgentConfig) -> Option<Arc<Self>> {
        let auth = &config.auth;
        if !auth.enabled {
            return None;
        }

        let jwt = auth.jwt_secret.as_deref().map(|secret| {
            let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
            validation.set_issuer(&[&auth.jwt_issuer]);
            validation.set_audience(&[&auth.jwt_audience]);
            (
                jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
                validation,
            )
        });

        Some(Arc::new(Self {
            accepts_mtls: auth.accepts_mtls(),
            accepts_bearer: auth.accepts_bearer(),
            open_data_plane: auth.open_data_plane,
            tls_handshake_is_auth: config.tls.enabled && config.tls.require_client_cert,
            fingerprint_header: auth
                .mtls_fingerprint_header
                .as_deref()
                .and_then(|h| h.parse::<HeaderName>().ok()),
            allowed_fingerprint_digests: auth
                .mtls_allowed_fingerprints
                .iter()
                .map(|fp| digest(fp.as_bytes()))
                .collect(),
            bearer_token_digest: auth
                .bearer_token
                .as_deref()
                .map(|t| (t.len(), digest(t.as_bytes()))),
            jwt,
            jwt_cache_hasher: std::hash::RandomState::new(),
            validated_jwts: DashMap::new(),
        }))
    }

    /// Does this request carry an accepted credential?
    fn authorize(&self, request: &Request) -> bool {
        if self.accepts_mtls {
            if self.tls_handshake_is_auth {
                return true;
            }
            if let Some(header) = &self.fingerprint_header {
                if let Some(fingerprint) = request
                    .headers()
                    .get(header)
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
                    .filter(|fp| !fp.is_empty())
                {
                    let fp_digest = digest(fingerprint.as_bytes());
                    if self.allowed_fingerprint_digests.is_empty()
                        || self.allowed_fingerprint_digests.contains(&fp_digest)
                    {
                        return true;
                    }
                }
            }
        }

        if self.accepts_bearer {
            if let Some(token) = request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
            {
                if let Some((expected_len, expected_digest)) = &self.bearer_token_digest {
                    // Digest compare: constant-time in the secret, no
                    // byte-position timing oracle on the static token.
                    if token.len() == *expected_len && *expected_digest == digest(token.as_bytes())
                    {
                        return true;
                    }
                }

                if let Some((key, validation)) = &self.jwt {
                    // Fast path: this exact token already passed HMAC
                    // verification and hasn't expired.
                    use std::hash::BuildHasher;
                    let cache_key = self.jwt_cache_hasher.hash_one(token.as_bytes());
                    if let Some(entry) = self.validated_jwts.get(&cache_key) {
                        let (cached_token, exp) = entry.value();
                        if cached_token.as_ref() == token && *exp > chrono::Utc::now().timestamp() {
                            return true;
                        }
                    }

                    // Slow path (once per token): full signature + claims
                    // verification, then cache until exp.
                    #[derive(serde::Deserialize)]
                    struct ExpOnly {
                        exp: i64,
                    }
                    if let Ok(data) = jsonwebtoken::decode::<ExpOnly>(token, key, validation) {
                        if self.validated_jwts.len() >= JWT_CACHE_MAX {
                            self.validated_jwts.clear();
                        }
                        self.validated_jwts
                            .insert(cache_key, (Box::from(token), data.claims.exp));
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// Default-deny inbound authentication middleware. Mounted only when
/// `config.auth.enabled` — a disabled config never pays for this call.
pub async fn require_agent_auth(
    State(verifier): State<Arc<AgentAuthVerifier>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if is_exempt(path)
        || (verifier.open_data_plane && is_data_plane(path))
        || verifier.authorize(&request)
    {
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        [(
            axum::http::header::WWW_AUTHENTICATE,
            "Bearer realm=\"reaper-agent\"",
        )],
        axum::Json(serde_json::json!({
            "error": "unauthorized",
            "message": "Agent inbound authentication required (mTLS client certificate or Bearer credential)",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaper_core::config::AgentAuthMode;

    fn config(auth: reaper_core::config::AgentAuthSettings) -> ReaperAgentConfig {
        ReaperAgentConfig {
            auth,
            ..Default::default()
        }
    }

    fn request(headers: &[(&str, &str)]) -> Request {
        let mut builder = axum::http::Request::builder().uri("/api/v1/messages");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    fn mint_jwt(secret: &str, exp_offset_secs: i64) -> String {
        use jsonwebtoken::{encode, EncodingKey, Header};
        let claims = serde_json::json!({
            "sub": "agent-1",
            "iss": "reaper-management",
            "aud": "reaper-agent",
            "exp": chrono::Utc::now().timestamp() + exp_offset_secs,
            "iat": chrono::Utc::now().timestamp(),
            "jti": "t1",
            "org_id": "8b1a9953-7f6a-4a3e-9d1a-000000000001",
            "scopes": ["agent:read"],
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn health_routes_are_exempt() {
        for p in ["/health", "/ready", "/live", "/metrics", "/metrics/x"] {
            assert!(is_exempt(p), "{p} must stay open for orchestrator probes");
        }
        for p in [
            "/api/v1/messages",
            "/api/v1/policies",
            "/api/v1/bundles/deploy",
            "/api/v1/data",
            "/debug/datastore",
        ] {
            assert!(!is_exempt(p), "{p} must require authentication");
        }
    }

    #[test]
    fn disabled_config_builds_no_verifier() {
        assert!(AgentAuthVerifier::from_config(&ReaperAgentConfig::default()).is_none());
    }

    #[test]
    fn data_plane_classification() {
        for p in [
            "/api/v1/messages",
            "/api/v1/fast-messages",
            "/api/v1/batch-messages",
            "/api/v1/check",
        ] {
            assert!(is_data_plane(p), "{p} is the eval hot path");
        }
        for p in [
            "/api/v1/bundles/deploy",
            "/api/v1/bundles/load",
            "/api/v1/policies/deploy",
            "/api/v1/data",
            "/api/v1/entities",
            "/debug/datastore",
        ] {
            assert!(!is_data_plane(p), "{p} is management, not the hot path");
        }
    }

    #[tokio::test]
    async fn sidecar_open_data_plane_gates_management_only() {
        use axum::routing::{get, post};
        use tower::ServiceExt;

        // Sidecar posture: auth enabled + open_data_plane. A co-located caller
        // hits the evaluator with no credential; a push still needs one.
        let verifier =
            AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
                enabled: true,
                open_data_plane: true,
                bearer_token: Some("tok-secret".into()),
                ..Default::default()
            }))
            .unwrap();

        async fn ok() -> &'static str {
            "ok"
        }
        let app = axum::Router::new()
            .route("/api/v1/messages", post(ok))
            .route("/api/v1/bundles/deploy", post(ok))
            .route("/health", get(ok))
            .layer(axum::middleware::from_fn_with_state(
                verifier,
                require_agent_auth,
            ));

        let call = |uri: &str, auth: Option<&str>| {
            let mut b = axum::http::Request::builder().uri(uri).method("POST");
            if let Some(a) = auth {
                b = b.header("authorization", a);
            }
            app.clone()
                .oneshot(b.body(axum::body::Body::empty()).unwrap())
        };

        // Eval path: open, no credential needed.
        assert_eq!(
            call("/api/v1/messages", None).await.unwrap().status(),
            StatusCode::OK
        );
        // Management path: 401 without a credential...
        assert_eq!(
            call("/api/v1/bundles/deploy", None).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        // ...and 200 with one.
        assert_eq!(
            call("/api/v1/bundles/deploy", Some("Bearer tok-secret"))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn without_open_data_plane_eval_is_gated_too() {
        use axum::routing::post;
        use tower::ServiceExt;

        let verifier =
            AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
                enabled: true,
                open_data_plane: false,
                bearer_token: Some("tok-secret".into()),
                ..Default::default()
            }))
            .unwrap();
        async fn ok() -> &'static str {
            "ok"
        }
        let app = axum::Router::new()
            .route("/api/v1/messages", post(ok))
            .layer(axum::middleware::from_fn_with_state(
                verifier,
                require_agent_auth,
            ));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/messages")
                    .method("POST")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn static_bearer_token() {
        let v = AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
            enabled: true,
            bearer_token: Some("tok-secret".into()),
            ..Default::default()
        }))
        .unwrap();

        assert!(v.authorize(&request(&[("authorization", "Bearer tok-secret")])));
        assert!(!v.authorize(&request(&[("authorization", "Bearer tok-wrong")])));
        assert!(!v.authorize(&request(&[])));
    }

    #[test]
    fn jwt_verification_pinning_and_cache() {
        let secret = "0123456789abcdef0123456789abcdef";
        let v = AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
            enabled: true,
            jwt_secret: Some(secret.into()),
            ..Default::default()
        }))
        .unwrap();

        let token = mint_jwt(secret, 3600);
        let auth_value = format!("Bearer {token}");
        assert!(v.authorize(&request(&[("authorization", auth_value.as_str())])));
        // Second call takes the cache path and must agree.
        assert!(v.authorize(&request(&[("authorization", auth_value.as_str())])));
        assert_eq!(v.validated_jwts.len(), 1);

        // Wrong secret, expired, or unsigned tokens are refused.
        let wrong = format!(
            "Bearer {}",
            mint_jwt("another-secret-another-secret!!!", 3600)
        );
        assert!(!v.authorize(&request(&[("authorization", wrong.as_str())])));
        let expired = format!("Bearer {}", mint_jwt(secret, -3600));
        assert!(!v.authorize(&request(&[("authorization", expired.as_str())])));
        assert!(!v.authorize(&request(&[("authorization", "Bearer not-a-jwt")])));
    }

    #[test]
    fn proxy_fingerprint_header() {
        let v = AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
            enabled: true,
            mode: AgentAuthMode::Mtls,
            mtls_fingerprint_header: Some("x-client-cert-fingerprint".into()),
            mtls_allowed_fingerprints: vec!["sha256:aabbcc".into()],
            ..Default::default()
        }))
        .unwrap();

        assert!(v.authorize(&request(&[("x-client-cert-fingerprint", "sha256:aabbcc")])));
        assert!(!v.authorize(&request(&[("x-client-cert-fingerprint", "sha256:other")])));
        assert!(!v.authorize(&request(&[("x-client-cert-fingerprint", "")])));
        // Bearer is refused in mtls-only mode.
        let secret_bearer = request(&[("authorization", "Bearer whatever")]);
        assert!(!v.authorize(&secret_bearer));
    }

    /// Steady-state overhead measurement (run explicitly: `--ignored`).
    #[test]
    #[ignore]
    fn measure_auth_overhead() {
        let secret = "0123456789abcdef0123456789abcdef";
        let v = AgentAuthVerifier::from_config(&config(reaper_core::config::AgentAuthSettings {
            enabled: true,
            bearer_token: Some("tok-secret".into()),
            jwt_secret: Some(secret.into()),
            ..Default::default()
        }))
        .unwrap();

        let static_req = request(&[("authorization", "Bearer tok-secret")]);
        let token = mint_jwt(secret, 3600);
        let jwt_value = format!("Bearer {token}");
        let jwt_req = request(&[("authorization", jwt_value.as_str())]);
        assert!(v.authorize(&jwt_req)); // warm the cache

        for (label, req) in [("static bearer", &static_req), ("cached JWT", &jwt_req)] {
            let start = std::time::Instant::now();
            let iters = 100_000;
            for _ in 0..iters {
                assert!(v.authorize(req));
            }
            println!(
                "{label}: {:.0} ns/authorize",
                start.elapsed().as_nanos() as f64 / iters as f64
            );
        }
    }
}
