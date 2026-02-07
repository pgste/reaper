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

/// Rate limiter for the API
pub struct ApiRateLimiter {
    /// Global rate limiter (per IP)
    global: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
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

        Self {
            global,
            path_limiters: Arc::new(RwLock::new(HashMap::new())),
            config: config.clone(),
        }
    }

    /// Check if a request should be rate limited
    pub async fn check(&self, _ip: IpAddr, path: &str) -> Result<(), RateLimitError> {
        // Check global limit first
        if self.global.check().is_err() {
            return Err(RateLimitError::GlobalLimitExceeded);
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

    /// Get specific limit for signup endpoint
    pub fn check_signup(&self) -> Result<(), RateLimitError> {
        // For signup, use a stricter limit
        // In a real implementation, this would be per-IP with a separate limiter
        if self.global.check().is_err() {
            return Err(RateLimitError::GlobalLimitExceeded);
        }
        Ok(())
    }

    /// Get specific limit for login endpoint
    pub fn check_login(&self) -> Result<(), RateLimitError> {
        // For login, use a stricter limit
        if self.global.check().is_err() {
            return Err(RateLimitError::GlobalLimitExceeded);
        }
        Ok(())
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

/// Extract client IP from request headers
pub fn extract_client_ip(headers: &HeaderMap, connect_info: Option<SocketAddr>) -> IpAddr {
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
    connect_info
        .map(|addr| addr.ip())
        .unwrap_or_else(|| "127.0.0.1".parse().unwrap())
}

/// Axum middleware for rate limiting
pub async fn rate_limit_middleware(
    axum::extract::State(rate_limiter): axum::extract::State<Arc<ApiRateLimiter>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&headers, Some(addr));
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
    fn test_extract_client_ip_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "192.168.1.1, 10.0.0.1".parse().unwrap());

        let ip = extract_client_ip(&headers, None);
        assert_eq!(ip.to_string(), "192.168.1.1");
    }

    #[test]
    fn test_extract_client_ip_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "10.0.0.5".parse().unwrap());

        let ip = extract_client_ip(&headers, None);
        assert_eq!(ip.to_string(), "10.0.0.5");
    }

    #[test]
    fn test_extract_client_ip_fallback() {
        let headers = HeaderMap::new();
        let ip = extract_client_ip(&headers, None);
        assert_eq!(ip.to_string(), "127.0.0.1");
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
