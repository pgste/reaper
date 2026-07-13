//! Deployment service types

use thiserror::Error;
use uuid::Uuid;

use crate::domain::agent_deployment::RollbackMode;
use crate::domain::deployment::{Rollout, RolloutWave};

/// Deployment service errors
#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("Bundle not found: {0}")]
    BundleNotFound(String),
    #[error("Strategy not found: {0}")]
    StrategyNotFound(String),
    #[error("Rollout not found: {0}")]
    RolloutNotFound(String),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Bundle not ready for deployment: {0}")]
    BundleNotReady(String),
    #[error("Active rollout exists for bundle: {0}")]
    ActiveRolloutExists(String),
    #[error("No agents available for deployment")]
    NoAgentsAvailable,
    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
}

/// Outcome of evaluating a rollout against its resolved auto-rollback
/// config — the single source of truth shared by the operator-facing
/// `check-rollback` / `rollback-status` endpoints and the autonomous
/// rollout supervisor (B2 / PROD R2-1).
#[derive(Debug, Clone)]
pub struct RollbackTriggerEvaluation {
    /// Whether auto-rollback is enabled for this rollout's namespace/org
    pub enabled: bool,
    /// Resolved config mode: monitor (alert only) or enforce (supervisor acts)
    pub mode: RollbackMode,
    /// Whether the trigger fired (error rate above threshold with enough data)
    pub should_rollback: bool,
    /// Current failure rate percentage across the rollout's agent deployments
    pub current_error_rate: f64,
    /// Configured error-rate threshold percentage
    pub threshold: f64,
    /// Completed (deployed + failed) agent deployments observed
    pub completed_count: u32,
    /// Minimum completed deployments before the trigger may fire
    pub min_requests: u32,
    /// Human-readable explanation of the decision
    pub reason: String,
}

/// Result of a rollout operation
#[derive(Debug)]
pub struct RolloutResult {
    pub rollout: Rollout,
    pub waves: Vec<RolloutWave>,
    pub target_agents: Vec<Uuid>,
}

/// Result of a dry-run rollout simulation
#[derive(Debug)]
pub struct DryRunResult {
    pub would_deploy_to: Vec<AgentInfo>,
    pub agents_skipped: Vec<SkippedAgent>,
    pub target_count: u32,
    pub validation_errors: Vec<String>,
    pub strategy: Option<StrategyInfo>,
}

/// Agent info for dry-run response
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
}

/// Skipped agent for dry-run response
#[derive(Debug, Clone)]
pub struct SkippedAgent {
    pub id: Uuid,
    pub name: String,
    pub reason: String,
}

/// Strategy info for dry-run response
#[derive(Debug, Clone)]
pub struct StrategyInfo {
    pub id: Uuid,
    pub name: String,
    pub strategy_type: String,
}
