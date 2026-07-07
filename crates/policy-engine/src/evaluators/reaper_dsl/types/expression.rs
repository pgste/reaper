//! Expression types for the Reaper DSL evaluator.

use serde::{Deserialize, Serialize};

use super::core::EntityType;

/// Expression type for assignments (uncompiled, uses String)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprType {
    // String method results
    StringLower {
        entity_type: EntityType,
        attribute: String,
    },
    StringUpper {
        entity_type: EntityType,
        attribute: String,
    },
    StringTrim {
        entity_type: EntityType,
        attribute: String,
    },
    StringSplit {
        entity_type: EntityType,
        attribute: String,
        delimiter: String,
    },

    // Aggregate method results
    CollectionCount {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionSum {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionMax {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionMin {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionFirst {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionLast {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionSlice {
        entity_type: EntityType,
        attribute: String,
        start: i64,
        end: i64,
    },
    CollectionReverse {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionSort {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionUnique {
        entity_type: EntityType,
        attribute: String,
    },
    CollectionDifference {
        entity_type: EntityType,
        attribute: String,
        other_entity_type: EntityType,
        other_attribute: String,
    },
    CollectionUnion {
        entity_type: EntityType,
        attribute: String,
        other_entity_type: EntityType,
        other_attribute: String,
    },
    CollectionIntersection {
        entity_type: EntityType,
        attribute: String,
        other_entity_type: EntityType,
        other_attribute: String,
    },

    // Set operation results
    SetIntersection {
        entity_type: EntityType,
        attribute: String,
        values: Vec<String>,
    },
    SetUnion {
        entity_type: EntityType,
        attribute: String,
        values: Vec<String>,
    },
    SetDifference {
        entity_type: EntityType,
        attribute: String,
        values: Vec<String>,
    },
    SetKeys {
        entity_type: EntityType,
        attribute: String,
    },
    SetValues {
        entity_type: EntityType,
        attribute: String,
    },

    // Time function results
    TimeNow,
    TimeNowMs,
    TimeNowNs,

    // String method results (returning boolean)
    StringContains {
        entity_type: EntityType,
        attribute: String,
        substring: String,
    },
    StringStartsWithExpr {
        entity_type: EntityType,
        attribute: String,
        prefix: String,
    },
    StringEndsWithExpr {
        entity_type: EntityType,
        attribute: String,
        suffix: String,
    },

    // Regex method results
    RegexMatches {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
    },
    // Regex find: first match of `pattern` in the attribute (or null).
    RegexFind {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
    },
    // Regex find-all: list of every match of `pattern` in the attribute.
    RegexFindAll {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
    },
    // Regex replace-all: `pattern` -> `replacement` in the attribute string.
    StringReplace {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
        replacement: String,
    },

    // Chained method call
    ChainedMethod {
        base: Box<ExprType>,
        method: ChainMethod,
    },

    // Variable reference for chained operations
    VariableRef {
        variable: String,
    },

    // Variable indexed access: row[_], row[0], row["key"]
    VariableIndexed {
        variable: String,
        index: ExprIndexType,
    },

    // Variable attribute access: first_group.items, record.value
    VariableAttrAccess {
        variable: String,
        attribute: String,
    },

    // Variable attribute indexed access: first_dept.projects[0]
    VariableAttrIndexed {
        variable: String,
        attribute: String,
        index: ExprIndexType,
    },
}

/// Index type for indexed access expressions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprIndexType {
    /// Wildcard index: row[_] - returns entire collection for nested iteration
    Wildcard,
    /// Numeric index: row[0]
    Number(i64),
    /// String index: row["key"]
    String(String),
}

/// Method that can be chained (uncompiled)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChainMethod {
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
    // Set operations with literal array
    Intersection { values: Vec<String> },
    Union { values: Vec<String> },
    Difference { values: Vec<String> },
}

/// Method that can be called on a variable
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VariableMethod {
    Count,
    Sum,
    Max,
    Min,
    First,
    Last,
    Reverse,
    Sort,
    Unique,
}

/// Collection methods that take a literal array argument
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VariableCollectionMethod {
    Intersection,
    Union,
    Difference,
}

/// String transformation methods that return a transformed string
/// Used as the first method in chained method comparisons: var.trim().count() > 0
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VariableStringTransform {
    Trim,
    Lower,
    Upper,
}

/// Methods that can be used in comprehension output expressions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputMethod {
    Lower,
    Upper,
    Trim,
}
