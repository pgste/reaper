//! Condition types for the Reaper DSL evaluator.
//!
//! ## Migration Note
//! V1 flat variants (UserEquals, ResourceEquals, etc.) have been replaced with
//! consolidated V2 types that reduce code duplication:
//! - AttributeCompare: All entity attribute comparisons
//! - StringOp: All string operations (contains, startswith, endswith, lower, upper)
//! - VariableStringOp: Variable string operations
//! - CountOp: Count comparisons
//! - TimeOp: Time comparisons
//! - CrossEntityCompare: Cross-entity comparisons
//! - WildcardCompare: Wildcard/existential comparisons

use serde::{Deserialize, Serialize};

use super::comprehension::{UncompiledComprehensionType, UncompiledIterationSource, UncompiledOutput};
use super::core::{EntityType, IndexExpr, LiteralValue};
use super::expression::{ExprType, VariableCollectionMethod, VariableMethod, VariableStringTransform};
use super::operators::{AttrCompareOp, ComprehensionFilterOp};
use super::v2::{
    AttributeComparison, CountCondition, CrossEntityComparison, StringOperationCondition,
    TimeCondition, VariableStringOperationCondition, WildcardComparison,
};

/// Policy condition (compiled from YAML/DSL)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// Always true
    Always,
    /// Compare action to literal value
    ActionEquals { value: String },
    /// Compare resource ID to literal value (for simple resource matching)
    ResourceIdEquals { value: String },

    // ============ Consolidated Attribute Comparisons ============
    /// Attribute comparison (replaces User/Resource/Context Equals/Greater/Less variants)
    AttributeCompare(AttributeComparison),

    // ============ Consolidated String Operations ============
    /// String operation (replaces StringContains/StartsWith/EndsWith/LowerEquals/UpperEquals)
    StringOp(StringOperationCondition),
    /// Variable string operation (replaces VariableStringContains/StartsWith/EndsWith)
    VariableStringOp(VariableStringOperationCondition),

    // ============ Consolidated Count Operations ============
    /// Count comparison (replaces CountGreaterEqual/Greater/Equal)
    CountOp(CountCondition),

    // ============ Consolidated Time Operations ============
    /// Time comparison (replaces TimeIsAfter/IsBefore)
    TimeOp(TimeCondition),

    // ============ Consolidated Cross-Entity Comparisons ============
    /// Cross-entity comparison (replaces UserEqualsResource, UserIntGreater, etc.)
    CrossEntityCompare(CrossEntityComparison),

    // ============ Consolidated Wildcard Comparisons ============
    /// Wildcard comparison (replaces UserWildcardEqualsResourceAttr, etc.)
    WildcardCompare(WildcardComparison),

    // ============ Same-Entity Comparisons ============
    /// Compare two attributes of the same entity
    SameEntityAttrCompare {
        entity_type: EntityType,
        left_attr: String,
        right_attr: String,
        op: AttrCompareOp,
    },

    // ============ Assignments ============
    /// Variable assignment: x := user.role
    Assignment {
        variable: String,
        entity_type: EntityType,
        attribute: String,
        index: Option<IndexExpr>,
    },

    // ============ Membership Tests ============
    /// Check membership in array/set: "admin" in user.roles
    MembershipTest {
        value: LiteralValue,
        entity_type: EntityType,
        attribute: String,
        index: Option<IndexExpr>,
    },

    // ============ Indexed Access ============
    /// Compare with bracket notation: user.roles[0] == "admin"
    IndexedEquals {
        entity_type: EntityType,
        attribute: String,
        index: IndexExpr,
        value: String,
    },
    /// Compare attribute with variable: user.role == role_var
    EqualsVariable {
        entity_type: EntityType,
        attribute: String,
        variable: String,
    },

    // ============ Regex Support ============
    RegexMatches {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
    },

    // ============ Type Check Functions ============
    IsString {
        entity_type: EntityType,
        attribute: String,
    },
    IsNumber {
        entity_type: EntityType,
        attribute: String,
    },
    IsBool {
        entity_type: EntityType,
        attribute: String,
    },

    // ============ Set Operations ============
    SetIntersectionCountGreater {
        entity_type: EntityType,
        attribute: String,
        values: Vec<String>,
        threshold: usize,
    },
    MapKeyExists {
        entity_type: EntityType,
        attribute: String,
        key: String,
    },

    // ============ Comprehension Support ============
    ComprehensionCountGreaterEqual {
        entity_type: EntityType,
        attribute: String,
        filter_attr: String,
        filter_value: LiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },
    ComprehensionCountEqual {
        entity_type: EntityType,
        attribute: String,
        filter_attr: String,
        filter_value: LiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },

    // ============ Expression Assignment ============
    ExpressionAssignment {
        variable: String,
        expr_type: ExprType,
    },

    // ============ Variable Comparisons ============
    VariableEqualsLiteral {
        variable: String,
        value: LiteralValue,
    },
    VariableCompare {
        variable: String,
        op: AttrCompareOp,
        value: LiteralValue,
    },
    VariableIsNull {
        variable: String,
    },
    VariableIsNotNull {
        variable: String,
    },
    ComparisonAssignment {
        variable: String,
        entity_type: EntityType,
        attribute: String,
        op: AttrCompareOp,
        value: LiteralValue,
    },
    ExprCompareAssignment {
        variable: String,
        expr_type: ExprType,
        op: AttrCompareOp,
        value: LiteralValue,
    },
    NullComparisonAssignment {
        variable: String,
        entity_type: EntityType,
        attribute: String,
        is_null_check: bool,
    },
    VariableMembershipTest {
        value: LiteralValue,
        variable: String,
    },
    VariableIsString {
        variable: String,
    },
    VariableIsNumber {
        variable: String,
    },
    VariableIsBool {
        variable: String,
    },
    VariableIsTruthy {
        variable: String,
    },
    VariableEqualsVariable {
        left: String,
        right: String,
    },
    VariableNotEqualsVariable {
        left: String,
        right: String,
    },
    VariableMethodWithLiteralArray {
        variable: String,
        method: VariableCollectionMethod,
        values: Vec<String>,
    },
    VariableMethodCompare {
        variable: String,
        method: VariableMethod,
        op: AttrCompareOp,
        value: LiteralValue,
    },
    VariableChainedMethodCompare {
        variable: String,
        transform_method: VariableStringTransform,
        compare_method: VariableMethod,
        op: AttrCompareOp,
        value: LiteralValue,
    },

    // ============ Variable Attribute Comparisons ============
    VariableAttrEqualsLiteral {
        variable: String,
        attribute: String,
        value: LiteralValue,
    },
    VariableAttrCompare {
        variable: String,
        attribute: String,
        op: AttrCompareOp,
        value: LiteralValue,
    },
    VariableAttrEqualsNull {
        variable: String,
        attribute: String,
    },
    VariableAttrNotEqualsNull {
        variable: String,
        attribute: String,
    },
    VarAttrNullCompareAssignment {
        result_variable: String,
        source_variable: String,
        attribute: String,
        is_null_check: bool,
    },
    VariableAttrContains {
        variable: String,
        attribute: String,
        substring: String,
    },

    // ============ General Comprehension Assignment ============
    ComprehensionAssignment {
        variable: String,
        comp_type: UncompiledComprehensionType,
        iterator_var: String,
        iterator_source: UncompiledIterationSource,
        filters: Vec<Condition>,
        output: Option<UncompiledOutput>,
        key_output: Option<UncompiledOutput>,
    },

    /// AND of multiple conditions
    And(Vec<Condition>),
    /// OR of multiple conditions
    Or(Vec<Condition>),
    /// NOT of a condition
    Not(Box<Condition>),
}
