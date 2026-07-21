//! Compiler for Reaper DSL conditions.
//!
//! Converts Condition AST nodes into CompiledCondition with pre-interned strings.
//! This happens once at construction time, not during evaluation.
//!
//! ## Architecture
//! Uses consolidated types to reduce code duplication:
//! - `CompiledAttributeComparison` for all entity attribute comparisons
//! - `CompiledStringOperation` for all string operations
//! - `CompiledCountCondition` for all count operations
//! - etc.

use super::types::{
    // Compiled types
    CompiledCompareTarget,
    CompiledComprehension,
    CompiledCondition,
    CompiledIterationSource,
    CompiledIterator,
    CompiledLiteralValue,
    CompiledOutput,
    CompiledRebacRef,
    // Compiled consolidated types
    CompiledRegexMatch,
    CompiledRule,
    ComprehensionType,
    // Uncompiled types
    Condition,
    EntityType,
    LiteralValue,
    UncompiledComprehensionType,
    UncompiledIterationSource,
    UncompiledOutput,
};
use crate::data::StringInterner;

// Import expression compilation from expr_compiler module
use super::expr_compiler::compile_expr_type;

// Re-export collection utilities from collect module for backward compatibility
pub use super::collect::{collect_membership_values, collect_regex_patterns};

/// Compile a condition with pre-interned strings for zero-lookup evaluation.
/// This is called once at construction time, not during evaluation.
pub fn compile_condition(condition: &Condition, interner: &StringInterner) -> CompiledCondition {
    match condition {
        Condition::Always => CompiledCondition::Always,

        // Taint: the key stays a raw String — it looks up the request's
        // provenance map at eval time, never the interner.
        Condition::TaintTrusted { key } => CompiledCondition::TaintTrusted { key: key.clone() },

        // Input comparison (R4-01 B.1): already pre-parsed at lowering;
        // nothing is interned (raw document keys/values).
        Condition::InputCompare { path, op, target } => CompiledCondition::InputCompare {
            path: path.clone(),
            op: *op,
            target: target.clone(),
        },

        Condition::RebacCheck {
            kind,
            subject,
            relation,
            object,
            via,
            max_depth,
        } => {
            use crate::evaluators::reaper_dsl::CompiledRebacRef;
            use crate::evaluators::reaper_dsl::RebacRef;
            let compile_ref = |r: &RebacRef| match r {
                RebacRef::Principal => CompiledRebacRef::Principal,
                RebacRef::ResourceId => CompiledRebacRef::ResourceId,
                RebacRef::Literal(s) => CompiledRebacRef::Literal(interner.intern(s)),
                RebacRef::Actor => CompiledRebacRef::Actor,
            };
            CompiledCondition::RebacCheck {
                kind: *kind,
                subject: compile_ref(subject),
                relation: interner.intern(relation),
                object: compile_ref(object),
                via: via.as_ref().map(|v| interner.intern(v)),
                max_depth: *max_depth,
            }
        }
        Condition::ActionEquals { value } => CompiledCondition::ActionEquals {
            value: interner.intern(value),
        },
        Condition::ResourceIdEquals { value } => CompiledCondition::ResourceIdEquals {
            value: interner.intern(value),
        },

        // ============ Consolidated Types ============
        Condition::AttributeCompare(comp) => {
            CompiledCondition::AttributeCompare(comp.to_compiled(interner))
        }
        Condition::StringOp(op) => CompiledCondition::StringOp(op.to_compiled(interner)),
        Condition::VariableStringOp(op) => {
            CompiledCondition::VariableStringOp(op.to_compiled(interner))
        }
        Condition::CountOp(cond) => CompiledCondition::CountOp(cond.to_compiled(interner)),
        Condition::TimeOp(cond) => CompiledCondition::TimeOp(cond.to_compiled(interner)),
        Condition::CrossEntityCompare(comp) => {
            CompiledCondition::CrossEntityCompare(comp.to_compiled(interner))
        }
        Condition::WildcardCompare(comp) => {
            CompiledCondition::WildcardCompare(comp.to_compiled(interner))
        }

        // ============ Same Entity Comparisons ============
        Condition::SameEntityAttrCompare {
            entity_type,
            left_attr,
            right_attr,
            op,
        } => CompiledCondition::SameEntityAttrCompare {
            entity_type: entity_type.clone(),
            left_attr: interner.intern(left_attr),
            right_attr: interner.intern(right_attr),
            op: *op,
        },

        // ============ Assignments & Membership ============
        Condition::Assignment {
            variable,
            entity_type,
            attribute,
            index,
        } => CompiledCondition::Assignment {
            variable: interner.intern(variable),
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            index: index.clone(),
        },
        Condition::MembershipTest {
            value,
            entity_type,
            attribute,
            index,
        } => CompiledCondition::MembershipTest {
            value: compile_literal(value, interner),
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            index: index.clone(),
        },
        Condition::IndexedEquals {
            entity_type,
            attribute,
            index,
            value,
        } => CompiledCondition::IndexedEquals {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            index: index.clone(),
            value: interner.intern(value),
        },
        Condition::EqualsVariable {
            entity_type,
            attribute,
            variable,
        } => CompiledCondition::EqualsVariable {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            variable: interner.intern(variable),
        },

        // ============ Regex Match ============
        Condition::RegexMatches {
            entity_type,
            attribute,
            pattern,
        } => CompiledCondition::RegexMatch(CompiledRegexMatch {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            pattern: pattern.clone(),
        }),

        // ============ Object / collection predicates ============
        Condition::ObjectHasKey {
            entity_type,
            attribute,
            key,
        } => CompiledCondition::ObjectHasKey {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            key: interner.intern(key),
        },
        Condition::CollectionAny {
            entity_type,
            attribute,
        } => CompiledCondition::CollectionAny {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        Condition::CollectionAll {
            entity_type,
            attribute,
        } => CompiledCondition::CollectionAll {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },

        // ============ Type Checks ============
        Condition::IsString {
            entity_type,
            attribute,
        } => CompiledCondition::IsString {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        Condition::IsNumber {
            entity_type,
            attribute,
        } => CompiledCondition::IsNumber {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        Condition::IsBool {
            entity_type,
            attribute,
        } => CompiledCondition::IsBool {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        Condition::SetIntersectionCountGreater {
            entity_type,
            attribute,
            values,
            threshold,
        } => CompiledCondition::SetIntersectionCountGreater {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            values: values.iter().map(|v| interner.intern(v)).collect(),
            threshold: *threshold,
        },
        Condition::MapKeyExists {
            entity_type,
            attribute,
            key,
        } => CompiledCondition::MapKeyExists {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            key: interner.intern(key),
        },
        Condition::ComprehensionCountGreaterEqual {
            entity_type,
            attribute,
            filter_attr,
            filter_value,
            filter_op,
            threshold,
        } => CompiledCondition::ComprehensionCountGreaterEqual {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            filter_attr: interner.intern(filter_attr),
            filter_value: compile_literal(filter_value, interner),
            filter_op: filter_op.clone(),
            threshold: *threshold,
        },
        Condition::ComprehensionCountEqual {
            entity_type,
            attribute,
            filter_attr,
            filter_value,
            filter_op,
            threshold,
        } => CompiledCondition::ComprehensionCountEqual {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            filter_attr: interner.intern(filter_attr),
            filter_value: compile_literal(filter_value, interner),
            filter_op: filter_op.clone(),
            threshold: *threshold,
        },

        // ============ Expression Assignment ============
        Condition::ExpressionAssignment {
            variable,
            expr_type,
        } => CompiledCondition::ExpressionAssignment {
            variable: interner.intern(variable),
            expr_type: compile_expr_type(expr_type, interner),
        },
        Condition::ExprCompareAssignment {
            variable,
            expr_type,
            op,
            value,
        } => CompiledCondition::ExprCompareAssignment {
            variable: interner.intern(variable),
            expr_type: compile_expr_type(expr_type, interner),
            op: *op,
            value: compile_literal(value, interner),
        },

        // ============ Variable Comparisons ============
        Condition::VariableEqualsLiteral { variable, value } => {
            CompiledCondition::VariableEqualsLiteral {
                variable: interner.intern(variable),
                value: compile_literal(value, interner),
            }
        }
        Condition::VariableNotEqualsLiteral { variable, value } => {
            CompiledCondition::VariableNotEqualsLiteral {
                variable: interner.intern(variable),
                value: compile_literal(value, interner),
            }
        }
        Condition::VariableCompare {
            variable,
            op,
            value,
        } => CompiledCondition::VariableCompare {
            variable: interner.intern(variable),
            op: *op,
            value: compile_literal(value, interner),
        },
        Condition::VariableIsNull { variable } => CompiledCondition::VariableIsNull {
            variable: interner.intern(variable),
        },
        Condition::VariableIsNotNull { variable } => CompiledCondition::VariableIsNotNull {
            variable: interner.intern(variable),
        },
        Condition::ComparisonAssignment {
            variable,
            entity_type,
            attribute,
            op,
            value,
        } => CompiledCondition::ComparisonAssignment {
            variable: interner.intern(variable),
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            op: *op,
            value: compile_literal(value, interner),
        },
        Condition::NullComparisonAssignment {
            variable,
            entity_type,
            attribute,
            is_null_check,
        } => CompiledCondition::NullComparisonAssignment {
            variable: interner.intern(variable),
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            is_null_check: *is_null_check,
        },
        Condition::VariableMembershipTest { value, variable } => {
            CompiledCondition::VariableMembershipTest {
                value: compile_literal(value, interner),
                variable: interner.intern(variable),
            }
        }
        Condition::VariableIsString { variable } => CompiledCondition::VariableIsString {
            variable: interner.intern(variable),
        },
        Condition::VariableIsNumber { variable } => CompiledCondition::VariableIsNumber {
            variable: interner.intern(variable),
        },
        Condition::VariableIsBool { variable } => CompiledCondition::VariableIsBool {
            variable: interner.intern(variable),
        },
        Condition::VariableIsTruthy { variable } => CompiledCondition::VariableIsTruthy {
            variable: interner.intern(variable),
        },
        Condition::VariableEqualsVariable { left, right } => {
            CompiledCondition::VariableEqualsVariable {
                left: interner.intern(left),
                right: interner.intern(right),
            }
        }
        Condition::VariableNotEqualsVariable { left, right } => {
            CompiledCondition::VariableNotEqualsVariable {
                left: interner.intern(left),
                right: interner.intern(right),
            }
        }
        Condition::VariableMethodWithLiteralArray {
            variable,
            method,
            values,
        } => CompiledCondition::VariableMethodWithLiteralArray {
            variable: interner.intern(variable),
            method: method.clone(),
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        Condition::VariableMethodCompare {
            variable,
            method,
            op,
            value,
        } => CompiledCondition::VariableMethodCompare {
            variable: interner.intern(variable),
            method: *method,
            op: *op,
            value: compile_literal(value, interner),
        },
        Condition::VariableChainedMethodCompare {
            variable,
            transform_method,
            compare_method,
            op,
            value,
        } => CompiledCondition::VariableChainedMethodCompare {
            variable: interner.intern(variable),
            transform_method: *transform_method,
            compare_method: *compare_method,
            op: *op,
            value: compile_literal(value, interner),
        },

        // ============ Variable Attribute Comparisons ============
        Condition::VariableAttrEqualsLiteral {
            variable,
            attribute,
            value,
        } => CompiledCondition::VariableAttrEqualsLiteral {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
            value: compile_literal(value, interner),
        },
        Condition::VariableAttrNotEqualsLiteral {
            variable,
            attribute,
            value,
        } => CompiledCondition::VariableAttrNotEqualsLiteral {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
            value: compile_literal(value, interner),
        },
        Condition::VariableAttrCompare {
            variable,
            attribute,
            op,
            value,
        } => CompiledCondition::VariableAttrCompare {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
            op: *op,
            value: compile_literal(value, interner),
        },
        Condition::VariableAttrEqualsNull {
            variable,
            attribute,
        } => CompiledCondition::VariableAttrEqualsNull {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
        },
        Condition::VariableAttrNotEqualsNull {
            variable,
            attribute,
        } => CompiledCondition::VariableAttrNotEqualsNull {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
        },
        Condition::VarAttrNullCompareAssignment {
            result_variable,
            source_variable,
            attribute,
            is_null_check,
        } => CompiledCondition::VarAttrNullCompareAssignment {
            result_variable: interner.intern(result_variable),
            source_variable: interner.intern(source_variable),
            attribute: interner.intern(attribute),
            is_null_check: *is_null_check,
        },
        Condition::VariableAttrContains {
            variable,
            attribute,
            substring,
        } => CompiledCondition::VariableAttrContains {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
            substring: interner.intern(substring),
        },

        // ============ Comprehension Assignment ============
        Condition::ComprehensionAssignment {
            variable,
            comp_type,
            iterator_var,
            iterator_source,
            filters,
            output,
            key_output,
        } => CompiledCondition::ComprehensionAssignment {
            variable: interner.intern(variable),
            comprehension: Box::new(compile_comprehension(
                comp_type,
                iterator_var,
                iterator_source,
                filters,
                output,
                key_output,
                interner,
            )),
        },

        // ============ Logical Operators ============
        Condition::And(conditions) => CompiledCondition::And(
            conditions
                .iter()
                .map(|c| compile_condition(c, interner))
                .collect(),
        ),
        Condition::Or(conditions) => CompiledCondition::Or(
            conditions
                .iter()
                .map(|c| compile_condition(c, interner))
                .collect(),
        ),
        Condition::Not(inner) => {
            CompiledCondition::Not(Box::new(compile_condition(inner, interner)))
        }
    }
}

/// Compile comprehension with pre-interned strings
fn compile_comprehension(
    comp_type: &UncompiledComprehensionType,
    iterator_var: &str,
    iterator_source: &UncompiledIterationSource,
    filters: &[Condition],
    output: &Option<UncompiledOutput>,
    key_output: &Option<UncompiledOutput>,
    interner: &StringInterner,
) -> CompiledComprehension {
    let compiled_type = match comp_type {
        UncompiledComprehensionType::Set => ComprehensionType::Set,
        UncompiledComprehensionType::Array => ComprehensionType::Array,
        UncompiledComprehensionType::Object => ComprehensionType::Object,
    };

    let compiled_source = match iterator_source {
        UncompiledIterationSource::EntityAttr {
            entity_type,
            attribute,
        } => CompiledIterationSource::EntityAttr {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        UncompiledIterationSource::Variable { variable } => CompiledIterationSource::Variable {
            variable: interner.intern(variable),
        },
    };

    let compiled_filters: Vec<CompiledCondition> = filters
        .iter()
        .map(|f| compile_condition(f, interner))
        .collect();

    // Helper to compile output
    let compile_output_helper = |o: &UncompiledOutput| match o {
        UncompiledOutput::Variable(var) => CompiledOutput::Variable(interner.intern(var)),
        UncompiledOutput::VarAttr {
            variable,
            attribute,
        } => CompiledOutput::VarAttr {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
        },
        UncompiledOutput::Literal(lit) => CompiledOutput::Literal(compile_literal(lit, interner)),
        UncompiledOutput::VarMethodCall { variable, method } => CompiledOutput::VarMethodCall {
            variable: interner.intern(variable),
            method: method.clone(),
        },
    };

    // For object comprehensions, compile key_value; for others, compile output
    let (compiled_output, compiled_key_value) =
        if matches!(comp_type, UncompiledComprehensionType::Object) {
            // Object comprehension: output is value, key_output is key
            let key = key_output.as_ref().map(compile_output_helper);
            let value = output.as_ref().map(compile_output_helper);
            match (key, value) {
                (Some(k), Some(v)) => (None, Some((k, v))),
                _ => (output.as_ref().map(compile_output_helper), None),
            }
        } else {
            (output.as_ref().map(compile_output_helper), None)
        };

    CompiledComprehension {
        comp_type: compiled_type,
        iterator: CompiledIterator {
            variable: interner.intern(iterator_var),
            source: compiled_source,
        },
        filters: compiled_filters,
        output: compiled_output,
        key_value: compiled_key_value,
    }
}

/// Compile a literal value with pre-interned strings
pub fn compile_literal(value: &LiteralValue, interner: &StringInterner) -> CompiledLiteralValue {
    match value {
        LiteralValue::String(s) => CompiledLiteralValue::String(interner.intern(s)),
        LiteralValue::Int(i) => CompiledLiteralValue::Int(*i),
        LiteralValue::Bool(b) => CompiledLiteralValue::Bool(*b),
    }
}

// ===========================================================================
// Tier-1 partial evaluation: deploy-time constant folding (R3 Plan 06 F)
// ===========================================================================

/// The canonical compiled `false`: `!true`. The parser lowers the `false`
/// literal to `Not(Always)`, so this is the only constant-false shape the
/// folder needs to recognize.
fn is_false(cond: &CompiledCondition) -> bool {
    matches!(cond, CompiledCondition::Not(inner) if matches!(**inner, CompiledCondition::Always))
}

/// Does evaluating this condition write to the rule-scoped `variables` map?
///
/// This is the SOUNDNESS GUARD for every eliminating fold. `And`/`Or`
/// evaluation short-circuits, but a fold that *replaces a subtree with a
/// constant* (or drops a child) skips evaluations the runtime would have
/// performed — and an assignment condition evaluated inside one branch is
/// visible to every LATER condition of the same rule (`variables` clears
/// between rules, not between siblings). Example that would break without
/// this guard: `(let x = user.role && false) || x == "admin"` — folding the
/// left disjunct to `false` and dropping it would leave `x` unbound and flip
/// the rule's outcome. Any subtree containing a binding form is therefore
/// left completely untouched by eliminating folds.
///
/// Conservative by construction: every `*Assignment` variant is a binding
/// form (they are exactly the variants whose evaluation calls
/// `variables.insert`); recursion covers bindings nested under `And`/`Or`/
/// `Not`. Unknown future variants default to NOT binding — a new binding
/// variant must be added here, which the exhaustiveness test below pins.
fn binds_variables(cond: &CompiledCondition) -> bool {
    match cond {
        CompiledCondition::Assignment { .. }
        | CompiledCondition::ExpressionAssignment { .. }
        | CompiledCondition::ExprCompareAssignment { .. }
        | CompiledCondition::ComparisonAssignment { .. }
        | CompiledCondition::NullComparisonAssignment { .. }
        | CompiledCondition::VarAttrNullCompareAssignment { .. }
        | CompiledCondition::ComprehensionAssignment { .. } => true,
        CompiledCondition::And(children) | CompiledCondition::Or(children) => {
            children.iter().any(binds_variables)
        }
        CompiledCondition::Not(inner) => binds_variables(inner),
        _ => false,
    }
}

/// Tier-1 partial evaluation (round-3 Plan 06 F): fold constants out of a
/// compiled condition at DEPLOY time, so the per-request evaluation loop never
/// visits a subtree whose truth is already known. Purely structural — no
/// entity data is consulted (that is tier 2, see
/// `docs/development/PARTIAL_EVALUATION.md`), so nothing here can go stale
/// when the `DataStore` mutates.
///
/// Folds applied (each preserves decision, matched-flag, rule-name, AND the
/// variable-binding side effects of evaluation — see [`binds_variables`]):
/// - `Not(Not(x))` → `x` (double negation; `x` is still evaluated, so its
///   bindings are preserved — no guard needed).
/// - `And`: fold children, splice nested `And`s in place (same depth-first
///   left-to-right evaluation order), drop `true` children (bind nothing).
///   A `false` child makes the whole conjunction `false` — applied only when
///   NO child binds a variable, else the conjunction is kept as-is.
///   Empty after drops → `true`; single child → unwrapped.
/// - `Or`: dual — splice nested `Or`s, drop `false` children (bind nothing);
///   a `true` child folds the disjunction to `true` only when NO child binds.
///   Empty after drops → `false`; single child → unwrapped.
///
/// Splicing vs short-circuit: `And(And(a, b), c)` and `And(a, b, c)` evaluate
/// the identical prefix under the runtime's short-circuiting (`a` false skips
/// `b` in both shapes), so flattening changes neither the outcome nor which
/// bindings occur.
///
/// Rules are deliberately NOT dropped when their condition folds to `false`:
/// a `deny if false` rule still occupies its slot (one cheap `is_false`-shaped
/// check per request) so `validate()`'s at-least-one-rule invariant and rule
/// counts stay untouched. The pruning-index extraction already maps a `false`
/// rule to the empty bound.
///
/// Fold order is bottom-up (children first), so constants produced by inner
/// folds propagate outward in one pass: `(false || true) && x` → `x`.
pub fn fold_condition(cond: CompiledCondition) -> CompiledCondition {
    match cond {
        CompiledCondition::Not(inner) => {
            let folded = fold_condition(*inner);
            match folded {
                // !!x → x. Bindings inside x still evaluate.
                CompiledCondition::Not(grand) => *grand,
                other => CompiledCondition::Not(Box::new(other)),
            }
        }
        CompiledCondition::And(children) => {
            let folded: Vec<CompiledCondition> = children.into_iter().map(fold_condition).collect();
            let any_binds = folded.iter().any(binds_variables);
            // A false conjunct decides the And — but only fold it away when no
            // sibling binds (skipping a binding evaluation is observable).
            if folded.iter().any(is_false) && !any_binds {
                return CompiledCondition::Not(Box::new(CompiledCondition::Always));
            }
            let mut out: Vec<CompiledCondition> = Vec::with_capacity(folded.len());
            for child in folded {
                match child {
                    // `true` conjunct: decides nothing, binds nothing — drop.
                    CompiledCondition::Always => {}
                    // Splice nested Ands in place (order-preserving).
                    CompiledCondition::And(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            match out.len() {
                0 => CompiledCondition::Always,
                1 => out.into_iter().next().expect("len checked"),
                _ => CompiledCondition::And(out),
            }
        }
        CompiledCondition::Or(children) => {
            let folded: Vec<CompiledCondition> = children.into_iter().map(fold_condition).collect();
            let any_binds = folded.iter().any(binds_variables);
            // A true disjunct decides the Or — but only fold it away when no
            // sibling binds (a dropped earlier disjunct may have bound a
            // variable a later rule condition reads).
            if folded
                .iter()
                .any(|c| matches!(c, CompiledCondition::Always))
                && !any_binds
            {
                return CompiledCondition::Always;
            }
            let mut out: Vec<CompiledCondition> = Vec::with_capacity(folded.len());
            for child in folded {
                if is_false(&child) {
                    // `false` disjunct: decides nothing, binds nothing — drop.
                    continue;
                }
                match child {
                    // Splice nested Ors in place (order-preserving).
                    CompiledCondition::Or(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            match out.len() {
                0 => CompiledCondition::Not(Box::new(CompiledCondition::Always)),
                1 => out.into_iter().next().expect("len checked"),
                _ => CompiledCondition::Or(out),
            }
        }
        // Leaves are already minimal.
        other => other,
    }
}

// ===========================================================================
// Tier-2 partial evaluation, F.3 DRY RUN (R3 Plan 06 F; design in
// docs/development/PARTIAL_EVALUATION.md §3.3/§6). Nothing here rewrites the
// serving rules — this is the fitness instrument that decides whether the
// F.4 specialization overlay is worth building. It classifies every compiled
// leaf by what its truth depends on, and simulates (via the REAL tier-1
// fold) how much shorter each rule would get if the data-static leaves were
// pre-evaluated to constants.
// ===========================================================================

/// What a compiled leaf condition's truth depends on (design §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafStaticness {
    /// Depends only on deploy-time data (the DataStore / relationship
    /// graph): pre-evaluable by the tier-2 overlay, invalidated by the
    /// `data_epoch`. Today this is exactly the literal-literal ReBAC shape —
    /// the compiled condition language anchors every attribute read to the
    /// request (`EntityType` is User/Resource/Context/Actor), so the
    /// "literal-named entity attribute" row of §2.1 is empty by construction.
    Static,
    /// Anchored on the request CONTEXT: static only under an
    /// operator-declared static-context config (region, tenant, deployment
    /// environment — design §2.1 last row), which does not exist yet. Counted
    /// separately as the upper bound such a config could unlock.
    StaticContext,
    /// Depends on the request (principal, resource, action, actor,
    /// variables, taint provenance): never specializable.
    Dynamic,
}

/// Classify one compiled condition node. `And`/`Or`/`Not` are structure, not
/// leaves — callers recurse; this returns `Dynamic` for them defensively.
///
/// The match is EXHAUSTIVE ON PURPOSE (no `_` arm): adding a
/// `CompiledCondition` variant must fail compilation here, forcing an
/// explicit staticness decision — the same pin discipline as
/// [`binds_variables`]. Classification is conservative: when in doubt a leaf
/// is `Dynamic`, which can only cost a missed optimization, never a stale
/// answer.
pub fn leaf_staticness(cond: &CompiledCondition) -> LeafStaticness {
    use CompiledCondition as C;
    use LeafStaticness::{Dynamic, Static, StaticContext};

    /// Context-anchored ⇒ static-context candidate; any other anchor ⇒
    /// request-dependent.
    fn by_entity(entity: &EntityType) -> LeafStaticness {
        match entity {
            EntityType::Context => StaticContext,
            EntityType::User | EntityType::Resource | EntityType::Actor => Dynamic,
        }
    }

    match cond {
        // -- The one provably data-static shape today: ReBAC with BOTH refs
        // literal. Every RebacKind (direct / reachable / inherited) reads
        // only the relationship graph, whose mutations bump the data epoch.
        C::RebacCheck {
            subject, object, ..
        } => match (subject, object) {
            (CompiledRebacRef::Literal(_), CompiledRebacRef::Literal(_)) => Static,
            _ => Dynamic,
        },

        // -- Entity-anchored reads: staticness follows the anchor. For
        // comparisons whose TARGET is itself an entity attribute or a
        // variable, the target must be context/literal too.
        C::AttributeCompare(c) => match (&c.entity_type, &c.target) {
            (EntityType::Context, CompiledCompareTarget::LiteralString(_))
            | (EntityType::Context, CompiledCompareTarget::LiteralNum(_))
            | (EntityType::Context, CompiledCompareTarget::LiteralBool(_))
            | (EntityType::Context, CompiledCompareTarget::LiteralNull) => StaticContext,
            (EntityType::Context, CompiledCompareTarget::EntityAttr { entity_type, .. }) => {
                by_entity(entity_type)
            }
            _ => Dynamic,
        },
        C::StringOp(c) => by_entity(&c.entity_type),
        C::CountOp(c) => by_entity(&c.entity_type),
        // Compares an entity attribute to a literal threshold baked in at
        // compile time — no clock read at eval time, so the anchor decides.
        C::TimeOp(c) => by_entity(&c.entity_type),
        C::RegexMatch(c) => by_entity(&c.entity_type),
        C::CrossEntityCompare(c) => match (by_entity(&c.left_entity), by_entity(&c.right_entity)) {
            (StaticContext, StaticContext) => StaticContext,
            _ => Dynamic,
        },
        C::WildcardCompare(c) => {
            match (by_entity(&c.collection_entity), by_entity(&c.scalar_entity)) {
                (StaticContext, StaticContext) => StaticContext,
                _ => Dynamic,
            }
        }
        C::ObjectHasKey { entity_type, .. }
        | C::CollectionAny { entity_type, .. }
        | C::CollectionAll { entity_type, .. }
        | C::SameEntityAttrCompare { entity_type, .. }
        | C::MembershipTest { entity_type, .. }
        | C::IndexedEquals { entity_type, .. }
        | C::IsString { entity_type, .. }
        | C::IsNumber { entity_type, .. }
        | C::IsBool { entity_type, .. }
        | C::SetIntersectionCountGreater { entity_type, .. }
        | C::MapKeyExists { entity_type, .. }
        | C::ComprehensionCountGreaterEqual { entity_type, .. }
        | C::ComprehensionCountEqual { entity_type, .. } => by_entity(entity_type),

        // -- Request-intrinsic: never static.
        C::ActionEquals { .. } | C::ResourceIdEquals { .. } => Dynamic,
        // Trust provenance is a property of the incoming request.
        C::TaintTrusted { .. } => Dynamic,
        // The input document is request-scoped by definition.
        C::InputCompare { .. } => Dynamic,

        // -- Anything touching the rule-scoped variables map: reads depend
        // on bindings made at eval time; writes ARE eval-time side effects
        // (and the binding guard forbids eliminating them anyway).
        C::Assignment { .. }
        | C::ExpressionAssignment { .. }
        | C::ExprCompareAssignment { .. }
        | C::ComparisonAssignment { .. }
        | C::NullComparisonAssignment { .. }
        | C::VarAttrNullCompareAssignment { .. }
        | C::ComprehensionAssignment { .. }
        | C::EqualsVariable { .. }
        | C::VariableStringOp(_)
        | C::VariableEqualsLiteral { .. }
        | C::VariableNotEqualsLiteral { .. }
        | C::VariableCompare { .. }
        | C::VariableIsNull { .. }
        | C::VariableIsNotNull { .. }
        | C::VariableMembershipTest { .. }
        | C::VariableIsString { .. }
        | C::VariableIsNumber { .. }
        | C::VariableIsBool { .. }
        | C::VariableIsTruthy { .. }
        | C::VariableEqualsVariable { .. }
        | C::VariableNotEqualsVariable { .. }
        | C::VariableMethodWithLiteralArray { .. }
        | C::VariableMethodCompare { .. }
        | C::VariableChainedMethodCompare { .. }
        | C::VariableAttrEqualsLiteral { .. }
        | C::VariableAttrNotEqualsLiteral { .. }
        | C::VariableAttrCompare { .. }
        | C::VariableAttrEqualsNull { .. }
        | C::VariableAttrNotEqualsNull { .. }
        | C::VariableAttrContains { .. } => Dynamic,

        // -- Constants and structure: no specialization opportunity in the
        // node itself. (`Always` is tier-1's business; And/Or/Not are walked
        // through by the callers, never classified.)
        C::Always => Dynamic,
        C::And(_) | C::Or(_) | C::Not(_) => Dynamic,
    }
}

/// Dry-run fitness numbers for the tier-2 specialization overlay
/// (design §6). Aggregatable across rules and policies via [`Self::merge`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpecializationFitness {
    /// Rules analyzed.
    pub total_rules: usize,
    /// Non-constant leaf conditions across all rules.
    pub total_leaves: usize,
    /// Leaves provably data-static today (literal-literal ReBAC).
    pub static_leaves: usize,
    /// Context-anchored leaves — static only under a (not yet existing)
    /// operator-declared static-context config; the hypothetical upper bound.
    pub static_context_leaves: usize,
    /// Rules containing at least one `Static` leaf.
    pub rules_with_static_leaf: usize,
    /// Rules the overlay would actually SHORTEN today: substituting the
    /// `Static` leaves with a constant truth value and running the real
    /// tier-1 fold removes at least one leaf (under the binding guard).
    pub rules_shortenable: usize,
    /// Same simulation with `StaticContext` leaves included — what a
    /// static-context config could additionally unlock.
    pub rules_shortenable_with_static_context: usize,
}

impl SpecializationFitness {
    /// Fold another measurement into this one (for corpus-level totals).
    pub fn merge(&mut self, other: &SpecializationFitness) {
        self.total_rules += other.total_rules;
        self.total_leaves += other.total_leaves;
        self.static_leaves += other.static_leaves;
        self.static_context_leaves += other.static_context_leaves;
        self.rules_with_static_leaf += other.rules_with_static_leaf;
        self.rules_shortenable += other.rules_shortenable;
        self.rules_shortenable_with_static_context += other.rules_shortenable_with_static_context;
    }
}

/// Count the non-constant leaves of a condition tree. Constants (`Always`,
/// `!Always`) count zero — a rule whose condition folds to a constant costs
/// nothing at eval time, which is exactly what "shorter" must measure.
fn leaf_count(cond: &CompiledCondition) -> usize {
    match cond {
        CompiledCondition::And(children) | CompiledCondition::Or(children) => {
            children.iter().map(leaf_count).sum()
        }
        CompiledCondition::Not(inner) => leaf_count(inner),
        CompiledCondition::Always => 0,
        _ => 1,
    }
}

/// Replace every qualifying static leaf with the constant `truth`
/// (`Always` / `!Always`), leaving all other nodes intact. The dry run then
/// feeds the result through [`fold_condition`], whose binding guard decides
/// whether the constant may actually be eliminated — the simulation reuses
/// the exact machinery F.4 would, so its "would shorten" answer is the real
/// one, not a reimplementation's.
fn substitute_static_leaves(
    cond: &CompiledCondition,
    include_context: bool,
    truth: bool,
) -> CompiledCondition {
    match cond {
        CompiledCondition::And(children) => CompiledCondition::And(
            children
                .iter()
                .map(|c| substitute_static_leaves(c, include_context, truth))
                .collect(),
        ),
        CompiledCondition::Or(children) => CompiledCondition::Or(
            children
                .iter()
                .map(|c| substitute_static_leaves(c, include_context, truth))
                .collect(),
        ),
        CompiledCondition::Not(inner) => CompiledCondition::Not(Box::new(
            substitute_static_leaves(inner, include_context, truth),
        )),
        leaf => {
            let qualifies = match leaf_staticness(leaf) {
                LeafStaticness::Static => true,
                LeafStaticness::StaticContext => include_context,
                LeafStaticness::Dynamic => false,
            };
            if qualifies {
                if truth {
                    CompiledCondition::Always
                } else {
                    CompiledCondition::Not(Box::new(CompiledCondition::Always))
                }
            } else {
                leaf.clone()
            }
        }
    }
}

/// Would specializing this condition's static leaves make it cheaper?
/// True iff SOME truth assignment (all-true or all-false — static leaves in
/// one rule share few enough shapes that mixed assignments add nothing the
/// bound needs) folds to fewer non-constant leaves than the rule has today.
fn condition_shortens(cond: &CompiledCondition, include_context: bool) -> bool {
    let baseline = leaf_count(cond);
    if baseline == 0 {
        return false;
    }
    [true, false].into_iter().any(|truth| {
        leaf_count(&fold_condition(substitute_static_leaves(
            cond,
            include_context,
            truth,
        ))) < baseline
    })
}

/// Tally one condition's leaves into (total, static, static-context).
fn tally_leaves(cond: &CompiledCondition, tally: &mut (usize, usize, usize)) {
    match cond {
        CompiledCondition::And(children) | CompiledCondition::Or(children) => {
            for child in children {
                tally_leaves(child, tally);
            }
        }
        CompiledCondition::Not(inner) => tally_leaves(inner, tally),
        CompiledCondition::Always => {}
        leaf => {
            tally.0 += 1;
            match leaf_staticness(leaf) {
                LeafStaticness::Static => tally.1 += 1,
                LeafStaticness::StaticContext => tally.2 += 1,
                LeafStaticness::Dynamic => {}
            }
        }
    }
}

/// Measure the tier-2 fitness of a set of compiled rules (design §6). Pure
/// analysis: consults no DataStore, mutates nothing, safe to run anywhere.
pub fn specialization_fitness(rules: &[CompiledRule]) -> SpecializationFitness {
    let mut fitness = SpecializationFitness {
        total_rules: rules.len(),
        ..Default::default()
    };
    for rule in rules {
        let mut tally = (0usize, 0usize, 0usize);
        tally_leaves(&rule.condition, &mut tally);
        fitness.total_leaves += tally.0;
        fitness.static_leaves += tally.1;
        fitness.static_context_leaves += tally.2;
        if tally.1 > 0 {
            fitness.rules_with_static_leaf += 1;
        }
        if condition_shortens(&rule.condition, false) {
            fitness.rules_shortenable += 1;
        }
        if condition_shortens(&rule.condition, true) {
            fitness.rules_shortenable_with_static_context += 1;
        }
    }
    fitness
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluators::reaper_dsl::types::{
        AttrCompareOp, AttributeComparison, CompareTarget, CompiledCompareTarget, CompiledExprType,
        CountCondition, CountOp, CrossEntityComparison, EntityType, NumericOp, StringOp,
        StringOperationCondition, TimeCondition, VariableStringOperationCondition,
        WildcardComparison,
    };

    #[test]
    fn test_compile_attribute_compare() {
        let interner = StringInterner::new();

        let cond = Condition::AttributeCompare(AttributeComparison {
            entity_type: EntityType::User,
            attribute: "age".to_string(),
            op: NumericOp::GreaterEqual,
            target: CompareTarget::LiteralNum(18.0),
        });

        let compiled = compile_condition(&cond, &interner);

        // Should compile to CompiledCondition::AttributeCompare
        if let CompiledCondition::AttributeCompare(comp) = compiled {
            assert!(matches!(comp.entity_type, EntityType::User));
            assert!(matches!(comp.op, NumericOp::GreaterEqual));
            assert!(matches!(comp.target, CompiledCompareTarget::LiteralNum(n) if n == 18.0));
        } else {
            panic!("Expected AttributeCompare, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_string_op() {
        let interner = StringInterner::new();

        let cond = Condition::StringOp(StringOperationCondition {
            entity_type: EntityType::User,
            attribute: "email".to_string(),
            op: StringOp::Contains,
            value: "@company.com".to_string(),
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::StringOp(op) = compiled {
            assert!(matches!(op.entity_type, EntityType::User));
            assert!(matches!(op.op, StringOp::Contains));
            assert_eq!(op.value, "@company.com");
        } else {
            panic!("Expected StringOp, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_count_op() {
        let interner = StringInterner::new();

        let cond = Condition::CountOp(CountCondition {
            entity_type: EntityType::Resource,
            attribute: "items".to_string(),
            op: CountOp::Greater,
            threshold: 10,
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::CountOp(c) = compiled {
            assert!(matches!(c.entity_type, EntityType::Resource));
            assert!(matches!(c.op, CountOp::Greater));
            assert_eq!(c.threshold, 10);
        } else {
            panic!("Expected CountOp, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_cross_entity_compare() {
        let interner = StringInterner::new();

        let cond = Condition::CrossEntityCompare(CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "level".to_string(),
            op: NumericOp::Greater,
            right_entity: EntityType::Resource,
            right_attr: "required_level".to_string(),
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::CrossEntityCompare(comp) = compiled {
            assert!(matches!(comp.left_entity, EntityType::User));
            assert!(matches!(comp.right_entity, EntityType::Resource));
            assert!(matches!(comp.op, NumericOp::Greater));
        } else {
            panic!("Expected CrossEntityCompare, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_wildcard_compare() {
        let interner = StringInterner::new();

        let cond = Condition::WildcardCompare(WildcardComparison {
            collection_entity: EntityType::User,
            collection_attr: "roles".to_string(),
            scalar_entity: EntityType::Resource,
            scalar_attr: "required_role".to_string(),
            negated: false,
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::WildcardCompare(comp) = compiled {
            assert!(matches!(comp.collection_entity, EntityType::User));
            assert!(matches!(comp.scalar_entity, EntityType::Resource));
        } else {
            panic!("Expected WildcardCompare, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_time_op() {
        let interner = StringInterner::new();

        let cond = Condition::TimeOp(TimeCondition {
            entity_type: EntityType::User,
            attribute: "expires_at".to_string(),
            op: NumericOp::Greater,
            threshold: 1700000000,
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::TimeOp(c) = compiled {
            assert!(matches!(c.entity_type, EntityType::User));
            assert!(matches!(c.op, NumericOp::Greater));
            assert_eq!(c.threshold, 1700000000);
        } else {
            panic!("Expected TimeOp, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_variable_string_op() {
        let interner = StringInterner::new();

        let cond = Condition::VariableStringOp(VariableStringOperationCondition {
            variable: "email".to_string(),
            op: StringOp::EndsWith,
            value: ".com".to_string(),
        });

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::VariableStringOp(op) = compiled {
            assert!(matches!(op.op, StringOp::EndsWith));
            assert_eq!(op.value, ".com");
        } else {
            panic!("Expected VariableStringOp, got {:?}", compiled);
        }
    }

    #[test]
    fn test_compile_and_with_nested() {
        let interner = StringInterner::new();

        // Test And with consolidated variants
        let cond = Condition::And(vec![
            Condition::AttributeCompare(AttributeComparison {
                entity_type: EntityType::User,
                attribute: "age".to_string(),
                op: NumericOp::GreaterEqual,
                target: CompareTarget::LiteralNum(18.0),
            }),
            Condition::StringOp(StringOperationCondition {
                entity_type: EntityType::User,
                attribute: "email".to_string(),
                op: StringOp::Contains,
                value: "@".to_string(),
            }),
        ]);

        let compiled = compile_condition(&cond, &interner);

        if let CompiledCondition::And(conditions) = compiled {
            assert_eq!(conditions.len(), 2);
            assert!(matches!(
                conditions[0],
                CompiledCondition::AttributeCompare(_)
            ));
            assert!(matches!(conditions[1], CompiledCondition::StringOp(_)));
        } else {
            panic!("Expected And, got {:?}", compiled);
        }
    }

    // =======================================================================
    // Tier-1 partial evaluation: fold_condition
    // =======================================================================

    fn t() -> CompiledCondition {
        CompiledCondition::Always
    }
    fn f() -> CompiledCondition {
        CompiledCondition::Not(Box::new(CompiledCondition::Always))
    }
    /// A non-constant, non-binding leaf.
    fn leaf(interner: &StringInterner, v: &str) -> CompiledCondition {
        CompiledCondition::ResourceIdEquals {
            value: interner.intern(v),
        }
    }
    /// A binding condition (writes the rule-scoped `variables` map).
    fn binding(interner: &StringInterner) -> CompiledCondition {
        CompiledCondition::Assignment {
            variable: interner.intern("x"),
            entity_type: EntityType::User,
            attribute: interner.intern("role"),
            index: None,
        }
    }

    #[test]
    fn test_fold_double_negation() {
        let i = StringInterner::new();
        let folded = fold_condition(CompiledCondition::Not(Box::new(CompiledCondition::Not(
            Box::new(leaf(&i, "r")),
        ))));
        assert!(matches!(folded, CompiledCondition::ResourceIdEquals { .. }));
        // But Not(Always) (canonical false) must NOT unwrap to Always.
        assert!(is_false(&fold_condition(f())));
    }

    #[test]
    fn test_fold_and_drops_true_and_unwraps() {
        let i = StringInterner::new();
        // true && r  →  r
        let folded = fold_condition(CompiledCondition::And(vec![t(), leaf(&i, "r")]));
        assert!(matches!(folded, CompiledCondition::ResourceIdEquals { .. }));
        // true && true  →  true
        assert!(matches!(
            fold_condition(CompiledCondition::And(vec![t(), t()])),
            CompiledCondition::Always
        ));
    }

    #[test]
    fn test_fold_and_false_short_circuits_without_bindings() {
        let i = StringInterner::new();
        // r && false  →  false (no bindings anywhere in the conjunction)
        let folded = fold_condition(CompiledCondition::And(vec![leaf(&i, "r"), f()]));
        assert!(is_false(&folded));
    }

    #[test]
    fn test_fold_and_false_kept_when_sibling_binds() {
        let i = StringInterner::new();
        // (let x = user.role) && false: the assignment must still evaluate
        // (a later Or branch of the same rule may read x), so the false child
        // is NOT allowed to erase the conjunction.
        let folded = fold_condition(CompiledCondition::And(vec![binding(&i), f()]));
        let CompiledCondition::And(children) = folded else {
            panic!("binding conjunction must not be erased");
        };
        assert_eq!(children.len(), 2);
        assert!(binds_variables(&children[0]));
        assert!(is_false(&children[1]));
    }

    #[test]
    fn test_fold_or_drops_false_and_unwraps() {
        let i = StringInterner::new();
        // false || r  →  r
        let folded = fold_condition(CompiledCondition::Or(vec![f(), leaf(&i, "r")]));
        assert!(matches!(folded, CompiledCondition::ResourceIdEquals { .. }));
        // false || false  →  false
        assert!(is_false(&fold_condition(CompiledCondition::Or(vec![
            f(),
            f()
        ]))));
    }

    #[test]
    fn test_fold_or_true_short_circuits_without_bindings() {
        let i = StringInterner::new();
        // r || true  →  true
        assert!(matches!(
            fold_condition(CompiledCondition::Or(vec![leaf(&i, "r"), t()])),
            CompiledCondition::Always
        ));
    }

    #[test]
    fn test_fold_or_true_kept_when_sibling_binds() {
        let i = StringInterner::new();
        // (let x = user.role) || true: folding to `true` would skip the
        // binding the runtime performs (Or evaluates left-to-right), so the
        // disjunction must survive.
        let folded = fold_condition(CompiledCondition::Or(vec![binding(&i), t()]));
        assert!(matches!(folded, CompiledCondition::Or(_)));
    }

    #[test]
    fn test_fold_flattens_nested_same_operator() {
        let i = StringInterner::new();
        // (a && b) && c  →  And(a, b, c), order preserved.
        let folded = fold_condition(CompiledCondition::And(vec![
            CompiledCondition::And(vec![leaf(&i, "a"), leaf(&i, "b")]),
            leaf(&i, "c"),
        ]));
        let CompiledCondition::And(children) = folded else {
            panic!("expected flattened And");
        };
        let names: Vec<String> = children
            .iter()
            .map(|c| match c {
                CompiledCondition::ResourceIdEquals { value } => {
                    i.resolve(*value).unwrap().to_string()
                }
                other => panic!("unexpected child {other:?}"),
            })
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn test_fold_propagates_bottom_up() {
        let i = StringInterner::new();
        // (false || true) && r  →  r  (inner Or folds to true, then drops).
        let folded = fold_condition(CompiledCondition::And(vec![
            CompiledCondition::Or(vec![f(), t()]),
            leaf(&i, "r"),
        ]));
        assert!(matches!(folded, CompiledCondition::ResourceIdEquals { .. }));
    }

    #[test]
    fn test_binds_variables_covers_every_assignment_variant() {
        // Exhaustiveness pin: every variant whose evaluation writes the
        // `variables` map must be flagged. If a new binding variant is added
        // to CompiledCondition without updating `binds_variables`, the
        // eliminating folds become unsound — extend BOTH, then this list.
        let i = StringInterner::new();
        let v = i.intern("x");
        let a = i.intern("attr");
        let all_binding: Vec<CompiledCondition> = vec![
            CompiledCondition::Assignment {
                variable: v,
                entity_type: EntityType::User,
                attribute: a,
                index: None,
            },
            CompiledCondition::ExpressionAssignment {
                variable: v,
                expr_type: CompiledExprType::CollectionCount {
                    entity_type: EntityType::User,
                    attribute: a,
                },
            },
            CompiledCondition::ExprCompareAssignment {
                variable: v,
                expr_type: CompiledExprType::CollectionCount {
                    entity_type: EntityType::User,
                    attribute: a,
                },
                op: AttrCompareOp::Equal,
                value: CompiledLiteralValue::Int(1),
            },
            CompiledCondition::ComparisonAssignment {
                variable: v,
                entity_type: EntityType::User,
                attribute: a,
                op: AttrCompareOp::Equal,
                value: CompiledLiteralValue::Int(1),
            },
            CompiledCondition::NullComparisonAssignment {
                variable: v,
                entity_type: EntityType::User,
                attribute: a,
                is_null_check: true,
            },
            CompiledCondition::VarAttrNullCompareAssignment {
                result_variable: v,
                source_variable: v,
                attribute: a,
                is_null_check: true,
            },
            CompiledCondition::ComprehensionAssignment {
                variable: v,
                comprehension: Box::new(CompiledComprehension {
                    comp_type: ComprehensionType::Array,
                    iterator: CompiledIterator {
                        variable: v,
                        source: CompiledIterationSource::EntityAttr {
                            entity_type: EntityType::User,
                            attribute: a,
                        },
                    },
                    filters: vec![],
                    output: Some(CompiledOutput::Variable(v)),
                    key_value: None,
                }),
            },
        ];
        for cond in &all_binding {
            assert!(
                binds_variables(cond),
                "binding variant not flagged: {cond:?}"
            );
            // And nested under logic operators.
            assert!(binds_variables(&CompiledCondition::And(vec![
                CompiledCondition::Always,
                cond.clone()
            ])));
        }
        // Non-binding leaves stay unflagged.
        assert!(!binds_variables(&leaf(&i, "r")));
        assert!(!binds_variables(&t()));
    }

    // =======================================================================
    // Tier-2 partial evaluation, F.3 dry run: leaf_staticness + fitness
    // =======================================================================

    use crate::evaluators::reaper_dsl::types::{CompiledAttributeComparison, CompiledRebacRef};
    use crate::PolicyAction;

    /// The one provably data-static shape: ReBAC with both refs literal.
    fn static_rebac(interner: &StringInterner) -> CompiledCondition {
        CompiledCondition::RebacCheck {
            kind: crate::evaluators::reaper_dsl::types::RebacKind::Direct,
            subject: CompiledRebacRef::Literal(interner.intern("team_a")),
            relation: interner.intern("owns"),
            object: CompiledRebacRef::Literal(interner.intern("repo_1")),
            via: None,
            max_depth: 1,
        }
    }

    fn context_leaf(interner: &StringInterner) -> CompiledCondition {
        CompiledCondition::AttributeCompare(CompiledAttributeComparison {
            entity_type: EntityType::Context,
            attribute: interner.intern("region"),
            op: NumericOp::Equal,
            target: CompiledCompareTarget::LiteralString(interner.intern("eu")),
        })
    }

    fn rule(cond: CompiledCondition) -> CompiledRule {
        CompiledRule {
            name: "r".into(),
            condition: cond,
            decision: PolicyAction::Allow,
        }
    }

    #[test]
    fn test_leaf_staticness_classes() {
        let i = StringInterner::new();

        // Literal-literal ReBAC: the only Static shape today.
        assert_eq!(leaf_staticness(&static_rebac(&i)), LeafStaticness::Static);

        // ReBAC with a request-bound ref: dynamic.
        let dynamic_rebac = CompiledCondition::RebacCheck {
            kind: crate::evaluators::reaper_dsl::types::RebacKind::Direct,
            subject: CompiledRebacRef::Principal,
            relation: i.intern("owns"),
            object: CompiledRebacRef::Literal(i.intern("repo_1")),
            via: None,
            max_depth: 1,
        };
        assert_eq!(leaf_staticness(&dynamic_rebac), LeafStaticness::Dynamic);

        // Context-anchored comparison against a literal: static only under a
        // declared static context.
        assert_eq!(
            leaf_staticness(&context_leaf(&i)),
            LeafStaticness::StaticContext
        );

        // Context compared against a USER attribute: the target drags in the
        // request — dynamic.
        let ctx_vs_user = CompiledCondition::AttributeCompare(CompiledAttributeComparison {
            entity_type: EntityType::Context,
            attribute: i.intern("region"),
            op: NumericOp::Equal,
            target: CompiledCompareTarget::EntityAttr {
                entity_type: EntityType::User,
                attribute: i.intern("region"),
            },
        });
        assert_eq!(leaf_staticness(&ctx_vs_user), LeafStaticness::Dynamic);

        // Request-intrinsic leaves.
        assert_eq!(leaf_staticness(&leaf(&i, "r")), LeafStaticness::Dynamic);
        assert_eq!(
            leaf_staticness(&CompiledCondition::TaintTrusted { key: "k".into() }),
            LeafStaticness::Dynamic
        );
        // Bindings are never static (side effects).
        assert_eq!(leaf_staticness(&binding(&i)), LeafStaticness::Dynamic);
    }

    #[test]
    fn test_fitness_counts_and_shortening() {
        let i = StringInterner::new();

        // Rule 1: static_rebac && dynamic — substituting the static leaf
        // with a constant and folding drops it (true) or collapses the And
        // (false): shortenable today.
        let r1 = rule(CompiledCondition::And(vec![
            static_rebac(&i),
            leaf(&i, "doc1"),
        ]));
        // Rule 2: purely dynamic — nothing to specialize.
        let r2 = rule(CompiledCondition::And(vec![
            leaf(&i, "doc2"),
            leaf(&i, "doc3"),
        ]));
        // Rule 3: context leaf && dynamic — shortenable only under the
        // hypothetical static-context config.
        let r3 = rule(CompiledCondition::And(vec![
            context_leaf(&i),
            leaf(&i, "doc4"),
        ]));

        let fitness = specialization_fitness(&[r1, r2, r3]);
        assert_eq!(fitness.total_rules, 3);
        assert_eq!(fitness.total_leaves, 6);
        assert_eq!(fitness.static_leaves, 1);
        assert_eq!(fitness.static_context_leaves, 1);
        assert_eq!(fitness.rules_with_static_leaf, 1);
        assert_eq!(fitness.rules_shortenable, 1);
        // Context inclusion unlocks rule 3 IN ADDITION to rule 1.
        assert_eq!(fitness.rules_shortenable_with_static_context, 2);
    }

    #[test]
    fn test_shortening_respects_the_binding_guard() {
        let i = StringInterner::new();
        // (let x = user.role) && static_rebac: substituting the rebac with
        // `false` may NOT erase the conjunction (the binding must still
        // evaluate) — the guard inside fold_condition enforces it. With
        // `true` the rebac conjunct IS droppable (dropping a true conjunct
        // skips no evaluation semantics the runtime needs: the binding still
        // runs). So the rule is shortenable — but only via the sound path.
        let cond = CompiledCondition::And(vec![binding(&i), static_rebac(&i)]);

        // Direct check of the unsound direction: false-substitution + fold
        // must keep the binding.
        let false_sub = fold_condition(substitute_static_leaves(&cond, false, false));
        let CompiledCondition::And(children) = &false_sub else {
            panic!("binding conjunction must survive false-substitution");
        };
        assert!(binds_variables(&children[0]));

        // And the aggregate still reports it shortenable (via true).
        let fitness = specialization_fitness(&[rule(cond)]);
        assert_eq!(fitness.rules_shortenable, 1);
    }

    #[test]
    fn test_fitness_single_static_leaf_rule_counts_as_shortenable() {
        // A rule that is NOTHING BUT a static leaf specializes to a
        // constant: leaf_count 1 → 0. That is the biggest win available and
        // must register as shortening.
        let i = StringInterner::new();
        let fitness = specialization_fitness(&[rule(static_rebac(&i))]);
        assert_eq!(fitness.total_leaves, 1);
        assert_eq!(fitness.rules_shortenable, 1);
    }

    #[test]
    fn test_fitness_static_leaf_under_or_shortens() {
        let i = StringInterner::new();
        // static || dynamic: false-substitution drops the disjunct;
        // true-substitution folds the Or to true. Either way shorter.
        let fitness = specialization_fitness(&[rule(CompiledCondition::Or(vec![
            static_rebac(&i),
            leaf(&i, "doc"),
        ]))]);
        assert_eq!(fitness.rules_shortenable, 1);
    }
}
