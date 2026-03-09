//! Two-phase commit methods for atomic package deployment.
//!
//! This module contains methods for staging and committing policy packages
//! atomically to ensure consistent policy deployment across multiple policies.

use super::{PolicyEngine, PolicyVersion, StagedPackage};
use reaper_core::{PolicyId, ReaperError, Result};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{info, instrument};
use uuid::Uuid;

impl PolicyEngine {
    // ========================================================================
    // Two-Phase Commit Methods for Atomic Package Deployment
    // ========================================================================

    /// Phase 1: Stage a policy package for atomic deployment
    ///
    /// This method validates and prepares all policies in the package for
    /// atomic deployment. The policies are stored in a staging area and
    /// can be committed atomically using `commit_staged_package()`.
    ///
    /// During staging:
    /// - All policies are compiled and validated
    /// - Evaluators are built with the provided DataStore
    /// - Pre-compilation hints are processed (string interning, regex caching)
    /// - Policies are stored in staging area (not yet visible to evaluations)
    ///
    /// # Arguments
    /// * `package` - The PolicyPackage to stage
    /// * `store` - DataStore containing entity data for evaluators
    ///
    /// # Returns
    /// - Ok(StagedPackage) with staging details and policy IDs
    /// - Err if any policy fails validation
    #[instrument(skip(self, package, store), fields(package_name = %package.metadata.name))]
    pub fn stage_package(
        &self,
        package: &crate::reap::PolicyPackage,
        store: Arc<crate::data::DataStore>,
    ) -> Result<StagedPackage> {
        // Check if staging is already in progress
        {
            let current_staging = self.current_staging_id.read();
            if current_staging.is_some() {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Another staging operation is already in progress. Call rollback_staged() first.".to_string(),
                });
            }
        }

        let staging_id = Uuid::new_v4();
        info!(
            staging_id = %staging_id,
            package_name = %package.metadata.name,
            policy_count = package.policies.len(),
            "Starting package staging"
        );

        // Pre-intern strings from hints
        let interner = store.interner();
        for s in &package.hints.strings_to_intern {
            interner.intern(s);
        }

        // Pre-warm regex cache
        let regex_count = package.hints.prewarm_regex_cache();
        if regex_count > 0 {
            info!(regex_count = regex_count, "Pre-warmed regex cache");
        }

        let mut staged_policy_ids = Vec::with_capacity(package.policies.len());
        let mut staged_policy_names = Vec::with_capacity(package.policies.len());
        let mut validation_errors = Vec::new();

        // Clear any previous staged data
        self.staged_policies.clear();
        self.staged_names.clear();

        // Stage each policy
        for entry in &package.policies {
            match self.stage_single_policy(&entry.policy, store.clone()) {
                Ok((policy_id, policy_name)) => {
                    staged_policy_ids.push(policy_id);
                    staged_policy_names.push(policy_name);
                }
                Err(e) => {
                    validation_errors.push(format!(
                        "Policy '{}' failed validation: {}",
                        entry.policy.name, e
                    ));
                }
            }
        }

        // If there were validation errors, clear staged data
        if !validation_errors.is_empty() {
            info!(
                error_count = validation_errors.len(),
                "Package staging failed with validation errors"
            );
            self.staged_policies.clear();
            self.staged_names.clear();
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Package staging failed with {} validation errors: {}",
                    validation_errors.len(),
                    validation_errors.join("; ")
                ),
            });
        }

        // Store the staging ID
        {
            let mut current_staging = self.current_staging_id.write();
            *current_staging = Some(staging_id);
        }

        let staged = StagedPackage {
            staging_id,
            staged_policy_ids,
            staged_policy_names,
            validation_errors,
            staged_at: chrono::Utc::now(),
        };

        info!(
            staging_id = %staging_id,
            policies_staged = staged.staged_policy_ids.len(),
            "Package staging complete"
        );

        Ok(staged)
    }

    /// Stage a single policy and return its ID and name
    pub(super) fn stage_single_policy(
        &self,
        policy: &crate::reap::Policy,
        store: Arc<crate::data::DataStore>,
    ) -> Result<(PolicyId, String)> {
        // Create a bundle from the policy
        let bundle = crate::reap::PolicyBundle::new(policy.clone());

        // Compile the policy with full evaluator
        let enhanced_policy = bundle.to_enhanced_policy_with_store(store)?;
        let policy_id = enhanced_policy.id;
        let policy_name = enhanced_policy.name.clone();
        let policy_arc = Arc::new(enhanced_policy);

        // Insert into staging area
        self.staged_policies.insert(policy_id, policy_arc.clone());
        self.staged_names.insert(policy_name.clone(), policy_id);

        Ok((policy_id, policy_name))
    }

    /// Phase 2: Atomically commit all staged policies
    ///
    /// This method moves all staged policies to the active store atomically.
    /// After commit:
    /// - All staged policies become immediately visible to evaluations
    /// - Version tracking is updated for each policy
    /// - Staging area is cleared
    ///
    /// Concurrent reads during commit will either see:
    /// - The old set of policies (before any commits)
    /// - The new set of policies (after all commits)
    ///
    /// # Arguments
    /// * `staged` - The StagedPackage from a successful stage_package() call
    ///
    /// # Returns
    /// Vector of PolicyVersion for each committed policy
    #[instrument(skip(self, staged), fields(staging_id = %staged.staging_id))]
    pub fn commit_staged_package(&self, staged: &StagedPackage) -> Result<Vec<PolicyVersion>> {
        // Verify this is the current staging operation
        {
            let current_staging = self.current_staging_id.read();
            match *current_staging {
                Some(id) if id == staged.staging_id => {}
                Some(id) => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Staging ID mismatch: expected {}, got {}",
                            staged.staging_id, id
                        ),
                    });
                }
                None => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "No staging operation in progress".to_string(),
                    });
                }
            }
        }

        info!(
            staging_id = %staged.staging_id,
            policies_to_commit = staged.staged_policy_ids.len(),
            "Committing staged package"
        );

        let mut versions = Vec::with_capacity(staged.staged_policy_ids.len());

        // Atomically move all staged policies to active store
        // We collect all policies first to minimize the window of inconsistency
        let policies_to_commit: Vec<_> = staged
            .staged_policy_ids
            .iter()
            .filter_map(|policy_id| {
                self.staged_policies
                    .remove(policy_id)
                    .map(|(_, policy)| (*policy_id, policy))
            })
            .collect();

        // Move names to active
        for policy_name in &staged.staged_policy_names {
            if let Some((_, policy_id)) = self.staged_names.remove(policy_name) {
                self.policy_names.insert(policy_name.clone(), policy_id);
            }
        }

        // Commit policies and create versions
        for (policy_id, policy) in policies_to_commit {
            let policy_name = policy.name.clone();
            let version_str = policy
                .metadata
                .get("bundle_version")
                .cloned()
                .unwrap_or_else(|| format!("v{}", policy.version));

            // Insert into active policies
            self.active_policies.insert(policy_id, policy.clone());

            // Create version metadata
            let bundle_hash = {
                let mut hasher = Sha256::new();
                hasher.update(policy.content.as_bytes());
                hasher.finalize().into()
            };

            let policy_version = PolicyVersion {
                version: version_str.clone(),
                deployed_at: SystemTime::now(),
                bundle_hash,
                policy_id: policy_id.to_string(),
            };

            // Store version
            self.versions
                .entry(policy_id)
                .or_default()
                .push(policy_version.clone());

            versions.push(policy_version);

            info!(
                policy_id = %policy_id,
                policy_name = %policy_name,
                version = %version_str,
                "Policy committed"
            );
        }

        // Clear staging ID
        {
            let mut current_staging = self.current_staging_id.write();
            *current_staging = None;
        }

        info!(
            staging_id = %staged.staging_id,
            policies_committed = versions.len(),
            "Package commit complete"
        );

        Ok(versions)
    }

    /// Discard all staged policies and clear staging state
    ///
    /// Call this when staging fails or when you need to abort a staged
    /// deployment. This is safe to call even if no staging is in progress.
    pub fn rollback_staged(&self) {
        let staging_id = {
            let mut current_staging = self.current_staging_id.write();
            current_staging.take()
        };

        let staged_count = self.staged_policies.len();
        self.staged_policies.clear();
        self.staged_names.clear();

        if let Some(id) = staging_id {
            info!(
                staging_id = %id,
                policies_discarded = staged_count,
                "Staged package rolled back"
            );
        }
    }

    /// Check if a staging operation is in progress
    pub fn is_staging_in_progress(&self) -> bool {
        self.current_staging_id.read().is_some()
    }

    /// Get the current staging ID if any
    pub fn get_staging_id(&self) -> Option<Uuid> {
        *self.current_staging_id.read()
    }

    /// Convenience method: stage and commit a package atomically
    ///
    /// This combines stage_package() and commit_staged_package() into a
    /// single operation. If staging fails, an error is returned and no
    /// policies are deployed. If staging succeeds, all policies are
    /// committed atomically.
    ///
    /// # Arguments
    /// * `package` - The PolicyPackage to deploy
    /// * `store` - DataStore containing entity data for evaluators
    ///
    /// # Returns
    /// Vector of PolicyVersion for each deployed policy
    #[instrument(skip(self, package, store), fields(package_name = %package.metadata.name))]
    pub fn deploy_package_atomic(
        &self,
        package: &crate::reap::PolicyPackage,
        store: Arc<crate::data::DataStore>,
    ) -> Result<Vec<PolicyVersion>> {
        // Stage the package
        let staged = self.stage_package(package, store)?;

        // If staging succeeded, commit
        self.commit_staged_package(&staged)
    }
}
