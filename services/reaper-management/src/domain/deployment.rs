//! Deployment domain models
//!
//! Provides controlled deployment strategies including canary, percentage-based,
//! and label-selector rollouts with health gates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Deployment strategy types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyType {
    /// Deploy to all agents immediately
    Immediate,
    /// Deploy to canary agents first, then proceed
    Canary,
    /// Progressive rollout by percentage
    Percentage,
    /// Deploy only to agents matching labels
    LabelSelector,
}

impl Default for StrategyType {
    fn default() -> Self {
        Self::Immediate
    }
}

impl std::fmt::Display for StrategyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Immediate => write!(f, "immediate"),
            Self::Canary => write!(f, "canary"),
            Self::Percentage => write!(f, "percentage"),
            Self::LabelSelector => write!(f, "label_selector"),
        }
    }
}

impl std::str::FromStr for StrategyType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "immediate" => Ok(Self::Immediate),
            "canary" => Ok(Self::Canary),
            "percentage" => Ok(Self::Percentage),
            "label_selector" | "labelselector" => Ok(Self::LabelSelector),
            _ => Err(format!("Unknown strategy type: {}", s)),
        }
    }
}

/// Deployment strategy entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentStrategy {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub name: String,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Strategy-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StrategyConfig {
    /// Immediate deployment config
    Immediate {},
    /// Canary deployment config
    Canary {
        /// Labels that identify canary agents
        canary_labels: HashMap<String, String>,
        /// Time to wait after canary deployment before proceeding
        wait_seconds: u64,
        /// Require manual approval before proceeding
        require_approval: bool,
    },
    /// Percentage-based rollout config
    Percentage {
        /// Rollout waves as percentages (e.g., [10, 25, 50, 100])
        waves: Vec<u8>,
        /// Delay between waves in seconds
        wave_delay_seconds: u64,
        /// Require manual approval between waves
        require_approval: bool,
    },
    /// Label selector deployment config
    LabelSelector {
        /// Required labels for deployment
        labels: HashMap<String, String>,
    },
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self::Immediate {}
    }
}

/// Rollout status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    /// Rollout is pending start
    Pending,
    /// Rollout is in progress
    InProgress,
    /// Awaiting manual approval
    AwaitingApproval,
    /// Rollout completed successfully
    Completed,
    /// Rollout failed
    Failed,
    /// Rollout was rolled back
    RolledBack,
    /// Rollout was cancelled
    Cancelled,
}

impl Default for RolloutStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for RolloutStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::AwaitingApproval => write!(f, "awaiting_approval"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::RolledBack => write!(f, "rolled_back"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for RolloutStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "awaiting_approval" => Ok(Self::AwaitingApproval),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "rolled_back" => Ok(Self::RolledBack),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Unknown rollout status: {}", s)),
        }
    }
}

/// Rollout entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
    pub status: RolloutStatus,
    pub current_wave: u32,
    pub target_agent_count: u32,
    pub deployed_agent_count: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Rollout {
    /// Check if rollout is terminal (cannot be changed)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            RolloutStatus::Completed
                | RolloutStatus::Failed
                | RolloutStatus::RolledBack
                | RolloutStatus::Cancelled
        )
    }

    /// Check if rollout can be cancelled
    pub fn can_cancel(&self) -> bool {
        matches!(
            self.status,
            RolloutStatus::Pending | RolloutStatus::InProgress | RolloutStatus::AwaitingApproval
        )
    }

    /// Check if rollout can proceed to next wave
    pub fn can_proceed(&self) -> bool {
        matches!(self.status, RolloutStatus::AwaitingApproval)
    }

    /// Get progress percentage
    pub fn progress_percent(&self) -> f64 {
        if self.target_agent_count == 0 {
            return 0.0;
        }
        (self.deployed_agent_count as f64 / self.target_agent_count as f64) * 100.0
    }
}

/// Rollout wave status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveStatus {
    Pending,
    Deploying,
    Completed,
    Failed,
}

impl Default for WaveStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for WaveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Deploying => write!(f, "deploying"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for WaveStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "deploying" => Ok(Self::Deploying),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("Unknown wave status: {}", s)),
        }
    }
}

/// Rollout wave entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutWave {
    pub id: Uuid,
    pub rollout_id: Uuid,
    pub wave_number: u32,
    pub target_agents: Vec<Uuid>,
    pub status: WaveStatus,
    pub deployed_count: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Version pin entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionPin {
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub pinned_by: Option<String>,
    pub reason: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl VersionPin {
    /// Check if the pin has expired
    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|e| e < Utc::now()).unwrap_or(false)
    }
}

/// Input for creating a deployment strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDeploymentStrategy {
    pub name: String,
    pub namespace_id: Option<Uuid>,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    #[serde(default)]
    pub is_default: bool,
}

/// Input for starting a rollout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRollout {
    pub bundle_id: Uuid,
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
}

/// Input for creating a version pin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVersionPin {
    pub bundle_id: Uuid,
    pub reason: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Rollback request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackRequest {
    pub reason: String,
    pub target_bundle_id: Option<Uuid>, // If not specified, rollback to previous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_type_parsing() {
        assert_eq!(
            "immediate".parse::<StrategyType>().unwrap(),
            StrategyType::Immediate
        );
        assert_eq!(
            "canary".parse::<StrategyType>().unwrap(),
            StrategyType::Canary
        );
        assert_eq!(
            "percentage".parse::<StrategyType>().unwrap(),
            StrategyType::Percentage
        );
        assert_eq!(
            "label_selector".parse::<StrategyType>().unwrap(),
            StrategyType::LabelSelector
        );
    }

    #[test]
    fn test_rollout_status_parsing() {
        assert_eq!(
            "pending".parse::<RolloutStatus>().unwrap(),
            RolloutStatus::Pending
        );
        assert_eq!(
            "in_progress".parse::<RolloutStatus>().unwrap(),
            RolloutStatus::InProgress
        );
        assert_eq!(
            "completed".parse::<RolloutStatus>().unwrap(),
            RolloutStatus::Completed
        );
    }

    #[test]
    fn test_rollout_terminal_states() {
        let mut rollout = Rollout {
            id: Uuid::new_v4(),
            bundle_id: Uuid::new_v4(),
            strategy_id: None,
            namespace_id: None,
            status: RolloutStatus::InProgress,
            current_wave: 0,
            target_agent_count: 10,
            deployed_agent_count: 5,
            started_at: Some(Utc::now()),
            completed_at: None,
            error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!rollout.is_terminal());
        assert!(rollout.can_cancel());

        rollout.status = RolloutStatus::Completed;
        assert!(rollout.is_terminal());
        assert!(!rollout.can_cancel());
    }

    #[test]
    fn test_rollout_progress() {
        let rollout = Rollout {
            id: Uuid::new_v4(),
            bundle_id: Uuid::new_v4(),
            strategy_id: None,
            namespace_id: None,
            status: RolloutStatus::InProgress,
            current_wave: 1,
            target_agent_count: 100,
            deployed_agent_count: 50,
            started_at: Some(Utc::now()),
            completed_at: None,
            error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(rollout.progress_percent(), 50.0);
    }

    #[test]
    fn test_version_pin_expiry() {
        let pin = VersionPin {
            agent_id: Uuid::new_v4(),
            bundle_id: Uuid::new_v4(),
            pinned_by: Some("admin".to_string()),
            reason: Some("Testing".to_string()),
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
            created_at: Utc::now(),
        };

        assert!(pin.is_expired());

        let future_pin = VersionPin {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..pin
        };

        assert!(!future_pin.is_expired());
    }
}
