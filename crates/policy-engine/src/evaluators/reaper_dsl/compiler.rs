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
    CompiledComprehension,
    CompiledCondition,
    CompiledIterationSource,
    CompiledIterator,
    CompiledLiteralValue,
    CompiledOutput,
    // Compiled consolidated types
    CompiledRegexMatch,
    ComprehensionType,
    // Uncompiled types
    Condition,
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
}
