//! Core types for the Reaper DSL evaluator.

use crate::PolicyAction;
use serde::{Deserialize, Serialize};

use super::condition::Condition;
use super::compiled_condition::CompiledCondition;

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Condition to evaluate
    pub condition: Condition,
    /// Decision if condition is true
    pub decision: PolicyAction,
}

/// Compiled rule with pre-interned condition for fast evaluation
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Pre-compiled condition with interned strings
    pub condition: CompiledCondition,
    /// Decision if condition is true
    pub decision: PolicyAction,
}

/// Entity type for condition evaluation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    User,
    Resource,
    Context,
}

/// Index expression for bracket notation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexExpr {
    /// Numeric index: [0], [1], [42]
    Number(i64),
    /// String key: ["department"], ["role"]
    String(String),
    /// Wildcard for iteration: [_] - iterates over all elements (existential quantification)
    Wildcard,
}

/// Literal value for comparisons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Bool(bool),
}
