//! Agent deployment tracking domain model
//!
//! Tracks per-agent deployment status for rollouts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of an agent deployment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentDeploymentStatus {
    /// Deployment is queued
    Pending,
    /// Deployment is in progress
    Deploying,
    /// Deployment completed successfully
    Deployed,
    /// Deployment failed
    Failed,
}

impl std::fmt::Display for AgentDeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentDeploymentStatus::Pending => write!(f, "pending"),
            AgentDeploymentStatus::Deploying => write!(f, "deploying"),
            AgentDeploymentStatus::Deployed => write!(f, "deployed"),
            AgentDeploymentStatus::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for AgentDeploymentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(AgentDeploymentStatus::Pending),
            "deploying" => Ok(AgentDeploymentStatus::Deploying),
            "deployed" => Ok(AgentDeploymentStatus::Deployed),
            "failed" => Ok(AgentDeploymentStatus::Failed),
            _ => Err(format!("Invalid agent deployment status: {}", s)),
        }
    }
}

/// Agent deployment record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDeployment {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub rollout_id: Option<Uuid>,
    pub status: AgentDeploymentStatus,
    pub error_message: Option<String>,
    pub deployed_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl AgentDeployment {
    /// Create a new pending deployment
    pub fn new(agent_id: Uuid, bundle_id: Uuid, rollout_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_id,
            bundle_id,
            rollout_id,
            status: AgentDeploymentStatus::Pending,
            error_message: None,
            deployed_at: None,
            acknowledged_at: None,
            created_at: Utc::now(),
        }
    }

    /// Mark deployment as in progress
    pub fn mark_deploying(&mut self) {
        self.status = AgentDeploymentStatus::Deploying;
    }

    /// Mark deployment as successful
    pub fn mark_deployed(&mut self) {
        self.status = AgentDeploymentStatus::Deployed;
        self.deployed_at = Some(Utc::now());
    }

    /// Mark deployment as failed
    pub fn mark_failed(&mut self, error: String) {
        self.status = AgentDeploymentStatus::Failed;
        self.error_message = Some(error);
    }

    /// Mark deployment as acknowledged by agent
    pub fn acknowledge(&mut self) {
        self.acknowledged_at = Some(Utc::now());
    }

    /// Check if deployment is terminal (deployed or failed)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            AgentDeploymentStatus::Deployed | AgentDeploymentStatus::Failed
        )
    }
}

/// Summary of agent deployments for a rollout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentSummary {
    pub total_agents: u32,
    pub pending: u32,
    pub deploying: u32,
    pub deployed: u32,
    pub failed: u32,
    pub acknowledged: u32,
}

impl DeploymentSummary {
    pub fn new() -> Self {
        Self {
            total_agents: 0,
            pending: 0,
            deploying: 0,
            deployed: 0,
            failed: 0,
            acknowledged: 0,
        }
    }

    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_agents == 0 {
            return 0.0;
        }
        (self.deployed as f64 / self.total_agents as f64) * 100.0
    }

    /// Calculate failure rate
    pub fn failure_rate(&self) -> f64 {
        if self.total_agents == 0 {
            return 0.0;
        }
        (self.failed as f64 / self.total_agents as f64) * 100.0
    }

    /// Check if all deployments are complete
    pub fn is_complete(&self) -> bool {
        self.pending == 0 && self.deploying == 0
    }
}

impl Default for DeploymentSummary {
    fn default() -> Self {
        Self::new()
    }
}

/// Auto-rollback configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackConfig {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub is_enabled: bool,
    pub error_rate_threshold: f64,
    pub window_seconds: u32,
    pub min_requests: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl RollbackConfig {
    /// Create a new rollback config with defaults
    pub fn new(org_id: Uuid, namespace_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            org_id,
            namespace_id,
            is_enabled: false,
            error_rate_threshold: 5.0,
            window_seconds: 300,
            min_requests: 100,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

/// Input for creating/updating rollback config
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRollbackConfig {
    pub is_enabled: Option<bool>,
    pub error_rate_threshold: Option<f64>,
    pub window_seconds: Option<u32>,
    pub min_requests: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deployment_status_display() {
        assert_eq!(AgentDeploymentStatus::Pending.to_string(), "pending");
        assert_eq!(AgentDeploymentStatus::Deployed.to_string(), "deployed");
        assert_eq!(AgentDeploymentStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_deployment_lifecycle() {
        let mut deployment = AgentDeployment::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
        );

        assert_eq!(deployment.status, AgentDeploymentStatus::Pending);
        assert!(!deployment.is_terminal());

        deployment.mark_deploying();
        assert_eq!(deployment.status, AgentDeploymentStatus::Deploying);
        assert!(!deployment.is_terminal());

        deployment.mark_deployed();
        assert_eq!(deployment.status, AgentDeploymentStatus::Deployed);
        assert!(deployment.is_terminal());
        assert!(deployment.deployed_at.is_some());
    }

    #[test]
    fn test_deployment_failure() {
        let mut deployment = AgentDeployment::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
        );

        deployment.mark_failed("Connection timeout".to_string());
        assert_eq!(deployment.status, AgentDeploymentStatus::Failed);
        assert!(deployment.is_terminal());
        assert_eq!(deployment.error_message.as_deref(), Some("Connection timeout"));
    }

    #[test]
    fn test_deployment_summary_rates() {
        let mut summary = DeploymentSummary::new();
        summary.total_agents = 100;
        summary.deployed = 90;
        summary.failed = 10;

        assert_eq!(summary.success_rate(), 90.0);
        assert_eq!(summary.failure_rate(), 10.0);
    }
}
