//! API error handling
//!
//! Provides consistent error responses across all API endpoints.

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

/// API error type.
///
/// `#[non_exhaustive]`: downstream matchers must carry a wildcard arm, so a new
/// error class (as Plan 07 keeps adding: preconditions, idempotency conflicts)
/// is not a breaking change (finding API-10).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    /// A write arrived without its `If-Match` precondition (Plan 07 Phase C,
    /// ADR-3: governed edits fail closed rather than blind-overwrite).
    #[error("Precondition required: {0}")]
    PreconditionRequired(String),

    /// The `If-Match` precondition did not match the resource's current state
    /// — the caller's copy is stale (a concurrent writer won). RFC 9110 §13.1.1.
    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),

    /// The operation is documented but its implementation is not complete —
    /// served as 501 so a caller can never mistake a stub for success
    /// (Plan 06 Phase E, R3-04).
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// A plan-limit quota would be exceeded (round-2 E4). 402 Payment Required:
    /// the request is well-formed and authorized, but the org's subscription
    /// tier does not permit it — the remedy is to upgrade, not to re-auth.
    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    /// The org's per-tenant request ceiling was hit (round-2 E4). 429 Too Many
    /// Requests: retryable after backoff, unlike a plan quota.
    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
}

/// RFC 9457 `application/problem+json` error body (Plan 07, Phase E). `code`
/// is a Reaper extension member kept for programmatic matching; `type` is a
/// stable, documentation-anchored URI reference per problem class. Part of
/// the published OpenAPI contract: every documented 4xx/5xx response body is
/// this shape (round-2 C4, findings R2-06/R2-08).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
    /// Problem-class URI reference (stable; documented under docs/api/).
    #[serde(rename = "type")]
    pub problem_type: String,
    /// Short, human-readable summary of the problem class.
    pub title: String,
    /// The HTTP status code, repeated in the body per RFC 9457.
    pub status: u16,
    /// Human-readable explanation specific to this occurrence.
    pub detail: String,
    /// URI reference identifying this specific occurrence: the request path
    /// (RFC 9457 §3.1.5, finding R2-08). Stamped by the `problem_instance`
    /// middleware on every served error response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Correlation id of the failing request (Reaper extension member;
    /// mirrors the `X-Request-ID` response header). Present when the
    /// correlation-id middleware is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Machine-readable Reaper error code (extension member).
    pub code: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg.clone()),
            ApiError::Validation(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                msg.clone(),
            ),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            ApiError::PreconditionRequired(msg) => (
                StatusCode::PRECONDITION_REQUIRED,
                "precondition_required",
                msg.clone(),
            ),
            ApiError::PreconditionFailed(msg) => (
                StatusCode::PRECONDITION_FAILED,
                "precondition_failed",
                msg.clone(),
            ),
            ApiError::NotImplemented(msg) => {
                (StatusCode::NOT_IMPLEMENTED, "not_implemented", msg.clone())
            }
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone()),
            ApiError::QuotaExceeded(msg) => {
                (StatusCode::PAYMENT_REQUIRED, "quota_exceeded", msg.clone())
            }
            ApiError::RateLimited(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limited", msg.clone())
            }
            ApiError::ServiceUnavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                msg.clone(),
            ),
            ApiError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".to_string(),
                )
            }
            ApiError::Database(e) => {
                tracing::error!("Database error: {}", e);
                match e {
                    crate::db::DatabaseError::NotFound(msg) => {
                        (StatusCode::NOT_FOUND, "not_found", msg.clone())
                    }
                    // Optimistic-concurrency guard lost the race after the
                    // handler's precondition check — same client remedy as a
                    // stale If-Match: re-GET and retry (RFC 9110 §13.1.1).
                    crate::db::DatabaseError::VersionConflict(msg) => (
                        StatusCode::PRECONDITION_FAILED,
                        "precondition_failed",
                        msg.clone(),
                    ),
                    // Constraint violations are CLIENT errors, not 500s
                    // (Plan 07 Phase E / finding API-7): a unique-constraint
                    // breach is a 409, a check/validation breach a 422.
                    crate::db::DatabaseError::Connection(sqlx_err) => {
                        match classify_sqlx(sqlx_err) {
                            Some(classified) => classified,
                            None => (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "database_error",
                                "A database error occurred".to_string(),
                            ),
                        }
                    }
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database_error",
                        "A database error occurred".to_string(),
                    ),
                }
            }
        };

        problem_response(status, code, message)
    }
}

/// Classify a raw sqlx error into a client-attributable HTTP outcome, if it is
/// one. PostgreSQL reports SQLSTATE `23505` (unique) / `23514` (check) /
/// `23503` (foreign key); SQLite has no SQLSTATE through the Any driver, so
/// its constraint failures are matched by message.
fn classify_sqlx(e: &sqlx::Error) -> Option<(StatusCode, &'static str, String)> {
    let sqlx::Error::Database(db_err) = e else {
        return None;
    };
    let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
    let msg = db_err.message().to_lowercase();

    if code == "23505" || msg.contains("unique constraint") {
        return Some((
            StatusCode::CONFLICT,
            "conflict",
            "A resource with these unique attributes already exists".to_string(),
        ));
    }
    if code == "23514" || msg.contains("check constraint") {
        return Some((
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            "The request violates a data constraint".to_string(),
        ));
    }
    if code == "23503" || msg.contains("foreign key constraint") {
        return Some((
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            "The request references a resource that does not exist".to_string(),
        ));
    }
    None
}

/// Build the RFC 9457 response: `application/problem+json` with a stable,
/// documentation-anchored problem type per error code.
fn problem_response(status: StatusCode, code: &str, detail: String) -> Response {
    let body = ProblemDetails {
        problem_type: format!("https://docs.reaper.dev/problems/{code}"),
        title: status
            .canonical_reason()
            .unwrap_or("Unknown Error")
            .to_string(),
        status: status.as_u16(),
        detail,
        // `IntoResponse` cannot see the request; the `problem_instance`
        // middleware stamps these onto the serialized body (R2-08).
        instance: None,
        request_id: None,
        code: code.to_string(),
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// Stamp the RFC 9457 `instance` member (and the `request_id` extension
/// member) onto outgoing `application/problem+json` responses (finding
/// R2-08).
///
/// `ApiError::into_response` cannot see the request, so the request path is
/// added HERE, in one place, for every problem response the router emits —
/// including rejections produced by extractors and by the auth gateway. The
/// middleware is idempotent (members already present are never overwritten),
/// so it may be layered both inside the served router and outside the auth
/// gateway without double-stamping.
///
/// Only error bodies are touched (gated on the problem+json content type);
/// success responses stream through untouched.
pub async fn problem_instance(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let request_id = request
        .extensions()
        .get::<crate::middleware::RequestId>()
        .map(|r| r.0.clone());

    let response = next.run(request).await;

    let is_problem = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/problem+json"));
    if !is_problem {
        return response;
    }

    // Problem bodies are small, locally generated JSON documents; buffering
    // them is cheap and only happens on error paths.
    let (mut parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        // The body was consumed and cannot be recovered; fail closed with an
        // empty body of the same status rather than a hung response.
        Err(_) => return parts.status.into_response(),
    };

    let Ok(serde_json::Value::Object(mut obj)) = serde_json::from_slice(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    obj.entry("instance")
        .or_insert_with(|| serde_json::Value::String(path));
    if let Some(id) = request_id {
        obj.entry("request_id")
            .or_insert_with(|| serde_json::Value::String(id));
    }

    let new_bytes =
        serde_json::to_vec(&serde_json::Value::Object(obj)).unwrap_or_else(|_| bytes.to_vec());
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(new_bytes))
}

/// Result type alias for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

// Error conversions
impl From<crate::auth::users::UserError> for ApiError {
    fn from(e: crate::auth::users::UserError) -> Self {
        use crate::auth::users::UserError;
        match e {
            UserError::NotFound => ApiError::NotFound("User not found".to_string()),
            UserError::EmailExists => ApiError::Conflict("Email already exists".to_string()),
            UserError::InvalidCredentials => {
                ApiError::Unauthorized("Invalid credentials".to_string())
            }
            UserError::SessionExpired => ApiError::Unauthorized("Session expired".to_string()),
            UserError::SessionNotFound => ApiError::Unauthorized("Session not found".to_string()),
            UserError::AccountSuspended => ApiError::Forbidden("Account suspended".to_string()),
            UserError::EmailNotVerified => ApiError::Forbidden("Email not verified".to_string()),
            UserError::InvalidToken => ApiError::BadRequest("Invalid token".to_string()),
            UserError::TokenExpired => ApiError::BadRequest("Token expired".to_string()),
            UserError::PasswordHash(msg) => ApiError::Internal(format!("Password error: {}", msg)),
            UserError::Database(e) => ApiError::Internal(format!("Database error: {}", e)),
        }
    }
}

impl From<crate::audit::AuditError> for ApiError {
    fn from(e: crate::audit::AuditError) -> Self {
        ApiError::Internal(format!("Audit error: {}", e))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        // Constraint violations are the CLIENT's error (409/422), not a 500 —
        // classified in IntoResponse via classify_sqlx. Everything else is
        // internal.
        if classify_sqlx(&e).is_some() {
            return ApiError::Database(crate::db::DatabaseError::Connection(e));
        }
        tracing::error!("SQLx error: {}", e);
        ApiError::Internal("Database error".to_string())
    }
}
