//! Landscape module for fleet visibility
//!
//! Provides aggregated views of agent fleet status, bundle distribution,
//! and performance metrics across organizations.

pub mod service;

pub use service::{AgentEntry, BundleDistribution, LandscapeSummary, LandscapeView, OrgMetrics};
