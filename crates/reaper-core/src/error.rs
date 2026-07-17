//! Error types for the Reaper platform

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ReaperError>;

#[derive(Error, Debug)]
pub enum ReaperError {
    #[error("Policy not found: {policy_id}")]
    PolicyNotFound { policy_id: String },

    #[error("Agent not found: {agent_id}")]
    AgentNotFound { agent_id: String },

    #[error("Invalid policy definition: {reason}")]
    InvalidPolicy { reason: String },

    /// The policy targets a newer DSL language version than this engine
    /// implements. Fail closed (round-3 Plan 04) — an old engine must never
    /// silently misinterpret a newer policy, mirroring the bundle format's
    /// newer-version reject.
    #[error("Unsupported policy language version: got {got}, this engine implements {supported}")]
    LanguageVersionUnsupported { got: u32, supported: u32 },

    #[error("Policy evaluation failed: {reason}")]
    EvaluationError { reason: String },

    #[error("Agent communication failed: {reason}")]
    AgentCommunicationError { reason: String },

    #[error("Platform operation failed: {reason}")]
    PlatformError { reason: String },

    #[error("Serialization error: {source}")]
    SerializationError {
        #[from]
        source: serde_json::Error,
    },

    #[error("Materialized view error: {0}")]
    ViewError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Binary serialization error: {0}")]
    BinarySerializationError(String),
}
