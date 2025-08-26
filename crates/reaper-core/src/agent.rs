//! Agent types and traits

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type AgentId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: AgentId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub status: AgentStatus,
    pub config: AgentConfig,
}
