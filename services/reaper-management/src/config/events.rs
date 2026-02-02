//! SSE events configuration

use serde::{Deserialize, Serialize};

/// SSE events configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsConfig {
    #[serde(default = "default_sse_keepalive")]
    pub sse_keepalive_seconds: u64,
    #[serde(default = "default_max_sse_connections")]
    pub max_connections_per_org: usize,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            sse_keepalive_seconds: default_sse_keepalive(),
            max_connections_per_org: 1000,
        }
    }
}

fn default_sse_keepalive() -> u64 {
    30
}

fn default_max_sse_connections() -> usize {
    1000
}
