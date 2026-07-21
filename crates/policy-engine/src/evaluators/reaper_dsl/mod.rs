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
mod input_eval;
mod string_eval;
#[cfg(test)]
mod tests;
mod time_eval;
mod types;
mod variable_eval;

// Re-export types for external use
pub use types::*;

// Tier-2 partial-evaluation dry-run instruments (R3 Plan 06 F.3): public so
// offline fitness tooling (examples/specialization_fitness.rs) can measure
// corpora without reaching into private modules.
pub use compiler::{
    leaf_staticness, specialization_fitness, LeafStaticness, SpecializationFitness,
};

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::data::{AttributeValue, DataStore, Entity, InternedString, StringInterner};
use crate::{PolicyAction, PolicyRequest};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::sync::Arc;

// ===========================================================================
// Per-evaluation transient-string reclamation
// ===========================================================================
//
// String-producing methods (lower/upper/trim/split/replace/find/find_all and
// comprehension transforms) compute NEW strings at eval time and must intern
// them to represent them as `AttributeValue::String(id)` (the compiled value
// model is interned-only). Those results live only for the one evaluation — but
// a plain `intern()` PINS them, so on the hot path a policy producing
// high-cardinality results (regex captures, per-request transforms) would grow
// the shared interner without bound.
//
// The fix: within an evaluation, produce results via `intern_transient`, which
// interns them *counted* and records the ids in a per-thread scratch frame. A
// `ScratchGuard` in `evaluate()` releases every recorded id when the evaluation
// unwinds (including on panic), so novel results are evicted while strings that
// are also owned by an entity (or pinned as a policy literal) are untouched
// (release is a no-op / a balanced decrement on those). Outside an evaluation
// (e.g. a unit test calling an eval helper directly) there is no scope to
// reclaim into, so `intern_transient` falls back to a plain pinned `intern`.

#[derive(Default)]
struct ScratchFrame {
    /// Active `evaluate()` nesting depth (re-entrancy guard).
    depth: u32,
    /// Counted ids interned this evaluation, released when depth returns to 0.
    ids: Vec<InternedString>,
}

thread_local! {
    static EVAL_SCRATCH: RefCell<ScratchFrame> = RefCell::new(ScratchFrame::default());
}

/// Intern a freshly-computed eval result. Counted + recorded inside an
/// evaluation (reclaimed at eval end); pinned if called with no active scope.
pub(crate) fn intern_transient(interner: &StringInterner, s: &str) -> InternedString {
    EVAL_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        if scratch.depth == 0 {
            interner.intern(s)
        } else {
            let id = interner.intern_counted(s);
            scratch.ids.push(id);
            id
        }
    })
}

/// RAII scope for one `evaluate()` call: bumps the scratch depth on entry and,
/// when the outermost scope exits, releases every transient id interned during
/// the evaluation. Drop runs on panic too, so a failing eval cannot leak.
struct ScratchGuard<'a> {
    interner: &'a StringInterner,
}

impl<'a> ScratchGuard<'a> {
    fn enter(interner: &'a StringInterner) -> Self {
        EVAL_SCRATCH.with(|s| s.borrow_mut().depth += 1);
        Self { interner }
    }
}

impl Drop for ScratchGuard<'_> {
    fn drop(&mut self) {
        EVAL_SCRATCH.with(|scratch| {
            // Take the ids to release without holding the borrow across
            // `release` (release never re-enters the scratch, but keep the
            // critical section minimal and re-entrancy-proof).
            let to_release = {
                let mut scratch = scratch.borrow_mut();
                scratch.depth -= 1;
                if scratch.depth == 0 {
                    std::mem::take(&mut scratch.ids)
                } else {
                    Vec::new()
                }
            };
            for id in to_release {
                self.interner.release(id);
            }
        });
    }
}

/// Truthiness of a collection element for `any()` / `all()`, matching the AST
/// `builtin_methods::method_any` / `method_all`: false/0/null/empty-string are
/// falsy; everything else (incl. floats and nested collections) is truthy.
#[inline]
fn is_truthy(v: &AttributeValue, interner: &crate::data::StringInterner) -> bool {
    match v {
        AttributeValue::Bool(b) => *b,
        AttributeValue::Int(i) => *i != 0,
        AttributeValue::Null => false,
        AttributeValue::String(id) => interner
            .resolve(*id)
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        _ => true,
    }
}

/// Zero-copy evaluation context — avoids HashMap clone per evaluation.
/// "action" and "resource" are served from borrowed fields;
/// all other keys fall through to the original request context.
pub(crate) struct EvalContext<'a> {
    action: &'a str,
    resource: &'a str,
    context: &'a std::collections::HashMap<String, String>,
    /// The request's structured `input` document (R4-01 B.1) — raw JSON,
    /// navigated by pre-parsed `InputPath`s. `None` on the ordinary
    /// entity-request path; `Some` only via the `*_with_input` entries.
    input: Option<&'a serde_json::Value>,
}

impl<'a> EvalContext<'a> {
    #[inline]
    fn new(
        action: &'a str,
        resource: &'a str,
        context: &'a std::collections::HashMap<String, String>,
        input: Option<&'a serde_json::Value>,
    ) -> Self {
        Self {
            action,
            resource,
            context,
            input,
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
    /// Interned "actor" type id, precomputed for the synthetic-actor path
    /// (an actor id that is not a loaded entity — F1 agentic authz).
    actor_type_id: crate::data::InternedString,
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
            // Tier-1 partial evaluation (round-3 Plan 06 F): constant-fold the
            // compiled condition once at deploy so the per-request loop never
            // re-derives statically-known truth. Purely structural — the
            // decision, matched-flag, rule name, and variable-binding side
            // effects are preserved (see `compiler::fold_condition`), which the
            // compiled-vs-AST differential pins.
            let compiled = CompiledRule {
                name: rule.name,
                condition: compiler::fold_condition(compiler::compile_condition(
                    &rule.condition,
                    interner,
                )),
                decision: rule.decision.clone(),
            };

            match rule.decision {
                PolicyAction::Deny => compiled_deny_rules.push(compiled),
                PolicyAction::Allow | PolicyAction::Log => compiled_allow_rules.push(compiled),
            }
        }

        let resource_type_id = interner.intern("resource");
        let actor_type_id = interner.intern("actor");

        Self {
            store,
            compiled_deny_rules,
            compiled_allow_rules,
            default_decision,
            resource_type_id,
            actor_type_id,
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
        bindings: EntityBindings<'_>,
        _context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
    ) -> bool {
        let interner = self.store.interner();

        match condition {
            CompiledCondition::Always => true,

            // taint::trusted("key") — true iff the key's request provenance
            // is >= verified under the fail-untrusted rule (taint mode off ⇒
            // platform; unlabeled key under taint mode ⇒ llm). One HashMap
            // get on the request's provenance map; no interner involved.
            CompiledCondition::TaintTrusted { key } => {
                bindings.context_trust(key) >= crate::TrustLevel::Verified
            }

            // input.<path> <op> <literal> (R4-01 B.1): pre-parsed path walked
            // over the request's raw JSON document; truth table mirrors the
            // interpreter leaf-for-leaf (see input_eval).
            CompiledCondition::InputCompare { path, op, target } => {
                input_eval::eval_input_compare(path, op, target, _context.input)
            }

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
                    CompiledRebacRef::Principal => Some(bindings.user.id),
                    CompiledRebacRef::ResourceId => Some(bindings.resource.id),
                    CompiledRebacRef::Literal(id) => Some(*id),
                    // Absent actor ⇒ the check cannot hold (fail closed); a
                    // present actor resolves to its (possibly synthesized)
                    // entity id, so a relation may name an actor id that is
                    // not itself a loaded entity — same as the AST evaluator.
                    CompiledRebacRef::Actor => bindings.actor.map(|a| a.id),
                };
                let (Some(subject_id), Some(object_id)) = (resolve(subject), resolve(object))
                else {
                    return false;
                };
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
                    comparison_eval::eval_attribute_comparison(comp, bindings, interner)
                }
            }

            CompiledCondition::StringOp(op) => {
                string_eval::eval_string_operation(op, bindings, interner)
            }

            CompiledCondition::VariableStringOp(op) => {
                string_eval::eval_variable_string_operation(op, variables, interner)
            }

            CompiledCondition::CountOp(cond) => {
                collection_eval::eval_count_operation(cond, bindings)
            }

            CompiledCondition::TimeOp(cond) => time_eval::eval_time_operation(cond, bindings),

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
                    comparison_eval::eval_cross_entity_comparison(comp, bindings, interner)
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
                            entity_helpers::get_nested_attr(etype, attr, bindings, interner).map(Ok)
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
                comparison_eval::eval_wildcard_comparison(comp, bindings, interner)
            }

            CompiledCondition::RegexMatch(m) => {
                string_eval::eval_regex_match(m, bindings, interner)
            }

            CompiledCondition::ObjectHasKey {
                entity_type,
                attribute,
                key,
            } => {
                let entity = match entity_type {
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return false,
                    // Absent actor: fall through to the missing-attribute
                    // path — the AST reads actor.* as Null when no actor is
                    // bound, and missing-attr semantics are identical.
                    EntityType::Actor => bindings.actor,
                };
                matches!(
                    entity.and_then(|e| e.get_attribute(*attribute)),
                    Some(AttributeValue::Object(map)) if map.contains_key(key)
                )
            }

            CompiledCondition::CollectionAny {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return false,
                    // Absent actor: fall through to the missing-attribute
                    // path — the AST reads actor.* as Null when no actor is
                    // bound, and missing-attr semantics are identical.
                    EntityType::Actor => bindings.actor,
                };
                match entity.and_then(|e| e.get_attribute(*attribute)) {
                    Some(AttributeValue::List(items)) => {
                        items.iter().any(|v| is_truthy(v, interner))
                    }
                    Some(AttributeValue::Set(items)) => {
                        items.iter().any(|v| is_truthy(v, interner))
                    }
                    _ => false,
                }
            }

            CompiledCondition::CollectionAll {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return false,
                    // Absent actor: fall through to the missing-attribute
                    // path — the AST reads actor.* as Null when no actor is
                    // bound, and missing-attr semantics are identical.
                    EntityType::Actor => bindings.actor,
                };
                match entity.and_then(|e| e.get_attribute(*attribute)) {
                    // Vacuously true on an empty collection, matching AST all().
                    Some(AttributeValue::List(items)) => {
                        items.iter().all(|v| is_truthy(v, interner))
                    }
                    Some(AttributeValue::Set(items)) => {
                        items.iter().all(|v| is_truthy(v, interner))
                    }
                    _ => false,
                }
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
                bindings,
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
                        EntityType::User => Some(bindings.user),
                        EntityType::Resource => Some(bindings.resource),
                        EntityType::Context => return false,
                        // Absent actor: no value, same as a missing attribute.
                        EntityType::Actor => bindings.actor,
                    };
                    entity.and_then(|e| {
                        collection_eval::get_indexed_value_compiled(e, *attribute, idx, interner)
                    })
                } else {
                    entity_helpers::get_nested_attr(entity_type, *attribute, bindings, interner)
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
                bindings,
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
                bindings,
                interner,
            ),

            CompiledCondition::EqualsVariable {
                entity_type,
                attribute,
                variable,
            } => {
                let entity = match entity_type {
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return false,
                    // Absent actor: fall through to the missing-attribute
                    // path — the AST reads actor.* as Null when no actor is
                    // bound, and missing-attr semantics are identical.
                    EntityType::Actor => bindings.actor,
                };

                let attr_val = entity.and_then(|e| e.get_attribute(*attribute));
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
                    let result = self.evaluate_compiled_condition(c, bindings, _context, variables);
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
                .any(|c| self.evaluate_compiled_condition(c, bindings, _context, variables)),

            CompiledCondition::Not(condition) => {
                !self.evaluate_compiled_condition(condition, bindings, _context, variables)
            }

            // Old flat variants removed - now handled by V2 types above
            CompiledCondition::IsString {
                entity_type,
                attribute,
            } => collection_eval::eval_is_string(entity_type, *attribute, bindings),

            CompiledCondition::IsNumber {
                entity_type,
                attribute,
            } => collection_eval::eval_is_number(entity_type, *attribute, bindings),

            CompiledCondition::IsBool {
                entity_type,
                attribute,
            } => collection_eval::eval_is_bool(entity_type, *attribute, bindings),

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
                bindings,
            ),

            CompiledCondition::MapKeyExists {
                entity_type,
                attribute,
                key,
            } => collection_eval::eval_map_key_exists_interned(
                entity_type,
                *attribute,
                key,
                bindings,
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
                bindings,
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
                bindings,
                interner,
            ),

            // ============ Expression Assignment ============
            CompiledCondition::ExpressionAssignment {
                variable,
                expr_type,
            } => {
                if let Some(value) =
                    self.evaluate_expr_type(expr_type, bindings, _context, variables, interner)
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
                if let Some(expr_value) =
                    self.evaluate_expr_type(expr_type, bindings, _context, variables, interner)
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
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return false,
                    // Absent actor: fall through to the missing-attribute
                    // path — the AST reads actor.* as Null when no actor is
                    // bound, and missing-attr semantics are identical.
                    EntityType::Actor => bindings.actor,
                };

                let attr_opt = entity.and_then(|e| e.get_attribute(*attribute));

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
                    bindings,
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
                    bindings,
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
        bindings: EntityBindings<'_>,
        _context: &EvalContext<'_>,
        variables: &std::collections::HashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        expr_eval::evaluate_compiled_expr_type(expr_type, bindings, variables, interner)
    }

    /// Evaluate a comprehension and return the resulting collection
    fn evaluate_comprehension(
        &self,
        comp: &CompiledComprehension,
        bindings: EntityBindings<'_>,
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
                    EntityType::User => Some(bindings.user),
                    EntityType::Resource => Some(bindings.resource),
                    EntityType::Context => return None,
                    // Absent actor: empty source, same as a missing attribute
                    // (total iteration — matches the AST contract below).
                    EntityType::Actor => bindings.actor,
                };
                match entity.and_then(|e| e.get_attribute(*attribute)) {
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
                let var_name = interner.resolve(*variable)?;
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
                        bindings,
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
                bindings,
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
        bindings: EntityBindings<'_>,
        context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        _interner: &crate::data::StringInterner,
    ) -> bool {
        for filter in filters {
            if !self.evaluate_compiled_condition(filter, bindings, context, variables) {
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
        bindings: EntityBindings<'_>,
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
                            bindings,
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
            bindings,
            context,
            &mut variables.clone(),
            interner,
        );
        if passes {
            // Update variables if this was an assignment filter
            self.apply_filter_variable_update(filter, variables, bindings, context, interner);
            self.evaluate_comprehension_filters_recursive(
                remaining, bindings, context, variables, output, interner, result,
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
        bindings: EntityBindings<'_>,
        context: &EvalContext<'_>,
        interner: &crate::data::StringInterner,
    ) {
        match filter {
            CompiledCondition::ExpressionAssignment {
                variable,
                expr_type,
            } => {
                if let Some(var_name) = interner.resolve(*variable) {
                    if let Some(val) =
                        self.evaluate_expr_type(expr_type, bindings, context, variables, interner)
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
        bindings: EntityBindings<'_>,
        context: &EvalContext<'_>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
        _interner: &crate::data::StringInterner,
    ) -> bool {
        // For comprehension filters, we need to handle VarAttr comparisons
        // For now, delegate to the regular evaluation
        self.evaluate_compiled_condition(condition, bindings, context, variables)
    }
}

impl ReaperDSLEvaluator {
    /// Evaluate `request`, returning `(action, matched)` where `matched` is
    /// `true` when a deny/allow rule actually fired and `false` when no rule
    /// matched and the per-policy `default_decision` was returned. The set-level
    /// combiner ([`crate::PolicyEngine::evaluate_set`]) uses the flag to treat
    /// an unmatched policy as non-decisive (Plan 08 Phase A). The public
    /// [`PolicyEvaluator::evaluate`] delegates here and discards the flag.
    pub(crate) fn evaluate_with_match(
        &self,
        request: &PolicyRequest,
    ) -> Result<(PolicyAction, bool, Option<&str>), reaper_core::ReaperError> {
        self.evaluate_with_match_input(request, None, false)
    }

    /// Like [`Self::evaluate_with_match`], with an optional structured
    /// `input` document (R4-01 B.1) — the compiled counterpart of the
    /// interpreter's `evaluate_with_input_named`. When `relax_principal` is
    /// set (the with-input ENTRY, document present or not), the
    /// unknown/absent-principal ERROR contract of the legacy entry is
    /// relaxed to a synthesized empty principal (attribute reads miss, fail
    /// closed): document policies routinely carry no principal, and the
    /// interpreter's with-input entry reads an absent principal as Null.
    /// The legacy entry (`relax_principal: false`) is byte-identical to
    /// before.
    pub(crate) fn evaluate_with_match_input(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
        relax_principal: bool,
    ) -> Result<(PolicyAction, bool, Option<&str>), reaper_core::ReaperError> {
        // One evaluation = one ReBAC traversal budget, shared across every
        // condition this policy checks (Plan 08 Phase E).
        crate::data::relationships::reset_traversal_budget();

        let interner = self.store.interner();

        // Reclaim transient result strings interned during this evaluation
        // (see intern_transient). Drop runs on every exit path, incl. panic.
        let _scratch = ScratchGuard::enter(interner);

        // Resolve entity ids from the request WITHOUT interning the request
        // values. Interning here would pin a per-request string in the shared
        // interner forever — an unbounded eval-path leak under high request
        // cardinality (many principals / resources) — and, when a principal is a
        // loaded entity, would pin that entity's id and defeat the data-plane's
        // refcounted reclamation. `lookup` only reads an already-interned id.
        let principal = request.context.get("principal");
        // A principal that was never interned cannot be a loaded entity.
        let user_id = principal.and_then(|p| interner.lookup(p));

        // The resource id is used only for the entity lookup and the synthesized
        // entity below; `resource == "x"` compares the raw request string
        // (ResourceIdEquals), not this id. A resource that was never interned is
        // not an entity and matches no literal, so a sentinel id (never inserted
        // into the interner) is a correct, allocation-free stand-in.
        let resource_id = interner
            .lookup(&request.resource)
            .unwrap_or(InternedString::from_id(u32::MAX));

        // Fast DataStore lookups (~20-50ns each). Legacy (no-input) entry
        // keeps its exact error contract for unknown principals; the
        // with-input entry synthesizes an empty principal instead (Null
        // reads, fail closed — matching the interpreter, see the entry doc).
        let user_found = user_id.and_then(|id| self.store.get(id));
        let temp_user;
        let user: &Entity = match &user_found {
            Some(entity) => entity,
            None => {
                if !relax_principal {
                    let principal =
                        principal.ok_or_else(|| reaper_core::ReaperError::EvaluationError {
                            reason: "Missing principal in context".to_string(),
                        })?;
                    return Err(match user_id {
                        Some(id) => reaper_core::ReaperError::EvaluationError {
                            reason: format!("User entity not found: {:?}", id),
                        },
                        None => reaper_core::ReaperError::EvaluationError {
                            reason: format!("User entity not found: {}", principal),
                        },
                    });
                }
                temp_user = Entity::new(
                    user_id.unwrap_or(InternedString::from_id(u32::MAX)),
                    self.resource_type_id,
                    std::collections::HashMap::new(),
                );
                &temp_user
            }
        };

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

        // Actor resolution (F1 agentic authz). Same non-interning discipline
        // as the resource above: `lookup` only — interning a per-request actor
        // id would pin it in the shared interner forever. An actor that names
        // no loaded entity still binds, as a synthesized STACK-LOCAL empty
        // entity: every attribute then reads as missing (exactly the AST's
        // Null reads), while rebac checks still see the looked-up id — a
        // relation can name a subject id that is not itself a loaded entity.
        // An id that was never interned gets the same never-inserted sentinel
        // as the resource path and can match no relation and no literal.
        let actor_found;
        let temp_actor;
        let actor: Option<&Entity> = match request.actor.as_deref() {
            None => None,
            Some(actor_str) => {
                let actor_id = interner
                    .lookup(actor_str)
                    .unwrap_or(InternedString::from_id(u32::MAX));
                actor_found = self.store.get(actor_id);
                Some(match &actor_found {
                    Some(entity) => entity,
                    None => {
                        temp_actor = Entity::new(
                            actor_id,
                            self.actor_type_id,
                            std::collections::HashMap::new(),
                        );
                        &temp_actor
                    }
                })
            }
        };

        // Zero-copy evaluation context — avoids HashMap clone + 2 String allocations per call
        let eval_context =
            EvalContext::new(&request.action, &request.resource, &request.context, input);

        // Entity bindings for this evaluation; Copy, so passing it down is
        // the same cost as the old ref pair.
        let bindings = EntityBindings {
            user,
            actor,
            resource,
            provenance: request.context_provenance.as_ref(),
        };

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
                bindings,
                &eval_context,
                &mut variables,
            ) {
                // Explicit deny - return immediately, no allow can override this
                return Ok((PolicyAction::Deny, true, Some(rule.name.as_str())));
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 2: No deny matched, evaluate ALLOW rules (pre-partitioned, no type checking needed)
        for rule in &self.compiled_allow_rules {
            let matches = self.evaluate_compiled_condition(
                &rule.condition,
                bindings,
                &eval_context,
                &mut variables,
            );

            if matches {
                tracing::trace!(
                    rule_name = %rule.name,
                    action = %request.action,
                    "Rule matched - returning Allow"
                );
                return Ok((PolicyAction::Allow, true, Some(rule.name.as_str())));
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 3: No rule matched - return default decision
        tracing::debug!(
            default_decision = ?self.default_decision,
            "No rules matched - returning default"
        );
        Ok((self.default_decision.clone(), false, None))
    }
}

impl PolicyEvaluator for ReaperDSLEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, reaper_core::ReaperError> {
        self.evaluate_with_match(request).map(|(action, ..)| action)
    }

    fn evaluate_matched(
        &self,
        request: &PolicyRequest,
    ) -> Result<(PolicyAction, bool), reaper_core::ReaperError> {
        self.evaluate_with_match(request)
            .map(|(action, matched, _)| (action, matched))
    }

    /// Allow-path explainability (F1-s4): surface the deciding rule's name,
    /// borrowed from the compiled rule — zero allocation on the eval loop.
    fn evaluate_named(
        &self,
        request: &PolicyRequest,
    ) -> Result<crate::evaluators::NamedOutcome<'_>, reaper_core::ReaperError> {
        let (decision, matched, rule_name) = self.evaluate_with_match(request)?;
        Ok(crate::evaluators::NamedOutcome {
            decision,
            matched,
            rule_name,
        })
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

    /// D2: DSL policies are prunable when every rule constrains the request
    /// resource to concrete literals. Delegates to the compiled-condition walk.
    fn resource_index_terms(&self) -> Option<Vec<String>> {
        self.compiled_resource_index_terms()
    }

    /// R3-P2-1: two-tier prunability. Delegates to the compiled-condition walk
    /// so ABAC/ReBAC policies bounded by `resource.type == "…"` become
    /// prunable, not only literal-id policies.
    fn resource_pruning(&self) -> crate::evaluators::ResourcePruning {
        self.compiled_resource_pruning()
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

/// A *disjunctive superset bound* on the requests one compiled condition can be
/// true for (R3-P2-1): the condition is provably false for every request whose
/// resource id is outside `ids` AND whose resource entity `type` attribute is
/// outside `types`. This is exactly the shape of the engine's two-tier
/// candidate lookup (id bucket ∪ type bucket), so bounds compose into the
/// index without translation. `Option<ResourceBound>`: `None` = unbounded.
#[derive(Debug, Clone, Default)]
struct ResourceBound {
    ids: Vec<String>,
    types: Vec<String>,
}

impl ResourceBound {
    fn is_pure_ids(&self) -> bool {
        self.types.is_empty()
    }
    fn is_pure_types(&self) -> bool {
        self.ids.is_empty()
    }
}

impl ReaperDSLEvaluator {
    /// D2 backward-compatible projection: `Some(ids)` iff the policy is bounded
    /// purely by resource-id literals (the original single-tier promise);
    /// type-bounded or unbounded policies return `None`.
    fn compiled_resource_index_terms(&self) -> Option<Vec<String>> {
        match self.compiled_resource_pruning() {
            crate::evaluators::ResourcePruning::Bounded { ids, types } if types.is_empty() => {
                Some(ids)
            }
            _ => None,
        }
    }

    /// D2 + R3-P2-1 (PRIMARY path): the two-tier prunability bound of this
    /// compiled policy. Walks the COMPILED conditions, not the AST.
    ///
    /// ## Soundness rule
    /// Exactly TWO compiled leaves tie a request to a concrete term (see
    /// `evaluate_compiled_condition`):
    /// - [`CompiledCondition::ResourceIdEquals`], true iff
    ///   `request.resource == value` → bounded to ids `{value}`.
    /// - `AttributeCompare { entity: Resource, attribute: "type", op: ==,
    ///   target: LiteralString(T) }`, true iff the `DataStore` entity for
    ///   `request.resource` has a string attribute `type` equal to `T`
    ///   (`eval_attribute_comparison`: a missing entity binds a synthesized
    ///   attribute-less entity and a missing/other-typed attribute compares
    ///   false, never an error) → bounded to types `{T}`. Only the *exact*
    ///   simple attribute `type` with `==` against a string literal qualifies;
    ///   `!=`, ordering ops, dotted paths, and non-string literals stay
    ///   unbounded.
    ///
    /// Every other leaf — action/time/rebac/string/variable predicates — says
    /// nothing usable about the request and is unbounded. Composition over a
    /// disjunctive bound `{ids} ∪ {types}`:
    /// - `And(children)`: any single bounded child is a valid bound for the
    ///   conjunction. Prefer intersecting the pure-id children (most
    ///   selective), else intersect the pure-type children, else take the
    ///   first mixed bound. (Field-wise intersection of *mixed* bounds is NOT
    ///   sound — `(id=A) ∧ (type=T)` can fire even though `ids ∩ ids' = ∅` —
    ///   so mixed bounds are never intersected.)
    /// - `Or(children)`: bounded iff EVERY child is bounded, to the field-wise
    ///   union (a disjunction of disjunctive bounds is their union).
    /// - `Not(Always)` (i.e. `false`) → bounded to `{} ∪ {}` (never matches);
    ///   any other `Not(_)` → unbounded (conservative — including
    ///   `Not(ResourceIdEquals)` and `resource.type != …`).
    /// - `Always` → unbounded (matches every request).
    ///
    /// The policy's bound is the field-wise UNION over every rule (deny +
    /// allow); if ANY rule is unbounded the whole policy is `Unprunable`.
    /// Because each rule's bound is a *superset* of the requests for which
    /// that rule can fire, a request outside both unions provably makes every
    /// rule non-matching (the set combiner treats that as non-decisive), so
    /// pruning it is safe. It can never fail open: an unrecognized shape
    /// yields unbounded, never a spurious bound.
    /// Evaluate with an optional structured `input` document, naming the
    /// deciding rule (R4-01 B.1) — the compiled counterpart of the
    /// interpreter's `evaluate_with_input_named`. `None` rule name = the
    /// per-policy default decided.
    pub fn evaluate_with_input_named(
        &self,
        request: &PolicyRequest,
        input: Option<&serde_json::Value>,
    ) -> Result<(PolicyAction, Option<&str>), reaper_core::ReaperError> {
        self.evaluate_with_match_input(request, input, true)
            .map(|(action, _, name)| (action, name))
    }

    /// Tier-2 specialization fitness of this evaluator's compiled rules
    /// (R3 Plan 06 F.3 dry run — design §6). Pure measurement: consults no
    /// data, rewrites nothing.
    pub fn specialization_fitness(&self) -> compiler::SpecializationFitness {
        let mut fitness = compiler::specialization_fitness(&self.compiled_deny_rules);
        fitness.merge(&compiler::specialization_fitness(
            &self.compiled_allow_rules,
        ));
        fitness
    }

    fn compiled_resource_pruning(&self) -> crate::evaluators::ResourcePruning {
        let interner = self.store.interner();
        let mut ids: Vec<String> = Vec::new();
        let mut types: Vec<String> = Vec::new();
        for rule in self
            .compiled_deny_rules
            .iter()
            .chain(self.compiled_allow_rules.iter())
        {
            // Any rule that can match an unbounded request set makes the whole
            // policy a candidate for every request.
            let Some(bound) = Self::condition_resource_bound(&rule.condition, interner) else {
                return crate::evaluators::ResourcePruning::Unprunable;
            };
            ids.extend(bound.ids);
            types.extend(bound.types);
        }
        ids.sort();
        ids.dedup();
        types.sort();
        types.dedup();
        crate::evaluators::ResourcePruning::Bounded { ids, types }
    }

    /// Disjunctive bound of one compiled condition, or `None` when unbounded.
    /// See [`Self::compiled_resource_pruning`] for the leaf and composition
    /// soundness rules.
    fn condition_resource_bound(
        cond: &CompiledCondition,
        interner: &StringInterner,
    ) -> Option<ResourceBound> {
        match cond {
            // `resource == "literal"`: true iff request.resource == value.
            CompiledCondition::ResourceIdEquals { value } => {
                // A literal that cannot be resolved back to a string is treated
                // as unbounded (fail safe, never fail open).
                interner.resolve(*value).map(|s| ResourceBound {
                    ids: vec![s.to_string()],
                    types: Vec::new(),
                })
            }
            // `resource.type == "T"`: true iff the resource entity's `type`
            // attribute is the string T. Guarded to exactly that shape; any
            // other entity/attribute/op/target falls through to unbounded.
            CompiledCondition::AttributeCompare(comp)
                if matches!(comp.entity_type, EntityType::Resource)
                    && matches!(comp.op, NumericOp::Equal) =>
            {
                let is_type_attr = interner
                    .resolve(comp.attribute)
                    .is_some_and(|name| &*name == "type");
                match (&comp.target, is_type_attr) {
                    (CompiledCompareTarget::LiteralString(t), true) => {
                        interner.resolve(*t).map(|s| ResourceBound {
                            ids: Vec::new(),
                            types: vec![s.to_string()],
                        })
                    }
                    _ => None,
                }
            }
            // Always-true rule matches every request.
            CompiledCondition::Always => None,
            // `false` compiles to Not(Always) and matches nothing; any other
            // negation is treated conservatively as unbounded.
            CompiledCondition::Not(inner) => {
                if matches!(**inner, CompiledCondition::Always) {
                    Some(ResourceBound::default())
                } else {
                    None
                }
            }
            CompiledCondition::And(children) => {
                // Any single bounded child bounds the conjunction. Prefer the
                // intersection of the pure-id children, else pure-type, else
                // the first mixed bound — mixed bounds must NOT be intersected
                // field-wise (see compiled_resource_pruning).
                let bounds: Vec<ResourceBound> = children
                    .iter()
                    .filter_map(|c| Self::condition_resource_bound(c, interner))
                    .collect();
                if bounds.is_empty() {
                    return None;
                }
                let intersect = |mut acc: Vec<String>, next: &[String]| -> Vec<String> {
                    acc.retain(|x| next.contains(x));
                    acc
                };
                if bounds.iter().any(|b| b.is_pure_ids()) {
                    let ids = bounds
                        .iter()
                        .filter(|b| b.is_pure_ids())
                        .map(|b| b.ids.clone())
                        .reduce(|acc, next| intersect(acc, &next))
                        .unwrap_or_default();
                    Some(ResourceBound {
                        ids,
                        types: Vec::new(),
                    })
                } else if bounds.iter().any(|b| b.is_pure_types()) {
                    let types = bounds
                        .iter()
                        .filter(|b| b.is_pure_types())
                        .map(|b| b.types.clone())
                        .reduce(|acc, next| intersect(acc, &next))
                        .unwrap_or_default();
                    Some(ResourceBound {
                        ids: Vec::new(),
                        types,
                    })
                } else {
                    bounds.into_iter().next()
                }
            }
            CompiledCondition::Or(children) => {
                // Field-wise union over children; a single unbounded child
                // (`?` → None) makes the whole disjunction unbounded.
                let mut union = ResourceBound::default();
                for child in children {
                    let bound = Self::condition_resource_bound(child, interner)?;
                    union.ids.extend(bound.ids);
                    union.types.extend(bound.types);
                }
                Some(union)
            }
            // Every other leaf constrains action/attributes/relationships/etc.
            // in ways the index cannot key on -> unbounded.
            _ => None,
        }
    }
}
