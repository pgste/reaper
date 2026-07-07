//! Expression evaluation for AST evaluator.
//!
//! This module handles evaluation of expressions:
//! - Literals
//! - Variable references
//! - Attribute access
//! - Indexed access
//! - Method calls
//! - Function calls

use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::reap::ast::{Entity, EntityAttr, Expr};
use reaper_core::ReaperError;

impl ReapAstEvaluator {
    /// Evaluate an expression
    pub(super) fn evaluate_expr(
        &self,
        expr: &Expr,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        match expr {
            Expr::Literal(val) => Ok(self.value_to_eval_value(val)),

            Expr::Variable(var_name) => {
                // Check if this is a pseudo-entity reference like "user.name" from entity method calls
                if var_name.starts_with("user.")
                    || var_name.starts_with("resource.")
                    || var_name.starts_with("context.")
                    || var_name.starts_with("input.")
                {
                    // Parse the entity and attribute
                    let parts: Vec<&str> = var_name.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        let entity = Entity::from(parts[0]);
                        let attribute = parts[1].to_string();
                        let entity_attr = EntityAttr {
                            entity,
                            attribute,
                            index: None,
                        };
                        return self.get_entity_attribute(&entity_attr, context);
                    }
                }

                // Regular variable lookup
                // First check variables, then fall back to request_context
                if let Some(val) = context.variables.get(var_name) {
                    Ok(val.clone())
                } else if let Some(val) = context.request_context.get(var_name) {
                    Ok(EvalValue::String(val.clone()))
                } else {
                    Err(ReaperError::InvalidPolicy {
                        reason: format!("Undefined variable: {}", var_name),
                    })
                }
            }

            Expr::AttributeAccess {
                variable,
                attribute,
            } => {
                // Check if this is an entity reference (user, resource, context)
                // These are special identifiers that refer to entities, not variables
                match variable.as_str() {
                    "user" => {
                        // Get user entity attribute
                        self.get_entity_attr_by_name(context.user_id, attribute)
                    }
                    "resource" => {
                        // Get resource entity attribute
                        self.get_entity_attr_by_name(context.resource_id, attribute)
                    }
                    "context" => {
                        // Request context values are flat strings.
                        Ok(context
                            .request_context
                            .get(attribute)
                            .map(|s| EvalValue::String(s.clone()))
                            .unwrap_or(EvalValue::Null))
                    }
                    // The structured request document: navigate without
                    // cloning the whole tree (expressions like
                    // jwt::decode(input.token) hit this path).
                    "input" => Ok(context
                        .input
                        .as_ref()
                        .map(|doc| super::entity_access::navigate_eval_path(doc, attribute))
                        .unwrap_or(EvalValue::Null)),
                    _ => {
                        // Regular variable attribute access
                        let var_value = context.variables.get(variable).ok_or_else(|| {
                            ReaperError::InvalidPolicy {
                                reason: format!("Undefined variable: {}", variable),
                            }
                        })?;

                        match var_value {
                            EvalValue::Object(map) => {
                                Ok(map.get(attribute).cloned().unwrap_or(EvalValue::Null))
                            }
                            EvalValue::Null => {
                                // Null propagation: accessing attributes on null returns null
                                Ok(EvalValue::Null)
                            }
                            _ => Err(ReaperError::InvalidPolicy {
                                reason: format!(
                                    "Cannot access attribute '{}' on non-object variable '{}'",
                                    attribute, variable
                                ),
                            }),
                        }
                    }
                }
            }

            Expr::IndexedAccess {
                variable,
                attribute,
                index,
            } => {
                // Check if this is an entity reference (user, resource, context)
                match variable.as_str() {
                    "user" | "resource" => {
                        let entity_id = if variable == "user" {
                            context.user_id
                        } else {
                            context.resource_id
                        };
                        let attr_value = self.get_entity_attr_by_name(entity_id, attribute)?;
                        self.apply_index(&attr_value, index)
                    }
                    "context" => {
                        // Context entity not yet fully supported
                        Ok(EvalValue::Null)
                    }
                    _ => {
                        // Regular variable indexed access
                        let var_value = context.variables.get(variable).ok_or_else(|| {
                            ReaperError::InvalidPolicy {
                                reason: format!("Undefined variable: {}", variable),
                            }
                        })?;

                        // If attribute is empty, index the variable directly
                        if attribute.is_empty() {
                            return self.apply_index(var_value, index);
                        }

                        match var_value {
                            EvalValue::Object(map) => {
                                let attr_value =
                                    map.get(attribute).cloned().unwrap_or(EvalValue::Null);
                                self.apply_index(&attr_value, index)
                            }
                            EvalValue::Null => {
                                // Null propagation: accessing attributes on null returns null
                                Ok(EvalValue::Null)
                            }
                            _ => Err(ReaperError::InvalidPolicy {
                                reason: format!(
                                    "Cannot access attribute '{}' on non-object variable '{}'",
                                    attribute, variable
                                ),
                            }),
                        }
                    }
                }
            }

            Expr::MethodCall {
                receiver,
                method,
                args,
            } => self.evaluate_method_call(receiver, method, args, context),

            Expr::FunctionCall {
                namespace,
                function,
                args,
            } => self.evaluate_function_call(namespace.as_deref(), function, args, context),
        }
    }
}
