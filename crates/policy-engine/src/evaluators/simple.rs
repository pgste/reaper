//! Simple Rule-Based Policy Evaluator
//!
//! This is Reaper's original high-performance rule matcher.
//! It uses simple wildcard matching for sub-microsecond evaluation.
//! Perfect for hot paths where performance is critical.
//!
//! Supports optional decision tree optimization for O(log r) evaluation.

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::optimizer::{DecisionTree, DecisionTreeBuilder};
use crate::{PolicyAction, PolicyRequest, PolicyRule};
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Simple rule-based evaluator for high-performance authorization
///
/// This evaluator uses basic pattern matching:
/// - Exact string match
/// - Wildcard `*` matches any resource
/// - First-match-wins semantics
/// - Default deny if no match
///
/// # Performance
/// Optimized for sub-microsecond evaluation. Ideal for:
/// - High-throughput APIs
/// - Latency-sensitive services
/// - Simple authorization patterns
///
/// # Optimization Modes
/// - **Linear Mode** (default): O(r) first-match-wins evaluation
/// - **Tree Mode** (opt-in): O(log r) decision tree evaluation
///
/// Enable tree mode via `with_tree_optimization()` for policies with 100+ rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePolicyEvaluator {
    /// Ordered list of rules (first match wins)
    pub rules: Vec<PolicyRule>,

    /// Optional decision tree for O(log r) evaluation (not serialized)
    #[serde(skip)]
    decision_tree: Option<Arc<DecisionTree>>,

    /// Whether tree optimization is enabled
    #[serde(default)]
    tree_optimized: bool,
}

impl SimplePolicyEvaluator {
    /// Create a new simple evaluator with the given rules (linear mode)
    pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self {
            rules,
            decision_tree: None,
            tree_optimized: false,
        }
    }

    /// Create evaluator with decision tree optimization enabled
    ///
    /// Recommended for policies with 100+ rules for optimal performance.
    /// Compilation adds ~1-10ms overhead but provides 10-600x faster evaluation.
    ///
    /// # Example
    /// ```text
    /// let evaluator = SimplePolicyEvaluator::with_tree_optimization(rules)?;
    /// // Evaluates in O(log r) time instead of O(r)
    /// ```
    pub fn with_tree_optimization(rules: Vec<PolicyRule>) -> Result<Self, ReaperError> {
        // Build decision tree
        let builder = DecisionTreeBuilder::new();
        let tree = builder.build_from_rules(&rules)?;

        Ok(Self {
            rules,
            decision_tree: Some(Arc::new(tree)),
            tree_optimized: true,
        })
    }

    /// Enable tree optimization on existing evaluator
    ///
    /// Compiles rules into decision tree for faster evaluation.
    pub fn enable_tree_optimization(&mut self) -> Result<(), ReaperError> {
        if !self.tree_optimized {
            let builder = DecisionTreeBuilder::new();
            let tree = builder.build_from_rules(&self.rules)?;
            self.decision_tree = Some(Arc::new(tree));
            self.tree_optimized = true;
        }
        Ok(())
    }

    /// Check if tree optimization is enabled
    pub fn is_tree_optimized(&self) -> bool {
        self.tree_optimized && self.decision_tree.is_some()
    }

    /// Check if a rule matches the request
    ///
    /// Currently supports:
    /// - Exact resource match
    /// - Wildcard `*` match
    ///
    /// Future: Add glob patterns, regex, etc.
    fn matches_rule(&self, rule: &PolicyRule, request: &PolicyRequest) -> bool {
        // Simple wildcard matching for MVP
        // TODO: Add glob patterns (e.g., "users/*", "api/v1/**")
        // TODO: Add regex support for advanced patterns
        rule.resource == "*" || rule.resource == request.resource
    }

    /// Evaluate with detailed information including matched rule index
    ///
    /// This is useful for debugging and audit purposes.
    /// Returns (PolicyAction, Option<matched_rule_index>)
    pub fn evaluate_with_details(
        &self,
        request: &PolicyRequest,
    ) -> Result<(PolicyAction, Option<usize>), ReaperError> {
        // Use decision tree if available (O(log r))
        if let Some(tree) = &self.decision_tree {
            // Note: DataStore is only used for ABAC features, empty for simple policies
            thread_local! {
                static STORE: crate::data::DataStore = crate::data::DataStore::new();
            }
            return STORE.with(|store| tree.evaluate_simple(request, store));
        }

        // Fallback to linear evaluation (O(r))
        for (index, rule) in self.rules.iter().enumerate() {
            if self.matches_rule(rule, request) {
                return Ok((rule.action.clone(), Some(index)));
            }
        }

        // Default deny if no rules match
        Ok((PolicyAction::Deny, None))
    }
}

impl PolicyEvaluator for SimplePolicyEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        // Use decision tree if available (O(log r))
        if let Some(tree) = &self.decision_tree {
            // Note: DataStore is only used for ABAC features, empty for simple policies
            thread_local! {
                static STORE: crate::data::DataStore = crate::data::DataStore::new();
            }
            let (action, _) = STORE.with(|store| tree.evaluate_simple(request, store))?;
            return Ok(action);
        }

        // Fallback to linear evaluation (O(r))
        for rule in &self.rules {
            if self.matches_rule(rule, request) {
                return Ok(rule.action.clone());
            }
        }

        // Default deny if no rules match
        Ok(PolicyAction::Deny)
    }

    fn validate(&self) -> Result<(), ReaperError> {
        // Validate that we have at least one rule
        if self.rules.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "Policy must have at least one rule".to_string(),
            });
        }

        // Validate each rule
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.resource.is_empty() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Rule {} has empty resource pattern", index),
                });
            }

            // Validate action is a known type (enforced by enum)
            // Additional validation can go here
        }

        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "simple"
    }

    fn metadata(&self) -> Option<EvaluatorMetadata> {
        let mut extra = std::collections::HashMap::new();
        extra.insert("rules".to_string(), self.rules.len().to_string());
        extra.insert(
            "tree_optimized".to_string(),
            self.tree_optimized.to_string(),
        );

        // Add tree statistics if optimized
        if let Some(tree) = &self.decision_tree {
            extra.insert(
                "tree_nodes".to_string(),
                tree.stats().node_count.to_string(),
            );
            extra.insert("tree_depth".to_string(), tree.stats().max_depth.to_string());
            extra.insert(
                "tree_decision_nodes".to_string(),
                tree.stats().decision_count.to_string(),
            );
            extra.insert(
                "tree_branch_nodes".to_string(),
                tree.stats().branch_count.to_string(),
            );
        }

        // Complexity: O(log r) for tree, O(r) for linear
        let complexity = if self.tree_optimized {
            // Logarithmic complexity
            ((self.rules.len() as f64).log2().ceil() as u8).min(20)
        } else {
            // Linear complexity
            self.rules.len().min(100) as u8
        };

        Some(EvaluatorMetadata {
            rule_count: self.rules.len(),
            complexity,
            extra,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_simple_evaluator_allow() {
        let evaluator = SimplePolicyEvaluator::new(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "test-resource".to_string(),
            conditions: vec![],
        }]);

        let request = PolicyRequest {
            resource: "test-resource".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_simple_evaluator_wildcard() {
        let evaluator = SimplePolicyEvaluator::new(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }]);

        let request = PolicyRequest {
            resource: "any-resource".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_simple_evaluator_deny_by_default() {
        let evaluator = SimplePolicyEvaluator::new(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "specific-resource".to_string(),
            conditions: vec![],
        }]);

        let request = PolicyRequest {
            resource: "other-resource".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_simple_evaluator_first_match_wins() {
        let evaluator = SimplePolicyEvaluator::new(vec![
            PolicyRule {
                action: PolicyAction::Deny,
                resource: "test-resource".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "test-resource".to_string(),
                conditions: vec![],
            },
        ]);

        let request = PolicyRequest {
            resource: "test-resource".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_validation_empty_rules() {
        let evaluator = SimplePolicyEvaluator::new(vec![]);
        assert!(evaluator.validate().is_err());
    }

    #[test]
    fn test_validation_empty_resource() {
        let evaluator = SimplePolicyEvaluator::new(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "".to_string(),
            conditions: vec![],
        }]);
        assert!(evaluator.validate().is_err());
    }

    #[test]
    fn test_metadata() {
        let evaluator = SimplePolicyEvaluator::new(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }]);

        let metadata = evaluator.metadata().unwrap();
        assert_eq!(metadata.rule_count, 1);
        assert_eq!(evaluator.evaluator_type(), "simple");
    }
}
