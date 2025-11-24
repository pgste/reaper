//! Data Loading from Various Formats
//!
//! Efficiently load entity data from JSON, YAML, or other formats
//! into the DataStore with automatic string interning.

use super::entity::{AttributeValue, EntityBuilder};
use super::store::DataStore;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Supported data formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Json,
    // Future: Yaml, Toml, Binary, etc.
}

/// Data loader for importing entities
pub struct DataLoader {
    store: DataStore,
}

impl DataLoader {
    /// Create a new data loader
    pub fn new(store: DataStore) -> Self {
        Self { store }
    }

    /// Load data from a JSON string
    ///
    /// Expected format:
    /// ```json
    /// {
    ///   "entities": [
    ///     {
    ///       "id": "alice",
    ///       "type": "User",
    ///       "attributes": {
    ///         "role": "admin",
    ///         "department": "engineering",
    ///         "age": 30,
    ///         "active": true
    ///       }
    ///     }
    ///   ]
    /// }
    /// ```
    pub fn load_json(&self, json: &str) -> Result<usize, ReaperError> {
        let data: DataDocument =
            serde_json::from_str(json).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to parse JSON: {}", e),
            })?;

        self.load_document(data)
    }

    /// Load a data document
    fn load_document(&self, doc: DataDocument) -> Result<usize, ReaperError> {
        let interner = self.store.interner();
        let mut count = 0;

        for entity_doc in doc.entities {
            // Intern strings
            let id = interner.intern(&entity_doc.id);
            let entity_type = interner.intern(&entity_doc.entity_type);

            // Build entity with attributes
            let mut builder = EntityBuilder::new(id, entity_type);

            for (key, value) in entity_doc.attributes {
                let key_id = interner.intern(&key);
                let attr_value = json_value_to_attribute(value, interner)?;
                builder = builder.with_attribute(key_id, attr_value);
            }

            // Set parent if specified
            if let Some(parent) = entity_doc.parent {
                let parent_id = interner.intern(&parent);
                builder = builder.with_parent(parent_id);
            }

            self.store.insert(builder.build());
            count += 1;
        }

        Ok(count)
    }

    /// Get the underlying data store
    pub fn store(&self) -> &DataStore {
        &self.store
    }
}

/// Convert JSON value to AttributeValue
fn json_value_to_attribute(
    value: JsonValue,
    interner: &super::interning::StringInterner,
) -> Result<AttributeValue, ReaperError> {
    match value {
        JsonValue::String(s) => {
            let id = interner.intern(&s);
            Ok(AttributeValue::String(id))
        }
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(AttributeValue::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(AttributeValue::Float(f))
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: "Invalid number format".to_string(),
                })
            }
        }
        JsonValue::Bool(b) => Ok(AttributeValue::Bool(b)),
        JsonValue::Null => Ok(AttributeValue::Null),
        JsonValue::Array(arr) => {
            let items: Result<Vec<_>, _> = arr
                .into_iter()
                .map(|v| json_value_to_attribute(v, interner))
                .collect();
            Ok(AttributeValue::List(items?))
        }
        JsonValue::Object(_) => Err(ReaperError::InvalidPolicy {
            reason: "Nested objects not supported as attribute values".to_string(),
        }),
    }
}

/// Data document structure
#[derive(Debug, Deserialize, Serialize)]
struct DataDocument {
    entities: Vec<EntityDocument>,
}

/// Single entity in a data document
#[derive(Debug, Deserialize, Serialize)]
struct EntityDocument {
    id: String,
    #[serde(rename = "type")]
    entity_type: String,
    attributes: HashMap<String, JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
}

/// Convenience function to create a DataStore from JSON
pub fn from_json(json: &str) -> Result<DataStore, ReaperError> {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(json)?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_json_simple() {
        let json = r#"
        {
            "entities": [
                {
                    "id": "alice",
                    "type": "User",
                    "attributes": {
                        "role": "admin",
                        "department": "engineering"
                    }
                }
            ]
        }
        "#;

        let store = from_json(json).unwrap();
        let interner = store.interner();

        let alice_id = interner.intern("alice");
        let entity = store.get(alice_id).unwrap();

        assert_eq!(entity.id, alice_id);

        let role_key = interner.intern("role");
        let role_value = entity.get_string_attribute(role_key, interner).unwrap();
        assert_eq!(role_value.as_ref(), "admin");
    }

    #[test]
    fn test_load_json_multiple_types() {
        let json = r#"
        {
            "entities": [
                {
                    "id": "alice",
                    "type": "User",
                    "attributes": {
                        "role": "admin"
                    }
                },
                {
                    "id": "doc1",
                    "type": "Document",
                    "attributes": {
                        "owner": "alice"
                    }
                }
            ]
        }
        "#;

        let store = from_json(json).unwrap();
        let stats = store.stats();

        assert_eq!(stats.total_entities, 2);
        assert_eq!(stats.unique_types, 2);
    }

    #[test]
    fn test_load_json_with_numbers() {
        let json = r#"
        {
            "entities": [
                {
                    "id": "alice",
                    "type": "User",
                    "attributes": {
                        "age": 30,
                        "score": 95.5,
                        "active": true
                    }
                }
            ]
        }
        "#;

        let store = from_json(json).unwrap();
        let interner = store.interner();

        let alice_id = interner.intern("alice");
        let entity = store.get(alice_id).unwrap();

        let age_key = interner.intern("age");
        assert_eq!(entity.get_int_attribute(age_key), Some(30));

        let active_key = interner.intern("active");
        assert_eq!(entity.get_bool_attribute(active_key), Some(true));
    }

    #[test]
    fn test_load_json_with_hierarchy() {
        let json = r#"
        {
            "entities": [
                {
                    "id": "engineering",
                    "type": "Department",
                    "attributes": {
                        "name": "Engineering"
                    }
                },
                {
                    "id": "alice",
                    "type": "User",
                    "attributes": {
                        "name": "Alice"
                    },
                    "parent": "engineering"
                }
            ]
        }
        "#;

        let store = from_json(json).unwrap();
        let interner = store.interner();

        let alice_id = interner.intern("alice");
        let eng_id = interner.intern("engineering");
        let alice = store.get(alice_id).unwrap();

        assert_eq!(alice.parent, Some(eng_id));
    }

    #[test]
    fn test_memory_efficiency_with_duplicates() {
        let json = r#"
        {
            "entities": [
                {"id": "user1", "type": "User", "attributes": {"role": "admin"}},
                {"id": "user2", "type": "User", "attributes": {"role": "admin"}},
                {"id": "user3", "type": "User", "attributes": {"role": "admin"}},
                {"id": "user4", "type": "User", "attributes": {"role": "admin"}},
                {"id": "user5", "type": "User", "attributes": {"role": "admin"}}
            ]
        }
        "#;

        let store = from_json(json).unwrap();
        let stats = store.stats();

        // Should have 5 entities
        assert_eq!(stats.total_entities, 5);

        // But only a few unique strings: "User", "role", "admin", "user1", "user2", etc.
        // Much less than if we stored "User" and "admin" 5 times each
        println!("Unique strings: {}", stats.interner_stats.unique_strings);
        println!("Estimated memory: {} bytes", stats.estimated_memory_bytes);
    }
}
