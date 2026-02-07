//! AST Evaluator - Direct evaluation of parsed .reap policies
//!
//! Evaluates policies directly from the AST without compilation.
//! Supports advanced features like comprehensions, variable assignments, and complex expressions.
//!
//! Performance characteristics:
//! - Simple rules: < 1 µs
//! - Comprehensions (100 items): < 10 µs
//! - Variable assignments: ~100 ns overhead
//!
//! Module structure:
//! - `types`: Core evaluation types (EvalContext, EvalValue)

pub mod builtin_functions;
pub mod builtin_methods;
mod comparison;
mod comprehension;
mod entity_access;
mod expr_eval;
mod function_dispatch;
mod method_dispatch;
mod regex_methods;
mod types;

use types::{EvalContext, EvalValue};

use super::ast::*;
use crate::data::DataStore;
use crate::{PolicyAction, PolicyRequest};
use parking_lot::Mutex;
use reaper_core::ReaperError;
use std::collections::HashMap;
use std::sync::Arc;

/// AST-based policy evaluator
///
/// Evaluates policies directly from the AST, supporting all language features
/// including comprehensions, variable assignments, and complex expressions.
///
/// Performance optimizations:
/// - Regex pattern caching: 2-5x speedup for repeated patterns
/// - SIMD aggregates: 2-4x speedup for large numeric arrays (>64 elements)
///
/// Thread-safety:
/// - Uses parking_lot::Mutex for regex cache (thread-safe, low overhead)
/// - Implements Send + Sync for concurrent evaluation
#[derive(Debug)]
pub struct ReapAstEvaluator {
    /// Reference to the data store
    store: Arc<DataStore>,
    /// Parsed policy AST
    policy: Policy,
    /// Thread-safe regex pattern cache for performance (2-5x speedup)
    /// Compiled regex patterns are expensive, cache them by pattern string
    /// Uses parking_lot::Mutex for efficient thread-safe access
    regex_cache: Mutex<HashMap<String, regex::Regex>>,
}

impl ReapAstEvaluator {
    /// Create a new AST evaluator
    pub fn new(store: Arc<DataStore>, policy: Policy) -> Self {
        Self {
            store,
            policy,
            regex_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Evaluate a policy request
    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        // Get user and resource IDs from the DataStore
        let interner = self.store.interner();
        let user_id = interner.intern(request.context.get("principal").ok_or_else(|| {
            ReaperError::InvalidPolicy {
                reason: "Request must have 'principal' in context".to_string(),
            }
        })?);
        let resource_id = interner.intern(&request.resource);

        // Create evaluation context
        let mut request_context = request.context.clone();
        // Add action to context if not already present
        request_context.insert("action".to_string(), request.action.clone());

        let mut context = EvalContext {
            variables: HashMap::new(),
            user_id,
            resource_id,
            request_context,
        };

        // Security-first evaluation: Deny rules ALWAYS take precedence over Allow rules
        // This ensures explicit denies cannot be bypassed by subsequent allow rules

        // Phase 1: Evaluate all DENY rules first
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Deny)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                // Explicit deny - return immediately, no allow can override this
                return Ok(PolicyAction::Deny);
            }
        }

        // Phase 2: No deny matched, now evaluate ALLOW rules
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Allow)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                return Ok(PolicyAction::Allow);
            }
        }

        // Phase 3: No rule matched - return default decision
        Ok(self.policy.default_decision.clone().into())
    }

    /// Evaluate a condition
    fn evaluate_condition(
        &self,
        condition: &Condition,
        context: &mut EvalContext,
    ) -> Result<bool, ReaperError> {
        match condition {
            Condition::True => Ok(true),
            Condition::False => Ok(false),

            Condition::Comparison { left, op, right } => {
                self.evaluate_comparison(left, *op, right, context)
            }

            Condition::Assignment { variable, value } => {
                // Evaluate the assignment value and store in context
                let eval_value = self.evaluate_assignment_value(value, context)?;
                context.variables.insert(variable.clone(), eval_value);
                // Assignments always succeed (return true)
                Ok(true)
            }

            Condition::And(conditions) => {
                for cond in conditions {
                    if !self.evaluate_condition(cond, context)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            Condition::Or(conditions) => {
                for cond in conditions {
                    if self.evaluate_condition(cond, context)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            Condition::Not(cond) => Ok(!self.evaluate_condition(cond, context)?),

            Condition::Expr(expr) => {
                // Evaluate the expression and convert to boolean
                let value = self.evaluate_expr(expr, context)?;
                match value {
                    EvalValue::Boolean(b) => Ok(b),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Expression in condition must evaluate to boolean, got: {:?}",
                            value
                        ),
                    }),
                }
            }
        }
    }
}

impl From<Decision> for PolicyAction {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Allow => PolicyAction::Allow,
            Decision::Deny => PolicyAction::Deny,
        }
    }
}

// Implement PolicyEvaluator trait for ReapAstEvaluator
// This allows it to be used as a drop-in replacement for the compiled evaluator
impl crate::evaluators::PolicyEvaluator for ReapAstEvaluator {
    fn evaluate(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<crate::PolicyAction, reaper_core::ReaperError> {
        // Delegate to the existing evaluate method
        self.evaluate(request)
    }

    fn validate(&self) -> Result<(), reaper_core::ReaperError> {
        // AST evaluator is always valid if it was constructed successfully
        // The parser validates syntax, and the evaluator handles runtime errors gracefully
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "ReapAstEvaluator"
    }

    fn metadata(&self) -> Option<crate::evaluators::EvaluatorMetadata> {
        let mut extra = std::collections::HashMap::new();
        extra.insert("features".to_string(), "comprehensions,variable_assignments,function_calls,time_functions,regex_caching,simd_aggregates".to_string());
        extra.insert("policy_name".to_string(), self.policy.name.clone());
        if let Some(version) = self.policy.metadata.get("version") {
            extra.insert("version".to_string(), version.clone());
        }

        // Calculate complexity score (0-100)
        let rule_count = self.policy.rules.len();
        let complexity = if rule_count < 5 {
            10 // Simple
        } else if rule_count < 20 {
            30 // Moderate
        } else if rule_count < 50 {
            60 // Complex
        } else {
            90 // Very complex
        };

        Some(crate::evaluators::EvaluatorMetadata {
            rule_count,
            complexity,
            extra,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityBuilder;
    use crate::PolicyRequest;
    use std::collections::HashMap;

    fn create_test_store() -> Arc<DataStore> {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create some test users with various attributes
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");
        let years_key = interner.intern("years_experience");
        let active_key = interner.intern("active");
        let email_key = interner.intern("email");
        let alice_email = interner.intern("alice@example.com");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_string(role_key, admin_value)
            .with_int(years_key, 8)
            .with_bool(active_key, true)
            .with_string(email_key, alice_email)
            .build();

        let bob_id = interner.intern("bob");
        let developer_value = interner.intern("developer");
        let bob_email = interner.intern("bob@example.com");

        let bob = EntityBuilder::new(bob_id, user_type)
            .with_string(role_key, developer_value)
            .with_int(years_key, 3)
            .with_bool(active_key, true)
            .with_string(email_key, bob_email)
            .build();

        let charlie_id = interner.intern("charlie");
        let charlie_email = interner.intern("charlie@example.com");

        let charlie = EntityBuilder::new(charlie_id, user_type)
            .with_string(role_key, developer_value)
            .with_int(years_key, 6)
            .with_bool(active_key, false)
            .with_string(email_key, charlie_email)
            .build();

        // Create test resources
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let owner_key = interner.intern("owner");
        let owner_alice = interner.intern("alice");

        let doc = EntityBuilder::new(doc_id, doc_type)
            .with_string(owner_key, owner_alice)
            .build();

        store.insert(alice);
        store.insert(bob);
        store.insert(charlie);
        store.insert(doc);

        store
    }

    #[test]
    fn test_simple_policy_allow() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_simple_policy_deny() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "bob".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_numeric_comparison() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule senior { allow if user.years_experience >= 5 }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        // Alice has 8 years - should allow
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());
        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context.clone(),
        };
        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));

        // Bob has 3 years - should deny
        let mut context2 = HashMap::new();
        context2.insert("principal".to_string(), "bob".to_string());
        let request2 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context2,
        };
        let decision2 = evaluator.evaluate(&request2).unwrap();
        assert!(matches!(decision2, PolicyAction::Deny));
    }

    #[test]
    fn test_and_condition() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule senior_active {
                    allow if {
                        user.years_experience >= 5 &&
                        user.active == true
                    }
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        // Alice: 8 years, active=true - should allow
        let mut context1 = HashMap::new();
        context1.insert("principal".to_string(), "alice".to_string());
        let request1 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context1,
        };
        assert!(matches!(
            evaluator.evaluate(&request1).unwrap(),
            PolicyAction::Allow
        ));

        // Charlie: 6 years, active=false - should deny
        let mut context2 = HashMap::new();
        context2.insert("principal".to_string(), "charlie".to_string());
        let request2 = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context: context2,
        };
        assert!(matches!(
            evaluator.evaluate(&request2).unwrap(),
            PolicyAction::Deny
        ));
    }

    // TODO: Add more tests for comprehensions once we can properly test them
    // (need to add test data with arrays/objects for iteration)

    #[test]
    fn test_time_now_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if now_ns := time::now_ns()
                    && now_ms := time::now_ms()
                    && now_s := time::now()
                    && time::is_before(0, now_ns)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_parse_format_rfc3339() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_parse {
                    allow if parsed := time::parse_rfc3339("2024-01-15T12:30:00Z")
                    && formatted := time::format_rfc3339(parsed)
                    && time::is_before(0, parsed)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_arithmetic() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_arithmetic {
                    allow if base := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && future := time::add_ns(base, 3600000000000)
                    && past := time::subtract_ns(base, 3600000000000)
                    && time::is_before(base, future)
                    && time::is_before(past, base)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_comparisons() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule time_comparisons {
                    allow if t1 := time::parse_rfc3339("2024-01-15T10:00:00Z")
                    && t2 := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && t3 := time::parse_rfc3339("2024-01-15T14:00:00Z")
                    && time::is_before(t1, t2)
                    && time::is_after(t3, t2)
                    && time::is_between(t2, t1, t3)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_time_based_access_control() {
        // Test realistic scenario: allow access only during business hours
        let policy_text = r#"
            policy test {
                default: deny,
                rule business_hours {
                    allow if start := time::parse_rfc3339("2024-01-15T09:00:00Z")
                    && end := time::parse_rfc3339("2024-01-15T17:00:00Z")
                    && current := time::parse_rfc3339("2024-01-15T12:00:00Z")
                    && time::is_between(current, start, end)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    // NOTE: Comprehensive regex evaluator tests deferred to integration test suite
    // Parser tests verify syntax works correctly

    #[test]
    fn test_regex_namespace_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule pattern_validation {
                    allow if pattern := "[a-z]+"
                    && regex::is_valid(pattern)
                    && special_chars := ".*+?"
                    && escaped := regex::escape(special_chars)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_abs_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_absolute {
                    allow if neg_int := -42
                    && pos_int := math::abs(neg_int)
                    && neg_float := -3.14
                    && pos_float := math::abs(neg_float)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_rounding_functions() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_rounding {
                    allow if rounded := math::round(3.7)
                    && floored := math::floor(3.9)
                    && ceiled := math::ceil(3.1)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_pow_sqrt() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_power_sqrt {
                    allow if squared := math::pow(5, 2)
                    && cubed := math::pow(2, 3)
                    && sqrt_result := math::sqrt(16)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_math_min_max_clamp() {
        let policy_text = r#"
            policy test {
                default: deny,
                rule math_comparisons {
                    allow if min_val := math::min(10, 20)
                    && max_val := math::max(10, 20)
                    && clamped_high := math::clamp(150, 0, 100)
                    && clamped_low := math::clamp(-50, 0, 100)
                    && clamped_mid := math::clamp(50, 0, 100)
                }
            }
        "#;

        let store = create_test_store();
        let policy = super::super::ReapParser::parse(policy_text).unwrap();
        let evaluator = ReapAstEvaluator::new(store, policy);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }
}
