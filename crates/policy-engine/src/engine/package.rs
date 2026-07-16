//! Package management methods for the PolicyEngine.
//!
//! This module contains methods for managing policy packages and
//! evaluating requests against packages of related policies.

use super::{
    AllPoliciesEvaluationResult, DenyInfo, EnhancedPolicy, PackageEvaluationResult, PackageInfo,
    PolicyAction, PolicyDecision, PolicyEngine, PolicyRequest,
};
use reaper_core::{ReaperError, Result};
use std::sync::Arc;

impl PolicyEngine {
    // ========================================================================
    // Package Management Methods
    // ========================================================================

    /// List all packages
    ///
    /// Returns a list of all package names that contain at least one policy.
    pub fn list_packages(&self) -> Vec<String> {
        self.package_index
            .iter()
            .filter(|entry| !entry.value().is_empty())
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all policies in a specific package
    ///
    /// Returns a vector of policies belonging to the specified package.
    /// Returns an empty vector if the package doesn't exist.
    pub fn get_policies_by_package(&self, package: &str) -> Vec<Arc<EnhancedPolicy>> {
        let active = self.active.load();
        self.package_index
            .get(package)
            .map(|entry| {
                entry
                    .value()
                    .iter()
                    .filter_map(|id| active.policies.get(id).map(|p| p.value().clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get information about a specific package
    ///
    /// Returns package metadata including policy count and names.
    pub fn get_package_info(&self, package: &str) -> Option<PackageInfo> {
        let active = self.active.load();
        self.package_index.get(package).map(|entry| {
            let policy_ids = entry.value();
            let policy_names: Vec<String> = policy_ids
                .iter()
                .filter_map(|id| active.policies.get(id).map(|p| p.name.clone()))
                .collect();
            PackageInfo {
                name: package.to_string(),
                policy_count: policy_names.len(),
                policy_names,
            }
        })
    }

    /// Evaluate all policies in a package against a request
    ///
    /// Security-first: Returns DENY if ANY policy in the package denies the request.
    /// This is useful for evaluating related policies as a group.
    ///
    /// # Arguments
    /// * `package` - The package name
    /// * `request` - The policy request to evaluate
    ///
    /// # Returns
    /// * `Ok(PackageEvaluationResult)` - Result with decision and details
    /// * `Err` - If the package doesn't exist
    pub fn evaluate_package(
        &self,
        package: &str,
        request: &PolicyRequest,
    ) -> Result<PackageEvaluationResult> {
        let policies = self.get_policies_by_package(package);

        if policies.is_empty() {
            return Err(ReaperError::PolicyNotFound {
                policy_id: format!("Package '{}' not found or empty", package),
            });
        }

        let mut results = Vec::with_capacity(policies.len());
        let start = crate::clock::Stopwatch::start();

        for policy in &policies {
            let eval_start = crate::clock::Stopwatch::start();

            if let Some(evaluator) = &policy.evaluator {
                // Evaluate and handle potential errors - treat errors as deny for security
                let decision = match evaluator.evaluate(request) {
                    Ok(d) => d,
                    Err(_) => PolicyAction::Deny, // Treat evaluation errors as deny
                };
                let eval_time = eval_start.elapsed_ns();

                let policy_decision = PolicyDecision {
                    decision: decision.clone(),
                    policy_id: policy.id,
                    policy_name: policy.name.clone(),
                    policy_version: policy.version,
                    evaluation_time_ns: eval_time,
                    matched_rule: None,
                    matched_rule_name: None,
                };

                // Security-first: any deny = overall deny
                if decision == PolicyAction::Deny {
                    return Ok(PackageEvaluationResult {
                        package: package.to_string(),
                        decision: PolicyAction::Deny,
                        denied_by: Some(DenyInfo {
                            policy_id: policy.id,
                            policy_name: policy.name.clone(),
                            package: package.to_string(),
                            matched_rule: None,
                        }),
                        policies_evaluated: results.len() + 1,
                        total_evaluation_time_ns: start.elapsed_ns(),
                        results,
                    });
                }

                results.push(policy_decision);
            }
        }

        Ok(PackageEvaluationResult {
            package: package.to_string(),
            decision: PolicyAction::Allow,
            denied_by: None,
            policies_evaluated: results.len(),
            total_evaluation_time_ns: start.elapsed_ns(),
            results,
        })
    }

    /// Evaluate ALL policies across ALL packages
    ///
    /// Security-first: Returns DENY if ANY policy denies the request.
    /// This is the most restrictive evaluation mode.
    ///
    /// # Arguments
    /// * `request` - The policy request to evaluate
    ///
    /// # Returns
    /// * `AllPoliciesEvaluationResult` - Result with decision and statistics
    pub fn evaluate_all(&self, request: &PolicyRequest) -> AllPoliciesEvaluationResult {
        let start = crate::clock::Stopwatch::start();
        let mut policies_evaluated = 0;
        let packages: Vec<String> = self.list_packages();

        let active = self.active.load();
        for policy_entry in active.policies.iter() {
            let policy = policy_entry.value();
            policies_evaluated += 1;

            if let Some(evaluator) = &policy.evaluator {
                // Evaluate and handle potential errors - treat errors as deny for security
                let decision = match evaluator.evaluate(request) {
                    Ok(d) => d,
                    Err(_) => PolicyAction::Deny, // Treat evaluation errors as deny
                };

                // Security-first: any deny = overall deny
                if decision == PolicyAction::Deny {
                    return AllPoliciesEvaluationResult {
                        decision: PolicyAction::Deny,
                        denied_by: Some(DenyInfo {
                            policy_id: policy.id,
                            policy_name: policy.name.clone(),
                            package: policy.package().to_string(),
                            matched_rule: None,
                        }),
                        policies_evaluated,
                        packages_evaluated: packages.len(),
                        total_evaluation_time_ns: start.elapsed_ns(),
                    };
                }
            }
        }

        AllPoliciesEvaluationResult {
            decision: PolicyAction::Allow,
            denied_by: None,
            policies_evaluated,
            packages_evaluated: packages.len(),
            total_evaluation_time_ns: start.elapsed_ns(),
        }
    }
}
