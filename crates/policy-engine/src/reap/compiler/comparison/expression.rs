//! Expression comparison compilation.
//!
//! This module handles compilation of comparisons involving expressions,
//! such as method calls (`.count()`, `.lower()`, `.upper()`).
//!
//! Uses V2 consolidated types for cleaner code.

use super::variable::{
    compile_chained_variable_method_comparison, compile_variable_method_comparison,
};
use crate::evaluators::reaper_dsl::{
    AttrCompareOp, Condition as DslCondition, CountCondition, CountOp, LiteralValue, StringOp,
    StringOperationCondition,
};
use crate::reap::ast::{ComparisonRight, Expr, MethodName, Operator, Value};
use crate::reap::compiler::helpers::{extract_entity_attr, parse_entity_type};
use reaper_core::ReaperError;

/// Compile expression comparison: user.skills.count() >= 5, user.name.lower() == "admin"
pub fn compile_expr_comparison(
    expr: Expr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Handle method calls like user.skills.count(), user.name.lower(), or variable.count()
    if let Expr::MethodCall {
        receiver,
        method,
        args: _,
    } = expr
    {
        // Check if receiver is a simple variable (e.g., all_skills.count()).
        // A dotted name whose prefix is a real entity type (user.skills,
        // resource.tags) is NOT a bound variable — it's an entity-attribute
        // path, and the parser emits it as `Variable("user.skills")`. Route
        // those to the entity-attribute handling below so `.count()` compiles
        // to a CountOp; only genuine bound variables (comprehension results
        // like `all_skills`) go to the variable-method path.
        if let Expr::Variable(var_name) = &*receiver {
            let is_entity_path = var_name
                .split_once('.')
                .map(|(prefix, _)| parse_entity_type(prefix).is_ok())
                .unwrap_or(false);
            if !is_entity_path {
                return compile_variable_method_comparison(var_name.clone(), method, op, right);
            }
        }

        // Check if receiver is a method call on a variable (e.g., t.trim().count())
        // This is a chained method comparison: var.method1().method2() op value
        if let Expr::MethodCall {
            receiver: inner_receiver,
            method: inner_method,
            args: _,
        } = &*receiver
        {
            if let Expr::Variable(var_name) = &**inner_receiver {
                // This is var.method1().method2() - e.g., t.trim().count()
                return compile_chained_variable_method_comparison(
                    var_name.clone(),
                    inner_method.clone(),
                    method,
                    op,
                    right,
                );
            }
        }

        let (entity_type, attribute) = extract_entity_attr(&receiver)?;

        // Handle .count() method - requires integer on right side
        if method == MethodName::Count {
            let threshold = match right {
                ComparisonRight::Value(Value::Integer(i)) => i as usize,
                ComparisonRight::Value(Value::Float(f)) => f as usize,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "Method call comparisons (e.g., .count()) require integer literal on right side".to_string(),
                    })
                }
            };

            return match op {
                Operator::GreaterEqual => Ok(DslCondition::CountOp(CountCondition {
                    entity_type,
                    attribute,
                    op: CountOp::GreaterEqual,
                    threshold,
                })),
                Operator::GreaterThan => Ok(DslCondition::CountOp(CountCondition {
                    entity_type,
                    attribute,
                    op: CountOp::Greater,
                    threshold,
                })),
                Operator::Equal => Ok(DslCondition::CountOp(CountCondition {
                    entity_type,
                    attribute,
                    op: CountOp::Equal,
                    threshold,
                })),
                Operator::LessEqual => Ok(DslCondition::CountOp(CountCondition {
                    entity_type,
                    attribute,
                    op: CountOp::LessEqual,
                    threshold,
                })),
                Operator::LessThan => Ok(DslCondition::CountOp(CountCondition {
                    entity_type,
                    attribute,
                    op: CountOp::Less,
                    threshold,
                })),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!("Operator {:?} not supported for .count() comparisons", op),
                }),
            };
        }

        // Handle .lower() method - user.name.lower() == "admin"
        if method == MethodName::Lower {
            let value = match right {
                ComparisonRight::Value(Value::String(s)) => s,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: ".lower() comparisons require string literal on right side"
                            .to_string(),
                    })
                }
            };

            return match op {
                Operator::Equal => Ok(DslCondition::StringOp(StringOperationCondition {
                    entity_type,
                    attribute,
                    op: StringOp::LowerEquals,
                    value,
                })),
                // NotEqual compiles NATIVELY, never as Not(Equal): a missing
                // attribute must fail the guard (fail closed), and Not() would
                // invert that miss into a pass.
                Operator::NotEqual => Ok(DslCondition::StringOp(StringOperationCondition {
                    entity_type,
                    attribute,
                    op: StringOp::LowerNotEquals,
                    value,
                })),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for .lower() comparisons. Use == or !=",
                        op
                    ),
                }),
            };
        }

        // Handle .upper() method - user.code.upper() == "ADMIN"
        if method == MethodName::Upper {
            let value = match right {
                ComparisonRight::Value(Value::String(s)) => s,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: ".upper() comparisons require string literal on right side"
                            .to_string(),
                    })
                }
            };

            return match op {
                Operator::Equal => Ok(DslCondition::StringOp(StringOperationCondition {
                    entity_type,
                    attribute,
                    op: StringOp::UpperEquals,
                    value,
                })),
                // Native NotEqual — see the .lower() arm for why.
                Operator::NotEqual => Ok(DslCondition::StringOp(StringOperationCondition {
                    entity_type,
                    attribute,
                    op: StringOp::UpperNotEquals,
                    value,
                })),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for .upper() comparisons. Use == or !=",
                        op
                    ),
                }),
            };
        }

        return Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported in compiled policy comparisons. \
                Supported methods: .count(), .lower(), .upper()",
                method.as_str()
            ),
        });
    }

    // Handle variable comparisons: lower_name == "admin"
    if let Expr::Variable(var_name) = expr {
        // Handle null comparisons specially
        if let ComparisonRight::Value(Value::Null) = right {
            return match op {
                Operator::Equal => Ok(DslCondition::VariableIsNull { variable: var_name }),
                Operator::NotEqual => Ok(DslCondition::VariableIsNotNull { variable: var_name }),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!("Null comparisons only support == and !=, got {:?}", op),
                }),
            };
        }

        // Check for variable-to-variable comparison
        if let ComparisonRight::Variable(other_var) = &right {
            return match op {
                Operator::Equal => Ok(DslCondition::VariableEqualsVariable {
                    left: var_name,
                    right: other_var.clone(),
                }),
                Operator::NotEqual => Ok(DslCondition::VariableNotEqualsVariable {
                    left: var_name,
                    right: other_var.clone(),
                }),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Variable-to-variable comparisons only support == and !=, got {:?}",
                        op
                    ),
                }),
            };
        }

        // Get the literal value from the right side
        let value = match right {
            ComparisonRight::Value(Value::String(s)) => LiteralValue::String(s),
            ComparisonRight::Value(Value::Integer(i)) => LiteralValue::Int(i),
            ComparisonRight::Value(Value::Boolean(b)) => LiteralValue::Bool(b),
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Variable comparisons require literal value on right side, got {:?}",
                        right
                    ),
                });
            }
        };

        return match op {
            Operator::Equal => Ok(DslCondition::VariableEqualsLiteral {
                variable: var_name,
                value,
            }),
            // Native NotEqual: an unbound variable must fail the guard.
            Operator::NotEqual => Ok(DslCondition::VariableNotEqualsLiteral {
                variable: var_name,
                value,
            }),
            Operator::GreaterEqual => Ok(DslCondition::VariableCompare {
                variable: var_name,
                op: AttrCompareOp::GreaterEqual,
                value,
            }),
            Operator::GreaterThan => Ok(DslCondition::VariableCompare {
                variable: var_name,
                op: AttrCompareOp::Greater,
                value,
            }),
            Operator::LessEqual => Ok(DslCondition::VariableCompare {
                variable: var_name,
                op: AttrCompareOp::LessEqual,
                value,
            }),
            Operator::LessThan => Ok(DslCondition::VariableCompare {
                variable: var_name,
                op: AttrCompareOp::Less,
                value,
            }),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!("Operator {:?} not supported for variable comparisons", op),
            }),
        };
    }

    Err(ReaperError::InvalidPolicy {
        reason:
            "Expression comparisons only support method calls like .count(), .lower(), .upper()"
                .to_string(),
    })
}
