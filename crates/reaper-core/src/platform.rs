//! Platform types and traits

use crate::agent::{Agent, AgentId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a platform (management layer) instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    /// Human-readable name identifying this platform instance.
    pub name: String,
}

/// Registry of all agents known to the platform, keyed by agent ID.
pub type AgentRegistry = HashMap<AgentId, Agent>;

/// The management layer's view of a deployment: its own configuration plus
/// the fleet of agents it coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    /// This platform instance's configuration.
    pub config: PlatformConfig,
    /// Agents currently registered with this platform.
    pub agents: AgentRegistry,
}
