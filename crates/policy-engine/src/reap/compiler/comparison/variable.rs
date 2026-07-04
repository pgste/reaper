//! Variable comparison compilation.
//!
//! This module handles compilation of comparisons involving variables,
//! including variable method calls and variable attribute comparisons.

use crate::evaluators::reaper_dsl::{
    AttrCompareOp, Condition as DslCondition, LiteralValue, VariableMethod, VariableStringTransform,
};
use crate::reap::ast::{ComparisonRight, MethodName, Operator, Value, VarAttr};
use reaper_core::ReaperError;

/// Compile comparison with method call on a variable: all_skills.count() >= 2
pub fn compile_variable_method_comparison(
    var_name: String,
    method: MethodName,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    let threshold = match right {
        ComparisonRight::Value(Value::Integer(i)) => i,
        ComparisonRight::Value(Value::Float(f)) => f as i64,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Variable method comparisons require integer literal on right side"
                    .to_string(),
            })
        }
    };

    let var_method = match method {
        MethodName::Count => VariableMethod::Count,
        MethodName::Sum => VariableMethod::Sum,
        MethodName::Max => VariableMethod::Max,
        MethodName::Min => VariableMethod::Min,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Method .{}() is not supported for variable comparisons. \
                    Supported: .count(), .sum(), .max(), .min()",
                    method.as_str()
                ),
            })
        }
    };

    let attr_op = match op {
        Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
        Operator::GreaterThan => AttrCompareOp::Greater,
        Operator::Equal => AttrCompareOp::Equal,
        Operator::NotEqual => AttrCompareOp::NotEqual,
        Operator::LessEqual => AttrCompareOp::LessEqual,
        Operator::LessThan => AttrCompareOp::Less,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Operator {:?} not supported for variable method comparisons",
                    op
                ),
            })
        }
    };

    Ok(DslCondition::VariableMethodCompare {
        variable: var_name,
        method: var_method,
        op: attr_op,
        value: LiteralValue::Int(threshold),
    })
}

/// Compile chained variable method comparison: t.trim().count() > 0
/// This handles patterns like var.transform_method().compare_method() op value
pub fn compile_chained_variable_method_comparison(
    var_name: String,
    first_method: MethodName,
    second_method: MethodName,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Convert first method to transform
    let transform_method = match first_method {
        MethodName::Trim => VariableStringTransform::Trim,
        MethodName::Lower => VariableStringTransform::Lower,
        MethodName::Upper => VariableStringTransform::Upper,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Method .{}() is not supported as the first method in chained comparisons. \
                    Supported: .trim(), .lower(), .upper()",
                    first_method.as_str()
                ),
            })
        }
    };

    // Convert second method to comparison method
    let compare_method = match second_method {
        MethodName::Count => VariableMethod::Count,
        MethodName::Sum => VariableMethod::Sum,
        MethodName::Max => VariableMethod::Max,
        MethodName::Min => VariableMethod::Min,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Method .{}() is not supported as the second method in chained comparisons. \
                    Supported: .count(), .sum(), .max(), .min()",
                    second_method.as_str()
                ),
            })
        }
    };

    // Get the threshold value
    let threshold = match right {
        ComparisonRight::Value(Value::Integer(i)) => i,
        ComparisonRight::Value(Value::Float(f)) => f as i64,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Chained method comparisons require integer literal on right side"
                    .to_string(),
            })
        }
    };

    // Convert operator
    let attr_op = match op {
        Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
        Operator::GreaterThan => AttrCompareOp::Greater,
        Operator::Equal => AttrCompareOp::Equal,
        Operator::NotEqual => AttrCompareOp::NotEqual,
        Operator::LessEqual => AttrCompareOp::LessEqual,
        Operator::LessThan => AttrCompareOp::Less,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Operator {:?} not supported for chained method comparisons",
                    op
                ),
            })
        }
    };

    Ok(DslCondition::VariableChainedMethodCompare {
        variable: var_name,
        transform_method,
        compare_method,
        op: attr_op,
        value: LiteralValue::Int(threshold),
    })
}

/// Compile comparison: var.attr op value (for comprehension filters)
pub fn compile_var_attr_comparison(
    var_attr: VarAttr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    match right {
        ComparisonRight::Value(value) => {
            // Handle null comparisons
            if matches!(value, Value::Null) {
                return match op {
                    Operator::Equal => Ok(DslCondition::VariableAttrEqualsNull {
                        variable: var_attr.variable,
                        attribute: var_attr.attribute,
                    }),
                    Operator::NotEqual => Ok(DslCondition::VariableAttrNotEqualsNull {
                        variable: var_attr.variable,
                        attribute: var_attr.attribute,
                    }),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Operator {:?} not supported for null comparisons. Use == or !=.",
                            op
                        ),
                    }),
                };
            }

            // Convert value to LiteralValue
            let literal_value = match value {
                Value::String(s) => LiteralValue::String(s),
                Value::Integer(i) => LiteralValue::Int(i),
                Value::Float(f) => LiteralValue::Int(f as i64), // Convert float to int
                Value::Boolean(b) => LiteralValue::Bool(b),
                Value::Null => unreachable!(), // handled above
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "Variable attribute comparisons only support primitive values, not arrays/objects".to_string(),
                    })
                }
            };

            match op {
                // NotEqual compiles NATIVELY (never Not(Equal)): a missing
                // attribute must fail the guard, not satisfy it (fail closed).
                Operator::Equal => Ok(DslCondition::VariableAttrEqualsLiteral {
                    variable: var_attr.variable,
                    attribute: var_attr.attribute,
                    value: literal_value,
                }),
                Operator::NotEqual => Ok(DslCondition::VariableAttrNotEqualsLiteral {
                    variable: var_attr.variable,
                    attribute: var_attr.attribute,
                    value: literal_value,
                }),
                Operator::GreaterEqual
                | Operator::GreaterThan
                | Operator::LessEqual
                | Operator::LessThan => {
                    // For numeric comparisons, use VariableAttrCompare
                    let attr_op = match op {
                        Operator::GreaterEqual => AttrCompareOp::GreaterEqual,
                        Operator::GreaterThan => AttrCompareOp::Greater,
                        Operator::LessEqual => AttrCompareOp::LessEqual,
                        Operator::LessThan => AttrCompareOp::Less,
                        _ => unreachable!(),
                    };
                    Ok(DslCondition::VariableAttrCompare {
                        variable: var_attr.variable,
                        attribute: var_attr.attribute,
                        op: attr_op,
                        value: literal_value,
                    })
                }
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Operator {:?} not supported for variable attribute comparisons.",
                        op
                    ),
                }),
            }
        }
        ComparisonRight::Variable(var_name) => {
            // var.attr == other_var - not commonly needed, but could be supported
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Variable attribute to variable comparisons not yet supported: {}.{} {:?} {}",
                    var_attr.variable, var_attr.attribute, op, var_name
                ),
            })
        }
        ComparisonRight::VarAttr(other_var_attr) => {
            // var.attr == other_var.attr - not commonly needed
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Variable attribute to variable attribute comparisons not yet supported: {}.{} {:?} {}.{}",
                    var_attr.variable, var_attr.attribute, op, other_var_attr.variable, other_var_attr.attribute
                ),
            })
        }
        ComparisonRight::EntityAttr(_) => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Variable attribute to entity attribute comparisons not yet supported: {}.{}",
                var_attr.variable, var_attr.attribute
            ),
        }),
        ComparisonRight::Expr(_) => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Variable attribute to expression comparisons not yet supported: {}.{}",
                var_attr.variable, var_attr.attribute
            ),
        }),
    }
}

/// Compile variable attribute comparison assignment: x := var.attr op value
/// This handles patterns like: is_active := p.active == true
pub fn compile_var_attr_comparison_assignment(
    result_variable: String,
    var_attr: VarAttr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Compile variable attribute comparison assignments that store the result in a variable
    match right {
        ComparisonRight::Value(value) => {
            // Handle null comparisons: has_items := first_cat.items != null
            if matches!(value, Value::Null) {
                return match op {
                    Operator::Equal => Ok(DslCondition::VarAttrNullCompareAssignment {
                        result_variable,
                        source_variable: var_attr.variable,
                        attribute: var_attr.attribute,
                        is_null_check: true, // == null
                    }),
                    Operator::NotEqual => Ok(DslCondition::VarAttrNullCompareAssignment {
                        result_variable,
                        source_variable: var_attr.variable,
                        attribute: var_attr.attribute,
                        is_null_check: false, // != null (result is true if NOT null)
                    }),
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Operator {:?} not supported for null comparisons. Use == or !=.",
                            op
                        ),
                    }),
                };
            }

            // Convert value to LiteralValue
            let literal_value = match value {
                Value::String(s) => LiteralValue::String(s),
                Value::Integer(i) => LiteralValue::Int(i),
                Value::Float(f) => LiteralValue::Int(f as i64),
                Value::Boolean(b) => LiteralValue::Bool(b),
                Value::Null => unreachable!(),
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "Variable attribute comparison assignments only support primitive values".to_string(),
                    })
                }
            };

            // AUDIT (differential correctness program): the previous
            // compilation DROPPED `result_variable` and emitted a bare guard
            // condition — `x := p.active == true` silently became
            // `p.active == true`, gating the rule instead of binding x. That
            // is a semantics divergence from the AST evaluator. Until a real
            // ComparisonResultAssignment exists for var.attr sources, reject
            // compilation so the engine falls back to the (correct) AST
            // evaluator for these policies.
            let _ = literal_value;
            Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "comparison-result assignment `{} := {}.{} {:?} <literal>` is not \
                     supported by the compiled evaluator yet (AST evaluator handles it)",
                    result_variable, var_attr.variable, var_attr.attribute, op
                ),
            })
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Variable attribute comparison assignments only support literal values on right side: {}.{} {:?} ...",
                var_attr.variable, var_attr.attribute, op
            ),
        }),
    }
}
