//! Compiled condition types for the Reaper DSL evaluator.

use crate::data::InternedString;

use super::compiled_expression::CompiledExprType;
use super::compiled_literal::CompiledLiteralValue;
use super::comprehension::CompiledComprehension;
use super::core::{EntityType, IndexExpr};
use super::expression::{VariableCollectionMethod, VariableMethod, VariableStringTransform};
use super::operators::{AttrCompareOp, ComprehensionFilterOp};
use super::v2::{
    CompiledAttributeComparison, CompiledCountCondition, CompiledCrossEntityComparison,
    CompiledRegexMatch, CompiledStringOperation, CompiledTimeCondition, CompiledVariableStringOp,
    CompiledWildcardComparison,
};

/// Compiled condition with pre-interned strings for zero-lookup evaluation.
/// This is the "hot path" version - all strings are pre-interned at construction time.
#[derive(Debug, Clone)]
pub enum CompiledCondition {
    // ============ V2 Consolidated Types ============
    AttributeCompare(CompiledAttributeComparison),
    StringOp(CompiledStringOperation),
    VariableStringOp(CompiledVariableStringOp),
    CountOp(CompiledCountCondition),
    TimeOp(CompiledTimeCondition),
    CrossEntityCompare(CompiledCrossEntityComparison),
    WildcardCompare(CompiledWildcardComparison),
    RegexMatch(CompiledRegexMatch),

    // ============ Core Conditions ============
    Always,

    /// ReBAC check with everything pre-interned: evaluation is DashMap gets on
    /// (u32, u32) keys + binary search / bounded BFS. No strings, no allocs on
    /// the direct path.
    RebacCheck {
        kind: super::condition::RebacKind,
        subject: CompiledRebacRef,
        relation: InternedString,
        object: CompiledRebacRef,
        via: Option<InternedString>,
        max_depth: u32,
    },
    ActionEquals {
        value: InternedString,
    },
    ResourceIdEquals {
        value: InternedString,
    },

    // ============ Same Entity Comparisons ============
    SameEntityAttrCompare {
        entity_type: EntityType,
        left_attr: InternedString,
        right_attr: InternedString,
        op: AttrCompareOp,
    },

    // ============ Assignments & Membership ============
    Assignment {
        variable: InternedString,
        entity_type: EntityType,
        attribute: InternedString,
        index: Option<IndexExpr>,
    },
    MembershipTest {
        value: CompiledLiteralValue,
        entity_type: EntityType,
        attribute: InternedString,
        index: Option<IndexExpr>,
    },
    IndexedEquals {
        entity_type: EntityType,
        attribute: InternedString,
        index: IndexExpr,
        value: InternedString,
    },
    EqualsVariable {
        entity_type: EntityType,
        attribute: InternedString,
        variable: InternedString,
    },

    // ============ Type Checks ============
    IsString {
        entity_type: EntityType,
        attribute: InternedString,
    },
    IsNumber {
        entity_type: EntityType,
        attribute: InternedString,
    },
    IsBool {
        entity_type: EntityType,
        attribute: InternedString,
    },

    // ============ Set Operations ============
    SetIntersectionCountGreater {
        entity_type: EntityType,
        attribute: InternedString,
        values: Vec<InternedString>,
        threshold: usize,
    },
    MapKeyExists {
        entity_type: EntityType,
        attribute: InternedString,
        key: InternedString,
    },

    // ============ Comprehensions ============
    ComprehensionCountGreaterEqual {
        entity_type: EntityType,
        attribute: InternedString,
        filter_attr: InternedString,
        filter_value: CompiledLiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },
    ComprehensionCountEqual {
        entity_type: EntityType,
        attribute: InternedString,
        filter_attr: InternedString,
        filter_value: CompiledLiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },
    ComprehensionAssignment {
        variable: InternedString,
        comprehension: Box<CompiledComprehension>,
    },

    // ============ Expression Assignment ============
    ExpressionAssignment {
        variable: InternedString,
        expr_type: CompiledExprType,
    },

    // ============ Variable Comparisons ============
    VariableEqualsLiteral {
        variable: InternedString,
        value: CompiledLiteralValue,
    },
    /// Native `var != literal` — an unbound variable fails the guard.
    VariableNotEqualsLiteral {
        variable: InternedString,
        value: CompiledLiteralValue,
    },
    VariableCompare {
        variable: InternedString,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },
    VariableIsNull {
        variable: InternedString,
    },
    VariableIsNotNull {
        variable: InternedString,
    },
    ComparisonAssignment {
        variable: InternedString,
        entity_type: EntityType,
        attribute: InternedString,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },
    ExprCompareAssignment {
        variable: InternedString,
        expr_type: CompiledExprType,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },
    NullComparisonAssignment {
        variable: InternedString,
        entity_type: EntityType,
        attribute: InternedString,
        is_null_check: bool,
    },
    VariableMembershipTest {
        value: CompiledLiteralValue,
        variable: InternedString,
    },
    VariableIsString {
        variable: InternedString,
    },
    VariableIsNumber {
        variable: InternedString,
    },
    VariableIsBool {
        variable: InternedString,
    },
    VariableIsTruthy {
        variable: InternedString,
    },
    VariableEqualsVariable {
        left: InternedString,
        right: InternedString,
    },
    VariableNotEqualsVariable {
        left: InternedString,
        right: InternedString,
    },
    VariableMethodWithLiteralArray {
        variable: InternedString,
        method: VariableCollectionMethod,
        values: Vec<InternedString>,
    },
    VariableMethodCompare {
        variable: InternedString,
        method: VariableMethod,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },
    VariableChainedMethodCompare {
        variable: InternedString,
        transform_method: VariableStringTransform,
        compare_method: VariableMethod,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },

    // ============ Variable Attribute Comparisons ============
    VariableAttrEqualsLiteral {
        variable: InternedString,
        attribute: InternedString,
        value: CompiledLiteralValue,
    },
    /// Native `var.attr != literal` — missing attribute fails the guard.
    VariableAttrNotEqualsLiteral {
        variable: InternedString,
        attribute: InternedString,
        value: CompiledLiteralValue,
    },
    VariableAttrCompare {
        variable: InternedString,
        attribute: InternedString,
        op: AttrCompareOp,
        value: CompiledLiteralValue,
    },
    VariableAttrEqualsNull {
        variable: InternedString,
        attribute: InternedString,
    },
    VariableAttrNotEqualsNull {
        variable: InternedString,
        attribute: InternedString,
    },
    VarAttrNullCompareAssignment {
        result_variable: InternedString,
        source_variable: InternedString,
        attribute: InternedString,
        is_null_check: bool,
    },
    VariableAttrContains {
        variable: InternedString,
        attribute: InternedString,
        substring: InternedString,
    },

    // ============ Logical Operators ============
    And(Vec<CompiledCondition>),
    Or(Vec<CompiledCondition>),
    Not(Box<CompiledCondition>),
}

// ============================================================================
// CompiledCondition extraction helpers for V2 types
// ============================================================================

impl CompiledCondition {
    /// Extract attribute comparison if this is an AttributeCompare variant
    pub fn as_attribute_comparison(&self) -> Option<&CompiledAttributeComparison> {
        match self {
            CompiledCondition::AttributeCompare(comp) => Some(comp),
            _ => None,
        }
    }

    /// Extract string operation if this is a StringOp variant
    pub fn as_string_operation(&self) -> Option<&CompiledStringOperation> {
        match self {
            CompiledCondition::StringOp(op) => Some(op),
            _ => None,
        }
    }

    /// Extract count condition if this is a CountOp variant
    pub fn as_count_condition(&self) -> Option<&CompiledCountCondition> {
        match self {
            CompiledCondition::CountOp(cond) => Some(cond),
            _ => None,
        }
    }

    /// Extract time condition if this is a TimeOp variant
    pub fn as_time_condition(&self) -> Option<&CompiledTimeCondition> {
        match self {
            CompiledCondition::TimeOp(cond) => Some(cond),
            _ => None,
        }
    }

    /// Extract cross-entity comparison if this is a CrossEntityCompare variant
    pub fn as_cross_entity_comparison(&self) -> Option<&CompiledCrossEntityComparison> {
        match self {
            CompiledCondition::CrossEntityCompare(comp) => Some(comp),
            _ => None,
        }
    }

    /// Extract wildcard comparison if this is a WildcardCompare variant
    pub fn as_wildcard_comparison(&self) -> Option<&CompiledWildcardComparison> {
        match self {
            CompiledCondition::WildcardCompare(comp) => Some(comp),
            _ => None,
        }
    }

    /// Extract regex match if this is a RegexMatch variant
    pub fn as_regex_match(&self) -> Option<&CompiledRegexMatch> {
        match self {
            CompiledCondition::RegexMatch(m) => Some(m),
            _ => None,
        }
    }
}

/// Pre-resolved rebac argument.
#[derive(Debug, Clone)]
pub enum CompiledRebacRef {
    Principal,
    ResourceId,
    Literal(InternedString),
}
