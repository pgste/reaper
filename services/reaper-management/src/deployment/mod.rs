//! Deployment module for managing rollouts and strategies
//!
//! Provides orchestration for controlled policy deployments including
//! canary, percentage-based, and label-selector rollouts.

pub mod service;
pub mod supervisor;

pub use service::{
    AgentInfo, DeploymentError, DeploymentService, DryRunResult, RollbackTriggerEvaluation,
    RolloutResult, SkippedAgent, StrategyInfo,
};
