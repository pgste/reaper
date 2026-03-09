//! Transport configuration for connecting to a Reaper Agent.
//!
//! The SDK supports two transports:
//! - **HTTP over TCP** (default) — uses reqwest, works across hosts
//! - **HTTP over Unix Domain Socket** — uses hyper, lower latency for same-host/pod

use std::path::PathBuf;

/// Transport configuration for connecting to a Reaper Agent.
#[derive(Debug, Clone)]
pub enum Transport {
    /// HTTP over TCP (default). Uses reqwest internally.
    Http {
        /// Base URL of the Reaper Agent (e.g., "http://localhost:8080")
        endpoint: String,
    },
    /// HTTP over Unix Domain Socket. Uses hyper internally.
    /// Only available on Unix-like systems.
    Unix {
        /// Path to the Unix socket file (e.g., "/var/run/reaper/agent.sock")
        socket_path: PathBuf,
    },
}

impl Transport {
    /// Create an HTTP transport.
    pub fn http(endpoint: &str) -> Self {
        Transport::Http {
            endpoint: endpoint.to_string(),
        }
    }

    /// Create a Unix Domain Socket transport.
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Transport::Unix {
            socket_path: path.into(),
        }
    }
}
