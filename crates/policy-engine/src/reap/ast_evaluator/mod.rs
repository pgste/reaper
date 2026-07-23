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
        self.evaluate_with_input(request, None)
    }

    /// Evaluate a policy request with an optional structured `input` document
    /// (arbitrary nested JSON — Terraform plans, K8s admission requests, any
    /// document the policy inspects via the `input` entity).
    ///
    /// `principal` is optional here: pure document policies have no subject.
    /// Rules that touch `user.*` against an absent principal simply fail to
    /// match (entity not found), they don't abort the evaluation.
    pub fn evaluate_with_input(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<PolicyAction, ReaperError> {
        self.evaluate_with_input_named(request, input)
            .map(|(action, _)| action)
    }

    /// Like [`Self::evaluate_with_input`], additionally naming the rule that
    /// decided (allow-path explainability, F1-s4). `None` = the per-policy
    /// default decided.
    pub fn evaluate_with_input_named(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<(PolicyAction, Option<&str>), ReaperError> {
        // One evaluation = one ReBAC traversal budget, shared across every
        // condition this policy checks (Plan 08 Phase E).
        crate::data::relationships::reset_traversal_budget();

        // Get user and resource IDs from the DataStore
        let interner = self.store.interner();
        let user_id = interner.intern(
            request
                .context
                .get("principal")
                .map(String::as_str)
                .unwrap_or(""),
        );
        let resource_id = interner.intern(&request.resource);
        // Actor (F1): intern the request's actor id so `actor.*` resolves the
        // loaded actor entity, exactly as `user` resolves the principal.
        let actor_id = request.actor.as_deref().map(|a| interner.intern(a));

        // Create evaluation context
        let mut request_context = request.context.clone();
        // Add action to context if not already present
        request_context.insert("action".to_string(), request.action.clone());

        // Convert the input document once per evaluation (rules then navigate
        // the tree with zero re-parsing).
        let input_value = input
            .map(super::ast_evaluator::builtin_functions::json::json_to_eval_value)
            .transpose()?;

        let mut context = EvalContext {
            variables: rebac_pseudo_vars(request),
            user_id,
            actor_id,
            resource_id,
            request_context,
            context_provenance: request.context_provenance.clone(),
            input: input_value,
        };

        // Security-first evaluation: Deny rules ALWAYS take precedence over Allow rules
        // This ensures explicit denies cannot be bypassed by subsequent allow rules

        // Phase 1: Evaluate all DENY rules first
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Deny)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                // Explicit deny - return immediately, no allow can override this
                return Ok((PolicyAction::Deny, Some(rule.name.as_str())));
            }
        }

        // Phase 2: No deny matched, now evaluate ALLOW rules
        for rule in &self.policy.rules {
            if matches!(rule.decision, super::ast::Decision::Allow)
                && self.evaluate_condition(&rule.condition, &mut context)?
            {
                return Ok((PolicyAction::Allow, Some(rule.name.as_str())));
            }
        }

        // Phase 3: No rule matched - return default decision
        Ok((self.policy.default_decision.clone().into(), None))
    }

    /// Check-mode evaluation (the conftest/gatekeeper driver): evaluate EVERY
    /// deny rule against the request + `input` document and collect all
    /// matching rules as violations, with their `with message` text rendered
    /// using the variables the rule bound. Decision-mode `evaluate()` is
    /// untouched (first-match, sub-microsecond path).
    ///
    /// `allowed` is true when no deny rule matched AND the policy would allow
    /// (an allow rule matches or the default is allow).
    pub fn check_with_input(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<CheckResult, ReaperError> {
        let interner = self.store.interner();
        let user_id = interner.intern(
            request
                .context
                .get("principal")
                .map(String::as_str)
                .unwrap_or(""),
        );
        let resource_id = interner.intern(&request.resource);
        let actor_id = request.actor.as_deref().map(|a| interner.intern(a));

        let mut request_context = request.context.clone();
        request_context.insert("action".to_string(), request.action.clone());

        let input_value = input
            .map(super::ast_evaluator::builtin_functions::json::json_to_eval_value)
            .transpose()?;

        let base = EvalContext {
            variables: rebac_pseudo_vars(request),
            user_id,
            actor_id,
            resource_id,
            request_context,
            context_provenance: request.context_provenance.clone(),
            input: input_value,
        };

        let mut violations = Vec::new();
        for rule in &self.policy.rules {
            if !matches!(rule.decision, super::ast::Decision::Deny) {
                continue;
            }
            // Scoped context per rule: variables bound by one rule's condition
            // must not leak into the next, and stay available for its message.
            let mut ctx = base.clone();
            if self.evaluate_condition(&rule.condition, &mut ctx)? {
                let message = match &rule.message {
                    Some(expr) => Some(eval_value_to_message(&self.evaluate_expr(expr, &ctx)?)),
                    None => None,
                };
                violations.push(Violation {
                    rule: rule.name.clone(),
                    message,
                });
            }
        }

        let allowed = if violations.is_empty() {
            match self.policy.default_decision {
                super::ast::Decision::Allow => true,
                super::ast::Decision::Deny => {
                    // default-deny policy: allowed only if an allow rule matches
                    let mut ctx = base.clone();
                    let mut any = false;
                    for rule in &self.policy.rules {
                        if matches!(rule.decision, super::ast::Decision::Allow)
                            && self.evaluate_condition(&rule.condition, &mut ctx)?
                        {
                            any = true;
                            break;
                        }
                    }
                    any
                }
            }
        } else {
            false
        };

        Ok(CheckResult {
            allowed,
            violations,
        })
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
                    // TOTAL EVALUATION: Null as a bare predicate is FALSE, not
                    // an error — it means a missing attribute / absent actor
                    // fed the expression (e.g. `actor.profile.has_key("env")`
                    // with no actor bound). Fail-closed non-match, same as the
                    // compiled evaluator's missing-receiver conditions.
                    EvalValue::Null => Ok(false),
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
        // The interpreter recurses over this AST at eval time (evaluate_condition
        // / evaluate_expr), so a tree that reached us via a non-pest path (YAML/
        // JSON policy formats, or a directly-built AST) must be depth-bounded
        // here — validate() runs once at deploy, before the policy serves any
        // request (see EnhancedPolicy::build_evaluator). Plan 05, Step 2.
        super::limits::enforce_policy_depth(&self.policy)?;
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "ReapAstEvaluator"
    }

    /// Allow-path explainability (F1-s4). `matched` stays `true` for every
    /// outcome — the AST evaluator has always used the trait's default
    /// (always-decisive) `evaluate_matched`, and set-combination semantics
    /// for AST-fallback policies must not change under a naming feature.
    fn evaluate_named(
        &self,
        request: &crate::PolicyRequest,
    ) -> Result<crate::evaluators::NamedOutcome<'_>, reaper_core::ReaperError> {
        let (decision, rule_name) = self.evaluate_with_input_named(request, None)?;
        Ok(crate::evaluators::NamedOutcome {
            decision,
            matched: true,
            rule_name,
        })
    }

    fn check_with_input(
        &self,
        request: &crate::PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<CheckResult, reaper_core::ReaperError> {
        ReapAstEvaluator::check_with_input(self, request, input)
    }

    // D2 secondary: AST-side resource extraction is a follow-up. AST-fallback
    // policies are the uncommon case (constructs the compiler doesn't yet
    // support), so we keep the trait default `resource_index_terms() -> None`
    // (unprunable = always a candidate = safe) rather than duplicating the
    // literal-extraction analysis against the AST condition tree.

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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }
}

/// Pseudo-variable bindings so entity ids can appear as bare arguments in
/// function calls (`rebac::related(user, "owner", resource)`): `user` = the
/// request principal id, `resource` = the request resource id, `actor` = the
/// request actor id (F1 agentic delegation — `rebac::related(actor,
/// "acts_for", user)`). Attribute access like `user.role` still routes to the
/// entity keyword path, so these only surface where a plain identifier is
/// evaluated as a variable.
fn rebac_pseudo_vars(request: &PolicyRequest) -> HashMap<String, types::EvalValue> {
    let mut vars = HashMap::with_capacity(3);
    vars.insert(
        "user".to_string(),
        types::EvalValue::String(
            request
                .context
                .get("principal")
                .cloned()
                .unwrap_or_default(),
        ),
    );
    vars.insert(
        "resource".to_string(),
        types::EvalValue::String(request.resource.clone()),
    );
    // Actor is optional; bind it only when present so `rebac::related(actor,
    // ...)` on an actor-less request evaluates `actor` as an undefined
    // variable (→ the rebac arg is rejected and the check is non-matching),
    // rather than silently binding an empty id that could match a "".
    if let Some(actor) = &request.actor {
        vars.insert("actor".to_string(), types::EvalValue::String(actor.clone()));
    }
    vars
}

/// One matched deny rule in check mode.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Violation {
    /// Rule name that matched.
    pub rule: String,
    /// Rendered `with message` text (None when the rule has no message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result of check-mode evaluation: every matching deny rule, not just the first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckResult {
    pub allowed: bool,
    pub violations: Vec<Violation>,
}

/// Render an evaluated message expression as human-readable text.
fn eval_value_to_message(value: &types::EvalValue) -> String {
    use types::EvalValue as V;
    match value {
        V::String(s) => s.clone(),
        V::Integer(i) => i.to_string(),
        V::Float(f) => f.to_string(),
        V::Boolean(b) => b.to_string(),
        V::Null => String::new(),
        V::Array(items) | V::Set(items) => items
            .iter()
            .map(eval_value_to_message)
            .collect::<Vec<_>>()
            .join(""),
        V::Object(_) => format!("{value:?}"),
    }
}
