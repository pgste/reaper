//! Operator types for the Reaper DSL evaluator.

use serde::{Deserialize, Serialize};

/// Comparison operators for same-entity attribute comparisons
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttrCompareOp {
    /// ==
    Equal,
    /// !=
    NotEqual,
    /// <=
    LessEqual,
    /// >=
    GreaterEqual,
    /// <
    Less,
    /// >
    Greater,
}

/// Numeric comparison operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericOp {
    /// ==
    Equal,
    /// !=
    NotEqual,
    /// >=
    GreaterEqual,
    /// >
    Greater,
    /// <=
    LessEqual,
    /// <
    Less,
}

/// String operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StringOp {
    /// string.contains(substring)
    Contains,
    /// string.startswith(prefix)
    StartsWith,
    /// string.endswith(suffix)
    EndsWith,
    /// string.lower() == value
    LowerEquals,
    /// string.upper() == value
    UpperEquals,
    /// string.lower() != value — native so a missing attribute FAILS the
    /// guard (Not(LowerEquals) would let absent attributes pass, fail-open).
    LowerNotEquals,
    /// string.upper() != value — native negation, same fail-closed contract.
    UpperNotEquals,
}

/// Collection count operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountOp {
    /// count() >= threshold
    GreaterEqual,
    /// count() > threshold
    Greater,
    /// count() == threshold
    Equal,
    /// count() < threshold
    Less,
    /// count() <= threshold
    LessEqual,
}

/// Filter operation for comprehensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComprehensionFilterOp {
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterEqual,
    LessEqual,
    Contains,
}

// ============================================================================
// Conversion implementations
// ============================================================================

impl From<NumericOp> for AttrCompareOp {
    fn from(op: NumericOp) -> Self {
        match op {
            NumericOp::Equal => AttrCompareOp::Equal,
            NumericOp::NotEqual => AttrCompareOp::NotEqual,
            NumericOp::GreaterEqual => AttrCompareOp::GreaterEqual,
            NumericOp::Greater => AttrCompareOp::Greater,
            NumericOp::LessEqual => AttrCompareOp::LessEqual,
            NumericOp::Less => AttrCompareOp::Less,
        }
    }
}

impl From<AttrCompareOp> for NumericOp {
    fn from(op: AttrCompareOp) -> Self {
        match op {
            AttrCompareOp::Equal => NumericOp::Equal,
            AttrCompareOp::NotEqual => NumericOp::NotEqual,
            AttrCompareOp::GreaterEqual => NumericOp::GreaterEqual,
            AttrCompareOp::Greater => NumericOp::Greater,
            AttrCompareOp::LessEqual => NumericOp::LessEqual,
            AttrCompareOp::Less => NumericOp::Less,
        }
    }
}

impl From<CountOp> for AttrCompareOp {
    fn from(op: CountOp) -> Self {
        match op {
            CountOp::GreaterEqual => AttrCompareOp::GreaterEqual,
            CountOp::Greater => AttrCompareOp::Greater,
            CountOp::Equal => AttrCompareOp::Equal,
            CountOp::Less => AttrCompareOp::Less,
            CountOp::LessEqual => AttrCompareOp::LessEqual,
        }
    }
}
