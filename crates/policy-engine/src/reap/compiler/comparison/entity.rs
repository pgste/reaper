//! Entity attribute comparison compilation.
//!
//! This module handles compilation of comparisons between entity attributes
//! and literal values, as well as cross-entity comparisons.
//!
//! Uses V2 consolidated types for cleaner code and reduced duplication.

use crate::evaluators::reaper_dsl::{
    AttrCompareOp, AttributeComparison, CompareTarget, Condition as DslCondition,
    CrossEntityComparison, EntityType, IndexExpr, NumericOp, WildcardComparison,
};
use crate::reap::ast::{Entity, EntityAttr, Index, Operator, Value};
use reaper_core::ReaperError;

/// Convert AST Entity to DSL EntityType
fn entity_to_type(entity: &Entity) -> Result<EntityType, ReaperError> {
    match entity {
        Entity::User => Ok(EntityType::User),
        Entity::Resource => Ok(EntityType::Resource),
        Entity::Context => Ok(EntityType::Context),
        Entity::Actor => Ok(EntityType::Actor),
        Entity::Input => Err(ReaperError::InvalidPolicy {
            reason: "`input` document access is not compiled yet; policy runs on the AST evaluator"
                .to_string(),
        }),
    }
}

/// Convert AST Operator to NumericOp
fn operator_to_numeric_op(op: &Operator) -> Result<NumericOp, ReaperError> {
    match op {
        Operator::Equal => Ok(NumericOp::Equal),
        Operator::NotEqual => Ok(NumericOp::NotEqual),
        Operator::GreaterEqual => Ok(NumericOp::GreaterEqual),
        Operator::GreaterThan => Ok(NumericOp::Greater),
        Operator::LessEqual => Ok(NumericOp::LessEqual),
        Operator::LessThan => Ok(NumericOp::Less),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Operator {:?} not supported for comparisons", op),
        }),
    }
}

/// Compile comparison: entity.attr op literal_value
pub fn compile_value_comparison(
    left: EntityAttr,
    op: Operator,
    value: Value,
) -> Result<DslCondition, ReaperError> {
    let entity_type = entity_to_type(&left.entity)?;

    // Bracket-indexed comparison: `user.roles[0] == "admin"`,
    // `user.tags[_] == "beta"`, `user.profile["tier"] == "gold"`. Only string
    // equality compiles (IndexedEquals); every other indexed shape falls back
    // to the AST evaluator. It must NEVER fall through to the un-indexed
    // compile below — that silently dropped the index and compared the whole
    // collection to the literal, a compiled-only wrong Deny (caught by the
    // compiled-vs-AST differential when the actor cases extended it).
    if let Some(index) = &left.index {
        return match (&op, &value) {
            (Operator::Equal, Value::String(s)) => Ok(DslCondition::IndexedEquals {
                entity_type,
                attribute: left.attribute,
                index: match index {
                    Index::Number(n) => IndexExpr::Number(*n),
                    Index::String(k) => IndexExpr::String(k.clone()),
                    Index::Wildcard => IndexExpr::Wildcard,
                },
                value: s.clone(),
            }),
            _ => Err(ReaperError::InvalidPolicy {
                reason: "indexed comparisons compile only as `entity.attr[idx] == \"string\"`; \
                         other operators/types run on the AST evaluator"
                    .to_string(),
            }),
        };
    }

    // Handle null comparisons
    if matches!(value, Value::Null) {
        let attr_op = match op {
            Operator::Equal => NumericOp::Equal,
            Operator::NotEqual => NumericOp::NotEqual,
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Null comparisons only support == and != operators".to_string(),
                })
            }
        };
        return Ok(DslCondition::AttributeCompare(AttributeComparison {
            entity_type,
            attribute: left.attribute,
            op: attr_op,
            target: CompareTarget::LiteralNull,
        }));
    }

    // Convert value to appropriate CompareTarget
    let (target, is_numeric) = match &value {
        Value::String(s) => (CompareTarget::LiteralString(s.clone()), false),
        Value::Integer(i) => (CompareTarget::LiteralNum(*i as f64), true),
        Value::Float(f) => (CompareTarget::LiteralNum(*f), true),
        Value::Boolean(b) => (CompareTarget::LiteralBool(*b), false),
        Value::Null => unreachable!(), // Handled above
        Value::Array(arr) => {
            let json = serde_json::to_string(&arr).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize array: {}", e),
            })?;
            (CompareTarget::LiteralString(json), false)
        }
        Value::Object(obj) => {
            let json = serde_json::to_string(&obj).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize object: {}", e),
            })?;
            (CompareTarget::LiteralString(json), false)
        }
        Value::Set(set) => {
            let json = serde_json::to_string(&set).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to serialize set: {}", e),
            })?;
            (CompareTarget::LiteralString(json), false)
        }
    };

    // For numeric comparisons (>, >=, <, <=), ensure we have a numeric value
    let needs_numeric = matches!(
        op,
        Operator::GreaterEqual | Operator::GreaterThan | Operator::LessEqual | Operator::LessThan
    );

    if needs_numeric && !is_numeric {
        // Try to parse string as number for numeric comparisons
        if let CompareTarget::LiteralString(s) = &target {
            if let Ok(num) = s.parse::<f64>() {
                let attr_op = operator_to_numeric_op(&op)?;
                return Ok(DslCondition::AttributeCompare(AttributeComparison {
                    entity_type,
                    attribute: left.attribute,
                    op: attr_op,
                    target: CompareTarget::LiteralNum(num),
                }));
            }
        }
        return Err(ReaperError::InvalidPolicy {
            reason: format!("Operator {:?} requires numeric value, got {:?}", op, value),
        });
    }

    let attr_op = operator_to_numeric_op(&op)?;

    // NOTE: NotEqual must compile NATIVELY, never as Not(Equal). A missing
    // attribute fails Equal (correct), and negating that made absence satisfy
    // every != guard — fail-open, caught by the differential oracle. The
    // evaluator's native NotEqual arm fails closed on missing values, giving
    // the specified semantics (missing satisfies no comparison except
    // explicit null presence checks).

    Ok(DslCondition::AttributeCompare(AttributeComparison {
        entity_type,
        attribute: left.attribute,
        op: attr_op,
        target,
    }))
}

/// Compile comparison: entity1.attr op entity2.attr
pub fn compile_attr_comparison(
    left: EntityAttr,
    op: Operator,
    right: EntityAttr,
) -> Result<DslCondition, ReaperError> {
    let left_type = entity_to_type(&left.entity)?;
    let right_type = entity_to_type(&right.entity)?;
    let left_has_wildcard = matches!(left.index, Some(Index::Wildcard));
    let right_has_wildcard = matches!(right.index, Some(Index::Wildcard));

    // Handle wildcard comparisons (existential quantification)
    if left_has_wildcard || right_has_wildcard {
        return compile_wildcard_comparison(left, op, right, left_has_wildcard, right_has_wildcard);
    }

    // Same-entity comparisons
    if left_type == right_type {
        let attr_op = match op {
            Operator::LessEqual => AttrCompareOp::LessEqual,
            Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
            Operator::LessThan => AttrCompareOp::Less,
            Operator::GreaterThan => AttrCompareOp::Greater,
            Operator::Equal => AttrCompareOp::Equal,
            Operator::NotEqual => AttrCompareOp::NotEqual,
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for same-entity comparisons",
                        op
                    ),
                })
            }
        };

        return Ok(DslCondition::SameEntityAttrCompare {
            entity_type: left_type,
            left_attr: left.attribute,
            right_attr: right.attribute,
            op: attr_op,
        });
    }

    // Cross-entity comparisons - validate operator
    let _numeric_op = operator_to_numeric_op(&op)?;

    match op {
        Operator::Equal => Ok(DslCondition::CrossEntityCompare(CrossEntityComparison {
            left_entity: left_type,
            left_attr: left.attribute,
            op: NumericOp::Equal,
            right_entity: right_type,
            right_attr: right.attribute,
        })),

        // Native NotEqual (never Not(Equal)): missing attributes must fail !=
        // guards, not satisfy them via negation (differential-oracle finding).
        Operator::NotEqual => Ok(DslCondition::CrossEntityCompare(CrossEntityComparison {
            left_entity: left_type,
            left_attr: left.attribute,
            op: NumericOp::NotEqual,
            right_entity: right_type,
            right_attr: right.attribute,
        })),

        Operator::GreaterThan => Ok(DslCondition::CrossEntityCompare(CrossEntityComparison {
            left_entity: left_type,
            left_attr: left.attribute,
            op: NumericOp::Greater,
            right_entity: right_type,
            right_attr: right.attribute,
        })),

        Operator::GreaterEqual => {
            // left >= right is (left > right) OR (left == right)
            Ok(DslCondition::Or(vec![
                DslCondition::CrossEntityCompare(CrossEntityComparison {
                    left_entity: left_type.clone(),
                    left_attr: left.attribute.clone(),
                    op: NumericOp::Greater,
                    right_entity: right_type.clone(),
                    right_attr: right.attribute.clone(),
                }),
                DslCondition::CrossEntityCompare(CrossEntityComparison {
                    left_entity: left_type,
                    left_attr: left.attribute,
                    op: NumericOp::Equal,
                    right_entity: right_type,
                    right_attr: right.attribute,
                }),
            ]))
        }

        Operator::LessEqual => {
            // left <= right is (left < right) OR (left == right)
            Ok(DslCondition::Or(vec![
                DslCondition::CrossEntityCompare(CrossEntityComparison {
                    left_entity: left_type.clone(),
                    left_attr: left.attribute.clone(),
                    op: NumericOp::Less,
                    right_entity: right_type.clone(),
                    right_attr: right.attribute.clone(),
                }),
                DslCondition::CrossEntityCompare(CrossEntityComparison {
                    left_entity: left_type,
                    left_attr: left.attribute,
                    op: NumericOp::Equal,
                    right_entity: right_type,
                    right_attr: right.attribute,
                }),
            ]))
        }

        Operator::LessThan => Ok(DslCondition::CrossEntityCompare(CrossEntityComparison {
            left_entity: left_type,
            left_attr: left.attribute,
            op: NumericOp::Less,
            right_entity: right_type,
            right_attr: right.attribute,
        })),

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Unsupported comparison: {:?}.{} {:?} {:?}.{}",
                left.entity, left.attribute, op, right.entity, right.attribute
            ),
        }),
    }
}

/// Compile wildcard comparison (e.g., user.roles[_] == resource.required_role)
fn compile_wildcard_comparison(
    left: EntityAttr,
    op: Operator,
    right: EntityAttr,
    left_has_wildcard: bool,
    _right_has_wildcard: bool,
) -> Result<DslCondition, ReaperError> {
    // Determine which side has the collection (wildcard) and which has the scalar
    let (collection_entity, collection_attr, scalar_entity, scalar_attr) = if left_has_wildcard {
        (
            entity_to_type(&left.entity)?,
            left.attribute,
            entity_to_type(&right.entity)?,
            right.attribute,
        )
    } else {
        (
            entity_to_type(&right.entity)?,
            right.attribute,
            entity_to_type(&left.entity)?,
            left.attribute,
        )
    };

    // NotEqual is a NEGATED flag, not Not(..): a missing collection or scalar
    // attribute must fail the guard under BOTH == and != (fail closed).
    let negated = match op {
        Operator::Equal => false,
        Operator::NotEqual => true,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("Wildcard comparisons only support == and !=, got {:?}", op),
            })
        }
    };
    Ok(DslCondition::WildcardCompare(WildcardComparison {
        collection_entity,
        collection_attr,
        scalar_entity,
        scalar_attr,
        negated,
    }))
}
