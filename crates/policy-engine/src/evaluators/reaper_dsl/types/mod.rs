//! Type definitions for the Reaper DSL evaluator.
//!
//! This module contains all enums and structs used by the evaluator, organized into:
//! - `core`: Core types (Rule, EntityType, IndexExpr, LiteralValue)
//! - `operators`: Comparison and operation operators
//! - `condition`: The Condition enum (uncompiled)
//! - `compiled_condition`: The CompiledCondition enum
//! - `expression`: Expression types (ExprType, ChainMethod, etc.)
//! - `compiled_expression`: Compiled expression types
//! - `comprehension`: Comprehension types
//! - `compiled_literal`: CompiledLiteralValue
//! - `v2`: V2 consolidated types for reduced enum explosion

mod compiled_condition;
mod compiled_expression;
mod compiled_literal;
mod comprehension;
mod condition;
mod core;
mod expression;
mod operators;
mod v2;

#[cfg(test)]
mod tests;

// ============================================================================
// Re-exports for public API
// ============================================================================

// Core types
pub use core::{CompiledRule, EntityType, IndexExpr, LiteralValue, Rule};

// Operators
pub use operators::{AttrCompareOp, ComprehensionFilterOp, CountOp, NumericOp, StringOp};

// Condition types
pub use condition::Condition;
pub use compiled_condition::CompiledCondition;

// Expression types
pub use expression::{
    ChainMethod, ExprIndexType, ExprType, OutputMethod, VariableCollectionMethod, VariableMethod,
    VariableStringTransform,
};
pub use compiled_expression::{CompiledChainMethod, CompiledExprIndexType, CompiledExprType};

// Literal types
pub use compiled_literal::CompiledLiteralValue;

// Comprehension types
pub use comprehension::{
    ComprehensionType, CompiledComprehension, CompiledIterationSource, CompiledIterator,
    CompiledOutput, UncompiledComprehensionType, UncompiledIterationSource, UncompiledOutput,
};

// V2 consolidated types (uncompiled)
pub use v2::{
    AttributeComparison, CompareTarget, CountCondition, CrossEntityComparison,
    StringOperationCondition, TimeCondition, VariableStringOperationCondition, WildcardComparison,
};

// V2 compiled types
pub use v2::{
    CompiledAttributeComparison, CompiledCompareTarget, CompiledCountCondition,
    CompiledCrossEntityComparison, CompiledRegexMatch, CompiledStringOperation,
    CompiledTimeCondition, CompiledVariableStringOp, CompiledWildcardComparison,
};
