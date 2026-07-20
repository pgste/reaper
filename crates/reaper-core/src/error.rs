//! Error types for the Reaper platform

use thiserror::Error;

/// Convenience alias for results whose error type is [`ReaperError`].
pub type Result<T> = std::result::Result<T, ReaperError>;

/// Top-level error type shared across the Reaper platform.
///
/// `#[non_exhaustive]`: new error cases are added as the platform grows, so
/// downstream matches must carry a wildcard arm (treat unknown errors as
/// failures, never ignore them).
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ReaperError {
    /// A lookup referenced a policy ID that is not in the store.
    #[error("Policy not found: {policy_id}")]
    PolicyNotFound {
        /// The policy ID that could not be resolved.
        policy_id: String,
    },

    /// An operation referenced an agent that is not registered.
    #[error("Agent not found: {agent_id}")]
    AgentNotFound {
        /// The agent ID that could not be resolved.
        agent_id: String,
    },

    /// A policy failed validation (parse error, bad rule, missing field).
    #[error("Invalid policy definition: {reason}")]
    InvalidPolicy {
        /// Human-readable description of what made the policy invalid.
        reason: String,
    },

    /// The policy targets a newer DSL language version than this engine
    /// implements. Fail closed (round-3 Plan 04) — an old engine must never
    /// silently misinterpret a newer policy, mirroring the bundle format's
    /// newer-version reject.
    #[error("Unsupported policy language version: got {got}, this engine implements {supported}")]
    LanguageVersionUnsupported {
        /// The language version the policy declared.
        got: u32,
        /// The highest language version this engine implements.
        supported: u32,
    },

    /// Evaluating a request against a policy failed at runtime.
    #[error("Policy evaluation failed: {reason}")]
    EvaluationError {
        /// Human-readable description of the evaluation failure.
        reason: String,
    },

    /// A request to an agent could not be completed (network, timeout, bad response).
    #[error("Agent communication failed: {reason}")]
    AgentCommunicationError {
        /// Human-readable description of the communication failure.
        reason: String,
    },

    /// A platform-side (management layer) operation failed.
    #[error("Platform operation failed: {reason}")]
    PlatformError {
        /// Human-readable description of the platform failure.
        reason: String,
    },

    /// JSON (de)serialization failed.
    #[error("Serialization error: {source}")]
    SerializationError {
        /// The underlying serde_json error.
        #[from]
        source: serde_json::Error,
    },

    /// A materialized data view could not be built or refreshed.
    #[error("Materialized view error: {0}")]
    ViewError(String),

    /// Parsing policy source text failed.
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Binary bundle encoding/decoding failed.
    #[error("Binary serialization error: {0}")]
    BinarySerializationError(String),
}
