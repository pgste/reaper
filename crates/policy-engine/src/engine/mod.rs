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
    PolicySourceMetadata, PolicyVersion, PruningIndexStats, SetEvalOutcome, SimpleAction,
    SimpleRule, StagedPackage,
};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use parking_lot::RwLock;
use reaper_core::{PolicyId, ReaperError, Result};
use std::sync::Arc;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::reap::PolicyBundle;

/// The active policy set: the id->policy map and the name->id index, held
/// together in one snapshot so they can be swapped **atomically**.
///
/// Single-policy hot-swaps mutate the current snapshot in place (DashMap gives
/// lock-free per-entry updates). A full bundle load builds a brand-new
/// `ActiveSet` and swaps the `ArcSwap` pointer, so readers observe either the
/// entire old set or the entire new set — never a mix — and any policies not in
/// the new bundle are dropped in the same atomic step.
pub(crate) struct ActiveSet {
    pub(crate) policies: DashMap<PolicyId, Arc<EnhancedPolicy>>,
    pub(crate) names: DashMap<String, PolicyId>,
    /// Resource pruning index (Plan 08 Phase A): concrete resource string ->
    /// ids of policies with a rule referencing *exactly* that resource. A
    /// request evaluates only the policies bucketed under its resource plus the
    /// always-candidate `unprunable` set, instead of the whole map. Held inside
    /// the `ActiveSet` so it swaps atomically with the policy map on a full
    /// bundle load (readers never see a partial index).
    pub(crate) resource_index: DashMap<String, Vec<PolicyId>>,
    /// Policies that must be evaluated for *every* resource because their match
    /// set cannot be statically narrowed to concrete resources — a Simple
    /// policy with a `*` rule, or any non-Simple language (DSL/Cedar) whose
    /// resource predicates we do not statically extract. Conservative by
    /// design: an unprunable policy is always a candidate, so pruning can never
    /// drop a policy that could have matched.
    pub(crate) unprunable: DashMap<PolicyId, ()>,
}

impl ActiveSet {
    fn new() -> Self {
        Self {
            policies: DashMap::new(),
            names: DashMap::new(),
            resource_index: DashMap::new(),
            unprunable: DashMap::new(),
        }
    }
}

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
    /// Active policy set (id->policy + name->id), swappable as one atomic unit
    /// for pure bundle loads. Reads are lock-free (ArcSwap load + DashMap get).
    pub(crate) active: Arc<ArcSwap<ActiveSet>>,
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
            .field("active_policies_count", &self.active.load().policies.len())
            .field("policy_names_count", &self.active.load().names.len())
            .field("package_count", &self.package_index.len())
            .field("has_default_policy", &self.default_policy.read().is_some())
            .finish()
    }
}

impl PolicyEngine {
    pub fn new() -> Self {
        info!("Initializing Reaper Policy Engine with lock-free storage");
        Self {
            active: Arc::new(ArcSwap::from_pointee(ActiveSet::new())),
            package_index: Arc::new(DashMap::new()),
            default_policy: Arc::new(RwLock::new(None)),
            versions: Arc::new(DashMap::new()),
            bundle_cache: Arc::new(DashMap::new()),
            staged_policies: Arc::new(DashMap::new()),
            staged_names: Arc::new(DashMap::new()),
            current_staging_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Static resource terms for the pruning index (Plan 08 Phase A; D2).
    ///
    /// Returns `Some(resources)` — the concrete resource strings this policy is
    /// bucketed under — when the policy's match set can be narrowed to specific
    /// resources, or `None` when it must be treated as **unprunable** (always a
    /// candidate). The answer is delegated to the policy's own evaluator via
    /// [`PolicyEvaluator::resource_index_terms`], so each language decides its
    /// own soundness next to its match semantics:
    /// - `Simple` — exact resource literals, unprunable on any `*` rule.
    /// - `ReaperDsl` (compiled, PRIMARY) — literal `resource == "…"` constraints
    ///   extracted from the compiled conditions (D2); unbounded predicates
    ///   (attributes, wildcards, dynamic ids) make the policy unprunable.
    /// - AST-fallback DSL and Cedar — default `None` (conservatively unprunable).
    ///
    /// A policy whose evaluator is somehow not built is treated as unprunable
    /// (`None`) — safe: pruning can never drop a policy that could have matched.
    ///
    /// [`PolicyEvaluator::resource_index_terms`]: crate::evaluators::PolicyEvaluator::resource_index_terms
    fn index_terms(policy: &EnhancedPolicy) -> Option<Vec<String>> {
        policy.get_evaluator().ok()?.resource_index_terms()
    }

    /// Add `policy_id` to `set`'s pruning index per `policy`'s terms.
    fn index_policy(set: &ActiveSet, policy_id: PolicyId, policy: &EnhancedPolicy) {
        match Self::index_terms(policy) {
            Some(terms) => {
                for term in terms {
                    let mut bucket = set.resource_index.entry(term).or_default();
                    if !bucket.contains(&policy_id) {
                        bucket.push(policy_id);
                    }
                }
            }
            None => {
                set.unprunable.insert(policy_id, ());
            }
        }
    }

    /// Remove `policy_id`'s references from `set`'s pruning index, using
    /// `policy`'s (its previous version's) terms so only the relevant buckets
    /// are touched. Empty buckets are pruned so `resource_buckets` stays honest.
    fn unindex_policy(set: &ActiveSet, policy_id: &PolicyId, policy: &EnhancedPolicy) {
        match Self::index_terms(policy) {
            Some(terms) => {
                for term in terms {
                    let now_empty = if let Some(mut bucket) = set.resource_index.get_mut(&term) {
                        bucket.retain(|id| id != policy_id);
                        bucket.is_empty()
                    } else {
                        false
                    };
                    // Guard dropped above before remove_if re-checks under lock
                    // (avoids a same-shard self-deadlock); remove_if won't drop a
                    // bucket a concurrent deploy just refilled.
                    if now_empty {
                        set.resource_index.remove_if(&term, |_, ids| ids.is_empty());
                    }
                }
            }
            None => {
                set.unprunable.remove(policy_id);
            }
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

        // Mutate the current active snapshot in place (lock-free per-entry).
        let active = self.active.load();

        // Check if this policy already exists (for package + pruning index update)
        let old_policy = active.policies.get(&policy_id).map(|p| p.value().clone());
        let old_package = old_policy.as_ref().map(|p| p.package().to_string());

        // Drop the outgoing version's pruning-index references before re-indexing
        // (its resource terms or language may have changed on redeploy).
        if let Some(old_policy) = &old_policy {
            Self::unindex_policy(&active, &policy_id, old_policy);
        }

        // Atomic insertion - old policy is automatically dropped
        active.policies.insert(policy_id, policy_arc.clone());
        active.names.insert(policy_name.clone(), policy_id);

        // Index the incoming version for resource pruning.
        Self::index_policy(&active, policy_id, &policy_arc);

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
        let active = self.active.load();
        let removed_policy = active
            .policies
            .remove(policy_id)
            .map(|(_, policy)| policy)
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Remove from name lookup
        active.names.retain(|_, &mut v| v != *policy_id);

        // Remove from the resource pruning index.
        Self::unindex_policy(&active, policy_id, &removed_policy);

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
        self.active
            .load()
            .policies
            .get(policy_id)
            .map(|entry| entry.value().clone())
    }

    /// Get policy by name
    pub fn get_policy_by_name(&self, name: &str) -> Option<Arc<EnhancedPolicy>> {
        // Resolve name->id and id->policy against the SAME snapshot so a
        // concurrent full-set swap can't leave the two indexes inconsistent.
        let active = self.active.load();
        active
            .names
            .get(name)
            .and_then(|id| active.policies.get(id.value()).map(|p| p.value().clone()))
    }

    /// List all active policies
    pub fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>> {
        self.active
            .load()
            .policies
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Candidate policy ids for a request `resource` (Plan 08 Phase A).
    ///
    /// Returns exactly the policies that could possibly decide a request for
    /// `resource`: those bucketed under the concrete resource plus every
    /// `unprunable` policy (wildcards / DSL / Cedar). Deduped and sorted for a
    /// deterministic evaluation order. This is the served evaluate-all path's
    /// replacement for `list_policies()` — it returns ≈ (matching + unprunable)
    /// ids instead of the whole set, and clones only ids (not `Arc<Policy>`).
    ///
    /// Correctness: a Simple policy that is neither in the bucket nor unprunable
    /// has only non-matching literal-resource rules, so it is necessarily
    /// non-decisive (`evaluate_matched` → `false`) for this resource — dropping
    /// it cannot change the set decision.
    pub fn candidate_policy_ids(&self, resource: &str) -> Vec<PolicyId> {
        let active = self.active.load();
        let mut ids: Vec<PolicyId> = active
            .resource_index
            .get(resource)
            .map(|b| b.value().clone())
            .unwrap_or_default();
        ids.reserve(active.unprunable.len());
        for entry in active.unprunable.iter() {
            ids.push(*entry.key());
        }
        ids.sort();
        ids.dedup();
        ids
    }

    /// Pruning-index statistics for monitoring and tests.
    pub fn get_index_stats(&self) -> PruningIndexStats {
        let active = self.active.load();
        let indexed_entries: usize = active.resource_index.iter().map(|e| e.value().len()).sum();
        PruningIndexStats {
            resource_buckets: active.resource_index.len(),
            indexed_entries,
            unprunable_policies: active.unprunable.len(),
            total_policies: active.policies.len(),
        }
    }

    /// Atomically replace the **entire** active policy set with `policies`.
    ///
    /// This is the "pure bundle load": a brand-new [`ActiveSet`] is built and
    /// swapped in a single atomic step, so any policy that was active but is not
    /// in `policies` is removed as part of the same swap (no floating leftovers),
    /// and no reader ever observes a partial set. The package index is rebuilt to
    /// match. Each policy must already have its evaluator built.
    pub fn replace_all_policies(&self, policies: Vec<EnhancedPolicy>) -> Result<()> {
        let new_set = ActiveSet::new();
        let new_packages: DashMap<String, Vec<PolicyId>> = DashMap::new();

        for policy in policies {
            let id = policy.id;
            let name = policy.name.clone();
            let package = policy.package().to_string();
            let policy_arc = Arc::new(policy);
            // Build the pruning index into the new set as we insert, so the
            // index swaps in atomically with the policy map (readers never see a
            // half-built index).
            Self::index_policy(&new_set, id, &policy_arc);
            new_set.policies.insert(id, policy_arc);
            new_set.names.insert(name, id);
            new_packages.entry(package).or_default().push(id);
        }

        let count = new_set.policies.len();

        // Atomic swap of the whole set — floating policies drop here.
        self.active.store(Arc::new(new_set));

        // Rebuild the package index to match the new set.
        self.package_index.clear();
        for entry in new_packages.iter() {
            self.package_index
                .insert(entry.key().clone(), entry.value().clone());
        }

        info!("Atomically replaced active policy set: {} policies", count);
        Ok(())
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
        let start_time = crate::clock::Stopwatch::start();

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
        let evaluation_time_ns = start_time.elapsed_ns();

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

    /// Evaluate one request against a SET of policies with the production
    /// decision-combination semantics — the single source of truth shared by
    /// the agent's serving path and the control plane's counterfactual replay
    /// engine, so the two can never diverge:
    ///
    /// - **Non-matching policies are non-decisive.** A policy that returns its
    ///   per-policy default because *no rule matched* the request says nothing
    ///   about it and is skipped (Plan 08 Phase A). This is what makes the
    ///   pruning index sound: a pruned policy and an evaluated-but-unmatched
    ///   policy reach the identical set outcome.
    /// - **Deny overrides.** Any policy whose rule *matched* with Deny ends
    ///   evaluation with Deny.
    /// - **First allow wins** (among matched allows) — sets the attribution, but
    ///   a later matched deny still overrides it.
    /// - **Log matches don't decide.**
    /// - **No policy matched ⇒ default deny** (nil policy id).
    /// - **Errors deny** (fail closed) and stop evaluation.
    /// - **Unknown policy id ⇒ deny** (fail closed) with the error surfaced — a
    ///   candidate id with no live policy is an inconsistency, not a skip.
    pub fn evaluate_set(&self, policy_ids: &[PolicyId], request: &PolicyRequest) -> SetEvalOutcome {
        let mut outcome = SetEvalOutcome {
            decision: PolicyAction::Deny,
            policy_id: PolicyId::nil(),
            policy_name: String::new(),
            policy_version: 0,
            matched_rule: None,
            total_eval_time_ns: 0,
            error: None,
        };
        let mut any_allow = false;

        for policy_id in policy_ids {
            let Some(policy) = self.get_policy(policy_id) else {
                outcome.decision = PolicyAction::Deny;
                outcome.policy_id = *policy_id;
                outcome.error = Some(
                    ReaperError::PolicyNotFound {
                        policy_id: policy_id.to_string(),
                    }
                    .to_string(),
                );
                return outcome;
            };

            let evaluator = match policy.get_evaluator() {
                Ok(e) => e,
                Err(e) => {
                    outcome.decision = PolicyAction::Deny;
                    outcome.error = Some(e.to_string());
                    return outcome;
                }
            };

            let start = crate::clock::Stopwatch::start();
            let result = evaluator.evaluate_matched(request);
            outcome.total_eval_time_ns += start.elapsed_ns();

            match result {
                // Nothing matched: non-decisive, this policy is silent.
                Ok((_, false)) => continue,
                Ok((PolicyAction::Deny, true)) => {
                    outcome.decision = PolicyAction::Deny;
                    Self::attribute(&mut outcome, &policy, request);
                    return outcome;
                }
                Ok((PolicyAction::Allow, true)) => {
                    if !any_allow {
                        any_allow = true;
                        outcome.decision = PolicyAction::Allow;
                        Self::attribute(&mut outcome, &policy, request);
                    }
                }
                Ok((PolicyAction::Log, true)) => {}
                Err(e) => {
                    outcome.decision = PolicyAction::Deny;
                    outcome.error = Some(e.to_string());
                    return outcome;
                }
            }
        }

        outcome
    }

    /// Record `policy` as the deciding policy on `outcome`. The `policy.name`
    /// clone happens only here — once, for the single decisive policy — not on
    /// every non-matching candidate (Perf P3-2).
    fn attribute(outcome: &mut SetEvalOutcome, policy: &EnhancedPolicy, request: &PolicyRequest) {
        outcome.policy_id = policy.id;
        outcome.policy_name = policy.name.clone();
        outcome.policy_version = policy.version;
        // matched_rule is only meaningful for Simple policies; mirror the
        // per-policy `evaluate` scan so audit output is unchanged.
        outcome.matched_rule = if policy.language == PolicyLanguage::Simple {
            policy
                .rules
                .iter()
                .position(|rule| rule.resource == "*" || rule.resource == request.resource)
        } else {
            None
        };
    }

    /// Get engine statistics for monitoring
    pub fn get_stats(&self) -> PolicyEngineStats {
        PolicyEngineStats {
            total_policies: self.active.load().policies.len(),
            has_default_policy: self.default_policy.read().is_some(),
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}
