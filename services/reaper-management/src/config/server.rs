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
    /// Optimistic-concurrency enforcement (Plan 07 Phase C, ADR-3): when true
    /// (the default since the round-2 hardening, R2-02), a `PUT` on a policy
    /// or bundle without an `If-Match` header is rejected with **428
    /// Precondition Required**. The ADR-3 warn-only transition release has
    /// shipped; operators migrating automation that never sent `If-Match`
    /// can opt back down for one release with `REAPER_REQUIRE_IF_MATCH=false`
    /// (or `server.require_if_match = false`) — in that mode the write
    /// proceeds unguarded and a deprecation warning is logged. A stale
    /// `If-Match`, when sent, always fails with 412 regardless of this flag.
    #[serde(default = "default_require_if_match")]
    pub require_if_match: bool,
    /// Mount the billing API (Plan 06 Phase E, R3-04/ADR-5). Default **off**:
    /// the surface is a stub that fabricates checkout sessions, so it is
    /// excluded from the router AND the OpenAPI contract until an operator
    /// explicitly opts in (`REAPER_ENABLE_BILLING=true`). When on, the spec
    /// tags the operations `x-experimental`.
    #[serde(default)]
    pub enable_billing: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
            serve_root_alias: false,
            require_if_match: default_require_if_match(),
            enable_billing: false,
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

fn default_require_if_match() -> bool {
    true
}
