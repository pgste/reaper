pub mod agent;
pub mod error;
pub mod platform;
pub mod policy;

pub use agent::{Agent, AgentConfig, AgentId, AgentStatus};
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
    pub const HEALTH: &str = "/health";
    pub const METRICS: &str = "/metrics";
    pub const API_V1_POLICIES: &str = "/api/v1/policies";
    pub const API_V1_AGENTS: &str = "/api/v1/agents";
    pub const API_V1_MESSAGES: &str = "/api/v1/messages";
}
