//! Policy Engine Implementation
//!
//! Features Rust's atomic operations for zero-downtime policy swapping
//! and lock-free lookups for sub-microsecond performance.
//!
//! Supports multiple policy languages through the PolicyEvaluator trait.
//!
//! ## Module Structure
//!
//! - `types`: Core type definitions (PolicyAction, PolicyDecision, etc.)
//! - `policy`: EnhancedPolicy struct with multi-language support
//! - `bundle`: Bundle deployment with version tracking
//! - `package`: Package management and evaluation
//! - `staging`: Two-phase commit for atomic package deployment

mod bundle;
mod package;
mod policy;
mod staging;
mod types;

#[cfg(test)]
mod tests;

// Re-export all public types
pub use policy::EnhancedPolicy;
pub use types::{
    AllPoliciesEvaluationResult, DenyInfo, PackageEvaluationResult, PackageInfo, PolicyAction,
    PolicyDecision, PolicyEngineStats, PolicyLanguage, PolicyRequest, PolicyRule, PolicySource,
    PolicySourceMetadata, PolicyVersion, SimpleAction, SimpleRule, StagedPackage,
};

use dashmap::DashMap;
use parking_lot::RwLock;
use reaper_core::{PolicyId, ReaperError, Result};
use std::sync::Arc;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::reap::PolicyBundle;

/// High-performance policy engine with atomic hot-swapping
///
/// Key Rust Features for End-User Value:
/// - Arc for zero-copy policy sharing across threads
/// - DashMap for lock-free concurrent access
/// - Atomic operations for zero-downtime policy updates
/// - Two-phase commit for atomic multi-policy deployment
/// - Package indexing for grouping related policies
#[derive(Clone)]
pub struct PolicyEngine {
    /// Active policies - lock-free for sub-microsecond lookups
    pub(crate) active_policies: Arc<DashMap<PolicyId, Arc<EnhancedPolicy>>>,
    /// Policy lookup by name for convenience
    pub(crate) policy_names: Arc<DashMap<String, PolicyId>>,
    /// Package-to-policies index for package-based evaluation
    pub(crate) package_index: Arc<DashMap<String, Vec<PolicyId>>>,
    /// Default policy for unknown policies
    pub(crate) default_policy: Arc<RwLock<Option<Arc<EnhancedPolicy>>>>,
    /// Version tracking for policy bundles
    pub(crate) versions: Arc<DashMap<PolicyId, Vec<PolicyVersion>>>,
    /// Bundle cache for rollback support (keyed by policy_id:version)
    pub(crate) bundle_cache: Arc<DashMap<String, PolicyBundle>>,
    /// Staged policies awaiting commit (Phase 1 of two-phase commit)
    pub(crate) staged_policies: Arc<DashMap<PolicyId, Arc<EnhancedPolicy>>>,
    /// Staged policy names awaiting commit
    pub(crate) staged_names: Arc<DashMap<String, PolicyId>>,
    /// Current staging ID (None if no staging in progress)
    pub(crate) current_staging_id: Arc<RwLock<Option<Uuid>>>,
}

impl std::fmt::Debug for PolicyEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyEngine")
            .field("active_policies_count", &self.active_policies.len())
            .field("policy_names_count", &self.policy_names.len())
            .field("package_count", &self.package_index.len())
            .field("has_default_policy", &self.default_policy.read().is_some())
            .finish()
    }
}

impl PolicyEngine {
    pub fn new() -> Self {
        info!("Initializing Reaper Policy Engine with lock-free storage");
        Self {
            active_policies: Arc::new(DashMap::new()),
            policy_names: Arc::new(DashMap::new()),
            package_index: Arc::new(DashMap::new()),
            default_policy: Arc::new(RwLock::new(None)),
            versions: Arc::new(DashMap::new()),
            bundle_cache: Arc::new(DashMap::new()),
            staged_policies: Arc::new(DashMap::new()),
            staged_names: Arc::new(DashMap::new()),
            current_staging_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Hot-swap a policy with zero downtime
    /// Uses atomic operations to ensure no request sees inconsistent state
    #[instrument(skip(self, policy), fields(policy_id = %policy.id, version = policy.version))]
    pub fn deploy_policy(&self, policy: EnhancedPolicy) -> Result<()> {
        let policy_id = policy.id;
        let policy_name = policy.name.clone();
        let package_name = policy.package().to_string();
        let policy_arc = Arc::new(policy);

        info!(
            "Hot-swapping policy '{}' (version {}, package '{}')",
            policy_name, policy_arc.version, package_name
        );

        // Check if this policy already exists (for package index update)
        let old_package = self
            .active_policies
            .get(&policy_id)
            .map(|p| p.package().to_string());

        // Atomic insertion - old policy is automatically dropped
        self.active_policies.insert(policy_id, policy_arc.clone());
        self.policy_names.insert(policy_name.clone(), policy_id);

        // Update package index
        // If the policy was in a different package, remove from old package
        if let Some(old_pkg) = old_package {
            if old_pkg != package_name {
                self.package_index.entry(old_pkg).and_modify(|ids| {
                    ids.retain(|id| *id != policy_id);
                });
            }
        }

        // Add to new package index
        self.package_index
            .entry(package_name.clone())
            .or_default()
            .push(policy_id);

        // Deduplicate in case of re-deployment
        self.package_index.entry(package_name).and_modify(|ids| {
            ids.sort();
            ids.dedup();
        });

        info!("Policy '{}' deployed successfully", policy_name);
        Ok(())
    }

    /// Remove a policy atomically
    #[instrument(skip(self), fields(policy_id = %policy_id))]
    pub fn remove_policy(&self, policy_id: &PolicyId) -> Result<EnhancedPolicy> {
        let removed_policy = self
            .active_policies
            .remove(policy_id)
            .map(|(_, policy)| policy)
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Remove from name lookup
        self.policy_names.retain(|_, &mut v| v != *policy_id);

        // Remove from package index
        let package_name = removed_policy.package().to_string();
        self.package_index.entry(package_name).and_modify(|ids| {
            ids.retain(|id| *id != *policy_id);
        });

        // Clean up empty packages
        self.package_index.retain(|_, ids| !ids.is_empty());

        info!("Policy {} removed successfully", policy_id);
        Ok(Arc::try_unwrap(removed_policy).unwrap_or_else(|arc| (*arc).clone()))
    }

    /// Get policy by ID - lock-free for maximum performance
    pub fn get_policy(&self, policy_id: &PolicyId) -> Option<Arc<EnhancedPolicy>> {
        self.active_policies
            .get(policy_id)
            .map(|entry| entry.value().clone())
    }

    /// Get policy by name
    pub fn get_policy_by_name(&self, name: &str) -> Option<Arc<EnhancedPolicy>> {
        self.policy_names
            .get(name)
            .and_then(|entry| self.get_policy(entry.value()))
    }

    /// List all active policies
    pub fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>> {
        self.active_policies
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Set default policy for unknown policy requests
    pub fn set_default_policy(&self, policy: EnhancedPolicy) {
        let mut default = self.default_policy.write();
        *default = Some(Arc::new(policy));
        info!("Default policy updated");
    }

    /// Evaluate a request against a policy.
    ///
    /// Optimized for sub-microsecond latency:
    /// - No `Arc::make_mut` (was cloning entire policy under concurrency)
    /// - No `#[instrument]` (was 200-800ns per call for span creation)
    /// - Evaluator accessed immutably from pre-built `Arc<dyn PolicyEvaluator>`
    /// - Returns `policy_name` to avoid caller re-lookup
    pub fn evaluate(
        &self,
        policy_id: &PolicyId,
        request: &PolicyRequest,
    ) -> Result<PolicyDecision> {
        let start_time = std::time::Instant::now();

        let policy = self
            .get_policy(policy_id)
            .or_else(|| self.default_policy.read().clone())
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Immutable access — no Arc::make_mut, no clone under concurrency
        let evaluator = policy.get_evaluator()?;

        // For Simple policies, find the matched rule index
        let matched_rule = if policy.language == PolicyLanguage::Simple {
            let mut matched_index = None;
            for (index, rule) in policy.rules.iter().enumerate() {
                if rule.resource == "*" || rule.resource == request.resource {
                    matched_index = Some(index);
                    break;
                }
            }
            matched_index
        } else {
            None
        };

        let decision = evaluator.evaluate(request)?;
        let evaluation_time_ns = start_time.elapsed().as_nanos() as u64;

        // Trace-level logging gated behind level check — zero cost at info/debug level
        if tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!(
                resource = %request.resource,
                action = %request.action,
                ?decision,
                evaluation_time_ns,
                "engine evaluate"
            );
        }

        Ok(PolicyDecision {
            decision,
            policy_id: policy.id,
            policy_name: policy.name.clone(),
            policy_version: policy.version,
            evaluation_time_ns,
            matched_rule,
        })
    }

    /// Get engine statistics for monitoring
    pub fn get_stats(&self) -> PolicyEngineStats {
        PolicyEngineStats {
            total_policies: self.active_policies.len(),
            has_default_policy: self.default_policy.read().is_some(),
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}
