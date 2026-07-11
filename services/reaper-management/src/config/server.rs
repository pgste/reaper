//! Server configuration

use serde::{Deserialize, Serialize};

use super::error::ConfigError;

/// Server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Transitional: also serve the resource API at the bare root (no
    /// `/api/v1` prefix), the pre-Plan-07 layout, with `Deprecation`/`Sunset`
    /// response headers. Default **off** — the API is served only under
    /// `/api/v1`. Enable for one release to give un-migrated clients a grace
    /// window (Plan 07 Phase B, ADR/Risk: `serve_root_alias`). Env override:
    /// `REAPER_SERVE_ROOT_ALIAS=true`.
    #[serde(default)]
    pub serve_root_alias: bool,
    /// Optimistic-concurrency enforcement (Plan 07 Phase C, ADR-3): when true,
    /// a `PUT` on a policy or bundle without an `If-Match` header is rejected
    /// with **428 Precondition Required**. Default **false** for one release
    /// (warn-only: the write proceeds unguarded and a deprecation warning is
    /// logged), then flips to true. A stale `If-Match`, when sent, always
    /// fails with 412 regardless of this flag. Env override:
    /// `REAPER_REQUIRE_IF_MATCH=true`.
    #[serde(default)]
    pub require_if_match: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
            serve_root_alias: false,
            require_if_match: false,
        }
    }
}

impl ServerConfig {
    /// Validate server configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate port range
        if self.port == 0 {
            return Err(ConfigError::InvalidPort(self.port));
        }

        // Validate bind address
        if self.bind_address.parse::<std::net::IpAddr>().is_err() {
            return Err(ConfigError::InvalidBindAddress(self.bind_address.clone()));
        }

        Ok(())
    }
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8081
}
