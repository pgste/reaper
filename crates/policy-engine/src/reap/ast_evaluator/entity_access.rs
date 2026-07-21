//! Entity and attribute access for AST evaluator.
//!
//! This module handles accessing entity attributes, variable attributes,
//! and converting between AttributeValue and EvalValue types.

use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::data::{AttributeValue, EntityId};
use crate::reap::ast::{Entity, EntityAttr, Index, Value, VarAttr};
use reaper_core::ReaperError;
use std::collections::HashMap;

impl ReapAstEvaluator {
    /// Get attribute value from an entity (user, resource, or context)
    pub(super) fn get_entity_attribute(
        &self,
        attr: &EntityAttr,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        // Handle context entity specially - it's not stored in DataStore
        if attr.entity == Entity::Context {
            let attr_parts: Vec<&str> = attr.attribute.split('.').collect();
            let value = context.request_context.get(attr_parts[0]);

            if attr_parts.len() == 1 {
                return Ok(value
                    .map(|s| EvalValue::String(s.clone()))
                    .unwrap_or(EvalValue::Null));
            } else {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!(
                        "Chained context attributes not yet supported: context.{}",
                        attr.attribute
                    ),
                });
            }
        }

        // `input` is the structured request document, not a DataStore entity:
        // navigate the converted JSON tree directly. Missing document or path
        // yields Null (existence checks are the policy's job, like OPA's
        // undefined — but total: comparisons against Null are just false).
        if attr.entity == Entity::Input {
            let Some(ref doc) = context.input else {
                return Ok(EvalValue::Null);
            };
            let value = navigate_eval_path(doc, &attr.attribute);
            return if let Some(index) = &attr.index {
                self.apply_index(&value, index)
            } else {
                Ok(value)
            };
        }

        // Actor (F1): no actor on the request ⇒ the whole access is Null, so
        // `actor.*` on a human (actor-less) request is simply non-matching
        // rather than an error. When present it resolves like any entity.
        let entity_id = match attr.entity {
            Entity::User => context.user_id,
            Entity::Actor => match context.actor_id {
                // An actor id that names no loaded entity also reads Null —
                // it can still satisfy rebac checks (a relation may name it),
                // but it has no attributes. Same rule as the compiled path's
                // synthesized empty actor entity.
                Some(id) if self.store.get(id).is_some() => id,
                _ => return Ok(EvalValue::Null),
            },
            Entity::Resource => context.resource_id,
            Entity::Context => unreachable!("Context entity handled above"),
            Entity::Input => unreachable!("Input entity handled above"),
        };

        // Missing entity reads Null — fail-closed non-match, never an
        // evaluation error. This is the documented contract (see
        // `evaluate_with_input`: an absent principal "simply fails to
        // match"), the same rule the actor arm above and missing ATTRIBUTES
        // below already follow, and the compiled evaluator's semantics for
        // the identical condition — pinned by the mixed-mode differential
        // (R4-01 A.2), which is what surfaced the old error path.
        let Some(entity) = self.store.get(entity_id) else {
            return Ok(EvalValue::Null);
        };

        // Handle chained attributes like "payload.valid"
        let interner = self.store.interner();
        let attr_parts: Vec<&str> = attr.attribute.split('.').collect();

        let first_attr_id = interner.intern(attr_parts[0]);
        let value = entity.get_attribute(first_attr_id);

        // Navigate through chained attributes
        if attr_parts.len() > 1 && value.is_some() {
            let mut current_value = self.attribute_value_to_eval_value(value, &None)?;

            for attr_name in &attr_parts[1..] {
                match current_value {
                    EvalValue::Object(ref map) => {
                        if let Some(nested_val) = map.get(*attr_name) {
                            current_value = nested_val.clone();
                        } else {
                            return Ok(EvalValue::Null);
                        }
                    }
                    EvalValue::Null => return Ok(EvalValue::Null),
                    _ => {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Cannot access attribute '{}' on non-object value",
                                attr_name
                            ),
                        })
                    }
                }
            }

            match &attr.index {
                Some(index) => self.apply_index(&current_value, index),
                None => Ok(current_value),
            }
        } else {
            match value {
                Some(attr_val) => self.attribute_value_to_eval_value(Some(attr_val), &attr.index),
                None => Ok(EvalValue::Null),
            }
        }
    }

    /// Actor attribute read: like [`Self::get_entity_attr_by_name`], but an
    /// actor id that names no loaded entity reads Null instead of erroring —
    /// the actor is request-supplied, so "unknown actor" must fail closed
    /// (non-matching), never fail the evaluation. Mirrors the compiled
    /// path's synthesized empty actor entity.
    pub(super) fn get_actor_attr_by_name(
        &self,
        actor_id: EntityId,
        attribute: &str,
    ) -> Result<EvalValue, ReaperError> {
        if self.store.get(actor_id).is_none() {
            return Ok(EvalValue::Null);
        }
        self.get_entity_attr_by_name(actor_id, attribute)
    }

    /// Get entity attribute by entity_id and attribute name
    pub(super) fn get_entity_attr_by_name(
        &self,
        entity_id: EntityId,
        attribute: &str,
    ) -> Result<EvalValue, ReaperError> {
        // Missing entity reads Null (fail-closed non-match) — same contract
        // as `get_entity_attribute` above.
        let Some(entity) = self.store.get(entity_id) else {
            return Ok(EvalValue::Null);
        };

        let interner = self.store.interner();
        let attr_id = interner.intern(attribute);
        let value = entity.get_attribute(attr_id);

        match value {
            Some(attr_val) => self.attribute_value_to_eval_value(Some(attr_val), &None),
            None => Ok(EvalValue::Null),
        }
    }

    /// Get attribute from a variable value
    pub(super) fn get_var_attribute(
        &self,
        var_attr: &VarAttr,
        context: &EvalContext,
    ) -> Result<EvalValue, ReaperError> {
        let var_value = context.variables.get(&var_attr.variable).ok_or_else(|| {
            ReaperError::InvalidPolicy {
                reason: format!("Undefined variable: {}", var_attr.variable),
            }
        })?;

        match var_value {
            EvalValue::Object(_) => {
                // Navigate the full dotted path (v.change.after.acl), not just
                // one level — document policies bind deeply nested objects.
                let value = navigate_eval_path(var_value, &var_attr.attribute);

                if let Some(index) = &var_attr.index {
                    self.apply_index(&value, index)
                } else {
                    Ok(value)
                }
            }
            EvalValue::Null => Ok(EvalValue::Null),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Cannot access attribute '{}' on non-object variable '{}'",
                    var_attr.attribute, var_attr.variable
                ),
            }),
        }
    }

    /// Apply an index to a value (array[n] or object["key"])
    pub(super) fn apply_index(
        &self,
        value: &EvalValue,
        index: &Index,
    ) -> Result<EvalValue, ReaperError> {
        match (value, index) {
            (_, Index::Wildcard) => Ok(value.clone()),
            (EvalValue::Array(arr), Index::Number(n)) => {
                let idx = *n as usize;
                Ok(arr.get(idx).cloned().unwrap_or(EvalValue::Null))
            }
            (EvalValue::Object(map), Index::String(key)) => {
                Ok(map.get(key).cloned().unwrap_or(EvalValue::Null))
            }
            _ => Err(ReaperError::InvalidPolicy {
                reason: "Invalid index operation".to_string(),
            }),
        }
    }

    /// Convert AttributeValue to EvalValue
    pub(super) fn attribute_value_to_eval_value(
        &self,
        value: Option<&AttributeValue>,
        index: &Option<Index>,
    ) -> Result<EvalValue, ReaperError> {
        let value = value.ok_or_else(|| ReaperError::InvalidPolicy {
            reason: "Attribute not found".to_string(),
        })?;

        let eval_value = match value {
            AttributeValue::String(id) => {
                let interner = self.store.interner();
                EvalValue::String(
                    interner
                        .resolve(*id)
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                )
            }
            AttributeValue::Int(i) => EvalValue::Integer(*i),
            AttributeValue::Float(f) => EvalValue::Float(*f),
            AttributeValue::Bool(b) => EvalValue::Boolean(*b),
            AttributeValue::List(list) => {
                let items: Vec<EvalValue> = list
                    .iter()
                    .map(|v| {
                        self.attribute_value_to_eval_value(Some(v), &None)
                            .unwrap_or(EvalValue::Null)
                    })
                    .collect();
                EvalValue::Array(items)
            }
            AttributeValue::Object(map) => {
                let interner = self.store.interner();
                let mut obj = HashMap::new();
                for (key, val) in map {
                    let key_str = interner
                        .resolve(*key)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let val_eval = self
                        .attribute_value_to_eval_value(Some(val), &None)
                        .unwrap_or(EvalValue::Null);
                    obj.insert(key_str, val_eval);
                }
                EvalValue::Object(obj)
            }
            AttributeValue::Set(set) => {
                let items: Vec<EvalValue> = set
                    .iter()
                    .map(|v| {
                        self.attribute_value_to_eval_value(Some(v), &None)
                            .unwrap_or(EvalValue::Null)
                    })
                    .collect();
                EvalValue::Set(items)
            }
            AttributeValue::Null => EvalValue::Null,
        };

        if let Some(idx) = index {
            self.apply_index(&eval_value, idx)
        } else {
            Ok(eval_value)
        }
    }

    /// Convert AST Value to EvalValue
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn value_to_eval_value(&self, value: &Value) -> EvalValue {
        match value {
            Value::String(s) => EvalValue::String(s.clone()),
            Value::Integer(i) => EvalValue::Integer(*i),
            Value::Float(f) => EvalValue::Float(*f),
            Value::Boolean(b) => EvalValue::Boolean(*b),
            Value::Null => EvalValue::Null,
            Value::Array(arr) => {
                EvalValue::Array(arr.iter().map(|v| self.value_to_eval_value(v)).collect())
            }
            Value::Object(obj) => EvalValue::Object(
                obj.iter()
                    .map(|(k, v)| (k.clone(), self.value_to_eval_value(v)))
                    .collect(),
            ),
            Value::Set(set) => {
                EvalValue::Set(set.iter().map(|v| self.value_to_eval_value(v)).collect())
            }
        }
    }
}

/// Walk a dotted attribute path ("change.after.acl") through nested
/// `EvalValue` objects. A missing key or non-object mid-path yields Null —
/// document policies existence-check rather than error, so a malformed or
/// partial document can never crash evaluation (it just fails the rule).
pub(super) fn navigate_eval_path(value: &EvalValue, dotted_path: &str) -> EvalValue {
    let mut current = value.clone();
    for part in dotted_path.split('.') {
        current = match current {
            EvalValue::Object(ref map) => map.get(part).cloned().unwrap_or(EvalValue::Null),
            _ => EvalValue::Null,
        };
        if matches!(current, EvalValue::Null) {
            return EvalValue::Null;
        }
    }
    current
}
