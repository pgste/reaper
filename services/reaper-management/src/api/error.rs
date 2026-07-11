//! API error handling
//!
//! Provides consistent error responses across all API endpoints.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

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

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
}

/// RFC 9457 `application/problem+json` error body (Plan 07, Phase E). `code`
/// is a Reaper extension member kept for programmatic matching; `type` is a
/// stable, documentation-anchored URI reference per problem class.
#[derive(Debug, Serialize)]
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
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone()),
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
        code: code.to_string(),
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/problem+json"),
    );
    response
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
