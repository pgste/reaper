//! Membership test compilation.
//!
//! This module handles compilation of membership tests like "admin" in user.roles.

use crate::evaluators::reaper_dsl::{Condition as DslCondition, EntityType, LiteralValue};
use crate::reap::ast::{ComparisonLeft, ComparisonRight, Entity, Expr, Value};
use reaper_core::ReaperError;

/// Compile a membership test: "admin" in user.roles or "value" in variable
pub fn compile_membership_test(
    left: ComparisonLeft,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Parser represents "value in collection" as: left=collection, op=In, right=value
    // So left is the entity attribute (collection) and right is the literal value to search for

    // Extract the literal value from the right side first
    let literal_value = match &right {
        ComparisonRight::Value(value) => match value {
            Value::String(s) => LiteralValue::String(s.clone()),
            Value::Integer(i) => LiteralValue::Int(*i),
            Value::Boolean(b) => LiteralValue::Bool(*b),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Only string, integer, and boolean literals are supported in membership tests".to_string(),
                })
            }
        },
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Right side of 'in' operator should be a literal value (e.g., \"admin\" in user.roles)".to_string(),
            })
        }
    };

    // Check left side - entity attribute or variable
    match left {
        ComparisonLeft::EntityAttr(attr) => {
            let entity_type = match attr.entity {
                Entity::User => EntityType::User,
                Entity::Resource => EntityType::Resource,
                Entity::Context => EntityType::Context,
                Entity::Actor => EntityType::Actor,
                Entity::Input => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "`input` document access is not compiled yet; policy runs on the AST evaluator".to_string(),
                    })
                },
            };
            Ok(DslCondition::MembershipTest {
                value: literal_value,
                entity_type,
                attribute: attr.attribute,
                index: None,
            })
        }
        ComparisonLeft::Expr(Expr::Variable(var_name)) => {
            // "value" in variable - membership test against a variable
            Ok(DslCondition::VariableMembershipTest {
                value: literal_value,
                variable: var_name,
            })
        }
        // "lit" in var.attr (R4-01 B.2b), e.g. `"delete" in rc.change.actions`
        // over comprehension elements. Dotted attribute paths navigate at eval.
        ComparisonLeft::VarAttr(var_attr) => Ok(DslCondition::VariableAttrMembershipTest {
            value: literal_value,
            variable: var_attr.variable,
            attribute: var_attr.attribute,
        }),
        _ => {
            Err(ReaperError::InvalidPolicy {
                reason: "Left side of 'in' operator should be an entity attribute collection (e.g., user.roles) or a variable".to_string(),
            })
        }
    }
}
