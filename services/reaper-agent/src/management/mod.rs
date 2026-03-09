//! Management plane client for Reaper Agent
//!
//! Provides HTTP client functionality for connecting to the Reaper Management Server.
//! When enabled, the agent can:
//! - Register with the management server
//! - Receive policy bundles
//! - Send heartbeats with metrics
//! - Subscribe to bundle update notifications via SSE
//!
//! When disabled (default), the agent runs in standalone mode using local policies.

mod client;
mod sse;
mod sync;
mod types;

pub use client::ManagementClient;
pub use sse::{ManagementEvent, SseClient, SseConfig};
pub use sync::SyncService;
pub use types::*;
