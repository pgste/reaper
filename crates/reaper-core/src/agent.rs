//! Agent types and traits

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an agent instance in the fleet.
pub type AgentId = Uuid;

/// Health state of a registered agent as seen by the platform.
///
/// `#[non_exhaustive]`: new states may be added (e.g. draining, degraded), so
/// downstream matches must carry a wildcard arm — treat unknown states as not
/// healthy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentStatus {
    /// The agent is responding to health checks and serving traffic.
    Healthy,
    /// The agent failed its most recent health check.
    Unhealthy,
    /// No health information is available yet (e.g., freshly registered).
    Unknown,
}

/// Static configuration an agent registers with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier of the agent this configuration belongs to.
    pub id: AgentId,
    /// Human-readable agent name (for dashboards and logs).
    pub name: String,
}

/// A registered agent as tracked by the platform: identity, current health,
/// and its registered configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier of the agent.
    pub id: AgentId,
    /// Last observed health state.
    pub status: AgentStatus,
    /// Configuration the agent registered with.
    pub config: AgentConfig,
}
