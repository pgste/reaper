//! Expression type compilation.
//!
//! This module compiles ExprType AST nodes into CompiledExprType with pre-interned strings.
//! This happens once at construction time, not during evaluation.

use super::types::{
    ChainMethod, CompiledChainMethod, CompiledExprIndexType, CompiledExprType, ExprIndexType,
    ExprType,
};
use crate::data::StringInterner;

/// Compile expression type with pre-interned strings
pub(super) fn compile_expr_type(
    expr_type: &ExprType,
    interner: &StringInterner,
) -> CompiledExprType {
    match expr_type {
        // Taint: raw-String key, looks up the request provenance at eval.
        ExprType::TaintLevel { key } => CompiledExprType::TaintLevel { key: key.clone() },

        // Scalar literal (R4-01 A.3): string literals are POLICY text, so
        // interning (pinning) them at compile is correct — same rule as
        // every other compiled literal.
        ExprType::Literal { value } => CompiledExprType::Literal {
            value: super::compiler::compile_literal(value, interner),
        },

        // Input read (R4-01 B.3): pre-parse the dotted path once at compile.
        // Path keys navigate raw serde_json objects at eval — nothing to
        // intern here.
        ExprType::InputRead { path } => CompiledExprType::InputRead {
            path: super::InputPath::from_dotted(path),
        },

        ExprType::StringLower {
            entity_type,
            attribute,
        } => CompiledExprType::StringLower {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::StringUpper {
            entity_type,
            attribute,
        } => CompiledExprType::StringUpper {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::StringTrim {
            entity_type,
            attribute,
        } => CompiledExprType::StringTrim {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::StringSplit {
            entity_type,
            attribute,
            delimiter,
        } => CompiledExprType::StringSplit {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            delimiter: delimiter.clone(),
        },
        ExprType::CollectionCount {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionCount {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionSum {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionSum {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionMax {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionMax {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionMin {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionMin {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionFirst {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionFirst {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionLast {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionLast {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionSlice {
            entity_type,
            attribute,
            start,
            end,
        } => CompiledExprType::CollectionSlice {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            start: *start,
            end: *end,
        },
        ExprType::CollectionReverse {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionReverse {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionSort {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionSort {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionUnique {
            entity_type,
            attribute,
        } => CompiledExprType::CollectionUnique {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::CollectionDifference {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => CompiledExprType::CollectionDifference {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            other_entity_type: other_entity_type.clone(),
            other_attribute: interner.intern(other_attribute),
        },
        ExprType::CollectionUnion {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => CompiledExprType::CollectionUnion {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            other_entity_type: other_entity_type.clone(),
            other_attribute: interner.intern(other_attribute),
        },
        ExprType::CollectionIntersection {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => CompiledExprType::CollectionIntersection {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            other_entity_type: other_entity_type.clone(),
            other_attribute: interner.intern(other_attribute),
        },
        ExprType::SetIntersection {
            entity_type,
            attribute,
            values,
        } => CompiledExprType::SetIntersection {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        ExprType::SetUnion {
            entity_type,
            attribute,
            values,
        } => CompiledExprType::SetUnion {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        ExprType::SetDifference {
            entity_type,
            attribute,
            values,
        } => CompiledExprType::SetDifference {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        ExprType::SetKeys {
            entity_type,
            attribute,
        } => CompiledExprType::SetKeys {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::SetValues {
            entity_type,
            attribute,
        } => CompiledExprType::SetValues {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
        },
        ExprType::TimeNow => CompiledExprType::TimeNow,
        ExprType::TimeNowMs => CompiledExprType::TimeNowMs,
        ExprType::TimeNowNs => CompiledExprType::TimeNowNs,
        ExprType::StringContains {
            entity_type,
            attribute,
            substring,
        } => CompiledExprType::StringContains {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            substring: substring.clone(),
        },
        ExprType::StringStartsWithExpr {
            entity_type,
            attribute,
            prefix,
        } => CompiledExprType::StringStartsWithExpr {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            prefix: prefix.clone(),
        },
        ExprType::StringEndsWithExpr {
            entity_type,
            attribute,
            suffix,
        } => CompiledExprType::StringEndsWithExpr {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            suffix: suffix.clone(),
        },
        ExprType::RegexMatches {
            entity_type,
            attribute,
            pattern,
        } => CompiledExprType::RegexMatches {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            pattern: pattern.clone(),
        },
        ExprType::RegexFind {
            entity_type,
            attribute,
            pattern,
        } => CompiledExprType::RegexFind {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            pattern: pattern.clone(),
        },
        ExprType::RegexFindAll {
            entity_type,
            attribute,
            pattern,
        } => CompiledExprType::RegexFindAll {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            pattern: pattern.clone(),
        },
        ExprType::StringReplace {
            entity_type,
            attribute,
            pattern,
            replacement,
        } => CompiledExprType::StringReplace {
            entity_type: entity_type.clone(),
            attribute: interner.intern(attribute),
            pattern: pattern.clone(),
            replacement: replacement.clone(),
        },
        ExprType::ChainedMethod { base, method } => CompiledExprType::ChainedMethod {
            base: Box::new(compile_expr_type(base, interner)),
            method: compile_chain_method(method, interner),
        },
        ExprType::VariableRef { variable } => CompiledExprType::VariableRef {
            variable: interner.intern(variable),
        },
        ExprType::VariableIndexed { variable, index } => CompiledExprType::VariableIndexed {
            variable: interner.intern(variable),
            index: match index {
                ExprIndexType::Wildcard => CompiledExprIndexType::Wildcard,
                ExprIndexType::Number(n) => CompiledExprIndexType::Number(*n),
                ExprIndexType::String(s) => CompiledExprIndexType::String(interner.intern(s)),
            },
        },
        ExprType::VariableAttrAccess {
            variable,
            attribute,
        } => CompiledExprType::VariableAttrAccess {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
        },
        ExprType::VariableAttrIndexed {
            variable,
            attribute,
            index,
        } => CompiledExprType::VariableAttrIndexed {
            variable: interner.intern(variable),
            attribute: interner.intern(attribute),
            index: match index {
                ExprIndexType::Wildcard => CompiledExprIndexType::Wildcard,
                ExprIndexType::Number(n) => CompiledExprIndexType::Number(*n),
                ExprIndexType::String(s) => CompiledExprIndexType::String(interner.intern(s)),
            },
        },
    }
}

/// Compile chain method
pub(super) fn compile_chain_method(
    method: &ChainMethod,
    interner: &StringInterner,
) -> CompiledChainMethod {
    match method {
        // String methods
        ChainMethod::Lower => CompiledChainMethod::Lower,
        ChainMethod::Upper => CompiledChainMethod::Upper,
        ChainMethod::Trim => CompiledChainMethod::Trim,
        ChainMethod::Contains { substring } => CompiledChainMethod::Contains {
            substring: substring.clone(),
        },
        ChainMethod::Startswith { prefix } => CompiledChainMethod::Startswith {
            prefix: prefix.clone(),
        },
        ChainMethod::Endswith { suffix } => CompiledChainMethod::Endswith {
            suffix: suffix.clone(),
        },
        // Collection methods
        ChainMethod::Count => CompiledChainMethod::Count,
        ChainMethod::Sum => CompiledChainMethod::Sum,
        ChainMethod::Max => CompiledChainMethod::Max,
        ChainMethod::Min => CompiledChainMethod::Min,
        ChainMethod::First => CompiledChainMethod::First,
        ChainMethod::Last => CompiledChainMethod::Last,
        ChainMethod::Reverse => CompiledChainMethod::Reverse,
        ChainMethod::Sort => CompiledChainMethod::Sort,
        ChainMethod::Unique => CompiledChainMethod::Unique,
        ChainMethod::Keys => CompiledChainMethod::Keys,
        ChainMethod::Intersection { values } => CompiledChainMethod::Intersection {
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        ChainMethod::Union { values } => CompiledChainMethod::Union {
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
        ChainMethod::Difference { values } => CompiledChainMethod::Difference {
            values: values.iter().map(|v| interner.intern(v)).collect(),
        },
    }
}
