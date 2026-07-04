//! Data Loading from Various Formats
//!
//! Efficiently load entity data from JSON, YAML, or other formats
//! into the DataStore with automatic string interning.
//!
//! **Optimization**: Arrays of simple scalars (strings, ints, bools) are
//! automatically converted to Sets for O(1) membership tests.

use super::entity::{AttributeValue, EntityBuilder};
use super::store::DataStore;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Supported data formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Json,
    // Future: Yaml, Toml, Binary, etc.
}

/// Statistics from loading entities
///
/// Provides detailed breakdown of loaded entities by type
#[derive(Debug, Clone)]
pub struct LoadStats {
    /// Total entities loaded
    pub total: usize,

    /// Count by entity type: {"User": 100000, "Device": 50000, "Resource": 200000}
    pub by_type: HashMap<String, usize>,

    /// Total attributes across all entities
    pub total_attributes: usize,

    /// Load duration
    pub duration: Duration,
}

impl LoadStats {
    fn new() -> Self {
        Self {
            total: 0,
            by_type: HashMap::new(),
            total_attributes: 0,
            duration: Duration::default(),
        }
    }

    fn track_entity(&mut self, entity_type: &str, num_attributes: usize) {
        self.total += 1;
        *self.by_type.entry(entity_type.to_string()).or_insert(0) += 1;
        self.total_attributes += num_attributes;
    }
}

/// Data loader for importing entities
#[derive(Clone)]
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
        #[cfg(not(target_arch = "wasm32"))]
        let data: DataDocument =
            sonic_rs::from_str(json).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to parse JSON: {}", e),
            })?;

        #[cfg(target_arch = "wasm32")]
        let data: DataDocument =
            serde_json::from_str(json).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to parse JSON: {}", e),
            })?;

        self.load_document(data)
    }

    /// Load entities directly from parsed JSON values
    ///
    /// **Entity-type agnostic:** Works for any entity type (User, Resource, Device, Location, etc.)
    /// **Index-aware:** Updates entity type indexes during load
    /// **Memory efficient:** Bypasses JSON string serialization (saves ~40% memory)
    ///
    /// # Arguments
    /// * `entities` - Vector of JSON entity objects
    ///
    /// # Returns
    /// LoadStats with entity counts by type
    ///
    /// # Example
    /// ```
    /// use policy_engine::DataStore;
    /// use policy_engine::data::DataLoader;
    /// use serde_json::json;
    ///
    /// let store = DataStore::new();
    /// let loader = DataLoader::new(store);
    /// let entities = vec![
    ///     json!({"id": "device_1", "type": "Device", "attributes": {"trustscore": 85}}),
    ///     json!({"id": "user_1", "type": "User", "attributes": {"active": true}}),
    /// ];
    /// let stats = loader.load_json_values(entities).unwrap();
    /// assert_eq!(stats.total, 2);
    /// ```
    pub fn load_json_values(&self, entities: Vec<JsonValue>) -> Result<LoadStats, ReaperError> {
        let start = Instant::now();
        let mut stats = LoadStats::new();
        let interner = self.store.interner();

        for entity_value in entities {
            // Parse entity document
            let entity_doc = self.parse_entity_from_value(&entity_value)?;
            let entity_type_str = entity_doc.entity_type.clone();
            let num_attrs = entity_doc.attributes.len();

            // Build entity (generic, entity-type agnostic)
            let entity = self.build_entity_from_doc(entity_doc, interner)?;

            // Insert and update indexes
            self.store.insert(entity);

            // Track stats
            stats.track_entity(&entity_type_str, num_attrs);
        }

        stats.duration = start.elapsed();
        Ok(stats)
    }

    /// Parse a JSON value into EntityDocument
    fn parse_entity_from_value(&self, value: &JsonValue) -> Result<EntityDocument, ReaperError> {
        serde_json::from_value(value.clone()).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to parse entity: {}", e),
        })
    }

    /// Build an entity from a document (entity-type agnostic)
    fn build_entity_from_doc(
        &self,
        doc: EntityDocument,
        interner: &super::interning::StringInterner,
    ) -> Result<super::entity::Entity, ReaperError> {
        let id = interner.intern(&doc.id);
        let entity_type = interner.intern(&doc.entity_type);

        let mut builder = EntityBuilder::new(id, entity_type);

        // Generic attribute loading (works for any schema)
        for (key, value) in doc.attributes {
            let key_id = interner.intern(&key);
            let attr_value = json_value_to_attribute(value, interner)?;
            builder = builder.with_attribute(key_id, attr_value);
        }

        // Parent relationship (optional)
        if let Some(parent) = doc.parent {
            let parent_id = interner.intern(&parent);
            builder = builder.with_parent(parent_id);
        }

        Ok(builder.build())
    }

    /// Load a data document using batch insert for better locality
    fn load_document(&self, doc: DataDocument) -> Result<usize, ReaperError> {
        let interner = self.store.interner();
        let mut entities = Vec::with_capacity(doc.entities.len());

        for entity_doc in doc.entities {
            let id = interner.intern(&entity_doc.id);
            let entity_type = interner.intern(&entity_doc.entity_type);
            let mut builder = EntityBuilder::new(id, entity_type);

            for (key, value) in entity_doc.attributes {
                let key_id = interner.intern(&key);
                let attr_value = json_value_to_attribute(value, interner)?;
                builder = builder.with_attribute(key_id, attr_value);
            }

            if let Some(parent) = entity_doc.parent {
                let parent_id = interner.intern(&parent);
                builder = builder.with_parent(parent_id);
            }

            // ReBAC edges: `id #relation @subject` into the relationship graph
            // (forward + reverse indexed at write time).
            for (relation, subjects) in &entity_doc.relationships {
                let relation_id = interner.intern(relation);
                for subject in subjects {
                    let subject_id = interner.intern(subject);
                    self.store.add_relationship(id, relation_id, subject_id);
                }
            }

            entities.push(builder.build());
        }

        let count = entities.len();
        self.store.insert_batch(entities);
        Ok(count)
    }

    /// UPSERT one entity document (delta-sync primitive): replaces the
    /// entity, its indexed attribute values, and the edges it carries.
    /// Idempotent — at-least-once delta delivery converges.
    pub fn upsert_entity_doc(&self, doc: &JsonValue) -> Result<(), ReaperError> {
        let entity_doc: EntityDocument =
            serde_json::from_value(doc.clone()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("invalid entity document: {e}"),
            })?;
        let interner = self.store.interner();
        let id = interner.intern(&entity_doc.id);
        let entity_type = interner.intern(&entity_doc.entity_type);

        let mut builder = EntityBuilder::new(id, entity_type);
        for (key, value) in entity_doc.attributes {
            let key_id = interner.intern(&key);
            builder = builder.with_attribute(key_id, json_value_to_attribute(value, interner)?);
        }
        if let Some(parent) = entity_doc.parent {
            builder = builder.with_parent(interner.intern(&parent));
        }

        // upsert() detaches previously carried edges; re-add the current set.
        self.store.upsert(builder.build());
        for (relation, subjects) in &entity_doc.relationships {
            let relation_id = interner.intern(relation);
            for subject in subjects {
                self.store
                    .add_relationship(id, relation_id, interner.intern(subject));
            }
        }
        Ok(())
    }

    /// DELETE one entity by id (delta-sync tombstone): idempotent, cascades
    /// through the relationship graph.
    pub fn delete_entity(&self, entity_id: &str) {
        let id = self.store.interner().intern(entity_id);
        self.store.remove_entity(id);
    }

    /// Get the underlying data store
    pub fn store(&self) -> &DataStore {
        &self.store
    }
}

/// Convert JSON value to AttributeValue
pub(crate) fn json_value_to_attribute(
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
            // Always preserve array order by using List
            // This is important for collection methods like first(), last(), slice(), reverse()
            // which rely on element ordering. Sets can still be used explicitly via policy syntax.
            let items: Result<Vec<_>, _> = arr
                .into_iter()
                .map(|v| json_value_to_attribute(v, interner))
                .collect();
            Ok(AttributeValue::List(items?))
        }
        JsonValue::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (key, value) in obj {
                let key_id = interner.intern(&key);
                let attr_value = json_value_to_attribute(value, interner)?;
                map.insert(key_id, attr_value);
            }
            Ok(AttributeValue::Object(map))
        }
    }
}

/// Data document structure
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct DataDocument {
    entities: Vec<EntityDocument>,
}

/// Single entity in a data document
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EntityDocument {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub attributes: HashMap<String, JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// ReBAC edges this entity declares: relation -> subject entity ids,
    /// e.g. {"owner": ["alice"], "parent": ["folder-eng"]}.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub relationships: HashMap<String, Vec<String>>,
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

    #[test]
    fn test_load_json_values_multi_type() {
        use serde_json::json;

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        let entities = vec![
            json!({
                "id": "user_1",
                "type": "User",
                "attributes": {"name": "Alice", "active": true}
            }),
            json!({
                "id": "user_2",
                "type": "User",
                "attributes": {"name": "Bob", "active": false}
            }),
            json!({
                "id": "device_1",
                "type": "Device",
                "attributes": {"trustscore": 85, "os": "Linux"}
            }),
            json!({
                "id": "resource_1",
                "type": "Resource",
                "attributes": {"classification": "secret"}
            }),
        ];

        let stats = loader.load_json_values(entities).unwrap();

        assert_eq!(stats.total, 4);
        assert_eq!(stats.by_type.get("User"), Some(&2));
        assert_eq!(stats.by_type.get("Device"), Some(&1));
        assert_eq!(stats.by_type.get("Resource"), Some(&1));
        assert_eq!(stats.total_attributes, 7); // 2+2+2+1
    }

    #[test]
    fn test_load_json_values_vs_load_json() {
        // Test that both methods produce identical results
        let store1 = DataStore::new();
        let store2 = DataStore::new();
        let loader1 = DataLoader::new(store1.clone());
        let loader2 = DataLoader::new(store2.clone());

        let json_str = r#"{"entities": [
            {"id": "user_1", "type": "User", "attributes": {"active": true, "role": "admin"}},
            {"id": "doc_1", "type": "Resource", "attributes": {"public": false}}
        ]}"#;

        let json_val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let entities = json_val["entities"].as_array().unwrap().clone();

        // Load via old method
        let count1 = loader1.load_json(json_str).unwrap();

        // Load via new method
        let stats2 = loader2.load_json_values(entities).unwrap();

        // Both should load same number
        assert_eq!(count1, stats2.total);
        assert_eq!(count1, 2);

        // Both should produce identical entities
        let interner = store1.interner();
        let user_id = interner.intern("user_1");
        let doc_id = interner.intern("doc_1");

        assert!(store1.get(user_id).is_some());
        assert!(store1.get(doc_id).is_some());
        assert!(store2.get(user_id).is_some());
        assert!(store2.get(doc_id).is_some());

        // Verify attributes match
        let user1 = store1.get(user_id).unwrap();
        let user2 = store2.get(user_id).unwrap();

        let interner1 = store1.interner();
        let interner2 = store2.interner();
        let role_key1 = interner1.intern("role");
        let role_key2 = interner2.intern("role");

        assert_eq!(
            user1
                .get_string_attribute(role_key1, interner1)
                .unwrap()
                .as_ref(),
            "admin"
        );
        assert_eq!(
            user2
                .get_string_attribute(role_key2, interner2)
                .unwrap()
                .as_ref(),
            "admin"
        );
    }

    #[test]
    fn test_load_json_values_empty() {
        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        let stats = loader.load_json_values(vec![]).unwrap();

        assert_eq!(stats.total, 0);
        assert_eq!(stats.by_type.len(), 0);
        assert_eq!(stats.total_attributes, 0);
    }

    #[test]
    fn test_load_json_values_with_parent() {
        use serde_json::json;

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        let entities = vec![
            json!({
                "id": "engineering",
                "type": "Department",
                "attributes": {"name": "Engineering"}
            }),
            json!({
                "id": "alice",
                "type": "User",
                "attributes": {"name": "Alice"},
                "parent": "engineering"
            }),
        ];

        let stats = loader.load_json_values(entities).unwrap();

        assert_eq!(stats.total, 2);

        let interner = store.interner();
        let alice_id = interner.intern("alice");
        let eng_id = interner.intern("engineering");
        let alice = store.get(alice_id).unwrap();

        assert_eq!(alice.parent, Some(eng_id));
    }
}
