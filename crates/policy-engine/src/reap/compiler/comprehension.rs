//! Comprehension compilation utilities.
//!
//! This module handles compilation of comprehension expressions (array, set, object comprehensions)
//! including output expressions and iterators.

use super::super::ast::{ComprehensionIterator, Entity, Expr, IterationSource, MethodName, Value};
use crate::evaluators::reaper_dsl::{
    EntityType as DslEntityType, LiteralValue, OutputMethod, UncompiledIterationSource,
    UncompiledOutput,
};
use reaper_core::ReaperError;

/// Compile comprehension output expression
pub fn compile_comprehension_output(output: &Expr) -> Result<UncompiledOutput, ReaperError> {
    match output {
        Expr::Variable(var) => Ok(UncompiledOutput::Variable(var.clone())),
        Expr::AttributeAccess {
            variable,
            attribute,
        } => Ok(UncompiledOutput::VarAttr {
            variable: variable.clone(),
            attribute: attribute.clone(),
        }),
        Expr::Literal(val) => {
            let literal = match val {
                Value::String(s) => LiteralValue::String(s.clone()),
                Value::Integer(i) => LiteralValue::Int(*i),
                Value::Boolean(b) => LiteralValue::Bool(*b),
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: format!(
                            "Literal value {:?} is not supported in comprehension output",
                            val
                        ),
                    })
                }
            };
            Ok(UncompiledOutput::Literal(literal))
        }
        Expr::MethodCall {
            receiver,
            method,
            args: _,
        } => {
            // Handle method calls on variables like t.trim()
            if let Expr::Variable(var) = &**receiver {
                let output_method = match method {
                    MethodName::Lower => OutputMethod::Lower,
                    MethodName::Upper => OutputMethod::Upper,
                    MethodName::Trim => OutputMethod::Trim,
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Method .{}() is not supported in comprehension output",
                                method.as_str()
                            ),
                        })
                    }
                };
                Ok(UncompiledOutput::VarMethodCall {
                    variable: var.clone(),
                    method: output_method,
                })
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Only method calls on variables are supported in comprehension output, got: {:?}",
                        receiver
                    ),
                })
            }
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: format!(
                "Comprehension output expression {:?} is not supported",
                output
            ),
        }),
    }
}

/// Compile comprehension iterator
pub fn compile_iterator(
    iterator: ComprehensionIterator,
) -> Result<(String, UncompiledIterationSource), ReaperError> {
    let var = iterator.variable;
    let source = match iterator.collection {
        IterationSource::EntityAttr(attr) => {
            let entity_type = match attr.entity {
                Entity::User => DslEntityType::User,
                Entity::Resource => DslEntityType::Resource,
                Entity::Context => DslEntityType::Context,
                Entity::Actor => DslEntityType::Actor,
                Entity::Input => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "`input` document access is not compiled yet; policy runs on the AST evaluator".to_string(),
                    })
                },
            };
            UncompiledIterationSource::EntityAttr {
                entity_type,
                attribute: attr.attribute,
            }
        }
        IterationSource::VarAttr(var_attr) => UncompiledIterationSource::Variable {
            variable: format!("{}.{}", var_attr.variable, var_attr.attribute),
        },
        IterationSource::IndexedVariable { variable, .. } => {
            UncompiledIterationSource::Variable { variable }
        }
    };
    Ok((var, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reap::ast::{EntityAttr, VarAttr};

    #[test]
    fn test_compile_output_variable() {
        let expr = Expr::Variable("x".to_string());
        let result = compile_comprehension_output(&expr).unwrap();
        assert!(matches!(result, UncompiledOutput::Variable(v) if v == "x"));
    }

    #[test]
    fn test_compile_output_var_attr() {
        let expr = Expr::AttributeAccess {
            variable: "item".to_string(),
            attribute: "name".to_string(),
        };
        let result = compile_comprehension_output(&expr).unwrap();
        assert!(matches!(
            result,
            UncompiledOutput::VarAttr { variable, attribute }
            if variable == "item" && attribute == "name"
        ));
    }

    #[test]
    fn test_compile_output_literal_string() {
        let expr = Expr::Literal(Value::String("test".to_string()));
        let result = compile_comprehension_output(&expr).unwrap();
        assert!(matches!(
            result,
            UncompiledOutput::Literal(LiteralValue::String(s)) if s == "test"
        ));
    }

    #[test]
    fn test_compile_output_method_trim() {
        let expr = Expr::MethodCall {
            receiver: Box::new(Expr::Variable("t".to_string())),
            method: MethodName::Trim,
            args: vec![],
        };
        let result = compile_comprehension_output(&expr).unwrap();
        assert!(matches!(
            result,
            UncompiledOutput::VarMethodCall { variable, method }
            if variable == "t" && matches!(method, OutputMethod::Trim)
        ));
    }

    #[test]
    fn test_compile_iterator_entity_attr() {
        let iterator = ComprehensionIterator {
            variable: "item".to_string(),
            collection: IterationSource::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "roles".to_string(),
                index: None,
            }),
        };
        let (var, source) = compile_iterator(iterator).unwrap();
        assert_eq!(var, "item");
        assert!(matches!(
            source,
            UncompiledIterationSource::EntityAttr { entity_type, attribute }
            if matches!(entity_type, DslEntityType::User) && attribute == "roles"
        ));
    }

    #[test]
    fn test_compile_iterator_var_attr() {
        let iterator = ComprehensionIterator {
            variable: "x".to_string(),
            collection: IterationSource::VarAttr(VarAttr {
                variable: "data".to_string(),
                attribute: "items".to_string(),
                index: None,
            }),
        };
        let (var, source) = compile_iterator(iterator).unwrap();
        assert_eq!(var, "x");
        assert!(matches!(
            source,
            UncompiledIterationSource::Variable { variable }
            if variable == "data.items"
        ));
    }
}
