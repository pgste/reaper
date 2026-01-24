//! Integration between DataStore and Cedar Policy Evaluator
//!
//! This module bridges Reaper's DataStore with Cedar's entity model,
//! enabling Cedar policies to query entity attributes from the DataStore.

#[cfg(test)]
use crate::data::{AttributeValue, DataStore, Entity};
#[cfg(test)]
use cedar_policy::{
    Entities, Entity as CedarEntity, EntityId as CedarEntityId, EntityTypeName, EntityUid,
    RestrictedExpression,
};
#[cfg(test)]
use reaper_core::ReaperError;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::str::FromStr;

/// Convert Reaper DataStore to Cedar Entities
///
/// This enables Cedar policies to query entity attributes loaded in the DataStore.
///
/// # Example
/// ```ignore
/// let store = DataStore::new();
/// // ... load entities into store ...
///
/// let cedar_entities = datastore_to_cedar_entities(&store)?;
/// // Use cedar_entities in Cedar policy evaluation
/// ```
#[cfg(test)]
pub fn datastore_to_cedar_entities(store: &DataStore) -> Result<Entities, ReaperError> {
    let interner = store.interner();
    let all_entities = store.all();

    let mut cedar_entities_vec = Vec::new();

    for entity in all_entities {
        let cedar_entity = convert_entity(&entity, interner)?;
        cedar_entities_vec.push(cedar_entity);
    }

    Entities::from_entities(
        cedar_entities_vec,
        None, // No schema validation for now
    )
    .map_err(|e| ReaperError::EvaluationError {
        reason: format!("Failed to create Cedar entities: {}", e),
    })
}

/// Create a Cedar EntityUid from a Reaper Entity
#[cfg(test)]
fn create_entity_uid(
    entity: &Entity,
    interner: &crate::data::StringInterner,
) -> Result<EntityUid, ReaperError> {
    let entity_type =
        interner
            .resolve_str(entity.entity_type)
            .ok_or_else(|| ReaperError::EvaluationError {
                reason: "Failed to resolve entity type".to_string(),
            })?;

    let entity_id =
        interner
            .resolve_str(entity.id)
            .ok_or_else(|| ReaperError::EvaluationError {
                reason: "Failed to resolve entity id".to_string(),
            })?;

    let type_name =
        EntityTypeName::from_str(&entity_type).map_err(|e| ReaperError::EvaluationError {
            reason: format!("Invalid entity type name '{}': {}", entity_type, e),
        })?;

    let eid = CedarEntityId::from_str(&entity_id).map_err(|e| ReaperError::EvaluationError {
        reason: format!("Invalid entity ID '{}': {}", entity_id, e),
    })?;

    Ok(EntityUid::from_type_name_and_id(type_name, eid))
}

/// Convert a Reaper Entity to a Cedar Entity
#[cfg(test)]
fn convert_entity(
    entity: &Entity,
    interner: &crate::data::StringInterner,
) -> Result<CedarEntity, ReaperError> {
    let uid = create_entity_uid(entity, interner)?;

    // Convert attributes to Cedar format
    let mut cedar_attrs = HashMap::new();
    for (key_id, value) in &entity.attributes {
        let key_str =
            interner
                .resolve_str(*key_id)
                .ok_or_else(|| ReaperError::EvaluationError {
                    reason: "Failed to resolve attribute key".to_string(),
                })?;

        let cedar_value = convert_attribute_value(value, interner)?;
        cedar_attrs.insert(key_str, cedar_value);
    }

    // Handle parent (if exists)
    let parents = if let Some(parent_id) = entity.parent {
        let parent_entity = Entity {
            id: parent_id,
            entity_type: entity.entity_type, // Simplified - in production, look up actual parent
            attributes: HashMap::new(),
            parent: None,
        };
        let parent_uid = create_entity_uid(&parent_entity, interner)?;
        vec![parent_uid]
    } else {
        vec![]
    };

    CedarEntity::new(uid, cedar_attrs, parents.into_iter().collect()).map_err(|e| {
        ReaperError::EvaluationError {
            reason: format!("Failed to create Cedar entity: {}", e),
        }
    })
}

/// Convert a Reaper AttributeValue to Cedar RestrictedExpression
#[cfg(test)]
fn convert_attribute_value(
    value: &AttributeValue,
    interner: &crate::data::StringInterner,
) -> Result<RestrictedExpression, ReaperError> {
    match value {
        AttributeValue::String(id) => {
            let s = interner
                .resolve_str(*id)
                .ok_or_else(|| ReaperError::EvaluationError {
                    reason: "Failed to resolve string value".to_string(),
                })?;

            RestrictedExpression::from_str(&format!("\"{}\"", s)).map_err(|e| {
                ReaperError::EvaluationError {
                    reason: format!("Failed to create Cedar string: {}", e),
                }
            })
        }
        AttributeValue::Int(i) => RestrictedExpression::from_str(&i.to_string()).map_err(|e| {
            ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar int: {}", e),
            }
        }),
        AttributeValue::Float(f) => {
            // Cedar doesn't support floats directly, convert to string or int
            // For now, we'll convert to int (truncate)
            let i = *f as i64;
            RestrictedExpression::from_str(&i.to_string()).map_err(|e| {
                ReaperError::EvaluationError {
                    reason: format!("Failed to create Cedar value from float: {}", e),
                }
            })
        }
        AttributeValue::Bool(b) => RestrictedExpression::from_str(&b.to_string()).map_err(|e| {
            ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar bool: {}", e),
            }
        }),
        AttributeValue::List(items) => {
            // Convert list to Cedar set literal
            let converted: Result<Vec<_>, _> = items
                .iter()
                .map(|v| convert_attribute_value(v, interner))
                .collect();

            let values = converted?;

            // Create a Cedar set expression
            let set_str = format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| format!("{:?}", v))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            RestrictedExpression::from_str(&set_str).map_err(|e| ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar list: {}", e),
            })
        }
        AttributeValue::Null => {
            // Cedar doesn't have null - use empty string or skip
            RestrictedExpression::from_str("\"\"").map_err(|e| ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar null representation: {}", e),
            })
        }
        AttributeValue::Object(map) => {
            // Convert object to Cedar record literal
            // For now, simplified conversion - Cedar records are more structured
            let pairs: Result<Vec<String>, ReaperError> =
                map.iter()
                    .map(|(k, v)| -> Result<String, ReaperError> {
                        let key_str = interner.resolve_str(*k).ok_or_else(|| {
                            ReaperError::EvaluationError {
                                reason: "Failed to resolve object key".to_string(),
                            }
                        })?;
                        let value_expr = convert_attribute_value(v, interner)?;
                        Ok(format!("{}: {:?}", key_str, value_expr))
                    })
                    .collect();

            let pairs_str = pairs?.join(", ");
            let record_str = format!("{{{}}}", pairs_str);

            RestrictedExpression::from_str(&record_str).map_err(|e| ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar record: {}", e),
            })
        }
        AttributeValue::Set(items) => {
            // Convert set to Cedar set literal
            let converted: Result<Vec<_>, _> = items
                .iter()
                .map(|v| convert_attribute_value(v, interner))
                .collect();

            let values = converted?;

            // Create a Cedar set expression
            let set_str = format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| format!("{:?}", v))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            RestrictedExpression::from_str(&set_str).map_err(|e| ReaperError::EvaluationError {
                reason: format!("Failed to create Cedar set: {}", e),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data::DataStore, EntityBuilder};

    #[test]
    fn test_convert_simple_entity() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");

        let entity = EntityBuilder::new(user_id, user_type)
            .with_string(role_key, admin_value)
            .build();

        store.insert(entity.clone());

        // Convert to Cedar
        let cedar_entities = datastore_to_cedar_entities(&store).unwrap();

        // Verify we can create it without errors - cedar_entities was created successfully
        let _ = cedar_entities;
    }

    #[test]
    fn test_convert_entity_with_multiple_attributes() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_id = interner.intern("bob");
        let user_type = interner.intern("User");

        let entity = EntityBuilder::new(user_id, user_type)
            .with_string(interner.intern("role"), interner.intern("manager"))
            .with_int(interner.intern("age"), 35)
            .with_bool(interner.intern("active"), true)
            .build();

        store.insert(entity);

        let cedar_entities = datastore_to_cedar_entities(&store).unwrap();
        // Verify we can create it without errors - cedar_entities was created successfully
        let _ = cedar_entities;
    }
}
