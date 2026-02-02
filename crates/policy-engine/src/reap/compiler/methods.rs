//! Method call compilation.
//!
//! This module handles compilation of method calls like `.contains()`, `.startswith()`,
//! `.matches()`, etc. to DSL conditions.
//!
//! Uses V2 consolidated types for cleaner code.

use super::helpers::{extract_entity_attr, extract_string_array, extract_string_literal};
use super::super::ast::{Expr, MethodName};
use crate::evaluators::reaper_dsl::{
    Condition as DslCondition, EntityType as DslEntityType, StringOp,
    StringOperationCondition, VariableCollectionMethod, VariableStringOperationCondition,
};
use reaper_core::ReaperError;

/// Compile a method call expression.
///
/// Routes to the appropriate compilation function based on receiver type:
/// - Entity attribute (user.email) -> compile_entity_method_call
/// - Variable (trimmed_email) -> compile_variable_method_call
/// - Variable attribute (d.permissions) -> compile_variable_attr_method_call
pub fn compile_method_call(
    receiver: Expr,
    method: MethodName,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    // Check if receiver is a variable (for variable-based method calls)
    if let Expr::Variable(var_name) = &receiver {
        // Check if this is a pseudo-variable representing an entity attribute (e.g., "user.email")
        // The parser creates these for method chains like user.email.contains(...)
        if let Some((entity, attr)) = var_name.split_once('.') {
            if matches!(entity, "user" | "resource" | "context") {
                // This is actually an entity attribute, not a variable
                // Convert back to entity attribute format and continue
                let entity_type = match entity {
                    "user" => DslEntityType::User,
                    "resource" => DslEntityType::Resource,
                    "context" => DslEntityType::Context,
                    _ => unreachable!(),
                };
                return compile_entity_method_call(entity_type, attr.to_string(), method, args);
            }
        }
        // It's a regular variable, compile as variable method call
        return compile_variable_method_call(var_name.clone(), method, args);
    }

    // Check if receiver is a variable attribute (e.g., d.permissions for comprehension variable d)
    // This happens when the variable is NOT an entity (user/resource/context)
    if let Expr::AttributeAccess { variable, attribute } = &receiver {
        let is_entity = matches!(variable.as_str(), "user" | "resource" | "context");
        if !is_entity {
            // It's a variable attribute method call, like d.permissions.contains("execute")
            return compile_variable_attr_method_call(
                variable.clone(),
                attribute.clone(),
                method,
                args,
            );
        }
    }

    // Extract entity type and attribute from receiver
    let (entity_type, attribute) = extract_entity_attr(&receiver)?;

    match method {
        MethodName::Contains => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".contains() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::Contains,
                value,
            }))
        }

        MethodName::Startswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".startswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::StartsWith,
                value,
            }))
        }

        MethodName::Endswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".endswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::EndsWith,
                value,
            }))
        }

        MethodName::Matches => {
            // .matches("pattern") is an alias for regex::matches
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".matches() requires 1 argument, got {}", args.len()),
                });
            }
            let pattern = extract_string_literal(&args[0])?;

            // Validate regex pattern at compile time
            if regex::Regex::new(&pattern).is_err() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Invalid regex pattern: {}", pattern),
                });
            }

            // RegexMatches stays as-is (no V2 equivalent yet)
            Ok(DslCondition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            })
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported in compiled policies. \
                Supported methods: .contains(), .startswith(), .endswith(), .matches()",
                method.as_str()
            ),
        }),
    }
}

/// Compile a method call on an entity attribute (e.g., user.email.contains("@"))
/// Used when the parser creates a pseudo-variable like "user.email"
pub fn compile_entity_method_call(
    entity_type: DslEntityType,
    attribute: String,
    method: MethodName,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    match method {
        MethodName::Contains => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".contains() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::Contains,
                value,
            }))
        }

        MethodName::Startswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".startswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::StartsWith,
                value,
            }))
        }

        MethodName::Endswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".endswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::StringOp(StringOperationCondition {
                entity_type,
                attribute,
                op: StringOp::EndsWith,
                value,
            }))
        }

        MethodName::Matches => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".matches() requires 1 argument, got {}", args.len()),
                });
            }
            let pattern = extract_string_literal(&args[0])?;

            // Validate regex pattern at compile time
            if regex::Regex::new(&pattern).is_err() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Invalid regex pattern: {}", pattern),
                });
            }

            // RegexMatches stays as-is (no V2 equivalent yet)
            Ok(DslCondition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            })
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported in compiled policies. \
                Supported methods: .contains(), .startswith(), .endswith(), .matches()",
                method.as_str()
            ),
        }),
    }
}

/// Compile a method call on a variable (e.g., trimmed_email.contains("@"))
pub fn compile_variable_method_call(
    variable: String,
    method: MethodName,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    match method {
        MethodName::Contains => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".contains() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::VariableStringOp(VariableStringOperationCondition {
                variable,
                op: StringOp::Contains,
                value,
            }))
        }

        MethodName::Startswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".startswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::VariableStringOp(VariableStringOperationCondition {
                variable,
                op: StringOp::StartsWith,
                value,
            }))
        }

        MethodName::Endswith => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".endswith() requires 1 argument, got {}", args.len()),
                });
            }
            let value = extract_string_literal(&args[0])?;
            Ok(DslCondition::VariableStringOp(VariableStringOperationCondition {
                variable,
                op: StringOp::EndsWith,
                value,
            }))
        }

        MethodName::Intersection => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".intersection() requires 1 argument, got {}", args.len()),
                });
            }
            let values = extract_string_array(&args)?;
            Ok(DslCondition::VariableMethodWithLiteralArray {
                variable,
                method: VariableCollectionMethod::Intersection,
                values,
            })
        }

        MethodName::Union => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".union() requires 1 argument, got {}", args.len()),
                });
            }
            let values = extract_string_array(&args)?;
            Ok(DslCondition::VariableMethodWithLiteralArray {
                variable,
                method: VariableCollectionMethod::Union,
                values,
            })
        }

        MethodName::Difference => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".difference() requires 1 argument, got {}", args.len()),
                });
            }
            let values = extract_string_array(&args)?;
            Ok(DslCondition::VariableMethodWithLiteralArray {
                variable,
                method: VariableCollectionMethod::Difference,
                values,
            })
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported on variables. \
                Supported: .contains(), .startswith(), .endswith(), .intersection(), .union(), .difference()",
                method.as_str()
            ),
        }),
    }
}

/// Compile a method call on a variable's attribute (e.g., d.permissions.contains("execute"))
/// Used for comprehension variables like d.permissions where d is not an entity
pub fn compile_variable_attr_method_call(
    variable: String,
    attribute: String,
    method: MethodName,
    args: Vec<Expr>,
) -> Result<DslCondition, ReaperError> {
    match method {
        MethodName::Contains => {
            if args.len() != 1 {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(".contains() requires 1 argument, got {}", args.len()),
                });
            }
            let substring = extract_string_literal(&args[0])?;
            Ok(DslCondition::VariableAttrContains {
                variable,
                attribute,
                substring,
            })
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Method .{}() is not supported on variable attributes. \
                Supported: .contains()",
                method.as_str()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reap::ast::Value;

    #[test]
    fn test_compile_entity_method_contains() {
        let result = compile_entity_method_call(
            DslEntityType::User,
            "email".to_string(),
            MethodName::Contains,
            vec![Expr::Literal(Value::String("@".to_string()))],
        )
        .unwrap();

        // Should compile to StringOp with Contains operator
        if let DslCondition::StringOp(cond) = result {
            assert_eq!(cond.entity_type, DslEntityType::User);
            assert_eq!(cond.attribute, "email");
            assert_eq!(cond.op, StringOp::Contains);
            assert_eq!(cond.value, "@");
        } else {
            panic!("Expected StringOp, got {:?}", result);
        }
    }

    #[test]
    fn test_compile_entity_method_startswith() {
        let result = compile_entity_method_call(
            DslEntityType::Resource,
            "path".to_string(),
            MethodName::Startswith,
            vec![Expr::Literal(Value::String("/api".to_string()))],
        )
        .unwrap();

        // Should compile to StringOp with StartsWith operator
        if let DslCondition::StringOp(cond) = result {
            assert_eq!(cond.entity_type, DslEntityType::Resource);
            assert_eq!(cond.attribute, "path");
            assert_eq!(cond.op, StringOp::StartsWith);
            assert_eq!(cond.value, "/api");
        } else {
            panic!("Expected StringOp, got {:?}", result);
        }
    }

    #[test]
    fn test_compile_variable_method_contains() {
        let result = compile_variable_method_call(
            "trimmed".to_string(),
            MethodName::Contains,
            vec![Expr::Literal(Value::String("test".to_string()))],
        )
        .unwrap();

        // Should compile to VariableStringOp with Contains operator
        if let DslCondition::VariableStringOp(cond) = result {
            assert_eq!(cond.variable, "trimmed");
            assert_eq!(cond.op, StringOp::Contains);
            assert_eq!(cond.value, "test");
        } else {
            panic!("Expected VariableStringOp, got {:?}", result);
        }
    }

    #[test]
    fn test_compile_variable_attr_method_contains() {
        let result = compile_variable_attr_method_call(
            "item".to_string(),
            "permissions".to_string(),
            MethodName::Contains,
            vec![Expr::Literal(Value::String("read".to_string()))],
        )
        .unwrap();

        assert!(matches!(
            result,
            DslCondition::VariableAttrContains { variable, attribute, substring }
            if variable == "item" && attribute == "permissions" && substring == "read"
        ));
    }

    #[test]
    fn test_compile_method_wrong_args() {
        let result = compile_entity_method_call(
            DslEntityType::User,
            "name".to_string(),
            MethodName::Contains,
            vec![], // No args - should fail
        );
        assert!(result.is_err());
    }
}
