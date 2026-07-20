//! Reaper Agent - High-performance policy enforcement service.
//!
//! This library exposes the core components of the Reaper Agent for:
//! - Unit testing individual modules
//! - Integration testing
//! - Embedding in other applications
//!
//! # Modules
//!
//! - [`state`]: Agent state and statistics management
//! - [`types`]: Request/response type definitions
//! - [`observability`]: Prometheus metrics and tracing
//! - [`handlers`]: HTTP request handlers
//! - [`cache`]: Policy caching layer
//! - [`bootstrap`]: Policy and data bootstrapping

pub mod api;
pub mod auth;
pub mod bootstrap;
pub mod cache;
pub mod capability_cache;
pub mod capability_gate;
pub mod decision_stream;
pub mod handlers;
pub mod http;
pub mod management;
pub mod metrics_cache;
pub mod observability;
pub mod panic_guard;
pub mod state;
pub mod tls;
pub mod types;
pub mod uds;

// Re-export commonly used types
pub use state::{AgentState, AgentStats};
pub use types::*;
