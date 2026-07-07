//! Reaper DSL - Native Policy Language
//!
//! A Rust-native policy language optimized for sub-microsecond evaluation.
//! Leverages DataStore directly for zero-copy, interned-string-based policies.
//!
//! ## Module Structure
//!
//! - `types`: All type definitions (Condition, CompiledCondition, Rule, etc.)
//! - `compiler`: Condition compilation with pre-interned strings
//! - `entity_helpers`: Entity access helpers for zero-overhead abstractions
//! - `variable_eval`: Variable condition evaluation

mod chain_method_eval;
mod collect;
mod collection_eval;
mod comparison_eval;
mod compiler;
mod comprehension_eval;
pub mod entity_helpers;
mod expr_compiler;
mod expr_eval;
#[cfg(test)]
mod expr_eval_tests;
mod string_eval;
#[cfg(test)]
mod tests;
mod time_eval;
mod types;
mod variable_eval;

// Re-export types for external use
pub use types::*;

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::data::{AttributeValue, DataStore, Entity, InternedString};
use crate::{PolicyAction, PolicyRequest};
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Zero-copy evaluation context — avoids HashMap clone per evaluation.
/// "action" and "resource" are served from borrowed fields;
/// all other keys fall through to the original request context.
pub(crate) struct EvalContext<'a> {
    action: &'a str,
    resource: &'a str,
    context: &'a std::collections::HashMap<String, String>,
}

impl<'a> EvalContext<'a> {
    #[inline]
    fn new(
        action: &'a str,
        resource: &'a str,
        context: &'a std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            action,
            resource,
            context,
        }
    }

    #[inline]
    fn get(&self, key: &str) -> Option<&str> {
        match key {
            "action" => Some(self.action),
            "resource" => Some(self.resource),
            _ => self.context.get(key).map(|s| s.as_str()),
        }
    }
}

/// Reaper DSL Policy Evaluator
///
/// Performance characteristics:
/// - Simple rules: < 500 ns
/// - Complex ABAC: < 10 µs
/// - Entity lookups: 20-50 ns (DataStore direct)
/// - Comparisons: 5-10 ns (interned string IDs)
/// - Regex matches: ~100-500 ns (pre-compiled patterns)
///
/// Security characteristics:
/// - Deny-precedence evaluation: All deny rules evaluated before any allow rules
/// - Explicit denies cannot be bypassed by subsequent allows
/// - Rules are pre-partitioned at construction for zero-overhead evaluation
///
/// **Expected: 1,000-20,000x faster than Cedar**
#[derive(Debug, Clone)]
pub struct ReaperDSLEvaluator {
    /// Reference to the data store
    store: Arc<DataStore>,
    /// Compiled deny rules (evaluated first for security) - zero HashMap lookups
    compiled_deny_rules: Vec<CompiledRule>,
    /// Compiled allow rules (evaluated after deny rules) - zero HashMap lookups
    compiled_allow_rules: Vec<CompiledRule>,
    /// Default decision if no rules match
    default_decision: PolicyAction,
    /// Interned "resource" type id, precomputed so the synthetic-resource path
    /// doesn't re-intern the constant on every request.
    resource_type_id: crate::data::InternedString,
    /// Pre-compiled regex patterns for O(1) lookup during evaluation
    #[allow(dead_code)]
    regex_cache: Arc<FxHashMap<String, regex::Regex>>,
    /// Pre-computed AttributeValue objects for membership tests
    #[allow(dead_code)]
    membership_cache: Arc<FxHashMap<String, AttributeValue>>,
}

impl ReaperDSLEvaluator {
    /// Create a new Reaper DSL evaluator
    ///
    /// All strings are pre-interned at construction time for zero-lookup evaluation.
    /// Rules are pre-partitioned into deny/allow for deny-precedence evaluation.
    pub fn new(store: Arc<DataStore>, rules: Vec<Rule>, default_decision: PolicyAction) -> Self {
        let interner = store.interner();

        // Pre-compile regex patterns from all rules
        let mut regex_cache = FxHashMap::default();
        for rule in &rules {
            compiler::collect_regex_patterns(&rule.condition, &mut regex_cache);
        }

        // Pre-compute membership values from all rules
        let mut membership_cache = FxHashMap::default();
        for rule in &rules {
            compiler::collect_membership_values(&rule.condition, &mut membership_cache, interner);
        }

        // Pre-compile all conditions with interned strings
        // AND partition into deny/allow for deny-precedence evaluation
        let mut compiled_deny_rules = Vec::new();
        let mut compiled_allow_rules = Vec::new();

        for rule in rules {
            let compiled = CompiledRule {
                name: rule.name,
                condition: compiler::compile_condition(&rule.condition, interner),
                decision: rule.decision.clone(),
            };

            match rule.decision {
                PolicyAction::Deny => compiled_deny_rules.push(compiled),
                PolicyAction::Allow | PolicyAction::Log => compiled_allow_rules.push(compiled),
            }
        }

        let resource_type_id = interner.intern("resource");

        Self {
            store,
            compiled_deny_rules,
            compiled_allow_rules,
            default_decision,
            resource_type_id,
            regex_cache: Arc::new(regex_cache),
            membership_cache: Arc::new(membership_cache),
        }
    }

    /// Evaluate a compiled condition against entities
    ///
    /// This is the FAST PATH - all strings are pre-interned at construction time.
    /// Zero HashMap lookups during evaluation (eliminated ~50 lookups per request).
    /// Performance: Direct InternedString comparisons (5ns vs 100ns with HashMap lookup)
    fn evaluate_compiled_condition(
        &self,
        condition: &CompiledCondition,
        user: &Entity,
        resource: &Entity,
        _context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
    ) -> bool {
        let interner = self.store.interner();

        match condition {
            CompiledCondition::Always => true,

            // ReBAC: pure interned graph lookups. Direct = one DashMap get +
            // binary search (~100ns); traversals are bounded BFS.
            CompiledCondition::RebacCheck {
                kind,
                subject,
                relation,
                object,
                via,
                max_depth,
            } => {
                use crate::evaluators::reaper_dsl::CompiledRebacRef;
                use crate::evaluators::reaper_dsl::RebacKind;
                let resolve = |r: &CompiledRebacRef| match r {
                    CompiledRebacRef::Principal => user.id,
                    CompiledRebacRef::ResourceId => resource.id,
                    CompiledRebacRef::Literal(id) => *id,
                };
                let subject_id = resolve(subject);
                let object_id = resolve(object);
                let graph = self.store.relationships();
                match kind {
                    RebacKind::Direct => graph.has_relation(object_id, *relation, subject_id),
                    RebacKind::Reachable => graph.has_relation_reachable(
                        object_id,
                        *relation,
                        subject_id,
                        via.expect("reachable always compiles with via"),
                        *max_depth as usize,
                    ),
                    RebacKind::Inherited => graph.has_relation_inherited(
                        object_id,
                        *relation,
                        subject_id,
                        via.expect("inherited always compiles with via"),
                        *max_depth as usize,
                    ),
                }
            }

            CompiledCondition::ActionEquals { value } => _context
                .get("action")
                .map(|a| interner.resolve(*value).map(|v| a == &*v).unwrap_or(false))
                .unwrap_or(false),

            CompiledCondition::ResourceIdEquals { value } => _context
                .get("resource")
                .map(|r| interner.resolve(*value).map(|v| r == &*v).unwrap_or(false))
                .unwrap_or(false),

            // ============ V2 Consolidated Types ============
            CompiledCondition::AttributeCompare(comp) => {
                // Handle Context entity comparisons specially (they use the context HashMap)
                if matches!(comp.entity_type, EntityType::Context) {
                    self.eval_context_attribute_comparison(comp, _context, interner)
                } else {
                    comparison_eval::eval_attribute_comparison(comp, user, resource, interner)
                }
            }

            CompiledCondition::StringOp(op) => {
                string_eval::eval_string_operation(op, user, resource, interner)
            }

            CompiledCondition::VariableStringOp(op) => {
                string_eval::eval_variable_string_operation(op, variables, interner)
            }

            CompiledCondition::CountOp(cond) => {
                collection_eval::eval_count_operation(cond, user, resource)
            }

            CompiledCondition::TimeOp(cond) => time_eval::eval_time_operation(cond, user, resource),

            CompiledCondition::CrossEntityCompare(comp) => {
                // context.* on either side resolves from the REQUEST, not an
                // entity. It must NOT intern the request value: interning a
                // per-request string (a principal, a token) would pin it in the
                // shared interner forever — an unbounded eval-path memory leak
                // under high request cardinality. A request value that is
                // already interned reuses its id (compares by id, exactly as an
                // entity value would); a novel one is carried as raw text and
                // compared by content, matching the AST evaluator (which returns
                // context values as owned strings and never interns them).
                let needs_ctx = matches!(comp.left_entity, EntityType::Context)
                    || matches!(comp.right_entity, EntityType::Context);
                if !needs_ctx {
                    comparison_eval::eval_cross_entity_comparison(comp, user, resource, interner)
                } else {
                    // Ok(v)  = a concrete AttributeValue (entity attr, parsed
                    //          number, or an already-interned request string).
                    // Err(s) = a request string that is not interned, so it
                    //          content-matches nothing already in the store.
                    let resolve = |etype: &EntityType,
                                   attr: crate::data::InternedString|
                     -> Option<Result<AttributeValue, Arc<str>>> {
                        if matches!(etype, EntityType::Context) {
                            let name = interner.resolve(attr)?;
                            // EvalContext::get special-cases "action"/"resource".
                            let raw: &str = _context.get(name.as_ref())?;
                            if let Ok(n) = raw.parse::<f64>() {
                                Some(Ok(AttributeValue::Float(n)))
                            } else if let Some(id) = interner.lookup(raw) {
                                Some(Ok(AttributeValue::String(id)))
                            } else {
                                Some(Err(Arc::from(raw)))
                            }
                        } else {
                            entity_helpers::get_nested_attr(etype, attr, user, resource, interner)
                                .map(Ok)
                        }
                    };
                    let op: AttrCompareOp = comp.op.into();
                    match (
                        resolve(&comp.left_entity, comp.left_attr),
                        resolve(&comp.right_entity, comp.right_attr),
                    ) {
                        // Both concrete: identical to the pre-existing comparator.
                        (Some(Ok(l)), Some(Ok(r))) => {
                            comparison_eval::compare_attr_values(Some(&l), Some(&r), &op)
                        }
                        // Two novel request strings: compare by content.
                        (Some(Err(a)), Some(Err(b))) => match op {
                            AttrCompareOp::Equal => a == b,
                            AttrCompareOp::NotEqual => a != b,
                            _ => false,
                        },
                        // A novel request string vs a resolved value. It was not
                        // interned, so it content-matches nothing in the store —
                        // unequal to everything. compare_attr_values only
                        // compares scalars (List/Set/Object/Null -> false), so
                        // mirror its type-strict NotEqual: != is true iff the
                        // other side is a present scalar.
                        (Some(Err(_)), Some(Ok(other))) | (Some(Ok(other)), Some(Err(_))) => {
                            match op {
                                AttrCompareOp::Equal => false,
                                AttrCompareOp::NotEqual => matches!(
                                    other,
                                    AttributeValue::String(_)
                                        | AttributeValue::Int(_)
                                        | AttributeValue::Float(_)
                                        | AttributeValue::Bool(_)
                                ),
                                _ => false,
                            }
                        }
                        // A Null / missing side satisfies neither == nor !=.
                        _ => false,
                    }
                }
            }

            CompiledCondition::WildcardCompare(comp) => {
                comparison_eval::eval_wildcard_comparison(comp, user, resource, interner)
            }

            CompiledCondition::RegexMatch(m) => {
                string_eval::eval_regex_match(m, user, resource, interner)
            }

            CompiledCondition::SameEntityAttrCompare {
                entity_type,
                left_attr,
                right_attr,
                op,
            } => comparison_eval::eval_same_entity_attr_compare(
                entity_type,
                *left_attr,
                *right_attr,
                op,
                user,
                resource,
                interner,
            ),

            CompiledCondition::Assignment {
                variable,
                entity_type,
                attribute,
                index,
            } => {
                // Use get_nested_attr to support nested attributes like "form_data.name"
                let value = if let Some(idx) = index {
                    let entity = match entity_type {
                        EntityType::User => user,
                        EntityType::Resource => resource,
                        EntityType::Context => return false,
                    };
                    collection_eval::get_indexed_value_compiled(entity, *attribute, idx, interner)
                } else {
                    entity_helpers::get_nested_attr(
                        entity_type,
                        *attribute,
                        user,
                        resource,
                        interner,
                    )
                };

                if let Some(val) = value {
                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    variables.insert(var_name, val);
                    true
                } else {
                    false
                }
            }

            CompiledCondition::MembershipTest {
                value,
                entity_type,
                attribute,
                index,
            } => collection_eval::eval_membership_test(
                value,
                entity_type,
                *attribute,
                index.as_ref(),
                user,
                resource,
                interner,
            ),

            CompiledCondition::IndexedEquals {
                entity_type,
                attribute,
                index,
                value,
            } => collection_eval::eval_indexed_equals(
                entity_type,
                *attribute,
                index,
                *value,
                user,
                resource,
                interner,
            ),

            CompiledCondition::EqualsVariable {
                entity_type,
                attribute,
                variable,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_val = entity.get_attribute(*attribute);
                if let Some(resolved) = interner.resolve(*variable) {
                    let var_val = variables.get(&*resolved);
                    match (attr_val, var_val) {
                        (Some(a), Some(v)) => a == v,
                        _ => false,
                    }
                } else {
                    false
                }
            }

            CompiledCondition::And(conditions) => {
                for (i, c) in conditions.iter().enumerate() {
                    let result =
                        self.evaluate_compiled_condition(c, user, resource, _context, variables);
                    if !result {
                        tracing::debug!(
                            condition_index = i,
                            condition_type = ?std::mem::discriminant(c),
                            condition_debug = ?c,
                            "AND sub-condition failed"
                        );
                        return false;
                    }
                }
                true
            }

            CompiledCondition::Or(conditions) => conditions
                .iter()
                .any(|c| self.evaluate_compiled_condition(c, user, resource, _context, variables)),

            CompiledCondition::Not(condition) => {
                !self.evaluate_compiled_condition(condition, user, resource, _context, variables)
            }

            // Old flat variants removed - now handled by V2 types above
            CompiledCondition::IsString {
                entity_type,
                attribute,
            } => collection_eval::eval_is_string(entity_type, *attribute, user, resource),

            CompiledCondition::IsNumber {
                entity_type,
                attribute,
            } => collection_eval::eval_is_number(entity_type, *attribute, user, resource),

            CompiledCondition::IsBool {
                entity_type,
                attribute,
            } => collection_eval::eval_is_bool(entity_type, *attribute, user, resource),

            CompiledCondition::SetIntersectionCountGreater {
                entity_type,
                attribute,
                values,
                threshold,
            } => collection_eval::eval_set_intersection_count_greater(
                entity_type,
                *attribute,
                values,
                *threshold,
                user,
                resource,
            ),

            CompiledCondition::MapKeyExists {
                entity_type,
                attribute,
                key,
            } => collection_eval::eval_map_key_exists_interned(
                entity_type,
                *attribute,
                key,
                user,
                resource,
            ),

            CompiledCondition::ComprehensionCountGreaterEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => comprehension_eval::eval_comprehension_count_gte(
                entity_type,
                *attribute,
                filter_attr,
                filter_value,
                filter_op,
                *threshold,
                user,
                resource,
                interner,
            ),

            CompiledCondition::ComprehensionCountEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => comprehension_eval::eval_comprehension_count_eq(
                entity_type,
                *attribute,
                filter_attr,
                filter_value,
                filter_op,
                *threshold,
                user,
                resource,
                interner,
            ),

            // ============ Expression Assignment ============
            CompiledCondition::ExpressionAssignment {
                variable,
                expr_type,
            } => {
                if let Some(value) = self
                    .evaluate_expr_type(expr_type, user, resource, _context, variables, interner)
                {
                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    variables.insert(var_name, value);
                    true
                } else {
                    false
                }
            }

            // ============ Expression Comparison Assignment ============
            // x := user.name.count() > 0
            CompiledCondition::ExprCompareAssignment {
                variable,
                expr_type,
                op,
                value,
            } => {
                if let Some(expr_value) = self
                    .evaluate_expr_type(expr_type, user, resource, _context, variables, interner)
                {
                    // Compare the expression result with the literal value
                    let result = match (&expr_value, value, op) {
                        // Integer comparisons
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::Equal,
                        ) => *i == *expected,
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::NotEqual,
                        ) => *i != *expected,
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::Greater,
                        ) => *i > *expected,
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::GreaterEqual,
                        ) => *i >= *expected,
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::Less,
                        ) => *i < *expected,
                        (
                            AttributeValue::Int(i),
                            CompiledLiteralValue::Int(expected),
                            AttrCompareOp::LessEqual,
                        ) => *i <= *expected,
                        // String comparisons
                        (
                            AttributeValue::String(s),
                            CompiledLiteralValue::String(expected),
                            AttrCompareOp::Equal,
                        ) => *s == *expected,
                        (
                            AttributeValue::String(s),
                            CompiledLiteralValue::String(expected),
                            AttrCompareOp::NotEqual,
                        ) => *s != *expected,
                        // Boolean comparisons
                        (
                            AttributeValue::Bool(b),
                            CompiledLiteralValue::Bool(expected),
                            AttrCompareOp::Equal,
                        ) => *b == *expected,
                        (
                            AttributeValue::Bool(b),
                            CompiledLiteralValue::Bool(expected),
                            AttrCompareOp::NotEqual,
                        ) => *b != *expected,
                        _ => false,
                    };

                    // Store the boolean result in the variable
                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    variables.insert(var_name, AttributeValue::Bool(result));
                    true
                } else {
                    false
                }
            }

            // ============ Variable Comparisons ============
            CompiledCondition::VariableEqualsLiteral { variable, value } => {
                variable_eval::eval_variable_equals_literal(*variable, value, variables, interner)
            }

            CompiledCondition::VariableNotEqualsLiteral { variable, value } => {
                variable_eval::eval_variable_not_equals_literal(
                    *variable, value, variables, interner,
                )
            }

            CompiledCondition::VariableCompare {
                variable,
                op,
                value,
            } => variable_eval::eval_variable_compare(*variable, op, value, variables, interner),

            CompiledCondition::VariableIsNull { variable } => {
                variable_eval::eval_variable_is_null(*variable, variables, interner)
            }

            CompiledCondition::VariableIsNotNull { variable } => {
                variable_eval::eval_variable_is_not_null(*variable, variables, interner)
            }

            // Comparison assignment: stores boolean result of comparison in variable
            CompiledCondition::ComparisonAssignment {
                variable,
                entity_type,
                attribute,
                op,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_opt = entity.get_attribute(*attribute);

                // Evaluate comparison based on value type
                let result = match value {
                    CompiledLiteralValue::String(lit_str) => {
                        // String comparison
                        if let Some(AttributeValue::String(attr_str)) = attr_opt {
                            let attr_resolved = interner.resolve(*attr_str).unwrap_or_default();
                            let lit_resolved = interner.resolve(*lit_str).unwrap_or_default();
                            match op {
                                AttrCompareOp::Equal => *attr_resolved == *lit_resolved,
                                AttrCompareOp::NotEqual => *attr_resolved != *lit_resolved,
                                // String comparisons for ordering use lexicographic order
                                AttrCompareOp::Greater => *attr_resolved > *lit_resolved,
                                AttrCompareOp::GreaterEqual => *attr_resolved >= *lit_resolved,
                                AttrCompareOp::Less => *attr_resolved < *lit_resolved,
                                AttrCompareOp::LessEqual => *attr_resolved <= *lit_resolved,
                            }
                        } else {
                            false
                        }
                    }
                    CompiledLiteralValue::Int(lit_int) => {
                        // Numeric comparison (integer)
                        let attr_num: Option<f64> = match attr_opt {
                            Some(AttributeValue::Int(i)) => Some(*i as f64),
                            Some(AttributeValue::Float(f)) => Some(*f),
                            _ => None,
                        };
                        if let Some(attr_val) = attr_num {
                            let lit_val = *lit_int as f64;
                            match op {
                                AttrCompareOp::GreaterEqual => attr_val >= lit_val,
                                AttrCompareOp::Greater => attr_val > lit_val,
                                AttrCompareOp::LessEqual => attr_val <= lit_val,
                                AttrCompareOp::Less => attr_val < lit_val,
                                AttrCompareOp::Equal => (attr_val - lit_val).abs() < f64::EPSILON,
                                AttrCompareOp::NotEqual => {
                                    (attr_val - lit_val).abs() >= f64::EPSILON
                                }
                            }
                        } else {
                            false
                        }
                    }
                    CompiledLiteralValue::Bool(lit_bool) => {
                        // Boolean comparison
                        if let Some(AttributeValue::Bool(attr_bool)) = attr_opt {
                            match op {
                                AttrCompareOp::Equal => *attr_bool == *lit_bool,
                                AttrCompareOp::NotEqual => *attr_bool != *lit_bool,
                                _ => false, // Other ops don't make sense for booleans
                            }
                        } else {
                            false
                        }
                    }
                };

                if let Some(var_name) = interner.resolve(*variable) {
                    variables.insert(var_name.to_string(), AttributeValue::Bool(result));
                }
                true // Assignment always succeeds (stores the boolean)
            }

            // Null comparison assignment: x := user.field != null
            // Also handles nested attributes like user.config.name != null
            CompiledCondition::NullComparisonAssignment {
                variable,
                entity_type,
                attribute,
                is_null_check,
            } => {
                // Check if the attribute is null (handles nested attributes like "config.name")
                let attr_is_null = entity_helpers::is_nested_attr_null(
                    entity_type,
                    *attribute,
                    user,
                    resource,
                    interner,
                );

                // Result depends on whether we're checking == null or != null
                let result = if *is_null_check {
                    attr_is_null // x := field == null -> true if null
                } else {
                    !attr_is_null // x := field != null -> true if not null
                };

                if let Some(var_name) = interner.resolve(*variable) {
                    variables.insert(var_name.to_string(), AttributeValue::Bool(result));
                }
                true // Assignment always succeeds
            }

            // Membership test against variable: "value" in var
            CompiledCondition::VariableMembershipTest { value, variable } => {
                variable_eval::eval_variable_membership_test(value, *variable, variables, interner)
            }

            // Variable type checks
            CompiledCondition::VariableIsString { variable } => {
                variable_eval::eval_variable_is_string(*variable, variables, interner)
            }

            CompiledCondition::VariableIsNumber { variable } => {
                variable_eval::eval_variable_is_number(*variable, variables, interner)
            }

            CompiledCondition::VariableIsBool { variable } => {
                variable_eval::eval_variable_is_bool(*variable, variables, interner)
            }

            // Variable as standalone condition: truthy check
            CompiledCondition::VariableIsTruthy { variable } => {
                variable_eval::eval_variable_is_truthy(*variable, variables, interner)
            }

            // Variable equals variable
            CompiledCondition::VariableEqualsVariable { left, right } => {
                variable_eval::eval_variable_equals_variable(*left, *right, variables, interner)
            }

            // Variable not equals variable
            CompiledCondition::VariableNotEqualsVariable { left, right } => {
                variable_eval::eval_variable_not_equals_variable(*left, *right, variables, interner)
            }

            // Variable method with literal array
            CompiledCondition::VariableMethodWithLiteralArray {
                variable,
                method,
                values,
            } => variable_eval::eval_variable_method_with_literal_array(
                *variable, method, values, variables, interner,
            ),

            CompiledCondition::VariableMethodCompare {
                variable,
                method,
                op,
                value,
            } => variable_eval::eval_variable_method_compare(
                *variable, method, op, value, variables, interner,
            ),

            // Chained variable method comparison: t.trim().count() > 0
            CompiledCondition::VariableChainedMethodCompare {
                variable,
                transform_method,
                compare_method,
                op,
                value,
            } => variable_eval::eval_variable_chained_method_compare(
                *variable,
                transform_method,
                compare_method,
                op,
                value,
                variables,
                interner,
            ),

            // ============ Variable Attribute Comparisons (for comprehension filters) ============
            CompiledCondition::VariableAttrEqualsLiteral {
                variable,
                attribute,
                value,
            } => variable_eval::eval_variable_attr_equals_literal(
                *variable, *attribute, value, variables, interner,
            ),

            CompiledCondition::VariableAttrNotEqualsLiteral {
                variable,
                attribute,
                value,
            } => variable_eval::eval_variable_attr_not_equals_literal(
                *variable, *attribute, value, variables, interner,
            ),

            CompiledCondition::VariableAttrCompare {
                variable,
                attribute,
                op,
                value,
            } => variable_eval::eval_variable_attr_compare(
                *variable, *attribute, op, value, variables, interner,
            ),

            CompiledCondition::VariableAttrEqualsNull {
                variable,
                attribute,
            } => variable_eval::eval_variable_attr_equals_null(
                *variable, *attribute, variables, interner,
            ),

            CompiledCondition::VariableAttrNotEqualsNull {
                variable,
                attribute,
            } => variable_eval::eval_variable_attr_not_equals_null(
                *variable, *attribute, variables, interner,
            ),

            // Variable attribute null comparison assignment: x := var.attr != null
            CompiledCondition::VarAttrNullCompareAssignment {
                result_variable,
                source_variable,
                attribute,
                is_null_check,
            } => {
                // Evaluate the null check
                let is_null = if *is_null_check {
                    variable_eval::eval_variable_attr_equals_null(
                        *source_variable,
                        *attribute,
                        variables,
                        interner,
                    )
                } else {
                    variable_eval::eval_variable_attr_not_equals_null(
                        *source_variable,
                        *attribute,
                        variables,
                        interner,
                    )
                };
                // Assign the result to the result variable
                if let Some(var_name) = interner.resolve(*result_variable) {
                    variables.insert(var_name.to_string(), AttributeValue::Bool(is_null));
                }
                true // Assignment always succeeds
            }

            // Variable attribute contains: d.permissions.contains("execute")
            CompiledCondition::VariableAttrContains {
                variable,
                attribute,
                substring,
            } => variable_eval::eval_variable_attr_contains(
                *variable, *attribute, *substring, variables, interner,
            ),

            // Context entity comparisons are now handled by AttributeCompare V2 type

            // ============ Comprehension Assignment ============
            CompiledCondition::ComprehensionAssignment {
                variable,
                comprehension,
            } => {
                if let Some(result) = self.evaluate_comprehension(
                    comprehension,
                    user,
                    resource,
                    _context,
                    variables,
                    interner,
                ) {
                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    variables.insert(var_name, result);
                    true
                } else {
                    false
                }
            }
        }
    }

    // ============ Helper Methods ============

    /// Evaluate context entity attribute comparison
    /// Context is special - it comes from the request context HashMap, not an Entity
    #[inline]
    fn eval_context_attribute_comparison(
        &self,
        comp: &CompiledAttributeComparison,
        context: &EvalContext<'_>,
        interner: &crate::data::StringInterner,
    ) -> bool {
        // Get the attribute name from the interner
        let attr_name = match interner.resolve(comp.attribute) {
            Some(name) => name,
            None => return false,
        };

        // Null checks compare PRESENCE (absent context key == null), so they
        // must be handled before requiring a value — previously they fell into
        // the catch-all and always returned false (`context.ticket != null`
        // denied on the fast path while the AST evaluator allowed; caught by
        // the policy-library parity suite).
        let ctx_val_opt = context.get(&attr_name);
        if matches!(&comp.target, CompiledCompareTarget::LiteralNull) {
            let is_null = ctx_val_opt.is_none();
            return match comp.op {
                NumericOp::Equal => is_null,
                NumericOp::NotEqual => !is_null,
                _ => false,
            };
        }

        // Get the context value
        let ctx_val = match ctx_val_opt {
            Some(v) => v,
            None => return false,
        };

        // Compare based on target type
        match &comp.target {
            CompiledCompareTarget::LiteralString(expected) => {
                if let Some(expected_str) = interner.resolve(*expected) {
                    match comp.op {
                        NumericOp::Equal => ctx_val == &*expected_str,
                        NumericOp::NotEqual => ctx_val != &*expected_str,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            CompiledCompareTarget::LiteralNum(threshold) => {
                if let Ok(num) = ctx_val.parse::<f64>() {
                    match comp.op {
                        NumericOp::Equal => (num - threshold).abs() < f64::EPSILON,
                        NumericOp::NotEqual => (num - threshold).abs() >= f64::EPSILON,
                        NumericOp::Greater => num > *threshold,
                        NumericOp::GreaterEqual => num >= *threshold,
                        NumericOp::Less => num < *threshold,
                        NumericOp::LessEqual => num <= *threshold,
                    }
                } else {
                    false
                }
            }
            CompiledCompareTarget::LiteralBool(expected) => {
                let expected_str = if *expected { "true" } else { "false" };
                match comp.op {
                    NumericOp::Equal => ctx_val == expected_str,
                    NumericOp::NotEqual => ctx_val != expected_str,
                    _ => false,
                }
            }
            _ => false, // EntityAttr and Variable not supported for context
        }
    }

    /// Evaluate an expression type and return the result value
    /// Evaluate an expression type and return the result.
    /// Delegates to the standalone function in expr_eval module.
    fn evaluate_expr_type(
        &self,
        expr_type: &CompiledExprType,
        user: &Entity,
        resource: &Entity,
        _context: &EvalContext<'_>,
        variables: &std::collections::HashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        expr_eval::evaluate_compiled_expr_type(expr_type, user, resource, variables, interner)
    }

    /// Evaluate a comprehension and return the resulting collection
    fn evaluate_comprehension(
        &self,
        comp: &CompiledComprehension,
        user: &Entity,
        resource: &Entity,
        context: &EvalContext<'_>,
        variables: &std::collections::HashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        // Get the source collection
        let source_items = match &comp.iterator.source {
            CompiledIterationSource::EntityAttr {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return None,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(items)) => items.clone(),
                    Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                    // TOTAL ITERATION (matches the AST contract): a missing
                    // or non-collection source is an EMPTY collection, so
                    // the comprehension yields empty and the assignment
                    // still binds — returning None here made the rule fail
                    // where the AST evaluator continued with an empty list.
                    _ => Vec::new(),
                }
            }
            CompiledIterationSource::Variable { variable } => {
                if let Some(var_name) = interner.resolve(*variable) {
                    if let Some(attr_val) = variables.get(&*var_name) {
                        match attr_val {
                            AttributeValue::List(items) => items.clone(),
                            AttributeValue::Set(items) => items.iter().cloned().collect(),
                            // Total iteration: non-collection = empty.
                            _ => Vec::new(),
                        }
                    } else {
                        // Total iteration: unbound variable = empty.
                        Vec::new()
                    }
                } else {
                    return None;
                }
            }
        };

        // Get the iterator variable name
        let iter_var_name = interner
            .resolve(comp.iterator.variable)
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Handle object comprehensions separately to avoid double-consuming source_items
        if matches!(comp.comp_type, ComprehensionType::Object) {
            // Object comprehensions: collect key-value pairs from key_value
            if let Some((key_output, value_output)) = &comp.key_value {
                let mut object_result: Vec<(InternedString, AttributeValue)> = Vec::new();
                // Clone once, reuse across iterations (saves N-1 clones)
                let mut local_vars = variables.clone();
                let snapshot_keys: Vec<String> = local_vars.keys().cloned().collect();
                for item in source_items {
                    // Reset to snapshot: remove keys added by previous iteration
                    local_vars.retain(|k, _| snapshot_keys.contains(k));
                    local_vars.insert(iter_var_name.clone(), item.clone());

                    // Evaluate filters
                    let passes = self.evaluate_object_comprehension_filters(
                        &comp.filters,
                        user,
                        resource,
                        context,
                        &mut local_vars,
                        interner,
                    );

                    if passes {
                        // Get key and value
                        let value_opt = Some(value_output.clone());
                        if let (Some(key), Some(value)) = (
                            self.get_comprehension_output_as_string(
                                key_output,
                                &local_vars,
                                interner,
                            ),
                            self.get_comprehension_output(&value_opt, &local_vars, interner),
                        ) {
                            object_result.push((key, value));
                        }
                    }
                }

                // Convert to HashMap (std HashMap, not FxHashMap)
                let map: std::collections::HashMap<InternedString, AttributeValue> =
                    object_result.into_iter().collect();
                return Some(AttributeValue::Object(map));
            } else {
                return None;
            }
        }

        // Filter and collect items with nested iteration support (for Array/Set)
        // Clone once, reuse across iterations (saves N-1 full HashMap clones)
        let mut result = Vec::new();
        let mut local_vars = variables.clone();
        let snapshot_keys: Vec<String> = local_vars.keys().cloned().collect();
        for item in source_items {
            // Reset to snapshot: remove keys added by previous iteration's filters
            local_vars.retain(|k, _| snapshot_keys.contains(k));
            // Restore original values for snapshot keys that may have been modified
            for key in &snapshot_keys {
                if let Some(original) = variables.get(key) {
                    local_vars.insert(key.clone(), original.clone());
                }
            }
            local_vars.insert(iter_var_name.clone(), item.clone());

            // Recursively evaluate filters with nested iteration support
            self.evaluate_comprehension_filters_recursive(
                &comp.filters,
                user,
                resource,
                context,
                &mut local_vars,
                &comp.output,
                interner,
                &mut result,
            );
        }

        match comp.comp_type {
            ComprehensionType::Array => Some(AttributeValue::List(result)),
            ComprehensionType::Set => {
                let set: rustc_hash::FxHashSet<AttributeValue> = result.into_iter().collect();
                Some(AttributeValue::Set(set))
            }
            ComprehensionType::Object => unreachable!(), // Handled above
        }
    }

    /// Evaluate filters for object comprehension (returns bool indicating pass/fail)
    fn evaluate_object_comprehension_filters(
        &self,
        filters: &[CompiledCondition],
        user: &Entity,
        resource: &Entity,
        context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        _interner: &crate::data::StringInterner,
    ) -> bool {
        for filter in filters {
            if !self.evaluate_compiled_condition(filter, user, resource, context, variables) {
                return false;
            }
        }
        true
    }

    /// Get comprehension output as an interned string (for object keys)
    fn get_comprehension_output_as_string(
        &self,
        output: &CompiledOutput,
        variables: &std::collections::HashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) -> Option<InternedString> {
        comprehension_eval::get_comprehension_output_as_string(output, variables, interner)
    }

    /// Recursively evaluate comprehension filters, handling nested iteration.
    /// When a filter is an assignment with VariableIndexed + Wildcard, iterate over elements.
    #[allow(clippy::too_many_arguments)]
    fn evaluate_comprehension_filters_recursive(
        &self,
        filters: &[CompiledCondition],
        user: &Entity,
        resource: &Entity,
        context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        output: &Option<CompiledOutput>,
        interner: &crate::data::StringInterner,
        result: &mut Vec<AttributeValue>,
    ) {
        if filters.is_empty() {
            // All filters passed, collect output
            if let Some(val) = self.get_comprehension_output(output, variables, interner) {
                result.push(val);
            }
            return;
        }

        let filter = &filters[0];
        let remaining = &filters[1..];

        // Check if this filter is a nested iteration assignment (val := row[_])
        if let CompiledCondition::ExpressionAssignment {
            variable,
            expr_type:
                CompiledExprType::VariableIndexed {
                    variable: src_var,
                    index: CompiledExprIndexType::Wildcard,
                },
        } = filter
        {
            // This is a nested iteration: val := row[_]
            if let Some(src_name) = interner.resolve(*src_var) {
                if let Some(src_val) = variables.get(&*src_name).cloned() {
                    // Get elements from the source collection
                    let elements: Vec<AttributeValue> = match src_val {
                        AttributeValue::List(items) => items,
                        AttributeValue::Set(items) => items.into_iter().collect(),
                        _ => return, // Not a collection, skip
                    };

                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    // Iterate over each element (nested iteration)
                    // Clone once and reuse across iterations
                    let mut inner_vars = variables.clone();
                    let inner_snapshot_keys: Vec<String> = inner_vars.keys().cloned().collect();
                    for element in elements {
                        // Reset to snapshot
                        inner_vars.retain(|k, _| inner_snapshot_keys.contains(k));
                        for key in &inner_snapshot_keys {
                            if let Some(original) = variables.get(key) {
                                inner_vars.insert(key.clone(), original.clone());
                            }
                        }
                        inner_vars.insert(var_name.clone(), element);
                        self.evaluate_comprehension_filters_recursive(
                            remaining,
                            user,
                            resource,
                            context,
                            &mut inner_vars,
                            output,
                            interner,
                            result,
                        );
                    }
                    return;
                }
            }
        }

        // Regular filter - evaluate and continue if it passes
        let passes = self.evaluate_compiled_condition_with_vars(
            filter,
            user,
            resource,
            context,
            &mut variables.clone(),
            interner,
        );
        if passes {
            // Update variables if this was an assignment filter
            self.apply_filter_variable_update(filter, variables, user, resource, context, interner);
            self.evaluate_comprehension_filters_recursive(
                remaining, user, resource, context, variables, output, interner, result,
            );
        }
    }

    /// Get the output value from a comprehension
    fn get_comprehension_output(
        &self,
        output: &Option<CompiledOutput>,
        variables: &std::collections::HashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        comprehension_eval::get_comprehension_output(output, variables, interner)
    }

    /// Apply variable updates from assignment filters
    fn apply_filter_variable_update(
        &self,
        filter: &CompiledCondition,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        user: &Entity,
        resource: &Entity,
        context: &EvalContext<'_>,
        interner: &crate::data::StringInterner,
    ) {
        match filter {
            CompiledCondition::ExpressionAssignment {
                variable,
                expr_type,
            } => {
                if let Some(var_name) = interner.resolve(*variable) {
                    if let Some(val) = self
                        .evaluate_expr_type(expr_type, user, resource, context, variables, interner)
                    {
                        variables.insert(var_name.to_string(), val);
                    }
                }
            }
            _ => {
                // Other filter types don't update variables directly
            }
        }
    }

    /// Helper for comprehension filter evaluation with local variables
    fn evaluate_compiled_condition_with_vars(
        &self,
        condition: &CompiledCondition,
        user: &Entity,
        resource: &Entity,
        context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        _interner: &crate::data::StringInterner,
    ) -> bool {
        // For comprehension filters, we need to handle VarAttr comparisons
        // For now, delegate to the regular evaluation
        self.evaluate_compiled_condition(condition, user, resource, context, variables)
    }
}

impl PolicyEvaluator for ReaperDSLEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, reaper_core::ReaperError> {
        let interner = self.store.interner();

        // Parse entity IDs from request
        // In production, these would be passed directly as InternedString
        let user_id = interner.intern(request.context.get("principal").ok_or_else(|| {
            reaper_core::ReaperError::EvaluationError {
                reason: "Missing principal in context".to_string(),
            }
        })?);

        let resource_id = interner.intern(&request.resource);

        // Fast DataStore lookups (~20-50ns each)
        let user =
            self.store
                .get(user_id)
                .ok_or_else(|| reaper_core::ReaperError::EvaluationError {
                    reason: format!("User entity not found: {:?}", user_id),
                })?;

        // Log user entity info at trace level
        #[cfg(debug_assertions)]
        {
            let user_attr_names: Vec<String> = user
                .attributes
                .keys()
                .filter_map(|k| interner.resolve(*k).map(|s| s.to_string()))
                .collect();
            tracing::trace!(
                user_id = ?user_id,
                user_attrs = ?user_attr_names,
                "User entity found"
            );
        }

        // Resource lookup. If the resource isn't a registered entity we still
        // want simple `resource == "value"` checks to work, so we synthesize a
        // minimal entity holding just the id. Crucially this is a STACK local
        // (borrowed below), not an `Arc::new(Entity)` — the previous code
        // heap-allocated an Arc + Entity on every request for any policy whose
        // resource is a bare id (e.g. RBAC over URL paths).
        let resource_found = self.store.get(resource_id);
        let temp_resource;
        let resource: &Entity = match &resource_found {
            Some(entity) => entity,
            None => {
                temp_resource = Entity::new(
                    resource_id,
                    self.resource_type_id,
                    std::collections::HashMap::new(),
                );
                &temp_resource
            }
        };

        // Log resource entity info at trace level
        #[cfg(debug_assertions)]
        {
            let resource_attr_names: Vec<String> = resource
                .attributes
                .keys()
                .filter_map(|k| interner.resolve(*k).map(|s| s.to_string()))
                .collect();
            tracing::trace!(
                resource_id = ?resource_id,
                resource_found = resource_found.is_some(),
                resource_attrs = ?resource_attr_names,
                "Resource entity lookup"
            );
        }

        // Zero-copy evaluation context — avoids HashMap clone + 2 String allocations per call
        let eval_context = EvalContext::new(&request.action, &request.resource, &request.context);

        // Variable context for local bindings (scoped to policy evaluation)
        // Performance: no allocation until first variable use (most policies have zero variables)
        let mut variables = std::collections::HashMap::new();

        // Security-first evaluation: Deny rules ALWAYS take precedence over Allow rules
        // Rules are pre-partitioned at construction time for optimal performance

        // Phase 1: Evaluate all DENY rules first (pre-partitioned, no type checking needed)
        // Using compiled conditions with pre-interned strings - zero HashMap lookups!
        for rule in &self.compiled_deny_rules {
            if self.evaluate_compiled_condition(
                &rule.condition,
                &user,
                resource,
                &eval_context,
                &mut variables,
            ) {
                // Explicit deny - return immediately, no allow can override this
                return Ok(PolicyAction::Deny);
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 2: No deny matched, evaluate ALLOW rules (pre-partitioned, no type checking needed)
        for rule in &self.compiled_allow_rules {
            let matches = self.evaluate_compiled_condition(
                &rule.condition,
                &user,
                resource,
                &eval_context,
                &mut variables,
            );

            if matches {
                tracing::trace!(
                    rule_name = %rule.name,
                    action = %request.action,
                    "Rule matched - returning Allow"
                );
                return Ok(PolicyAction::Allow);
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 3: No rule matched - return default decision
        tracing::debug!(
            default_decision = ?self.default_decision,
            "No rules matched - returning default"
        );
        Ok(self.default_decision.clone())
    }

    fn validate(&self) -> Result<(), reaper_core::ReaperError> {
        // Validation happens at construction time
        // Check we have at least one rule
        if self.compiled_deny_rules.is_empty() && self.compiled_allow_rules.is_empty() {
            return Err(reaper_core::ReaperError::InvalidPolicy {
                reason: "Policy must have at least one rule".to_string(),
            });
        }
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "reaper_dsl"
    }

    fn metadata(&self) -> Option<EvaluatorMetadata> {
        let mut extra = std::collections::HashMap::new();
        extra.insert(
            "estimated_complexity".to_string(),
            "O(n) where n = number of rules".to_string(),
        );
        extra.insert("supports_streaming".to_string(), "false".to_string());
        Some(EvaluatorMetadata {
            rule_count: self.compiled_deny_rules.len() + self.compiled_allow_rules.len(),
            complexity: 50, // Medium complexity
            extra,
        })
    }
}
