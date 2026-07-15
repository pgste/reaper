//! Compiler helper functions for extraction and conversion.
//!
//! This module contains utility functions for extracting values from AST expressions
//! and converting between types.

use crate::evaluators::reaper_dsl::EntityType;
use crate::reap::ast::{Expr, Value};
use reaper_core::ReaperError;

/// Extract entity type and attribute from an expression.
///
/// Handles both:
/// - `Expr::AttributeAccess { variable: "user", attribute: "email" }`
/// - `Expr::Variable("user.email")` format
pub fn extract_entity_attr(expr: &Expr) -> Result<(EntityType, String), ReaperError> {
    match expr {
        Expr::AttributeAccess {
            variable,
            attribute,
        } => {
            let entity_type = parse_entity_type(variable)?;
            Ok((entity_type, attribute.clone()))
        }

        // Handle Variable("user.email") format - split on dot
        Expr::Variable(var_name) => {
            if let Some((entity, attr)) = var_name.split_once('.') {
                let entity_type = parse_entity_type(entity)?;
                Ok((entity_type, attr.to_string()))
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Variable '{}' is not a valid entity.attribute format",
                        var_name
                    ),
                })
            }
        }

        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Expected entity.attribute access (e.g., user.email), got {:?}",
                expr
            ),
        }),
    }
}

/// Parse an entity type string into EntityType enum.
pub fn parse_entity_type(entity: &str) -> Result<EntityType, ReaperError> {
    match entity {
        "user" => Ok(EntityType::User),
        "resource" => Ok(EntityType::Resource),
        "context" => Ok(EntityType::Context),
        // `actor` (F1 agentic authz): the optional non-human actor. Compiled
        // since F1-s2c; an absent actor reads every attribute as missing
        // (fail-closed), mirroring the AST evaluator's Null reads.
        "actor" => Ok(EntityType::Actor),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Unknown entity type '{}'. Expected 'user', 'actor', 'resource', or 'context'",
                entity
            ),
        }),
    }
}

/// Extract a string literal from an expression.
pub fn extract_string_literal(expr: &Expr) -> Result<String, ReaperError> {
    match expr {
        Expr::Literal(Value::String(s)) => Ok(s.clone()),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Expected string literal, got {:?}", expr),
        }),
    }
}

/// Extract an integer literal from an expression.
pub fn extract_int_literal(expr: &Expr) -> Result<i64, ReaperError> {
    match expr {
        Expr::Literal(Value::Integer(i)) => Ok(*i),
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!("Expected integer literal, got {:?}", expr),
        }),
    }
}

/// Extract a string array from function arguments.
///
/// Handles both:
/// - Single array literal argument: `["a", "b", "c"]`
/// - Multiple string arguments: `"a", "b", "c"`
pub fn extract_string_array(args: &[Expr]) -> Result<Vec<String>, ReaperError> {
    if args.is_empty() {
        return Err(ReaperError::InvalidPolicy {
            reason: "Expected at least one argument".to_string(),
        });
    }

    // Check if first arg is an array literal
    if let Expr::Literal(Value::Array(arr)) = &args[0] {
        let mut result = Vec::new();
        for val in arr {
            if let Value::String(s) = val {
                result.push(s.clone());
            } else {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Expected string literal in array, got {:?}", val),
                });
            }
        }
        return Ok(result);
    }

    // Otherwise, treat each arg as a string
    let mut result = Vec::new();
    for arg in args {
        result.push(extract_string_literal(arg)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entity_type() {
        assert!(matches!(parse_entity_type("user"), Ok(EntityType::User)));
        assert!(matches!(
            parse_entity_type("resource"),
            Ok(EntityType::Resource)
        ));
        assert!(matches!(
            parse_entity_type("context"),
            Ok(EntityType::Context)
        ));
        assert!(parse_entity_type("invalid").is_err());
    }

    #[test]
    fn test_extract_string_literal() {
        let expr = Expr::Literal(Value::String("test".to_string()));
        assert_eq!(extract_string_literal(&expr).unwrap(), "test");

        let expr = Expr::Literal(Value::Integer(42));
        assert!(extract_string_literal(&expr).is_err());
    }

    #[test]
    fn test_extract_int_literal() {
        let expr = Expr::Literal(Value::Integer(42));
        assert_eq!(extract_int_literal(&expr).unwrap(), 42);

        let expr = Expr::Literal(Value::String("test".to_string()));
        assert!(extract_int_literal(&expr).is_err());
    }

    #[test]
    fn test_extract_entity_attr_attribute_access() {
        let expr = Expr::AttributeAccess {
            variable: "user".to_string(),
            attribute: "email".to_string(),
        };
        let (entity, attr) = extract_entity_attr(&expr).unwrap();
        assert!(matches!(entity, EntityType::User));
        assert_eq!(attr, "email");
    }

    #[test]
    fn test_extract_entity_attr_variable() {
        let expr = Expr::Variable("resource.owner".to_string());
        let (entity, attr) = extract_entity_attr(&expr).unwrap();
        assert!(matches!(entity, EntityType::Resource));
        assert_eq!(attr, "owner");
    }

    #[test]
    fn test_extract_string_array_from_array() {
        let args = vec![Expr::Literal(Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]))];
        let result = extract_string_array(&args).unwrap();
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn test_extract_string_array_from_args() {
        let args = vec![
            Expr::Literal(Value::String("a".to_string())),
            Expr::Literal(Value::String("b".to_string())),
        ];
        let result = extract_string_array(&args).unwrap();
        assert_eq!(result, vec!["a", "b"]);
    }
}
