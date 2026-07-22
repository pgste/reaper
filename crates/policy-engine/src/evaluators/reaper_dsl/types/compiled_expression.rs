//! Compiled expression types for the Reaper DSL evaluator.

use crate::data::InternedString;

use super::compiled_literal::CompiledLiteralValue;
use super::core::EntityType;

/// Compiled expression type for expression-based assignments (e.g., x := user.name.lower())
#[derive(Debug, Clone)]
pub enum CompiledExprType {
    // String method results
    StringLower {
        entity_type: EntityType,
        attribute: InternedString,
    },
    StringUpper {
        entity_type: EntityType,
        attribute: InternedString,
    },
    StringTrim {
        entity_type: EntityType,
        attribute: InternedString,
    },
    StringSplit {
        entity_type: EntityType,
        attribute: InternedString,
        delimiter: String,
    },

    // Aggregate method results
    CollectionCount {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionSum {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionMax {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionMin {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionFirst {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionLast {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionSlice {
        entity_type: EntityType,
        attribute: InternedString,
        start: i64,
        end: i64,
    },
    CollectionReverse {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionSort {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionUnique {
        entity_type: EntityType,
        attribute: InternedString,
    },
    CollectionDifference {
        entity_type: EntityType,
        attribute: InternedString,
        other_entity_type: EntityType,
        other_attribute: InternedString,
    },
    CollectionUnion {
        entity_type: EntityType,
        attribute: InternedString,
        other_entity_type: EntityType,
        other_attribute: InternedString,
    },
    CollectionIntersection {
        entity_type: EntityType,
        attribute: InternedString,
        other_entity_type: EntityType,
        other_attribute: InternedString,
    },

    // Set operation results
    SetIntersection {
        entity_type: EntityType,
        attribute: InternedString,
        values: Vec<InternedString>,
    },
    SetUnion {
        entity_type: EntityType,
        attribute: InternedString,
        values: Vec<InternedString>,
    },
    SetDifference {
        entity_type: EntityType,
        attribute: InternedString,
        values: Vec<InternedString>,
    },
    SetKeys {
        entity_type: EntityType,
        attribute: InternedString,
    },
    SetValues {
        entity_type: EntityType,
        attribute: InternedString,
    },

    // Time function results
    TimeNow,
    TimeNowMs,
    TimeNowNs,

    // String method results (returning boolean)
    StringContains {
        entity_type: EntityType,
        attribute: InternedString,
        substring: String,
    },
    StringStartsWithExpr {
        entity_type: EntityType,
        attribute: InternedString,
        prefix: String,
    },
    StringEndsWithExpr {
        entity_type: EntityType,
        attribute: InternedString,
        suffix: String,
    },

    // Regex method results
    RegexMatches {
        entity_type: EntityType,
        attribute: InternedString,
        pattern: String,
    },
    RegexFind {
        entity_type: EntityType,
        attribute: InternedString,
        pattern: String,
    },
    RegexFindAll {
        entity_type: EntityType,
        attribute: InternedString,
        pattern: String,
    },
    StringReplace {
        entity_type: EntityType,
        attribute: InternedString,
        pattern: String,
        replacement: String,
    },

    // Chained method call (e.g., user.name.lower().trim())
    ChainedMethod {
        base: Box<CompiledExprType>,
        method: CompiledChainMethod,
    },

    // Variable reference for chained operations
    VariableRef {
        variable: InternedString,
    },

    // Variable indexed access: row[_], row[0], row["key"]
    VariableIndexed {
        variable: InternedString,
        index: CompiledExprIndexType,
    },

    // Variable attribute access: first_group.items, record.value
    VariableAttrAccess {
        variable: InternedString,
        attribute: InternedString,
    },

    // Variable attribute indexed access: first_dept.projects[0]
    VariableAttrIndexed {
        variable: InternedString,
        attribute: InternedString,
        index: CompiledExprIndexType,
    },

    /// `taint::level("key")` — trust level of one context key as a string,
    /// read from the request provenance under the fail-untrusted rule.
    TaintLevel {
        key: String,
    },

    /// A scalar literal, pre-interned where applicable (R4-01 A.3). The
    /// cheapest expression there is: evaluation is a constant load.
    Literal {
        value: CompiledLiteralValue,
    },

    /// An `input` document read (R4-01 B.3): resolve the pre-parsed path
    /// against the request's raw JSON document and materialize the value
    /// into the variable domain via transient interning. Missing document
    /// or path evaluates to `Null` — the assignment still SUCCEEDS, exactly
    /// like the AST interpreter's total input access. Needs the eval
    /// context's input handle, so it is evaluated in the context-aware
    /// wrapper (`ReaperDSLEvaluator::evaluate_expr_type`), not in
    /// `expr_eval::evaluate_compiled_expr_type`.
    InputRead {
        path: super::input::InputPath,
    },
}

/// Compiled index type for indexed access expressions
#[derive(Debug, Clone)]
pub enum CompiledExprIndexType {
    /// Wildcard index: row[_] - returns entire collection for nested iteration
    Wildcard,
    /// Numeric index: row[0]
    Number(i64),
    /// String index: row["key"]
    String(InternedString),
}

/// Method that can be chained after another expression
#[derive(Debug, Clone)]
pub enum CompiledChainMethod {
    // String methods
    Lower,
    Upper,
    Trim,
    Contains { substring: String },
    Startswith { prefix: String },
    Endswith { suffix: String },
    // Collection methods
    Count,
    Sum,
    Max,
    Min,
    First,
    Last,
    Reverse,
    Sort,
    Unique,
    Keys,
    // Set operations with literal array (interned)
    Intersection { values: Vec<InternedString> },
    Union { values: Vec<InternedString> },
    Difference { values: Vec<InternedString> },
}
