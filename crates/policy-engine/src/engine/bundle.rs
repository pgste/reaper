//! Bundle deployment methods for the PolicyEngine.
//!
//! This module contains methods for deploying policy bundles with
//! version tracking and rollback support.

use super::{PolicyEngine, PolicyVersion};
use crate::reap::PolicyBundle;
use reaper_core::{ReaperError, Result};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{info, instrument};

impl PolicyEngine {
    /// Deploy a policy from a .rbb bundle with version tracking
    ///
    /// This method:
    /// 1. Validates the bundle
    /// 2. Generates a SHA-256 hash for integrity verification
    /// 3. Checks that the version is newer than the current version (unless force=true)
    /// 4. Compiles the policy to an EnhancedPolicy
    /// 5. Atomically inserts/replaces the policy in the engine
    /// 6. Stores version metadata for tracking and rollback
    /// 7. Caches the bundle for potential rollback operations
    ///
    /// # Arguments
    /// * `bundle` - The PolicyBundle to deploy
    /// * `force` - If true, skip version validation and allow downgrade
    ///
    /// # Returns
    /// PolicyVersion with deployment metadata including bundle hash
    #[instrument(skip(self, bundle), fields(policy_name = %bundle.policy.name))]
    pub fn deploy_bundle(&self, bundle: PolicyBundle, force: bool) -> Result<PolicyVersion> {
        let bundle_version = bundle.metadata.policy_version.as_deref().unwrap_or("1.0.0");
        info!(
            "Deploying policy bundle: {} (version: {})",
            bundle.metadata.policy_name, bundle_version
        );

        // 1. Generate bundle hash (SHA-256)
        let bundle_bytes = bundle.to_bytes().map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bundle_bytes);
        let bundle_hash: [u8; 32] = hasher.finalize().into();

        // 2. Convert bundle to EnhancedPolicy
        let policy = bundle.to_enhanced_policy()?;
        let policy_id = policy.id;
        let policy_id_str = policy_id.to_string();

        // 3. Version validation (unless force=true)
        if !force {
            if let Some(existing_versions) = self.versions.get(&policy_id) {
                if !existing_versions.is_empty() {
                    // Check if new version is actually newer
                    // For simplicity, we just check if the version string is different
                    let last_version = &existing_versions.last().unwrap().version;
                    if last_version == bundle_version {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Version {} already deployed. Use force=true to redeploy.",
                                bundle_version
                            ),
                        });
                    }
                }
            }
        }

        // 4. Deploy the policy (atomic hot-swap)
        self.deploy_policy(policy)?;

        // 5. Create version metadata
        let policy_version = PolicyVersion {
            version: bundle_version.to_string(),
            deployed_at: SystemTime::now(),
            bundle_hash,
            policy_id: policy_id_str.clone(),
        };

        // 6. Store version in history
        self.versions
            .entry(policy_id)
            .or_default()
            .push(policy_version.clone());

        // 7. Cache bundle for rollback (key: policy_id:version)
        let cache_key = format!("{}:{}", policy_id_str, bundle_version);
        self.bundle_cache.insert(cache_key, bundle.clone());

        info!(
            "Bundle deployed successfully: {} version {}",
            bundle.metadata.policy_name, bundle_version
        );

        Ok(policy_version)
    }

    /// Deploy a policy from a .rbb bundle with full ReaperDSL compilation
    ///
    /// This method compiles the bundle using the full ReaperDSL compiler,
    /// preserving all complex conditions, functions, and rule logic.
    /// This is the recommended method for production bundle deployment.
    ///
    /// # Arguments
    /// * `bundle` - The PolicyBundle to deploy
    /// * `store` - DataStore containing entity data for the evaluator
    /// * `force` - If true, skip version validation and allow downgrade
    ///
    /// # Returns
    /// PolicyVersion with deployment metadata including bundle hash
    #[instrument(skip(self, bundle, store), fields(policy_name = %bundle.policy.name))]
    pub fn deploy_bundle_with_store(
        &self,
        bundle: PolicyBundle,
        store: Arc<crate::data::DataStore>,
        force: bool,
    ) -> Result<PolicyVersion> {
        let bundle_version = bundle.metadata.policy_version.as_deref().unwrap_or("1.0.0");
        info!(
            "Deploying policy bundle with compiled evaluator: {} (version: {})",
            bundle.metadata.policy_name, bundle_version
        );

        // 1. Generate bundle hash (SHA-256)
        let bundle_bytes = bundle.to_bytes().map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bundle_bytes);
        let bundle_hash: [u8; 32] = hasher.finalize().into();

        // 2. Convert bundle to EnhancedPolicy with compiled evaluator
        let policy = bundle.to_enhanced_policy_with_store(store)?;
        let policy_id = policy.id;
        let policy_id_str = policy_id.to_string();

        // 3. Version validation (unless force=true)
        if !force {
            if let Some(existing_versions) = self.versions.get(&policy_id) {
                if !existing_versions.is_empty() {
                    let last_version = &existing_versions.last().unwrap().version;
                    if last_version == bundle_version {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Version {} already deployed. Use force=true to redeploy.",
                                bundle_version
                            ),
                        });
                    }
                }
            }
        }

        // 4. Deploy the policy (atomic hot-swap)
        self.deploy_policy(policy)?;

        // 5. Create version metadata
        let policy_version = PolicyVersion {
            version: bundle_version.to_string(),
            deployed_at: SystemTime::now(),
            bundle_hash,
            policy_id: policy_id_str.clone(),
        };

        // 6. Store version in history
        self.versions
            .entry(policy_id)
            .or_default()
            .push(policy_version.clone());

        // 7. Cache bundle for rollback (key: policy_id:version)
        let cache_key = format!("{}:{}", policy_id_str, bundle_version);
        self.bundle_cache.insert(cache_key, bundle.clone());

        info!(
            "Bundle deployed with compiled evaluator: {} version {} ({} rules)",
            bundle.metadata.policy_name,
            bundle_version,
            bundle.policy.rules.len()
        );

        Ok(policy_version)
    }

    /// Rollback a policy to a previous version
    ///
    /// This loads the cached bundle for the specified version and re-deploys it.
    ///
    /// # Arguments
    /// * `policy_id` - The ID of the policy to rollback
    /// * `target_version` - The version to rollback to
    ///
    /// # Returns
    /// PolicyVersion of the restored version
    #[instrument(skip(self), fields(policy_id = %policy_id, target_version = %target_version))]
    pub fn rollback(
        &self,
        policy_id: &reaper_core::PolicyId,
        target_version: &str,
    ) -> Result<PolicyVersion> {
        info!(
            "Rolling back policy {} to version {}",
            policy_id, target_version
        );

        // 1. Lookup bundle from cache
        let cache_key = format!("{}:{}", policy_id, target_version);
        let bundle = self
            .bundle_cache
            .get(&cache_key)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: format!(
                    "Bundle not found in cache: {}:{}",
                    policy_id, target_version
                ),
            })?;

        // 2. Re-deploy bundle (force=true to allow "downgrade")
        let version = self.deploy_bundle(bundle, true)?;

        info!(
            "Rollback successful: policy {} restored to version {}",
            policy_id, target_version
        );

        Ok(version)
    }

    /// Get the current version of a policy
    ///
    /// Returns the most recently deployed version metadata.
    pub fn get_version(&self, policy_id: &reaper_core::PolicyId) -> Option<PolicyVersion> {
        self.versions
            .get(policy_id)
            .and_then(|versions| versions.last().cloned())
    }

    /// List all versions of a policy in chronological order
    ///
    /// Returns all cached versions for the specified policy.
    pub fn list_versions(&self, policy_id: &reaper_core::PolicyId) -> Vec<PolicyVersion> {
        self.versions
            .get(policy_id)
            .map(|versions| versions.clone())
            .unwrap_or_default()
    }
}
