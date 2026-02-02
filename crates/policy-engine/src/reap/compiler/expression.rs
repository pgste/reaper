//! Expression type compilation.
//!
//! This module handles compilation of expressions to ExprType, which represents
//! operations like method calls, function calls, and indexed access.

use super::helpers::{extract_entity_attr, extract_int_literal, extract_string_array, extract_string_literal};
use super::super::ast::{ComparisonRight, Expr, Index, MethodName, Operator, Value};
use crate::evaluators::reaper_dsl::{
    AttrCompareOp, ChainMethod, Condition as DslCondition, ExprIndexType, ExprType, LiteralValue,
};
use reaper_core::ReaperError;

/// Compile an expression assignment: x := user.name.lower()
pub fn compile_expression_assignment(variable: String, expr: Expr) -> Result<DslCondition, ReaperError> {
    let expr_type = compile_expr_to_type(expr)?;
    Ok(DslCondition::ExpressionAssignment { variable, expr_type })
}

/// Compile an expression comparison assignment: x := user.name.count() > 0
/// Evaluates the expression, compares with a literal, and stores the boolean result
pub fn compile_expr_compare_assignment(
    variable: String,
    expr: Expr,
    op: Operator,
    right: ComparisonRight,
) -> Result<DslCondition, ReaperError> {
    // Compile the expression to an ExprType
    let expr_type = compile_expr_to_type(expr)?;

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
                reason: format!(
                    "Operator {:?} not supported for expression comparison assignments",
                    op
                ),
            })
        }
    };

    // Extract literal value from right
    let value = match right {
        ComparisonRight::Value(Value::Integer(i)) => LiteralValue::Int(i),
        ComparisonRight::Value(Value::Float(f)) => LiteralValue::Int(f as i64),
        ComparisonRight::Value(Value::String(s)) => LiteralValue::String(s),
        ComparisonRight::Value(Value::Boolean(b)) => LiteralValue::Bool(b),
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "Expression comparison assignments require literal value on right side"
                    .to_string(),
            })
        }
    };

    Ok(DslCondition::ExprCompareAssignment {
        variable,
        expr_type,
        op: attr_op,
        value,
    })
}

/// Compile an expression to an ExprType
pub fn compile_expr_to_type(expr: Expr) -> Result<ExprType, ReaperError> {
    match expr {
        Expr::MethodCall {
            receiver,
            method,
            args,
        } => {
            // Check if receiver is an entity attribute access
            if let Ok((entity_type, attribute)) = extract_entity_attr(&receiver) {
                // Direct method call on entity attribute
                match method {
                    MethodName::Lower => Ok(ExprType::StringLower {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Upper => Ok(ExprType::StringUpper {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Trim => Ok(ExprType::StringTrim {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Split => {
                        let delimiter = if args.is_empty() {
                            " ".to_string()
                        } else {
                            extract_string_literal(&args[0])?
                        };
                        Ok(ExprType::StringSplit {
                            entity_type,
                            attribute,
                            delimiter,
                        })
                    }
                    MethodName::Count => Ok(ExprType::CollectionCount {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Sum => Ok(ExprType::CollectionSum {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Max => Ok(ExprType::CollectionMax {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Min => Ok(ExprType::CollectionMin {
                        entity_type,
                        attribute,
                    }),
                    MethodName::First => Ok(ExprType::CollectionFirst {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Last => Ok(ExprType::CollectionLast {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Slice => {
                        // .slice(start, end)
                        if args.len() != 2 {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!(".slice() requires 2 arguments, got {}", args.len()),
                            });
                        }
                        let start = extract_int_literal(&args[0])?;
                        let end = extract_int_literal(&args[1])?;
                        Ok(ExprType::CollectionSlice {
                            entity_type,
                            attribute,
                            start,
                            end,
                        })
                    }
                    MethodName::Reverse => Ok(ExprType::CollectionReverse {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Sort => Ok(ExprType::CollectionSort {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Unique => Ok(ExprType::CollectionUnique {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Difference => {
                        // .difference(other_collection) - need to parse other entity.attr
                        if args.len() != 1 {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!(".difference() requires 1 argument, got {}", args.len()),
                            });
                        }
                        // args[0] should be an entity attribute like user.forbidden_content
                        let (other_entity_type, other_attribute) = extract_entity_attr(&args[0])?;
                        Ok(ExprType::CollectionDifference {
                            entity_type,
                            attribute,
                            other_entity_type,
                            other_attribute,
                        })
                    }
                    MethodName::Keys => Ok(ExprType::SetKeys {
                        entity_type,
                        attribute,
                    }),
                    MethodName::Intersection => {
                        // .intersection(other_collection) - parse entity.attr
                        if args.len() != 1 {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!(".intersection() requires 1 argument, got {}", args.len()),
                            });
                        }
                        let (other_entity_type, other_attribute) = extract_entity_attr(&args[0])?;
                        Ok(ExprType::CollectionIntersection {
                            entity_type,
                            attribute,
                            other_entity_type,
                            other_attribute,
                        })
                    }
                    MethodName::Union => {
                        // .union(other_collection) - parse entity.attr
                        if args.len() != 1 {
                            return Err(ReaperError::InvalidPolicy {
                                reason: format!(".union() requires 1 argument, got {}", args.len()),
                            });
                        }
                        let (other_entity_type, other_attribute) = extract_entity_attr(&args[0])?;
                        Ok(ExprType::CollectionUnion {
                            entity_type,
                            attribute,
                            other_entity_type,
                            other_attribute,
                        })
                    }
                    MethodName::Matches => {
                        let pattern = extract_string_literal(&args[0])?;
                        Ok(ExprType::RegexMatches {
                            entity_type,
                            attribute,
                            pattern,
                        })
                    }
                    MethodName::Contains => {
                        let substring = extract_string_literal(&args[0])?;
                        Ok(ExprType::StringContains {
                            entity_type,
                            attribute,
                            substring,
                        })
                    }
                    MethodName::Startswith => {
                        let prefix = extract_string_literal(&args[0])?;
                        Ok(ExprType::StringStartsWithExpr {
                            entity_type,
                            attribute,
                            prefix,
                        })
                    }
                    MethodName::Endswith => {
                        let suffix = extract_string_literal(&args[0])?;
                        Ok(ExprType::StringEndsWithExpr {
                            entity_type,
                            attribute,
                            suffix,
                        })
                    }
                    _ => Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Method .{}() is not supported for expression assignments",
                            method.as_str()
                        ),
                    }),
                }
            } else {
                // Chained method call: could be variable.method() or expr.method()
                let base = compile_expr_to_type(*receiver)?;
                let chain_method = match method {
                    // String methods
                    MethodName::Lower => ChainMethod::Lower,
                    MethodName::Upper => ChainMethod::Upper,
                    MethodName::Trim => ChainMethod::Trim,
                    MethodName::Contains => {
                        let substring = extract_string_literal(&args[0])?;
                        ChainMethod::Contains { substring }
                    }
                    MethodName::Startswith => {
                        let prefix = extract_string_literal(&args[0])?;
                        ChainMethod::Startswith { prefix }
                    }
                    MethodName::Endswith => {
                        let suffix = extract_string_literal(&args[0])?;
                        ChainMethod::Endswith { suffix }
                    }
                    // Collection methods
                    MethodName::Count => ChainMethod::Count,
                    MethodName::Sum => ChainMethod::Sum,
                    MethodName::Max => ChainMethod::Max,
                    MethodName::Min => ChainMethod::Min,
                    MethodName::First => ChainMethod::First,
                    MethodName::Last => ChainMethod::Last,
                    MethodName::Reverse => ChainMethod::Reverse,
                    MethodName::Sort => ChainMethod::Sort,
                    MethodName::Unique => ChainMethod::Unique,
                    MethodName::Keys => ChainMethod::Keys,
                    // Set operations with literal array
                    MethodName::Intersection => {
                        let values = extract_string_array(&args)?;
                        ChainMethod::Intersection { values }
                    }
                    MethodName::Union => {
                        let values = extract_string_array(&args)?;
                        ChainMethod::Union { values }
                    }
                    MethodName::Difference => {
                        let values = extract_string_array(&args)?;
                        ChainMethod::Difference { values }
                    }
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Method .{}() is not supported for chained expression assignments",
                                method.as_str()
                            ),
                        })
                    }
                };
                Ok(ExprType::ChainedMethod {
                    base: Box::new(base),
                    method: chain_method,
                })
            }
        }

        Expr::FunctionCall {
            namespace,
            function,
            args,
        } => {
            let ns = namespace.as_deref().unwrap_or("");
            match (ns, function.as_str()) {
                ("time", "now") => Ok(ExprType::TimeNow),
                ("time", "now_ms") => Ok(ExprType::TimeNowMs),
                ("time", "now_ns") => Ok(ExprType::TimeNowNs),
                ("regex", "matches") => {
                    if args.len() != 2 {
                        return Err(ReaperError::InvalidPolicy {
                            reason: "regex::matches requires 2 arguments".to_string(),
                        });
                    }
                    let (entity_type, attribute) = extract_entity_attr(&args[0])?;
                    let pattern = extract_string_literal(&args[1])?;
                    Ok(ExprType::RegexMatches {
                        entity_type,
                        attribute,
                        pattern,
                    })
                }
                _ => Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Function {}::{} is not supported for expression assignments",
                        ns, function
                    ),
                }),
            }
        }

        Expr::Variable(var_name) => Ok(ExprType::VariableRef { variable: var_name }),

        Expr::IndexedAccess {
            variable,
            attribute,
            index,
        } => {
            // Handle variable indexed access: row[_], row[0], row["key"]
            // or variable attribute indexed access: first_dept.projects[0]
            let index_type = match index {
                Index::Wildcard => ExprIndexType::Wildcard,
                Index::Number(n) => ExprIndexType::Number(n),
                Index::String(s) => ExprIndexType::String(s),
            };
            if attribute.is_empty() {
                // Simple variable indexed access: row[0]
                Ok(ExprType::VariableIndexed {
                    variable,
                    index: index_type,
                })
            } else {
                // Variable attribute indexed access: first_dept.projects[0]
                Ok(ExprType::VariableAttrIndexed {
                    variable,
                    attribute,
                    index: index_type,
                })
            }
        }

        Expr::AttributeAccess {
            variable,
            attribute,
        } => {
            // Check if this is an entity (user/resource/context)
            let is_entity = matches!(variable.as_str(), "user" | "resource" | "context");
            if is_entity {
                // Entity attribute access - this should have been handled earlier
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Entity attribute access {}.{} should use entity-specific expression types",
                        variable, attribute
                    ),
                });
            }
            // Variable attribute access: first_group.items
            Ok(ExprType::VariableAttrAccess {
                variable,
                attribute,
            })
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Expression type {:?} is not supported for expression assignments",
                expr
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_variable_ref() {
        let expr = Expr::Variable("my_var".to_string());
        let result = compile_expr_to_type(expr).unwrap();
        assert!(matches!(result, ExprType::VariableRef { variable } if variable == "my_var"));
    }

    #[test]
    fn test_compile_time_now() {
        let expr = Expr::FunctionCall {
            namespace: Some("time".to_string()),
            function: "now".to_string(),
            args: vec![],
        };
        let result = compile_expr_to_type(expr).unwrap();
        assert!(matches!(result, ExprType::TimeNow));
    }

    #[test]
    fn test_compile_indexed_access() {
        let expr = Expr::IndexedAccess {
            variable: "items".to_string(),
            attribute: String::new(),
            index: Index::Number(0),
        };
        let result = compile_expr_to_type(expr).unwrap();
        assert!(matches!(
            result,
            ExprType::VariableIndexed { variable, index }
            if variable == "items" && matches!(index, ExprIndexType::Number(0))
        ));
    }

    #[test]
    fn test_compile_var_attr_access() {
        let expr = Expr::AttributeAccess {
            variable: "group".to_string(),
            attribute: "name".to_string(),
        };
        let result = compile_expr_to_type(expr).unwrap();
        assert!(matches!(
            result,
            ExprType::VariableAttrAccess { variable, attribute }
            if variable == "group" && attribute == "name"
        ));
    }

    #[test]
    fn test_compile_chained_method() {
        let expr = Expr::MethodCall {
            receiver: Box::new(Expr::Variable("text".to_string())),
            method: MethodName::Lower,
            args: vec![],
        };
        let result = compile_expr_to_type(expr).unwrap();
        assert!(matches!(
            result,
            ExprType::ChainedMethod { method: ChainMethod::Lower, .. }
        ));
    }
}
