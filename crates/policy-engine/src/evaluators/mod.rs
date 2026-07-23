//! Policy Evaluator Trait and Implementations
//!
//! This module provides a pluggable architecture for different policy languages.
//! Each evaluator implements the PolicyEvaluator trait, allowing Reaper to support
//! multiple policy languages while maintaining consistent performance guarantees.

use crate::{PolicyAction, PolicyRequest};
use reaper_core::ReaperError;
use std::fmt::Debug;

#[cfg(feature = "cedar")]
pub mod cedar;
#[cfg(feature = "cedar")]
pub mod cedar_integration;
pub mod reaper_dsl;
pub mod simple;

#[cfg(feature = "cedar")]
pub use cedar::CedarPolicyEvaluator;
pub use simple::SimplePolicyEvaluator;
// Policy-language tiers (round-3 Plan 04 / E-02):
//   * `SimplePolicyEvaluator` — the BASIC STARTING POINT: allow/deny by resource
//     pattern only (no principal/action/conditions). A quick on-ramp, not RBAC/ABAC.
//   * `.reap` DSL — the MAIN REAL PATH, served by two intentionally co-existing
//     surfaces (tiered compilation): the compiled `ReaperDSLEvaluator` is primary,
//     with the AST `ReapAstEvaluator` as FALLBACK and as the compiler's differential
//     oracle (`reap::ReaperPolicy::build_preferred`). They are pinned identical by a
//     blocking differential — do not converge them.
// See docs/reference/DSL_COMPATIBILITY.md for the tier guidance.

/// Outcome of [`PolicyEvaluator::evaluate_named`]: the decision, whether a
/// rule actually matched (vs the per-policy default), and — when the policy
/// language has named rules — the deciding rule's name, borrowed from the
/// evaluator so the eval loop allocates nothing.
#[derive(Debug, Clone)]
pub struct NamedOutcome<'a> {
    pub decision: PolicyAction,
    pub matched: bool,
    pub rule_name: Option<&'a str>,
}

/// Core trait for policy evaluation across different languages
///
/// This trait enables pluggable policy languages while maintaining
/// Reaper's performance guarantees. Implementations should:
/// - Be thread-safe (Send + Sync)
/// - Optimize for sub-microsecond evaluation where possible
/// - Provide validation before deployment
/// - Handle errors gracefully
pub trait PolicyEvaluator: Send + Sync + Debug {
    /// Evaluate a policy request and return a decision
    ///
    /// # Performance
    /// Implementations should target sub-microsecond latency for hot paths.
    /// Complex policies may take longer but should still be optimized.
    ///
    /// # Arguments
    /// * `request` - The policy request containing resource, action, and context
    ///
    /// # Returns
    /// * `Ok(PolicyAction)` - The authorization decision
    /// * `Err(ReaperError)` - If evaluation fails
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError>;

    /// Evaluate and additionally report whether a rule actually **matched**
    /// (`true`) or the policy's per-policy default was returned because nothing
    /// matched (`false`).
    ///
    /// Set-level combination ([`PolicyEngine::evaluate_set`]) treats unmatched
    /// outcomes as **non-decisive** (Plan 08 Phase A): a policy that says
    /// nothing about a request must not decide it — the set-level default deny
    /// applies only when *no* policy matched. Single-policy evaluation
    /// ([`PolicyEngine::evaluate`]) is unaffected and keeps returning the
    /// per-policy default.
    ///
    /// The default implementation is conservative: it assumes the outcome
    /// matched (decisive), preserving pre-existing behavior for evaluators that
    /// cannot distinguish the two.
    ///
    /// [`PolicyEngine::evaluate_set`]: crate::PolicyEngine::evaluate_set
    /// [`PolicyEngine::evaluate`]: crate::PolicyEngine::evaluate
    fn evaluate_matched(
        &self,
        request: &PolicyRequest,
    ) -> Result<(PolicyAction, bool), ReaperError> {
        Ok((self.evaluate(request)?, true))
    }

    /// Like [`Self::evaluate_matched`], additionally naming the rule that
    /// decided (allow-path explainability, F1-s4): for AI-actor traffic the
    /// ALLOWS are the dangerous decisions, and "which rule allowed this"
    /// must be answerable without replaying the request.
    ///
    /// The name is borrowed from the evaluator (rule names live in the
    /// compiled policy), so the hot path allocates nothing; the engine
    /// clones it once, only for the single decisive policy — the same
    /// discipline as policy-name attribution. `None` = the language has no
    /// named rules (Simple/Cedar) or the per-policy default decided.
    ///
    /// The default implementation preserves each evaluator's existing
    /// `evaluate_matched` semantics exactly and reports no name.
    fn evaluate_named(&self, request: &PolicyRequest) -> Result<NamedOutcome<'_>, ReaperError> {
        let (decision, matched) = self.evaluate_matched(request)?;
        Ok(NamedOutcome {
            decision,
            matched,
            rule_name: None,
        })
    }

    /// Validate the policy before deployment
    ///
    /// This is called during policy deployment to catch errors early.
    /// Implementations should check syntax, semantics, and any other
    /// constraints specific to the policy language.
    ///
    /// # Returns
    /// * `Ok(())` - Policy is valid
    /// * `Err(ReaperError)` - Validation failed with details
    fn validate(&self) -> Result<(), ReaperError>;

    /// Get a human-readable name for this evaluator type
    ///
    /// Used for logging, metrics, and debugging.
    fn evaluator_type(&self) -> &str;

    /// Optional: Get metadata about the policy for monitoring
    ///
    /// Returns information like rule count, complexity metrics, etc.
    /// Default implementation returns None.
    fn metadata(&self) -> Option<EvaluatorMetadata> {
        None
    }

    /// Check-mode evaluation (the conftest/gatekeeper driver): evaluate EVERY
    /// deny rule against the request + optional `input` JSON document and
    /// collect all matching rules as violations with their rendered
    /// `with message` text. This is the serving entry for document-validation
    /// surfaces (agent `/api/v1/check`, admission webhooks, CLI check) — it
    /// dispatches to whichever implementation (compiled / mixed / AST) the
    /// policy was built with, so callers get the fast driver without knowing
    /// the concrete evaluator.
    ///
    /// The default errs: only the Reaper DSL evaluators have check-mode
    /// semantics (all-violations + messages). Simple/Cedar policies report
    /// "unsupported" rather than inventing a lossy mapping from first-match
    /// decisions.
    fn check_with_input(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<crate::reap::CheckResult, ReaperError> {
        let _ = (request, input);
        Err(ReaperError::EvaluationError {
            reason: format!(
                "check mode is not supported by the '{}' evaluator (Reaper DSL policies only)",
                self.evaluator_type()
            ),
        })
    }

    /// The finite set of resource strings this policy can match, or `None` if
    /// its match set is unbounded (wildcards, attribute/prefix/negation/dynamic
    /// resource predicates). `Some(v)` is a PROMISE: the evaluator matches NO
    /// resource outside `v`, so the pruning index may safely drop this policy
    /// for any resource not in `v`. Default `None` = always a candidate (safe).
    ///
    /// Soundness lives *here*, per evaluator, because each evaluator alone knows
    /// its own match semantics: the resource pruning index
    /// ([`crate::PolicyEngine::candidate_policy_ids`]) trusts this promise, so a
    /// false `Some` is an authorization bug (fail-open pruning) while a false
    /// `None` is only a missed optimization. When in doubt, return `None`.
    fn resource_index_terms(&self) -> Option<Vec<String>> {
        None
    }

    /// Two-tier prunability summary (round-3 Plan 06 C, R3-P2-1). Same promise
    /// as [`resource_index_terms`](Self::resource_index_terms) but the bound is
    /// **disjunctive across two dimensions**: the evaluator matches no request
    /// whose resource id is outside `ids` AND whose resource entity `type`
    /// attribute is outside `types`. A false bound is an authorization bug
    /// (fail-open pruning); when in doubt, return
    /// [`ResourcePruning::Unprunable`].
    ///
    /// The default derives from `resource_index_terms()` so evaluators that
    /// only know id-level bounds (Simple) stay correct with no change.
    fn resource_pruning(&self) -> ResourcePruning {
        match self.resource_index_terms() {
            Some(ids) => ResourcePruning::Bounded {
                ids,
                types: Vec::new(),
            },
            None => ResourcePruning::Unprunable,
        }
    }
}

/// How a policy participates in the engine's pruning index (R3-P2-1).
///
/// `Bounded { ids, types }` is a *disjunctive superset bound*: the policy can
/// only decide a request whose resource id is in `ids` **or** whose resource
/// entity has a string `type` attribute in `types` (resolved from the same
/// `DataStore` the policy's evaluator reads). A request outside both sets
/// provably makes every rule non-matching, so dropping the policy from its
/// candidate set cannot change the set decision. Both sets empty means the
/// policy can never match (e.g. a `false` condition) — it is indexed nowhere
/// and is never a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePruning {
    /// Always a candidate — the match set could not be statically bounded.
    Unprunable,
    /// Candidate iff `request.resource ∈ ids` OR `type_of(resource) ∈ types`.
    Bounded {
        /// Concrete request-resource strings the policy can match.
        ids: Vec<String>,
        /// Resource entity `type`-attribute values the policy can match.
        types: Vec<String>,
    },
}

/// Metadata about a policy evaluator for monitoring and debugging
#[derive(Debug, Clone)]
pub struct EvaluatorMetadata {
    /// Number of rules/statements in the policy
    pub rule_count: usize,
    /// Estimated complexity (0-100 scale)
    pub complexity: u8,
    /// Additional context-specific metadata
    pub extra: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // Test helper: mock evaluator
    #[derive(Debug)]
    struct MockEvaluator {
        decision: PolicyAction,
    }

    impl PolicyEvaluator for MockEvaluator {
        fn evaluate(&self, _request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
            Ok(self.decision.clone())
        }

        fn validate(&self) -> Result<(), ReaperError> {
            Ok(())
        }

        fn evaluator_type(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn test_mock_evaluator() {
        let evaluator = MockEvaluator {
            decision: PolicyAction::Allow,
        };

        let request = PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),

            ..Default::default()
        };

        let result = evaluator.evaluate(&request).unwrap();
        assert!(matches!(result, PolicyAction::Allow));
    }
}
