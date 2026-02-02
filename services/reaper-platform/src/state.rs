//! Platform state management.
//!
//! This module contains the shared state structures used across the platform.

use parking_lot::RwLock;
use policy_engine::{PolicyBundle, PolicyEngine};
use std::collections::HashMap;
use std::sync::Arc;

/// Shared platform state
#[derive(Clone)]
pub struct PlatformState {
    pub policy_engine: PolicyEngine,
    pub deployment_stats: Arc<RwLock<DeploymentStats>>,
    /// Bundle storage: policy_id -> PolicyBundle
    pub bundle_storage: Arc<RwLock<HashMap<String, PolicyBundle>>>,
    /// Registered agents: agent_id -> agent_url
    #[allow(dead_code)]
    pub agents: Arc<RwLock<HashMap<String, String>>>,
}

// Add Debug manually since PolicyEngine doesn't implement Debug
impl std::fmt::Debug for PlatformState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformState")
            .field("deployment_stats", &self.deployment_stats)
            .finish_non_exhaustive()
    }
}

/// Deployment statistics
#[derive(Debug, Default)]
pub struct DeploymentStats {
    pub total_deployments: u64,
    pub successful_deployments: u64,
    pub failed_deployments: u64,
}
