//! Compiled literal value types for the Reaper DSL evaluator.

use crate::data::InternedString;

/// Compiled literal value with pre-interned strings
#[derive(Debug, Clone)]
pub enum CompiledLiteralValue {
    String(InternedString),
    Int(i64),
    Bool(bool),
}
