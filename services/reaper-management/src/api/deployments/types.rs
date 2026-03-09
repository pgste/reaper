//! Request and response types for deployment API endpoints.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    deployment::{
        AgentInfo as ServiceAgentInfo, DryRunResult, SkippedAgent as ServiceSkippedAgent,
        StrategyInfo as ServiceStrategyInfo,
    },
    domain::agent_deployment::{AgentDeployment, DeploymentSummary, RollbackConfig},
    domain::deployment::{
        DeploymentStrategy, Rollout, RolloutWave, StrategyConfig, StrategyType, VersionPin,
    },
};

// ==================== Strategy Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    pub namespace_id: Option<Uuid>,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct StrategiesQuery {
    pub namespace_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub name: String,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    pub is_default: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<DeploymentStrategy> for StrategyResponse {
    fn from(s: DeploymentStrategy) -> Self {
        Self {
            id: s.id,
            org_id: s.org_id,
            namespace_id: s.namespace_id,
            name: s.name,
            strategy_type: s.strategy_type,
            config: s.config,
            is_default: s.is_default,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StrategyInfo {
    pub id: Uuid,
    pub name: String,
    pub strategy_type: String,
}

impl From<ServiceStrategyInfo> for StrategyInfo {
    fn from(s: ServiceStrategyInfo) -> Self {
        Self {
            id: s.id,
            name: s.name,
            strategy_type: s.strategy_type,
        }
    }
}

// ==================== Rollout Types ====================

#[derive(Debug, Deserialize)]
pub struct RolloutRequest {
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct RolloutsQuery {
    pub namespace_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    50
}

#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct RolloutResponse {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
    pub status: String,
    pub current_wave: u32,
    pub target_agent_count: u32,
    pub deployed_agent_count: u32,
    pub progress_percent: f64,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Rollout> for RolloutResponse {
    fn from(r: Rollout) -> Self {
        let progress = r.progress_percent();
        Self {
            id: r.id,
            bundle_id: r.bundle_id,
            strategy_id: r.strategy_id,
            namespace_id: r.namespace_id,
            status: r.status.to_string(),
            current_wave: r.current_wave,
            target_agent_count: r.target_agent_count,
            deployed_agent_count: r.deployed_agent_count,
            progress_percent: progress,
            started_at: r.started_at,
            completed_at: r.completed_at,
            error: r.error,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RolloutDetailResponse {
    #[serde(flatten)]
    pub rollout: RolloutResponse,
    pub waves: Vec<WaveResponse>,
}

#[derive(Debug, Serialize)]
pub struct WaveResponse {
    pub id: Uuid,
    pub wave_number: u32,
    pub target_agent_count: usize,
    pub deployed_count: u32,
    pub status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<RolloutWave> for WaveResponse {
    fn from(w: RolloutWave) -> Self {
        Self {
            id: w.id,
            wave_number: w.wave_number,
            target_agent_count: w.target_agents.len(),
            deployed_count: w.deployed_count,
            status: w.status.to_string(),
            started_at: w.started_at,
            completed_at: w.completed_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RolloutStartResponse {
    pub rollout: RolloutResponse,
    pub waves: Vec<WaveResponse>,
    pub target_agent_count: usize,
}

/// Unified response for rollout - either dry-run or actual start
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RolloutOrDryRun {
    DryRun(DryRunResponse),
    Rollout(RolloutStartResponse),
}

// ==================== Dry-Run Types ====================

/// Dry-run response showing what would happen without executing
#[derive(Debug, Serialize)]
pub struct DryRunResponse {
    /// Agents that would receive the deployment
    pub would_deploy_to: Vec<AgentInfo>,
    /// Agents that would be skipped with reasons
    pub agents_skipped: Vec<SkippedAgent>,
    /// Total count of target agents
    pub target_count: u32,
    /// Validation errors (if any)
    pub validation_errors: Vec<String>,
    /// Strategy that would be used
    pub strategy: Option<StrategyInfo>,
}

impl From<DryRunResult> for DryRunResponse {
    fn from(r: DryRunResult) -> Self {
        Self {
            would_deploy_to: r.would_deploy_to.into_iter().map(Into::into).collect(),
            agents_skipped: r.agents_skipped.into_iter().map(Into::into).collect(),
            target_count: r.target_count,
            validation_errors: r.validation_errors,
            strategy: r.strategy.map(Into::into),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
}

impl From<ServiceAgentInfo> for AgentInfo {
    fn from(a: ServiceAgentInfo) -> Self {
        Self {
            id: a.id,
            name: a.name,
            hostname: a.hostname,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SkippedAgent {
    pub id: Uuid,
    pub name: String,
    pub reason: String,
}

impl From<ServiceSkippedAgent> for SkippedAgent {
    fn from(s: ServiceSkippedAgent) -> Self {
        Self {
            id: s.id,
            name: s.name,
            reason: s.reason,
        }
    }
}

// ==================== Rollback Types ====================

#[derive(Debug, Deserialize)]
pub struct RollbackRequest {
    pub reason: String,
    pub target_bundle_id: Option<Uuid>,
}

// ==================== Version Pin Types ====================

#[derive(Debug, Deserialize)]
pub struct CreatePinRequest {
    pub bundle_id: Uuid,
    pub reason: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct PinResponse {
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub pinned_by: Option<String>,
    pub reason: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_expired: bool,
}

impl From<VersionPin> for PinResponse {
    fn from(p: VersionPin) -> Self {
        let is_expired = p.is_expired();
        Self {
            agent_id: p.agent_id,
            bundle_id: p.bundle_id,
            pinned_by: p.pinned_by,
            reason: p.reason,
            expires_at: p.expires_at,
            created_at: p.created_at,
            is_expired,
        }
    }
}

// ==================== Deployment Status Types ====================

/// Response for agent deployment
#[derive(Debug, Serialize)]
pub struct AgentDeploymentResponse {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub rollout_id: Option<Uuid>,
    pub status: String,
    pub error_message: Option<String>,
    pub deployed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub acknowledged_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<AgentDeployment> for AgentDeploymentResponse {
    fn from(d: AgentDeployment) -> Self {
        Self {
            id: d.id,
            agent_id: d.agent_id,
            bundle_id: d.bundle_id,
            rollout_id: d.rollout_id,
            status: d.status.to_string(),
            error_message: d.error_message,
            deployed_at: d.deployed_at,
            acknowledged_at: d.acknowledged_at,
            created_at: d.created_at,
        }
    }
}

/// Response for deployment summary
#[derive(Debug, Serialize)]
pub struct DeploymentSummaryResponse {
    pub total_agents: u32,
    pub pending: u32,
    pub deploying: u32,
    pub deployed: u32,
    pub failed: u32,
    pub acknowledged: u32,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub is_complete: bool,
}

impl From<DeploymentSummary> for DeploymentSummaryResponse {
    fn from(s: DeploymentSummary) -> Self {
        Self {
            total_agents: s.total_agents,
            pending: s.pending,
            deploying: s.deploying,
            deployed: s.deployed,
            failed: s.failed,
            acknowledged: s.acknowledged,
            success_rate: s.success_rate(),
            failure_rate: s.failure_rate(),
            is_complete: s.is_complete(),
        }
    }
}

// ==================== Auto-Rollback Types ====================

/// Response for auto-rollback configuration
#[derive(Debug, Serialize)]
pub struct RollbackConfigResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub is_enabled: bool,
    pub error_rate_threshold: f64,
    pub window_seconds: u32,
    pub min_requests: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<RollbackConfig> for RollbackConfigResponse {
    fn from(c: RollbackConfig) -> Self {
        Self {
            id: c.id,
            org_id: c.org_id,
            namespace_id: c.namespace_id,
            is_enabled: c.is_enabled,
            error_rate_threshold: c.error_rate_threshold,
            window_seconds: c.window_seconds,
            min_requests: c.min_requests,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

/// Result of checking auto-rollback trigger
#[derive(Debug, Serialize)]
pub struct CheckRollbackResponse {
    /// Whether rollback should be triggered
    pub should_rollback: bool,
    /// Current error rate percentage
    pub current_error_rate: f64,
    /// Configured threshold
    pub threshold: f64,
    /// Number of completed deployments in window
    pub completed_count: u32,
    /// Required minimum requests
    pub min_requests: u32,
    /// Reason for the decision
    pub reason: String,
}
