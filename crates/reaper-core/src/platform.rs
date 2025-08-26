//! Platform types and traits

use crate::agent::{Agent, AgentId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub name: String,
}

pub type AgentRegistry = HashMap<AgentId, Agent>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub config: PlatformConfig,
    pub agents: AgentRegistry,
}
