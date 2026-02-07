//! V2 Consolidated Types - Reduces enum explosion through hierarchical design.
//!
//! These types use nested structures instead of flat enum variants to reduce
//! code duplication and improve maintainability.

use crate::data::{InternedString, StringInterner};
use serde::{Deserialize, Serialize};

use super::core::EntityType;
use super::operators::{CountOp, NumericOp, StringOp};

// ============================================================================
// UNCOMPILED V2 TYPES
// ============================================================================

/// Target of a comparison (right-hand side)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareTarget {
    /// Literal string value
    LiteralString(String),
    /// Literal numeric value
    LiteralNum(f64),
    /// Literal boolean value
    LiteralBool(bool),
    /// Literal null value
    LiteralNull,
    /// Another entity's attribute
    EntityAttr {
        entity_type: EntityType,
        attribute: String,
    },
    /// A variable
    Variable(String),
}

/// Consolidated attribute comparison condition (V2)
/// Replaces: UserEquals, ResourceEquals, UserGreaterEqualLiteral, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeComparison {
    /// Which entity to compare (User, Resource, Context)
    pub entity_type: EntityType,
    /// Attribute name on the entity
    pub attribute: String,
    /// Comparison operator
    pub op: NumericOp,
    /// Target value to compare against
    pub target: CompareTarget,
}

/// Consolidated string operation condition (V2)
/// Replaces: StringContains, StringStartsWith, StringEndsWith, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringOperationCondition {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the string
    pub attribute: String,
    /// String operation type
    pub op: StringOp,
    /// Value for the operation (substring, prefix, suffix, or comparison value)
    pub value: String,
}

/// Consolidated variable string operation condition (V2)
/// Replaces: VariableStringContains, VariableStringStartsWith, VariableStringEndsWith
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableStringOperationCondition {
    /// Variable name
    pub variable: String,
    /// String operation type
    pub op: StringOp,
    /// Value for the operation
    pub value: String,
}

/// Consolidated count condition (V2)
/// Replaces: CountGreaterEqual, CountGreater, CountEqual
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountCondition {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the collection
    pub attribute: String,
    /// Count comparison operator
    pub op: CountOp,
    /// Threshold to compare against
    pub threshold: usize,
}

/// Consolidated time condition (V2)
/// Replaces: TimeIsAfter, TimeIsBefore
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeCondition {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the timestamp
    pub attribute: String,
    /// Comparison operator (Greater = IsAfter, Less = IsBefore)
    pub op: NumericOp,
    /// Unix timestamp threshold
    pub threshold: i64,
}

/// Cross-entity comparison (V2)
/// Replaces: UserEqualsResource, UserIntGreater, ResourceIntGreater
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossEntityComparison {
    /// First entity type
    pub left_entity: EntityType,
    /// First attribute
    pub left_attr: String,
    /// Comparison operator
    pub op: NumericOp,
    /// Second entity type
    pub right_entity: EntityType,
    /// Second attribute
    pub right_attr: String,
}

/// Wildcard comparison (V2)
/// Replaces: UserWildcardEqualsResourceAttr, ResourceWildcardEqualsUserAttr
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WildcardComparison {
    /// Entity with the collection
    pub collection_entity: EntityType,
    /// Collection attribute
    pub collection_attr: String,
    /// Entity with the scalar value
    pub scalar_entity: EntityType,
    /// Scalar attribute
    pub scalar_attr: String,
}

// ============================================================================
// COMPILED V2 TYPES - With pre-interned strings for zero-lookup evaluation
// ============================================================================

/// Compiled comparison target (right-hand side) with interned strings
#[derive(Debug, Clone)]
pub enum CompiledCompareTarget {
    /// Literal string value (interned)
    LiteralString(InternedString),
    /// Literal numeric value
    LiteralNum(f64),
    /// Literal boolean value
    LiteralBool(bool),
    /// Literal null value
    LiteralNull,
    /// Another entity's attribute
    EntityAttr {
        entity_type: EntityType,
        attribute: InternedString,
    },
    /// A variable
    Variable(InternedString),
}

/// Compiled attribute comparison (V2)
/// Replaces 12+ flat variants: User/Resource/Context × Greater/Less/Equal/etc.
#[derive(Debug, Clone)]
pub struct CompiledAttributeComparison {
    /// Which entity to compare (User, Resource, Context)
    pub entity_type: EntityType,
    /// Attribute name on the entity (interned)
    pub attribute: InternedString,
    /// Comparison operator
    pub op: NumericOp,
    /// Target value to compare against
    pub target: CompiledCompareTarget,
}

/// Compiled string operation (V2)
/// Replaces: StringContains, StringStartsWith, StringEndsWith, StringLower/UpperEquals
#[derive(Debug, Clone)]
pub struct CompiledStringOperation {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the string (interned)
    pub attribute: InternedString,
    /// String operation type
    pub op: StringOp,
    /// Value for the operation (kept as String for memchr/comparison)
    pub value: String,
}

/// Compiled variable string operation (V2)
/// Replaces: VariableStringContains, VariableStringStartsWith, VariableStringEndsWith
#[derive(Debug, Clone)]
pub struct CompiledVariableStringOp {
    /// Variable name (interned)
    pub variable: InternedString,
    /// String operation type
    pub op: StringOp,
    /// Value for the operation
    pub value: String,
}

/// Compiled count condition (V2)
/// Replaces: CountGreaterEqual, CountGreater, CountEqual
#[derive(Debug, Clone)]
pub struct CompiledCountCondition {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the collection (interned)
    pub attribute: InternedString,
    /// Count comparison operator
    pub op: CountOp,
    /// Threshold to compare against
    pub threshold: usize,
}

/// Compiled time condition (V2)
/// Replaces: TimeIsAfter, TimeIsBefore
#[derive(Debug, Clone)]
pub struct CompiledTimeCondition {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name containing the timestamp (interned)
    pub attribute: InternedString,
    /// Comparison operator (Greater = IsAfter, Less = IsBefore)
    pub op: NumericOp,
    /// Unix timestamp threshold
    pub threshold: i64,
}

/// Compiled cross-entity comparison (V2)
/// Replaces: UserEqualsResource, UserIntGreater, ResourceIntGreater
#[derive(Debug, Clone)]
pub struct CompiledCrossEntityComparison {
    /// First entity type
    pub left_entity: EntityType,
    /// First attribute (interned)
    pub left_attr: InternedString,
    /// Comparison operator
    pub op: NumericOp,
    /// Second entity type
    pub right_entity: EntityType,
    /// Second attribute (interned)
    pub right_attr: InternedString,
}

/// Compiled wildcard comparison (V2)
/// Replaces: UserWildcardEqualsResourceAttr, ResourceWildcardEqualsUserAttr
#[derive(Debug, Clone)]
pub struct CompiledWildcardComparison {
    /// Entity with the collection
    pub collection_entity: EntityType,
    /// Collection attribute (interned)
    pub collection_attr: InternedString,
    /// Entity with the scalar value
    pub scalar_entity: EntityType,
    /// Scalar attribute (interned)
    pub scalar_attr: InternedString,
}

/// Compiled regex match condition (V2)
#[derive(Debug, Clone)]
pub struct CompiledRegexMatch {
    /// Which entity to check
    pub entity_type: EntityType,
    /// Attribute name (interned)
    pub attribute: InternedString,
    /// Regex pattern (kept as String for regex cache lookup)
    pub pattern: String,
}

// ============================================================================
// Conversion implementations from uncompiled to compiled types
// ============================================================================

impl AttributeComparison {
    /// Convert to compiled attribute comparison
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledAttributeComparison {
        let attribute = interner.intern(&self.attribute);
        let target = match &self.target {
            CompareTarget::LiteralNum(n) => CompiledCompareTarget::LiteralNum(*n),
            CompareTarget::LiteralString(s) => {
                CompiledCompareTarget::LiteralString(interner.intern(s))
            }
            CompareTarget::LiteralBool(b) => CompiledCompareTarget::LiteralBool(*b),
            CompareTarget::LiteralNull => CompiledCompareTarget::LiteralNull,
            CompareTarget::EntityAttr {
                entity_type,
                attribute: attr,
            } => CompiledCompareTarget::EntityAttr {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attr),
            },
            CompareTarget::Variable(v) => CompiledCompareTarget::Variable(interner.intern(v)),
        };
        CompiledAttributeComparison {
            entity_type: self.entity_type.clone(),
            attribute,
            op: self.op,
            target,
        }
    }
}

impl StringOperationCondition {
    /// Convert to compiled string operation
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledStringOperation {
        CompiledStringOperation {
            entity_type: self.entity_type.clone(),
            attribute: interner.intern(&self.attribute),
            op: self.op,
            value: self.value.clone(),
        }
    }
}

impl VariableStringOperationCondition {
    /// Convert to compiled variable string operation
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledVariableStringOp {
        CompiledVariableStringOp {
            variable: interner.intern(&self.variable),
            op: self.op,
            value: self.value.clone(),
        }
    }
}

impl CountCondition {
    /// Convert to compiled count condition
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledCountCondition {
        CompiledCountCondition {
            entity_type: self.entity_type.clone(),
            attribute: interner.intern(&self.attribute),
            op: self.op,
            threshold: self.threshold,
        }
    }
}

impl TimeCondition {
    /// Convert to compiled time condition
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledTimeCondition {
        CompiledTimeCondition {
            entity_type: self.entity_type.clone(),
            attribute: interner.intern(&self.attribute),
            op: self.op,
            threshold: self.threshold,
        }
    }
}

impl CrossEntityComparison {
    /// Convert to compiled cross-entity comparison
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledCrossEntityComparison {
        CompiledCrossEntityComparison {
            left_entity: self.left_entity.clone(),
            left_attr: interner.intern(&self.left_attr),
            op: self.op,
            right_entity: self.right_entity.clone(),
            right_attr: interner.intern(&self.right_attr),
        }
    }
}

impl WildcardComparison {
    /// Convert to compiled wildcard comparison
    pub fn to_compiled(&self, interner: &StringInterner) -> CompiledWildcardComparison {
        CompiledWildcardComparison {
            collection_entity: self.collection_entity.clone(),
            collection_attr: interner.intern(&self.collection_attr),
            scalar_entity: self.scalar_entity.clone(),
            scalar_attr: interner.intern(&self.scalar_attr),
        }
    }
}
