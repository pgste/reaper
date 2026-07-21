//! Comprehension types for the Reaper DSL evaluator.

use crate::data::InternedString;
use serde::{Deserialize, Serialize};

use super::compiled_condition::CompiledCondition;
use super::compiled_literal::CompiledLiteralValue;
use super::core::{EntityType, LiteralValue};
use super::expression::OutputMethod;

// ============ Uncompiled Comprehension Types ============

/// Uncompiled comprehension type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UncompiledComprehensionType {
    Set,
    Array,
    Object,
}

/// Uncompiled iteration source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UncompiledIterationSource {
    EntityAttr {
        entity_type: EntityType,
        attribute: String,
    },
    Variable {
        variable: String,
    },
    /// `input.<dotted.path>[_]` — iterate an array in the request's input
    /// document (R4-01 B.2). Appended so serialized encodings stay stable.
    Input {
        path: super::input::InputPath,
    },
}

/// Uncompiled output expression
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UncompiledOutput {
    Variable(String),
    VarAttr {
        variable: String,
        attribute: String,
    },
    Literal(LiteralValue),
    /// Method call on variable: t.trim(), item.upper()
    VarMethodCall {
        variable: String,
        method: OutputMethod,
    },
}

// ============ Compiled Comprehension Types ============

/// Compiled comprehension for set/array/object comprehensions
#[derive(Debug, Clone)]
pub struct CompiledComprehension {
    /// Type of comprehension (Set, Array, Object)
    pub comp_type: ComprehensionType,
    /// Iterator source
    pub iterator: CompiledIterator,
    /// Filter conditions
    pub filters: Vec<CompiledCondition>,
    /// Output expression (for Set/Array)
    pub output: Option<CompiledOutput>,
    /// Key/Value expressions (for Object)
    pub key_value: Option<(CompiledOutput, CompiledOutput)>,
}

/// Type of comprehension
#[derive(Debug, Clone)]
pub enum ComprehensionType {
    Set,
    Array,
    Object,
}

/// Compiled iterator for comprehensions
#[derive(Debug, Clone)]
pub struct CompiledIterator {
    /// Variable name to bind each element
    pub variable: InternedString,
    /// Source collection
    pub source: CompiledIterationSource,
}

/// Compiled iteration source
#[derive(Debug, Clone)]
pub enum CompiledIterationSource {
    EntityAttr {
        entity_type: EntityType,
        attribute: InternedString,
    },
    Variable {
        variable: InternedString,
    },
    /// `input.<dotted.path>[_]` (R4-01 B.2): the pre-parsed path resolves
    /// in the request document; elements are materialized into the variable
    /// domain via transient interning (reclaimed at eval end).
    Input {
        path: super::input::InputPath,
    },
}

/// Compiled output expression for comprehensions
#[derive(Debug, Clone)]
pub enum CompiledOutput {
    Variable(InternedString),
    VarAttr {
        variable: InternedString,
        attribute: InternedString,
    },
    Literal(CompiledLiteralValue),
    /// Method call on variable: t.trim(), item.upper()
    VarMethodCall {
        variable: InternedString,
        method: OutputMethod,
    },
}
