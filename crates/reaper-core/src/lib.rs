//! Core types and traits shared across the Reaper platform: policy and agent
//! identities, the common error type, configuration, bundle signing and
//! revocation, and agentic capabilities. Both the enforcement layer (agent)
//! and the management layer (platform) build on this crate.
#![deny(missing_docs)]

pub mod agent;
pub mod bundle_signing;
pub mod capability;
pub mod config;
pub mod error;
pub mod platform;
pub mod policy;
pub mod revocation;

pub use agent::{Agent, AgentConfig, AgentId, AgentStatus};
pub use config::{
    resolve_bind, AgentSettings, CacheSettings, ConfigError, DataSettings, ManagementSettings,
    ObservabilitySettings, PerformanceSettings, PolicySettings, ReaperAgentConfig, TlsSettings,
};
pub use error::{ReaperError, Result};
pub use platform::{AgentRegistry, Platform, PlatformConfig};
pub use policy::{Policy, PolicyEngine, PolicyId, PolicyVersion};

/// Current Reaper version for compatibility checks
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Reaper build information for telemetry and debugging
pub const BUILD_INFO: &str = concat!(
    "Reaper ",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("CARGO_PKG_NAME"),
    ")"
);

/// Standard API endpoints for Reaper services
pub mod endpoints {
    /// Health check endpoint (agent and platform).
    pub const HEALTH: &str = "/health";
    /// Metrics endpoint (Prometheus text format).
    pub const METRICS: &str = "/metrics";
    /// Policy CRUD / listing endpoint.
    pub const API_V1_POLICIES: &str = "/api/v1/policies";
    /// Agent registry endpoint (platform).
    pub const API_V1_AGENTS: &str = "/api/v1/agents";
    /// Policy evaluation endpoint (agent).
    pub const API_V1_MESSAGES: &str = "/api/v1/messages";
}
