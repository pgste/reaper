//! Reaper Management Server Library
//!
//! Multi-tenant policy management server with:
//! - Organizations as the multi-tenancy unit
//! - User authentication with email/password and sessions
//! - Multiple policy sources (Git, External API, S3, Bundle URLs)
//! - Pluggable storage backends (Filesystem, S3, SQLite, PostgreSQL, MongoDB, DynamoDB)
//! - Bundle compilation and promotion workflow
//! - Controlled deployments (canary, percentage-based, label-selector)
//! - Agent self-registration with API key + JWKS authentication
//! - Server-Sent Events for real-time notifications
//! - Namespace hierarchy for scoped policies and deployments
//! - Audit logging for compliance

pub mod api;
pub mod audit;
pub mod auth;
pub mod billing;
pub mod bundle;
pub mod config;
pub mod db;
pub mod decisions;
pub mod deployment;
pub mod domain;
pub mod graceful;
pub mod landscape;
pub mod metrics;
pub mod middleware;
pub mod rate_limit;
pub mod state;
pub mod storage;
pub mod sync;
pub mod validation;
pub mod webhook;

pub use audit::{
    ActorType, AuditEntry, AuditError, AuditQuery, AuditRepository, ClientInfo, ResourceType,
};
pub use billing::{BillingConfig, BillingError, BillingService};
pub use bundle::{BundleError, BundleService};
pub use config::Config;
pub use db::{Database, DatabaseError};
pub use deployment::{DeploymentError, DeploymentService};
pub use landscape::{LandscapeSummary, LandscapeView, OrgMetrics};
pub use state::AppState;
pub use sync::SyncService;
pub use validation::{PolicyValidationResult, ValidationError, ValidationService};
pub use webhook::WebhookDeliveryService;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
