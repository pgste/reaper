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

/// API error type
#[derive(Debug, Error)]
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

/// Error response body
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
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
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database_error",
                        "A database error occurred".to_string(),
                    ),
                }
            }
        };

        let body = ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details: None,
            },
        };

        (status, Json(body)).into_response()
    }
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
        tracing::error!("SQLx error: {}", e);
        ApiError::Internal("Database error".to_string())
    }
}
