//! HTTP request handlers for the platform API.
//!
//! This module contains all request handlers organized by domain:
//! - `health`: Health check and metrics endpoints
//! - `policies`: Policy CRUD operations
//! - `bundles`: Bundle management operations
//! - `agents`: Agent management (placeholder)

pub mod agents;
pub mod bundles;
pub mod health;
pub mod policies;

pub use agents::*;
pub use bundles::*;
pub use health::*;
pub use policies::*;
