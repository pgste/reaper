//! Production middleware for Reaper Management Server
//!
//! Provides security headers, request tracking, timeouts, and correlation IDs.

use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderValue, Response, StatusCode},
    middleware::Next,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn, Span};
use uuid::Uuid;

use crate::graceful::ShutdownSignal;
use crate::metrics;

/// Security headers middleware
/// Adds standard security headers to all responses
pub async fn security_headers(
    request: Request,
    next: Next,
) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // Prevent clickjacking
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );

    // Prevent MIME type sniffing
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    // Enable XSS protection (legacy browsers)
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block"),
    );

    // Referrer policy
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Content Security Policy (API server - strict)
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );

    // Permissions Policy (disable all browser features)
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("geolocation=(), microphone=(), camera=(), payment=()"),
    );

    // Remove server identification header
    headers.remove(header::SERVER);

    response
}

/// HSTS middleware (only enable in production with HTTPS)
pub async fn hsts_headers(
    request: Request,
    next: Next,
) -> Response<Body> {
    // Check for HTTPS before consuming the request
    let is_https = request
        .headers()
        .get("X-Forwarded-Proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "https")
        .unwrap_or(false);

    let mut response = next.run(request).await;

    if is_https {
        response.headers_mut().insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}

/// Request correlation ID middleware
/// Adds a unique request ID to each request for tracing
pub async fn correlation_id(
    mut request: Request,
    next: Next,
) -> Response<Body> {
    // Check if client provided a request ID
    let request_id = request
        .headers()
        .get("X-Request-ID")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Store in request extensions for use by handlers
    request.extensions_mut().insert(RequestId(request_id.clone()));

    // Add to tracing span
    Span::current().record("request_id", &request_id);

    let mut response = next.run(request).await;

    // Add request ID to response headers
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("X-Request-ID", value);
    }

    response
}

/// Request ID extension type
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Request tracking middleware for graceful shutdown
pub async fn request_tracking(
    shutdown_signal: Arc<ShutdownSignal>,
    request: Request,
    next: Next,
) -> Result<Response<Body>, StatusCode> {
    // Reject new requests if shutting down
    if shutdown_signal.is_shutting_down() {
        warn!("Rejecting request during shutdown");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    shutdown_signal.request_started();
    let response = next.run(request).await;
    shutdown_signal.request_finished();

    Ok(response)
}

/// Request timeout middleware configuration
#[derive(Clone)]
pub struct TimeoutConfig {
    pub default_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
        }
    }
}

/// Request metrics middleware
/// Records request duration and counts
pub async fn request_metrics(
    request: Request,
    next: Next,
) -> Response<Body> {
    let start = Instant::now();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    // Normalize path for metrics (remove UUIDs and specific IDs)
    let normalized_path = normalize_path_for_metrics(&path);

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status().as_u16().to_string();

    // Record metrics
    metrics::API_REQUESTS
        .with_label_values(&[&method, &normalized_path, &status])
        .inc();

    metrics::API_LATENCY
        .with_label_values(&[&method, &normalized_path])
        .observe(duration.as_secs_f64());

    // Log slow requests
    if duration > Duration::from_secs(5) {
        warn!(
            method = %method,
            path = %path,
            status = %status,
            duration_ms = %duration.as_millis(),
            "Slow request detected"
        );
    }

    response
}

/// Normalize path for metrics to avoid high cardinality
fn normalize_path_for_metrics(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let normalized: Vec<&str> = parts
        .iter()
        .map(|part| {
            // Replace UUIDs with placeholder
            if Uuid::parse_str(part).is_ok() {
                "{id}"
            } else if part.chars().all(|c| c.is_ascii_digit()) && !part.is_empty() {
                "{id}"
            } else {
                *part
            }
        })
        .collect();
    normalized.join("/")
}

/// Request body size limit middleware
pub async fn body_size_limit(
    request: Request,
    next: Next,
) -> Result<Response<Body>, StatusCode> {
    // Check Content-Length header
    if let Some(content_length) = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
    {
        // 10MB limit
        const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
        if content_length > MAX_BODY_SIZE {
            warn!(
                content_length = content_length,
                max_size = MAX_BODY_SIZE,
                "Request body too large"
            );
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
    }

    Ok(next.run(request).await)
}

/// Log all requests (access log style)
pub async fn access_log(
    request: Request,
    next: Next,
) -> Response<Body> {
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();

    // Get client IP from X-Forwarded-For or X-Real-IP
    let client_ip = request
        .headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            request
                .headers()
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "-".to_string());

    let user_agent = request
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status();

    info!(
        target: "access_log",
        client_ip = %client_ip,
        method = %method,
        uri = %uri,
        version = ?version,
        status = %status.as_u16(),
        duration_ms = %duration.as_millis(),
        user_agent = %user_agent,
        "request"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path_for_metrics("/orgs/550e8400-e29b-41d4-a716-446655440000/agents"),
            "/orgs/{id}/agents"
        );
        assert_eq!(
            normalize_path_for_metrics("/health"),
            "/health"
        );
        assert_eq!(
            normalize_path_for_metrics("/orgs/test-org/bundles/123"),
            "/orgs/test-org/bundles/{id}"
        );
    }

    #[test]
    fn test_request_id() {
        let id = RequestId("test-123".to_string());
        assert_eq!(id.0, "test-123");
    }
}
