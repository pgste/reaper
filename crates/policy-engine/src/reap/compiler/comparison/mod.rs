//! Comparison compilation.
//!
//! This module handles compilation of comparison operations between entities,
//! variables, and literal values.
//!
//! Submodules:
//! - `entity`: Entity attribute comparisons (user.role == "admin")
//! - `variable`: Variable comparisons (x == "value", x.count() >= 5)
//! - `expression`: Expression comparisons (user.skills.count() >= 5)
//! - `membership`: Membership tests ("admin" in user.roles)

mod entity;
mod expression;
mod membership;
mod variable;

#[cfg(test)]
mod tests;

use crate::evaluators::reaper_dsl::{
    AttrCompareOp, Condition as DslCondition, EntityType, LiteralValue,
};
use crate::reap::ast::{ComparisonLeft, ComparisonRight, Entity, Expr, Operator, Value};
use crate::reap::compiler::expression::compile_expr_compare_assignment;
use reaper_core::ReaperError;

pub use entity::{compile_attr_comparison, compile_value_comparison};
pub use expression::compile_expr_comparison;
pub use membership::compile_membership_test;
pub use variable::{compile_var_attr_comparison, compile_var_attr_comparison_assignment};

/// Compile a comparison into the appropriate DslCondition variant.
///
/// This is the main entry point for comparison compilation, handling:
/// - Entity attribute comparisons (user.role == "admin")
/// - Variable comparisons (x == "value")
/// - Expression comparisons (user.skills.count() >= 5)
/// - Membership tests ("admin" in user.roles)
pub fn compile_comparison(
    left: ComparisonLeft,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Special case: check if this is an "action" or "resource" variable comparison
    if let ComparisonLeft::Expr(Expr::Variable(var_name)) = &left {
        if var_name == "action" || var_name == "resource" {
            // Handle action == "value" and resource == "value" comparisons
            if let ComparisonRight::Value(value) = right {
                let value_str = match value {
                    Value::String(s) => s,
                    Value::Integer(i) => i.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Null => "null".to_string(),
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "{} comparisons only support simple literal values",
                                var_name
                            ),
                        })
                    }
                };
                return match (var_name.as_str(), op) {
                    ("action", Operator::Equal) => {
                        Ok(DslCondition::ActionEquals { value: value_str })
                    }
                    ("action", Operator::NotEqual) => {
                        Ok(DslCondition::Not(Box::new(DslCondition::ActionEquals {
                            value: value_str,
                        })))
                    }
                    ("resource", Operator::Equal) => {
                        Ok(DslCondition::ResourceIdEquals { value: value_str })
                    }
                    ("resource", Operator::NotEqual) => Ok(DslCondition::Not(Box::new(
                        DslCondition::ResourceIdEquals { value: value_str },
                    ))),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Operator {:?} not supported for {} comparisons. Use == or !=.",
                            op, var_name
                        ),
                    }),
                };
            } else {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "{} comparisons must be against literal values (e.g., {} == \"value\")",
                        var_name, var_name
                    ),
                });
            }
        }
    }

    // Handle "in" operator for membership tests: "admin" in user.roles
    if op == Operator::In {
        return compile_membership_test(left, right);
    }

    // Extract EntityAttr from left - handle var attributes for comprehension filters
    let left_attr = match left {
        ComparisonLeft::EntityAttr(attr) => attr,
        ComparisonLeft::VarAttr(var_attr) => {
            // Handle variable attribute comparisons for comprehension filters: item.priority == "high"
            return compile_var_attr_comparison(var_attr, op, right);
        }
        ComparisonLeft::Expr(expr) => {
            // Handle method calls like user.skills.count() >= 5
            return compile_expr_comparison(expr, op, right);
        }
    };

    match right {
        ComparisonRight::Value(value) => compile_value_comparison(left_attr, op, value),
        ComparisonRight::EntityAttr(right_attr) => {
            compile_attr_comparison(left_attr, op, right_attr)
        }
        ComparisonRight::VarAttr(var_attr) => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Variable attribute access '{}.{}' is not supported in compiled policies. \
                    Variable attributes require direct AST evaluation.",
                var_attr.variable, var_attr.attribute
            ),
        }),
        ComparisonRight::Variable(_) => Err(ReaperError::InvalidPolicy {
            reason: "Variable references are not yet supported in compiled policies".to_string(),
        }),
        ComparisonRight::Expr(_) => Err(ReaperError::InvalidPolicy {
            reason: "Expression comparisons (method calls, etc.) are not supported in compiled policies. \
                Use .reap format with AST evaluation for expression support.".to_string(),
        }),
    }
}

/// Compile a comparison assignment: x := user.value >= 0
pub fn compile_comparison_assignment(
    variable: String,
    left: ComparisonLeft,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Handle variable attribute comparisons: x := p.active == false
    if let ComparisonLeft::VarAttr(var_attr) = &left {
        return compile_var_attr_comparison_assignment(variable, var_attr.clone(), op, right);
    }

    // Extract entity attribute from left
    let (entity_type, attribute) = match left {
        ComparisonLeft::EntityAttr(attr) => {
            let etype = match attr.entity {
                Entity::User => EntityType::User,
                Entity::Resource => EntityType::Resource,
                Entity::Context => EntityType::Context,
            };
            (etype, attr.attribute)
        }
        ComparisonLeft::Expr(expr) => {
            // Handle expression comparison assignment: x := user.name.count() > 0
            // Compile the expression and create an ExprCompareAssignment
            return compile_expr_compare_assignment(variable, expr, op, right);
        }
        ComparisonLeft::VarAttr(_) => unreachable!(), // Handled above
    };

    // Check for null comparison first
    if let ComparisonRight::Value(Value::Null) = &right {
        let is_null_check = match op {
            Operator::Equal => true,     // x := entity.field == null
            Operator::NotEqual => false, // x := entity.field != null
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Null comparisons only support == and != operators".to_string(),
                })
            }
        };
        return Ok(DslCondition::NullComparisonAssignment {
            variable,
            entity_type,
            attribute,
            is_null_check,
        });
    }

    // Extract literal value from right
    let value = match right {
        ComparisonRight::Value(Value::Integer(i)) => LiteralValue::Int(i),
        ComparisonRight::Value(Value::Float(f)) => LiteralValue::Int(f as i64),
        ComparisonRight::Value(Value::String(s)) => LiteralValue::String(s),
        ComparisonRight::Value(Value::Boolean(b)) => LiteralValue::Bool(b),
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Comparison assignments require literal value on right side".to_string(),
            })
        }
    };

    // Convert operator
    let attr_op = match op {
        Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
        Operator::GreaterThan => AttrCompareOp::Greater,
        Operator::LessEqual => AttrCompareOp::LessEqual,
        Operator::LessThan => AttrCompareOp::Less,
        Operator::Equal => AttrCompareOp::Equal,
        Operator::NotEqual => AttrCompareOp::NotEqual,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("Operator {:?} not supported for comparison assignments", op),
            })
        }
    };

    Ok(DslCondition::ComparisonAssignment {
        variable,
        entity_type,
        attribute,
        op: attr_op,
        value,
    })
}
