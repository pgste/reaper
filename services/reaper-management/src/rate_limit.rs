//! Rate limiting module for Reaper Management Server
//!
//! Provides request rate limiting using the Governor library with
//! IP-based and path-specific rate limits.

use axum::{
    body::Body,
    extract::{ConnectInfo, Request},
    http::{header::FORWARDED, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::keyed::DefaultKeyedStateStore,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};
use tokio::sync::RwLock;

use crate::config::RateLimitConfig;

/// Per-IP keyed rate limiter.
type KeyedIpLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Per-org keyed rate limiter (round-2 E4).
type KeyedOrgLimiter = RateLimiter<uuid::Uuid, DefaultKeyedStateStore<uuid::Uuid>, DefaultClock>;

/// Per-tenant request ceiling: a token bucket keyed by org id, enforcing the
/// `api_per_org_per_minute` config so one tenant cannot exhaust the shared
/// control plane. Unlike the global/per-IP limiters (a pre-auth middleware),
/// this is checked **post-auth** on the resource-creating paths, where the org
/// identity is verified.
pub struct OrgRateLimiter {
    limiter: KeyedOrgLimiter,
}

impl OrgRateLimiter {
    /// Build a per-org limiter allowing `per_minute` requests per org (min 1).
    pub fn new(per_minute: u32) -> Self {
        let quota = Quota::per_minute(
            NonZeroU32::new(per_minute).unwrap_or_else(|| NonZeroU32::new(1).unwrap()),
        );
        Self {
            limiter: RateLimiter::keyed(quota),
        }
    }

    /// True if the request is admitted; false when the org's per-minute ceiling
    /// is exhausted.
    pub fn allow(&self, org_id: uuid::Uuid) -> bool {
        self.limiter.check_key(&org_id).is_ok()
    }
}

/// Rate limiter for the API
pub struct ApiRateLimiter {
    /// Global rate limiter (all requests)
    global: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
    /// Per-IP limiter for login attempts (brute-force protection)
    login: Arc<KeyedIpLimiter>,
    /// Per-IP limiter for signups (abuse protection)
    signup: Arc<KeyedIpLimiter>,
    /// Path-specific rate limiters
    path_limiters: Arc<RwLock<HashMap<String, PathLimiter>>>,
    /// Configuration
    config: RateLimitConfig,
}

struct PathLimiter {
    limiter: RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>,
    pattern: String,
}

impl ApiRateLimiter {
    /// Create a new rate limiter from configuration
    pub fn new(config: &RateLimitConfig) -> Self {
        // Create global limiter
        let quota = Quota::per_second(
            NonZeroU32::new(config.requests_per_second).unwrap_or(NonZeroU32::new(100).unwrap()),
        )
        .allow_burst(NonZeroU32::new(config.burst_size).unwrap_or(NonZeroU32::new(200).unwrap()));

        let global = Arc::new(RateLimiter::direct(quota));

        // Per-IP limits for the sensitive auth endpoints.
        let login_quota = Quota::per_minute(
            NonZeroU32::new(config.login_per_minute).unwrap_or(NonZeroU32::new(10).unwrap()),
        );
        let signup_quota = Quota::per_hour(
            NonZeroU32::new(config.signup_per_hour).unwrap_or(NonZeroU32::new(5).unwrap()),
        );

        Self {
            global,
            login: Arc::new(RateLimiter::keyed(login_quota)),
            signup: Arc::new(RateLimiter::keyed(signup_quota)),
            path_limiters: Arc::new(RwLock::new(HashMap::new())),
            config: config.clone(),
        }
    }

    /// Check if a request should be rate limited.
    pub async fn check(&self, ip: IpAddr, path: &str) -> Result<(), RateLimitError> {
        // Check global limit first
        if self.global.check().is_err() {
            return Err(RateLimitError::GlobalLimitExceeded);
        }

        // Stricter PER-IP limits on the sensitive auth endpoints so a single
        // client cannot brute-force login or spam signups within the generous
        // global budget.
        let clean_path = path.split('?').next().unwrap_or(path);
        if clean_path.starts_with("/auth/login") {
            self.check_login(ip)?;
        } else if clean_path.starts_with("/auth/signup") {
            self.check_signup(ip)?;
        }

        // Check path-specific limits
        let path_prefix = get_path_prefix(path);
        if let Some(limiter) = self.get_path_limiter(&path_prefix).await {
            if limiter.limiter.check().is_err() {
                return Err(RateLimitError::PathLimitExceeded { path: path_prefix });
            }
        }

        Ok(())
    }

    async fn get_path_limiter(&self, path_prefix: &str) -> Option<PathLimiter> {
        let limiters = self.path_limiters.read().await;
        limiters.get(path_prefix).map(|l| PathLimiter {
            limiter: RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(self.config.requests_per_second).unwrap(),
            )),
            pattern: l.pattern.clone(),
        })
    }

    /// Per-IP signup limit (default: `signup_per_hour`).
    pub fn check_signup(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.signup
            .check_key(&ip)
            .map_err(|_| RateLimitError::IpLimitExceeded { ip })
    }

    /// Per-IP login limit (default: `login_per_minute`) — brute-force protection.
    pub fn check_login(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.login
            .check_key(&ip)
            .map_err(|_| RateLimitError::IpLimitExceeded { ip })
    }
}

/// Rate limit error types
#[derive(Debug, Clone)]
pub enum RateLimitError {
    GlobalLimitExceeded,
    PathLimitExceeded { path: String },
    IpLimitExceeded { ip: IpAddr },
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateLimitError::GlobalLimitExceeded => write!(f, "Global rate limit exceeded"),
            RateLimitError::PathLimitExceeded { path } => {
                write!(f, "Rate limit exceeded for path: {}", path)
            }
            RateLimitError::IpLimitExceeded { ip } => {
                write!(f, "Rate limit exceeded for IP: {}", ip)
            }
        }
    }
}

impl std::error::Error for RateLimitError {}

/// Extract path prefix for rate limiting (e.g., "/auth/signup" -> "/auth/signup")
fn get_path_prefix(path: &str) -> String {
    // For specific endpoints, return exact path
    if path.starts_with("/auth/signup")
        || path.starts_with("/auth/login")
        || path.starts_with("/auth/password")
    {
        return path.split('?').next().unwrap_or(path).to_string();
    }

    // For API endpoints, use first two segments
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        format!("/{}/{}", parts[0], parts[1])
    } else {
        path.to_string()
    }
}

/// Extract client IP from request headers.
///
/// When `trust_proxy_headers` is false (the default), forwarding headers are
/// ignored and the real connection IP is used — otherwise an attacker could
/// spoof `X-Forwarded-For` to evade per-IP rate limits. Only pass `true` when
/// the service is behind a trusted reverse proxy that sets these headers.
pub fn extract_client_ip(
    headers: &HeaderMap,
    connect_info: Option<SocketAddr>,
    trust_proxy_headers: bool,
) -> IpAddr {
    let connection_ip = || {
        connect_info
            .map(|addr| addr.ip())
            .unwrap_or_else(|| "127.0.0.1".parse().unwrap())
    };

    if !trust_proxy_headers {
        return connection_ip();
    }

    // Try X-Forwarded-For first (common proxy header)
    if let Some(forwarded_for) = headers.get("x-forwarded-for") {
        if let Ok(value) = forwarded_for.to_str() {
            if let Some(ip_str) = value.split(',').next() {
                if let Ok(ip) = ip_str.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }

    // Try X-Real-IP (nginx)
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(value) = real_ip.to_str() {
            if let Ok(ip) = value.parse::<IpAddr>() {
                return ip;
            }
        }
    }

    // Try Forwarded header (RFC 7239)
    if let Some(forwarded) = headers.get(FORWARDED) {
        if let Ok(value) = forwarded.to_str() {
            // Parse "for=ip" from the header
            for part in value.split(';') {
                let part = part.trim();
                if let Some(ip_part) = part.strip_prefix("for=") {
                    // Remove quotes and brackets
                    let ip_str = ip_part.trim_matches(|c| c == '"' || c == '[' || c == ']');
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        return ip;
                    }
                }
            }
        }
    }

    // Fall back to connection info
    connection_ip()
}

/// Axum middleware for rate limiting
pub async fn rate_limit_middleware(
    axum::extract::State(rate_limiter): axum::extract::State<Arc<ApiRateLimiter>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(
        &headers,
        Some(addr),
        rate_limiter.config.trust_proxy_headers,
    );
    let path = request.uri().path().to_string();

    // Check rate limit
    match rate_limiter.check(ip, &path).await {
        Ok(()) => next.run(request).await,
        Err(e) => {
            tracing::warn!(
                ip = %ip,
                path = %path,
                error = %e,
                "Rate limit exceeded"
            );

            (
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", "60"), ("X-RateLimit-Remaining", "0")],
                "Too many requests. Please try again later.",
            )
                .into_response()
        }
    }
}

/// Create rate limiter layer for Axum
pub fn create_rate_limiter(config: &RateLimitConfig) -> Option<Arc<ApiRateLimiter>> {
    if config.enabled {
        Some(Arc::new(ApiRateLimiter::new(config)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_rate_limiter_enforces_per_org_ceiling() {
        // 3/min burst 3: the first three requests for an org pass, the fourth is
        // refused — and a different org is bucketed independently.
        let limiter = OrgRateLimiter::new(3);
        let org_a = uuid::Uuid::new_v4();
        let org_b = uuid::Uuid::new_v4();
        assert!(limiter.allow(org_a));
        assert!(limiter.allow(org_a));
        assert!(limiter.allow(org_a));
        assert!(
            !limiter.allow(org_a),
            "4th request over the ceiling is refused"
        );
        // A different tenant has its own bucket.
        assert!(limiter.allow(org_b), "per-org isolation");
    }

    #[test]
    fn test_get_path_prefix() {
        assert_eq!(get_path_prefix("/auth/signup"), "/auth/signup");
        assert_eq!(get_path_prefix("/auth/login"), "/auth/login");
        assert_eq!(
            get_path_prefix("/auth/password/reset"),
            "/auth/password/reset"
        );
        assert_eq!(get_path_prefix("/orgs/acme/agents"), "/orgs/acme");
        assert_eq!(get_path_prefix("/api/v1/test"), "/api/v1");
    }

    #[test]
    fn test_extract_client_ip_trusts_forwarded_when_enabled() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "192.168.1.1, 10.0.0.1".parse().unwrap());

        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip.to_string(), "192.168.1.1");
    }

    #[test]
    fn test_extract_client_ip_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "10.0.0.5".parse().unwrap());

        let ip = extract_client_ip(&headers, None, true);
        assert_eq!(ip.to_string(), "10.0.0.5");
    }

    #[test]
    fn test_extract_client_ip_ignores_spoofed_forwarded_by_default() {
        // Untrusted forwarding headers must NOT override the real connection IP,
        // otherwise per-IP rate limits are trivially bypassed by spoofing XFF.
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        let conn: SocketAddr = "10.0.0.9:5000".parse().unwrap();

        let ip = extract_client_ip(&headers, Some(conn), false);
        assert_eq!(ip.to_string(), "10.0.0.9");
    }

    #[test]
    fn test_extract_client_ip_fallback() {
        let headers = HeaderMap::new();
        let ip = extract_client_ip(&headers, None, false);
        assert_eq!(ip.to_string(), "127.0.0.1");
    }

    #[tokio::test]
    async fn test_login_per_ip_limit_enforced() {
        let config = RateLimitConfig {
            enabled: true,
            requests_per_second: 1000,
            burst_size: 1000,
            login_per_minute: 3,
            ..Default::default()
        };
        let limiter = ApiRateLimiter::new(&config);
        let ip: IpAddr = "203.0.113.7".parse().unwrap();

        // First 3 login attempts from this IP succeed, the 4th is limited.
        for _ in 0..3 {
            assert!(limiter.check(ip, "/auth/login").await.is_ok());
        }
        assert!(matches!(
            limiter.check(ip, "/auth/login").await,
            Err(RateLimitError::IpLimitExceeded { .. })
        ));

        // A different IP is unaffected.
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        assert!(limiter.check(other, "/auth/login").await.is_ok());
    }

    #[tokio::test]
    async fn test_rate_limiter_allows_requests() {
        let config = RateLimitConfig {
            enabled: true,
            requests_per_second: 100,
            burst_size: 200,
            ..Default::default()
        };

        let limiter = ApiRateLimiter::new(&config);

        // First request should succeed
        let result = limiter.check("127.0.0.1".parse().unwrap(), "/test").await;
        assert!(result.is_ok());
    }
}
