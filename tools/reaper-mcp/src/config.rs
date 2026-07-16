//! Adapter configuration, sourced from environment variables at startup.

use anyhow::Context;
use reaper_sdk::Transport;

/// Environment-derived adapter configuration.
///
/// | Variable | Meaning | Default |
/// |---|---|---|
/// | `REAPER_MCP_AGENT_URL` | Agent HTTP endpoint | `http://127.0.0.1:8080` |
/// | `REAPER_MCP_AGENT_SOCKET` | Agent Unix socket (takes precedence) | unset |
/// | `REAPER_MCP_POLICY` | Default policy name for evaluations | unset |
/// | `REAPER_MCP_PRINCIPAL` | Default principal | unset |
/// | `REAPER_MCP_ACTOR` | Default actor (the agent identity) | unset |
/// | `REAPER_MCP_CAPABILITY_FILE` | Path to a signed-capability JSON file attached to every call by default | unset |
/// | `REAPER_MCP_SERVER_LABEL` | Value for the platform-trusted `mcp.server` context key | unset |
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Transport to the Reaper Agent (HTTP or Unix socket).
    pub transport: Transport,
    /// Default policy name when a call does not name one.
    pub default_policy: Option<String>,
    /// Default principal when a call does not name one.
    pub default_principal: Option<String>,
    /// Default actor when a call does not name one.
    pub default_actor: Option<String>,
    /// Default signed capability (opaque JSON, forwarded verbatim).
    pub default_capability: Option<serde_json::Value>,
    /// Label for the platform-trusted `mcp.server` context key.
    pub server_label: Option<String>,
}

impl AdapterConfig {
    /// Build the configuration from the process environment.
    ///
    /// Fails fast on a malformed capability file — a misconfigured enforcing
    /// edge must not start and silently authorize without its capability.
    pub fn from_env() -> anyhow::Result<Self> {
        let transport = match std::env::var("REAPER_MCP_AGENT_SOCKET") {
            Ok(socket) if !socket.is_empty() => Transport::unix(socket),
            _ => {
                let url = std::env::var("REAPER_MCP_AGENT_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
                Transport::http(&url)
            }
        };

        let default_capability = match std::env::var("REAPER_MCP_CAPABILITY_FILE") {
            Ok(path) if !path.is_empty() => {
                let raw = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading REAPER_MCP_CAPABILITY_FILE {path}"))?;
                let value: serde_json::Value = serde_json::from_str(&raw)
                    .with_context(|| format!("parsing capability JSON in {path}"))?;
                Some(value)
            }
            _ => None,
        };

        Ok(Self {
            transport,
            default_policy: non_empty_env("REAPER_MCP_POLICY"),
            default_principal: non_empty_env("REAPER_MCP_PRINCIPAL"),
            default_actor: non_empty_env("REAPER_MCP_ACTOR"),
            default_capability,
            server_label: non_empty_env("REAPER_MCP_SERVER_LABEL"),
        })
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}
