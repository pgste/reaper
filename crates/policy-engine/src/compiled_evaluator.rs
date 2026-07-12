//! Compiled Policy Evaluator - Actually Executable
//!
//! ⚠️  **EXPERIMENTAL - NOT RECOMMENDED FOR SIMPLE POLICIES**
//!
//! **Performance Reality**: For simple policies (< 50 rules), this is 3x **SLOWER**
//! than the baseline Simple evaluator due to abstraction overhead.
//!
//! ## Actual Benchmark Results (10 rules, Release)
//!
//! | Configuration | Mean Latency | vs Baseline |
//! |---------------|--------------|-------------|
//! | Baseline (Simple) | 37 ns | 1.0x |
//! | Compiled | 109 ns | **0.34x (3x slower)** |
//! | Compiled + Partial | 62 ns | **0.60x (1.7x slower)** |
//!
//! **Why?** Enum dispatch + HashMap lookups > inline condition checks.
//!
//! **Recommendation**: Use Simple evaluator for most cases (341ns mean, 2.9M req/s).
//!
//! **Potential Use Case**: Very complex policies (100+ rules with complex conditions)
//! where compile-time optimization might outweigh overhead. NOT TESTED YET.
//!
//! This code is kept for research and potential future optimization of
//! extremely complex policies.
//!
//! ## Design
//!
//! This module provides a policy evaluator that executes
//! pre-optimized, flattened policy rules with minimal overhead.
//!
//! Unlike policy_compilation.rs which generates Rust code text,
//! this creates executable evaluation logic.

use crate::engine::{EnhancedPolicy, PolicyAction, PolicyRequest};
use crate::evaluators::{EvaluatorMetadata, PolicyEvaluator};
use crate::partial_evaluation::{Condition, PartialEvaluator};
use reaper_core::{ReaperError, Result};
use std::collections::HashMap;
use uuid::Uuid;

/// A compiled rule with pre-optimized conditions
#[derive(Debug, Clone)]
struct CompiledRule {
    /// Action to take if rule matches
    action: PolicyAction,

    /// Resource pattern (exact match, prefix, or wildcard)
    resource_pattern: ResourcePattern,

    /// Pre-optimized condition (simplified at compile time)
    condition: Option<Condition>,
}

#[derive(Debug, Clone)]
enum ResourcePattern {
    Exact(String),
    Prefix(String),
    Wildcard,
}

impl ResourcePattern {
    fn matches(&self, resource: &str) -> bool {
        match self {
            ResourcePattern::Exact(pattern) => resource == pattern,
            ResourcePattern::Prefix(prefix) => resource.starts_with(prefix),
            ResourcePattern::Wildcard => true,
        }
    }
}

/// Compiled policy evaluator - executes pre-optimized rules
#[derive(Debug)]
pub struct CompiledPolicyEvaluator {
    #[allow(dead_code)]
    policy_id: Uuid,
    #[allow(dead_code)]
    policy_version: u64,

    /// Flattened, pre-optimized rules
    compiled_rules: Vec<CompiledRule>,

    /// Default action if no rules match
    default_action: PolicyAction,
}

impl CompiledPolicyEvaluator {
    /// Compile a policy into an optimized evaluator
    ///
    /// This applies multiple optimizations:
    /// 1. Flattens policy structure (no Arc indirection)
    /// 2. Pre-parses resource patterns
    /// 3. Applies partial evaluation to simplify conditions
    /// 4. Inlines simple checks
    pub fn compile(
        policy: &EnhancedPolicy,
        static_context: Option<&HashMap<String, String>>,
    ) -> Result<Self> {
        let mut compiled_rules = Vec::with_capacity(policy.rules.len());

        // If we have static context, apply partial evaluation
        let optimized_policy = if let Some(context) = static_context {
            let evaluator = PartialEvaluator::new();
            match evaluator.partial_evaluate(policy, context) {
                Ok(optimized) => optimized,
                Err(_) => policy.clone(),
            }
        } else {
            policy.clone()
        };

        // Compile each rule
        for rule in &optimized_policy.rules {
            let resource_pattern = Self::compile_resource_pattern(&rule.resource);
            let condition = Self::compile_conditions(&rule.conditions);

            compiled_rules.push(CompiledRule {
                action: rule.action.clone(),
                resource_pattern,
                condition,
            });
        }

        Ok(Self {
            policy_id: policy.id,
            policy_version: policy.version,
            compiled_rules,
            default_action: PolicyAction::Deny,
        })
    }

    /// Compile resource pattern into optimized form
    fn compile_resource_pattern(resource: &str) -> ResourcePattern {
        if resource == "*" {
            ResourcePattern::Wildcard
        } else if resource.ends_with('*') {
            ResourcePattern::Prefix(resource.trim_end_matches('*').to_string())
        } else {
            ResourcePattern::Exact(resource.to_string())
        }
    }

    /// Compile conditions into optimized form
    fn compile_conditions(conditions: &[String]) -> Option<Condition> {
        if conditions.is_empty() {
            return None;
        }

        // Parse conditions into structured form
        let parsed: Vec<Condition> = conditions
            .iter()
            .filter_map(|cond| Self::parse_condition(cond))
            .collect();

        if parsed.is_empty() {
            return None;
        }

        // Combine with AND
        if parsed.len() == 1 {
            Some(parsed[0].clone())
        } else {
            Some(Condition::And(parsed))
        }
    }

    /// Parse a single condition string
    fn parse_condition(cond: &str) -> Option<Condition> {
        // Handle equality checks (e.g., "role==admin")
        if let Some((key, value)) = cond.split_once("==") {
            return Some(Condition::Equals(key.to_string(), value.to_string()));
        }

        // Handle NOT conditions (e.g., "!suspended")
        if let Some(stripped) = cond.strip_prefix('!') {
            if let Some(inner) = Self::parse_condition(stripped) {
                return Some(Condition::Not(Box::new(inner)));
            }
        }

        None
    }

    /// Evaluate a condition against the request
    fn evaluate_condition(condition: &Condition, request: &PolicyRequest) -> bool {
        match condition {
            Condition::True => true,
            Condition::False => false,
            Condition::Equals(key, value) => request
                .context
                .get(key)
                .map(|v| v == value)
                .unwrap_or(false),
            Condition::And(conditions) => conditions
                .iter()
                .all(|c| Self::evaluate_condition(c, request)),
            Condition::Or(conditions) => conditions
                .iter()
                .any(|c| Self::evaluate_condition(c, request)),
            Condition::Not(cond) => !Self::evaluate_condition(cond, request),
            Condition::LessThan(key, value) => {
                // Try to parse as numbers for comparison
                if let Some(context_value) = request.context.get(key) {
                    if let (Ok(cv), Ok(v)) = (context_value.parse::<i64>(), value.parse::<i64>()) {
                        return cv < v;
                    }
                }
                false
            }
            Condition::GreaterThan(key, value) => {
                // Try to parse as numbers for comparison
                if let Some(context_value) = request.context.get(key) {
                    if let (Ok(cv), Ok(v)) = (context_value.parse::<i64>(), value.parse::<i64>()) {
                        return cv > v;
                    }
                }
                false
            }
        }
    }

    /// Fast evaluation path - minimal overhead.
    ///
    /// Returns `(action, matched)` where `matched` is `true` when a rule
    /// actually fired and `false` when the default action was returned because
    /// no rule matched. Set-level combination treats the unmatched case as
    /// non-decisive (Plan 08 Phase A).
    fn evaluate_fast_matched(&self, request: &PolicyRequest) -> (PolicyAction, bool) {
        // One evaluation = one ReBAC traversal budget, shared across every
        // condition this policy checks (Plan 08 Phase E).
        crate::data::relationships::reset_traversal_budget();

        // Check each compiled rule in order
        for rule in &self.compiled_rules {
            // Fast resource check (no string allocations)
            if !rule.resource_pattern.matches(&request.resource) {
                continue;
            }

            // Fast condition check (or skip if no conditions)
            if let Some(condition) = &rule.condition {
                if !Self::evaluate_condition(condition, request) {
                    continue;
                }
            }

            // Rule matches - return action
            return (rule.action.clone(), true);
        }

        // No rules matched - return default (non-decisive for set combination)
        (self.default_action.clone(), false)
    }

    /// Fast evaluation path returning only the action.
    fn evaluate_fast(&self, request: &PolicyRequest) -> PolicyAction {
        self.evaluate_fast_matched(request).0
    }
}

impl PolicyEvaluator for CompiledPolicyEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> std::result::Result<PolicyAction, ReaperError> {
        Ok(self.evaluate_fast(request))
    }

    fn evaluate_matched(
        &self,
        request: &PolicyRequest,
    ) -> std::result::Result<(PolicyAction, bool), ReaperError> {
        Ok(self.evaluate_fast_matched(request))
    }

    fn validate(&self) -> std::result::Result<(), ReaperError> {
        // Compiled policies are already validated at compile time
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "Compiled"
    }

    fn metadata(&self) -> Option<EvaluatorMetadata> {
        Some(EvaluatorMetadata {
            rule_count: self.compiled_rules.len(),
            complexity: (self.compiled_rules.len().min(100)) as u8,
            extra: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::PolicyRule;

    #[test]
    fn test_compile_simple_policy() {
        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users".to_string(),
                conditions: vec!["role==admin".to_string()],
            }],
        );

        let evaluator = CompiledPolicyEvaluator::compile(&policy, None).unwrap();
        assert_eq!(evaluator.compiled_rules.len(), 1);
    }

    #[test]
    fn test_evaluate_matching_request() {
        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users".to_string(),
                conditions: vec!["role==admin".to_string()],
            }],
        );

        let evaluator = CompiledPolicyEvaluator::compile(&policy, None).unwrap();

        let mut context = HashMap::new();
        context.insert("role".to_string(), "admin".to_string());

        let request = PolicyRequest {
            resource: "/api/users".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_evaluate_non_matching_request() {
        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users".to_string(),
                conditions: vec!["role==admin".to_string()],
            }],
        );

        let evaluator = CompiledPolicyEvaluator::compile(&policy, None).unwrap();

        let mut context = HashMap::new();
        context.insert("role".to_string(), "user".to_string());

        let request = PolicyRequest {
            resource: "/api/users".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_partial_evaluation_at_compile() {
        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users".to_string(),
                conditions: vec!["role==admin".to_string(), "department==eng".to_string()],
            }],
        );

        // Provide static context for department
        let mut static_context = HashMap::new();
        static_context.insert("department".to_string(), "eng".to_string());

        let evaluator = CompiledPolicyEvaluator::compile(&policy, Some(&static_context)).unwrap();

        // The compiled policy should have simplified conditions
        // (department check should be removed since it's always true)
        assert_eq!(evaluator.compiled_rules.len(), 1);
    }
}
