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
}
