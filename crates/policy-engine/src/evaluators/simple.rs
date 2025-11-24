//! Simple Rule-Based Policy Evaluator
//!
//! This is Reaper's original high-performance rule matcher.
//! It uses simple wildcard matching for sub-microsecond evaluation.
//! Perfect for hot paths where performance is critical.

use crate::{PolicyAction, PolicyRequest, PolicyRule};
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use super::{PolicyEvaluator, EvaluatorMetadata};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePolicyEvaluator {
    /// Ordered list of rules (first match wins)
    pub rules: Vec<PolicyRule>,
}

impl SimplePolicyEvaluator {
    /// Create a new simple evaluator with the given rules
    pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self { rules }
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
    pub fn evaluate_with_details(&self, request: &PolicyRequest) -> Result<(PolicyAction, Option<usize>), ReaperError> {
        // First-match-wins evaluation
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
        // First-match-wins evaluation
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

        Some(EvaluatorMetadata {
            rule_count: self.rules.len(),
            complexity: (self.rules.len().min(100) as u8), // Simple linear complexity
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
        let evaluator = SimplePolicyEvaluator::new(vec![
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
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_simple_evaluator_wildcard() {
        let evaluator = SimplePolicyEvaluator::new(vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            },
        ]);

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
        let evaluator = SimplePolicyEvaluator::new(vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "specific-resource".to_string(),
                conditions: vec![],
            },
        ]);

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
        let evaluator = SimplePolicyEvaluator::new(vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "".to_string(),
                conditions: vec![],
            },
        ]);
        assert!(evaluator.validate().is_err());
    }

    #[test]
    fn test_metadata() {
        let evaluator = SimplePolicyEvaluator::new(vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            },
        ]);

        let metadata = evaluator.metadata().unwrap();
        assert_eq!(metadata.rule_count, 1);
        assert_eq!(evaluator.evaluator_type(), "simple");
    }
}
