//! Reaper Management Server Library
//!
//! Multi-tenant policy management server with:
//! - Organizations as the multi-tenancy unit
//! - Multiple policy sources (Git, External API)
//! - Pluggable storage backends (Filesystem, S3, SQLite, PostgreSQL, MongoDB, DynamoDB)
//! - Bundle compilation and promotion workflow
//! - Agent self-registration with API key + JWKS authentication
//! - Server-Sent Events for real-time notifications

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod domain;
pub mod state;
pub mod storage;

pub use config::Config;
pub use db::{Database, DatabaseError};
pub use state::AppState;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
