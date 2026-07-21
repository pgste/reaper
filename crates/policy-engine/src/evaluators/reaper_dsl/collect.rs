//! Collection utilities for Reaper DSL compiler.
//!
//! These functions recursively collect and pre-process data from conditions
//! for optimization during policy compilation:
//! - Regex patterns: Pre-compile regex for evaluation
//! - Strings for interning: Pre-intern strings for O(1) lookup
//! - Membership values: Pre-compute AttributeValue for membership tests

use super::types::{
    ChainMethod, CompareTarget, Condition, ExprIndexType, ExprType, LiteralValue,
    UncompiledIterationSource, UncompiledOutput,
};
use crate::data::{AttributeValue, InternedString, StringInterner};
use rustc_hash::FxHashMap;

/// Recursively collect and compile regex patterns from a condition
pub fn collect_regex_patterns(condition: &Condition, cache: &mut FxHashMap<String, regex::Regex>) {
    match condition {
        Condition::RegexMatches { pattern, .. } => {
            if !cache.contains_key(pattern) {
                if let Ok(re) = regex::Regex::new(pattern) {
                    cache.insert(pattern.clone(), re);
                }
            }
        }
        Condition::And(conditions) | Condition::Or(conditions) => {
            for c in conditions {
                collect_regex_patterns(c, cache);
            }
        }
        Condition::Not(inner) => {
            collect_regex_patterns(inner, cache);
        }
        _ => {} // Other conditions don't have regex patterns
    }
}

/// Recursively collect and pre-intern all strings from a condition
/// This includes attribute names and string literals for O(1) lookup during evaluation
#[allow(dead_code)]
pub fn collect_strings_for_interning(
    condition: &Condition,
    cache: &mut FxHashMap<String, InternedString>,
    interner: &StringInterner,
) {
    // Helper to intern and cache a string
    let mut intern = |s: &String| {
        if !cache.contains_key(s) {
            cache.insert(s.clone(), interner.intern(s));
        }
    };

    match condition {
        Condition::ActionEquals { value } => intern(value),

        // Taint keys look up the request's provenance map by raw string —
        // nothing to pre-intern.
        Condition::TaintTrusted { .. } => {}

        // Input paths/literals are raw request-document strings — nothing to
        // pre-intern (design: input values never touch the interner).
        Condition::InputCompare { .. } => {}

        // Raw-string comparison value; variable/attribute intern at compile.
        Condition::VariableAttrStringOp { .. } => {}
        // Literal interns via compile_literal; names intern at compile.
        Condition::VariableAttrMembershipTest { .. } => {}

        // Pre-intern rebac strings so compilation is alloc-free at eval time.
        Condition::RebacCheck {
            subject,
            relation,
            object,
            via,
            ..
        } => {
            use crate::evaluators::reaper_dsl::RebacRef;
            intern(relation);
            if let Some(v) = via {
                intern(v);
            }
            for r in [subject, object] {
                if let RebacRef::Literal(id) = r {
                    intern(id);
                }
            }
        }
        Condition::ResourceIdEquals { value } => intern(value),

        // ============ Consolidated Types ============
        Condition::AttributeCompare(comp) => {
            intern(&comp.attribute);
            match &comp.target {
                CompareTarget::LiteralString(s) => intern(s),
                CompareTarget::EntityAttr { attribute, .. } => intern(attribute),
                CompareTarget::Variable(v) => intern(v),
                _ => {}
            }
        }
        Condition::StringOp(op) => {
            intern(&op.attribute);
            intern(&op.value);
        }
        Condition::VariableStringOp(op) => {
            intern(&op.variable);
            intern(&op.value);
        }
        Condition::CountOp(cond) => {
            intern(&cond.attribute);
        }
        Condition::TimeOp(cond) => {
            intern(&cond.attribute);
        }
        Condition::CrossEntityCompare(comp) => {
            intern(&comp.left_attr);
            intern(&comp.right_attr);
        }
        Condition::WildcardCompare(comp) => {
            intern(&comp.collection_attr);
            intern(&comp.scalar_attr);
        }

        // ============ Other Variants ============
        Condition::Assignment {
            variable,
            attribute,
            ..
        } => {
            intern(variable);
            intern(attribute);
        }
        Condition::MembershipTest {
            attribute, value, ..
        } => {
            intern(attribute);
            // Also pre-intern the literal value for membership test
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::IndexedEquals {
            attribute, value, ..
        } => {
            intern(attribute);
            intern(value);
        }
        Condition::EqualsVariable {
            attribute,
            variable,
            ..
        } => {
            intern(attribute);
            intern(variable);
        }
        Condition::RegexMatches { attribute, .. } => {
            intern(attribute);
        }
        Condition::ObjectHasKey { attribute, key, .. } => {
            intern(attribute);
            intern(key);
        }
        Condition::CollectionAny { attribute, .. } | Condition::CollectionAll { attribute, .. } => {
            intern(attribute);
        }
        // Type check functions
        Condition::IsString { attribute, .. }
        | Condition::IsNumber { attribute, .. }
        | Condition::IsBool { attribute, .. } => {
            intern(attribute);
        }
        // Set operations
        Condition::SetIntersectionCountGreater {
            attribute, values, ..
        } => {
            intern(attribute);
            for v in values {
                intern(v);
            }
        }
        Condition::MapKeyExists { attribute, key, .. } => {
            intern(attribute);
            intern(key);
        }
        // Comprehensions
        Condition::ComprehensionCountGreaterEqual {
            attribute,
            filter_attr,
            ..
        }
        | Condition::ComprehensionCountEqual {
            attribute,
            filter_attr,
            ..
        } => {
            intern(attribute);
            intern(filter_attr);
        }
        // Same-entity attribute comparisons
        Condition::SameEntityAttrCompare {
            left_attr,
            right_attr,
            ..
        } => {
            intern(left_attr);
            intern(right_attr);
        }
        // Expression assignment
        Condition::ExpressionAssignment {
            variable,
            expr_type,
        } => {
            intern(variable);
            collect_expr_type_strings(expr_type, cache, interner);
        }
        // Expression comparison assignment
        Condition::ExprCompareAssignment {
            variable,
            expr_type,
            value,
            ..
        } => {
            intern(variable);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
            // Must be called last after all intern() calls since it also borrows cache
            collect_expr_type_strings(expr_type, cache, interner);
        }
        // Variable comparisons
        Condition::VariableEqualsLiteral { variable, value }
        | Condition::VariableNotEqualsLiteral { variable, value } => {
            intern(variable);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::VariableCompare {
            variable, value, ..
        } => {
            intern(variable);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::VariableIsNull { variable } | Condition::VariableIsNotNull { variable } => {
            intern(variable);
        }
        Condition::ComparisonAssignment {
            variable,
            attribute,
            value,
            ..
        } => {
            intern(variable);
            intern(attribute);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::NullComparisonAssignment {
            variable,
            attribute,
            ..
        } => {
            intern(variable);
            intern(attribute);
        }
        Condition::VariableMembershipTest { value, variable } => {
            if let LiteralValue::String(s) = value {
                intern(s);
            }
            intern(variable);
        }
        Condition::VariableIsString { variable }
        | Condition::VariableIsNumber { variable }
        | Condition::VariableIsBool { variable }
        | Condition::VariableIsTruthy { variable } => {
            intern(variable);
        }
        Condition::VariableEqualsVariable { left, right }
        | Condition::VariableNotEqualsVariable { left, right } => {
            intern(left);
            intern(right);
        }
        Condition::VariableMethodWithLiteralArray {
            variable, values, ..
        } => {
            intern(variable);
            for v in values {
                intern(v);
            }
        }
        Condition::VariableMethodCompare {
            variable, value, ..
        } => {
            intern(variable);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::VariableChainedMethodCompare {
            variable, value, ..
        } => {
            intern(variable);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        // Variable attribute comparisons (for comprehension filters)
        Condition::VariableAttrEqualsLiteral {
            variable,
            attribute,
            value,
        }
        | Condition::VariableAttrNotEqualsLiteral {
            variable,
            attribute,
            value,
        } => {
            intern(variable);
            intern(attribute);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::VariableAttrCompare {
            variable,
            attribute,
            value,
            ..
        } => {
            intern(variable);
            intern(attribute);
            if let LiteralValue::String(s) = value {
                intern(s);
            }
        }
        Condition::VariableAttrEqualsNull {
            variable,
            attribute,
        }
        | Condition::VariableAttrNotEqualsNull {
            variable,
            attribute,
        } => {
            intern(variable);
            intern(attribute);
        }
        Condition::VarAttrNullCompareAssignment {
            result_variable,
            source_variable,
            attribute,
            ..
        } => {
            intern(result_variable);
            intern(source_variable);
            intern(attribute);
        }
        Condition::VariableAttrContains {
            variable,
            attribute,
            substring,
        } => {
            intern(variable);
            intern(attribute);
            intern(substring);
        }
        // Comprehension assignment
        Condition::ComprehensionAssignment {
            variable,
            iterator_var,
            iterator_source,
            filters,
            output,
            key_output,
            ..
        } => {
            intern(variable);
            intern(iterator_var);
            // Collect iterator source strings
            match iterator_source {
                UncompiledIterationSource::EntityAttr { attribute, .. } => {
                    intern(attribute);
                }
                UncompiledIterationSource::Variable { variable: v } => {
                    intern(v);
                }
                // Input paths are raw document keys — nothing to pre-intern.
                UncompiledIterationSource::Input { .. } => {}
            }
            // Collect output strings
            if let Some(out) = output {
                collect_output_strings(out, cache, interner);
            }
            // Collect key output strings for object comprehensions
            if let Some(key) = key_output {
                collect_output_strings(key, cache, interner);
            }
            for f in filters {
                collect_strings_for_interning(f, cache, interner);
            }
        }
        Condition::And(conditions) | Condition::Or(conditions) => {
            for c in conditions {
                collect_strings_for_interning(c, cache, interner);
            }
        }
        Condition::Not(inner) => {
            collect_strings_for_interning(inner, cache, interner);
        }
        Condition::Always => {}
    }
}

/// Collect strings from expression type
fn collect_expr_type_strings(
    expr_type: &ExprType,
    cache: &mut FxHashMap<String, InternedString>,
    interner: &StringInterner,
) {
    let mut intern = |s: &String| {
        if !cache.contains_key(s) {
            cache.insert(s.clone(), interner.intern(s));
        }
    };

    match expr_type {
        ExprType::StringLower { attribute, .. }
        | ExprType::StringUpper { attribute, .. }
        | ExprType::StringTrim { attribute, .. }
        | ExprType::CollectionCount { attribute, .. }
        | ExprType::CollectionSum { attribute, .. }
        | ExprType::CollectionMax { attribute, .. }
        | ExprType::CollectionMin { attribute, .. }
        | ExprType::CollectionFirst { attribute, .. }
        | ExprType::CollectionLast { attribute, .. }
        | ExprType::CollectionReverse { attribute, .. }
        | ExprType::CollectionSort { attribute, .. }
        | ExprType::CollectionUnique { attribute, .. }
        | ExprType::SetKeys { attribute, .. }
        | ExprType::SetValues { attribute, .. } => {
            intern(attribute);
        }
        ExprType::CollectionSlice { attribute, .. } => {
            intern(attribute);
        }
        ExprType::CollectionDifference {
            attribute,
            other_attribute,
            ..
        }
        | ExprType::CollectionUnion {
            attribute,
            other_attribute,
            ..
        }
        | ExprType::CollectionIntersection {
            attribute,
            other_attribute,
            ..
        } => {
            intern(attribute);
            intern(other_attribute);
        }
        ExprType::StringSplit {
            attribute,
            delimiter,
            ..
        } => {
            intern(attribute);
            intern(delimiter);
        }
        ExprType::SetIntersection {
            attribute, values, ..
        }
        | ExprType::SetUnion {
            attribute, values, ..
        }
        | ExprType::SetDifference {
            attribute, values, ..
        } => {
            intern(attribute);
            for v in values {
                intern(v);
            }
        }
        ExprType::StringContains { attribute, .. } => {
            intern(attribute);
        }
        ExprType::StringStartsWithExpr { attribute, .. } => {
            intern(attribute);
        }
        ExprType::StringEndsWithExpr { attribute, .. } => {
            intern(attribute);
        }
        ExprType::RegexMatches { attribute, .. } => {
            intern(attribute);
        }
        ExprType::RegexFind { attribute, .. } => {
            intern(attribute);
        }
        ExprType::RegexFindAll { attribute, .. } => {
            intern(attribute);
        }
        ExprType::StringReplace { attribute, .. } => {
            intern(attribute);
        }
        ExprType::ChainedMethod { base, method } => {
            // First collect strings from chain method values (before recursive call)
            match method {
                ChainMethod::Intersection { values }
                | ChainMethod::Union { values }
                | ChainMethod::Difference { values } => {
                    for v in values {
                        if !cache.contains_key(v) {
                            cache.insert(v.clone(), interner.intern(v));
                        }
                    }
                }
                _ => {}
            }
            // Then recurse into base
            collect_expr_type_strings(base, cache, interner);
        }
        ExprType::VariableRef { variable } => {
            intern(variable);
        }
        ExprType::VariableIndexed { variable, index } => {
            intern(variable);
            if let ExprIndexType::String(s) = index {
                intern(s);
            }
        }
        ExprType::VariableAttrAccess {
            variable,
            attribute,
        } => {
            intern(variable);
            intern(attribute);
        }
        ExprType::VariableAttrIndexed {
            variable,
            attribute,
            index,
        } => {
            intern(variable);
            intern(attribute);
            if let ExprIndexType::String(s) = index {
                intern(s);
            }
        }
        ExprType::TimeNow | ExprType::TimeNowMs | ExprType::TimeNowNs => {}

        // Taint keys look up the request's provenance map by raw string —
        // nothing to pre-intern.
        ExprType::TaintLevel { .. } => {}

        // Literals intern (string case) inside compile_expr_type itself —
        // nothing to pre-collect here.
        ExprType::Literal { .. } => {}

        // Input reads navigate the raw JSON document by string key; values
        // materialize with TRANSIENT interning at eval — nothing to pin.
        ExprType::InputRead { .. } => {}
    }
}

/// Collect strings from comprehension output expressions
fn collect_output_strings(
    output: &UncompiledOutput,
    cache: &mut FxHashMap<String, InternedString>,
    interner: &StringInterner,
) {
    let mut intern = |s: &String| {
        if !cache.contains_key(s) {
            cache.insert(s.clone(), interner.intern(s));
        }
    };

    match output {
        UncompiledOutput::Variable(var) => {
            intern(var);
        }
        UncompiledOutput::VarAttr {
            variable,
            attribute,
        } => {
            intern(variable);
            intern(attribute);
        }
        UncompiledOutput::Literal(lit) => {
            if let LiteralValue::String(s) = lit {
                intern(s);
            }
        }
        UncompiledOutput::VarMethodCall { variable, .. } => {
            intern(variable);
        }
    }
}

/// Recursively collect and pre-compute AttributeValue objects for membership tests
/// This avoids allocating AttributeValue::String during evaluation
/// Only caches String values since Int/Bool are Copy types (no allocation)
pub fn collect_membership_values(
    condition: &Condition,
    cache: &mut FxHashMap<String, AttributeValue>,
    interner: &StringInterner,
) {
    match condition {
        Condition::MembershipTest {
            value: LiteralValue::String(s),
            ..
        } => {
            // Only pre-compute String values (Int/Bool are Copy types)
            if !cache.contains_key(s) {
                let interned = interner.intern(s);
                cache.insert(s.clone(), AttributeValue::String(interned));
            }
        }
        Condition::MembershipTest { .. } => {
            // Int/Bool are Copy types, no pre-computation needed
        }
        Condition::And(conditions) | Condition::Or(conditions) => {
            for c in conditions {
                collect_membership_values(c, cache, interner);
            }
        }
        Condition::Not(inner) => {
            collect_membership_values(inner, cache, interner);
        }
        _ => {} // Other conditions don't have membership tests
    }
}
