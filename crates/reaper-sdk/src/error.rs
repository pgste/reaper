//! Error types for the Reaper SDK

use thiserror::Error;

/// Result type for SDK operations
pub type Result<T> = std::result::Result<T, ReaperError>;

/// Errors that can occur when using the Reaper SDK
#[derive(Debug, Error)]
pub enum ReaperError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// HTTP error with status code
    #[error("HTTP error: {0}")]
    HttpStatus(reqwest::StatusCode),

    /// Invalid endpoint URL
    #[error("Invalid endpoint URL: {0}")]
    InvalidEndpoint(String),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Policy evaluation was denied
    #[error("Policy evaluation denied")]
    PolicyDenied,

    /// Agent returned an error
    #[error("Agent error: {0}")]
    AgentError(String),

    /// Bundle operation failed
    #[error("Bundle operation failed: {0}")]
    BundleError(String),

    /// Entity operation failed
    #[error("Entity operation failed: {0}")]
    EntityError(String),

    /// Unix socket connection failed
    #[error("Unix socket error: {0}")]
    UnixSocketError(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}
