//! Attribute Indexing for Fast Queries
//!
//! Enables fast attribute-based queries on entity data:
//! - Equality: `user.role == "admin"`
//! - Range: `device.trustscore >= 75`
//! - Set membership: `user.department in ["engineering", "security"]`
//!
//! # Performance
//! - Index creation: O(n) where n = entities of type
//! - Query: O(m) where m = matching entities (vs O(n) full scan)
//! - Memory: O(n * unique_values) per index

use super::entity::{AttributeValue, EntityId};
use super::store::DataStore;
use dashmap::DashMap;
use reaper_core::ReaperError;
use std::collections::HashSet;
use std::sync::Arc;

/// Manages attribute-based indexes for fast queries
///
/// # Example
/// ```ignore
/// let index_manager = IndexManager::new(store);
///
/// // Create indexes
/// index_manager.create_index("Device", "trustscore")?;
/// index_manager.create_index("User", "role")?;
///
/// // Query with predicates
/// let high_trust = index_manager.query(
///     "Device",
///     "trustscore",
///     |v| matches!(v, AttributeValue::Int(score) if *score >= 75)
/// );
/// ```
pub struct IndexManager {
    store: Arc<DataStore>,
    indexes: DashMap<String, AttributeIndex>,
}

/// Single attribute index
pub struct AttributeIndex {
    /// Inverted index: attribute_value -> Set<entity_id>
    index: DashMap<AttributeValue, HashSet<EntityId>>,

    /// Entity type this index is for
    #[allow(dead_code)]
    entity_type: String,

    /// Attribute name
    #[allow(dead_code)]
    attribute_name: String,

    /// Number of entities indexed
    entity_count: usize,

    /// Number of unique values
    unique_values: usize,
}

impl IndexManager {
    /// Create a new index manager
    pub fn new(store: Arc<DataStore>) -> Self {
        Self {
            store,
            indexes: DashMap::new(),
        }
    }

    /// Create an index for fast attribute lookups
    ///
    /// # Arguments
    /// * `entity_type` - Entity type to index (e.g., "User", "Device")
    /// * `attribute` - Attribute name to index (e.g., "role", "trustscore")
    ///
    /// # Returns
    /// Statistics about the created index
    ///
    /// # Example
    /// ```ignore
    /// let stats = index_manager.create_index("Device", "trustscore")?;
    /// println!("Indexed {} entities with {} unique values",
    ///          stats.entity_count, stats.unique_values);
    /// ```
    pub fn create_index(
        &self,
        entity_type: &str,
        attribute: &str,
    ) -> Result<IndexStats, ReaperError> {
        let index_key = format!("{}.{}", entity_type, attribute);

        // Check if index already exists
        if self.indexes.contains_key(&index_key) {
            return Err(ReaperError::InvalidPolicy {
                reason: format!("Index already exists: {}", index_key),
            });
        }

        let interner = self.store.interner();
        let entity_type_id = interner.intern(entity_type);
        let attr_id = interner.intern(attribute);

        // Build inverted index
        let index_map = DashMap::new();
        let mut entity_count = 0;

        for entity_id in self.store.get_by_type(entity_type_id) {
            if let Some(attr_value) = entity_id.get_attribute(attr_id) {
                index_map
                    .entry(attr_value.clone())
                    .or_insert_with(HashSet::new)
                    .insert(entity_id.id);
                entity_count += 1;
            }
        }

        let unique_values = index_map.len();

        let attribute_index = AttributeIndex {
            index: index_map,
            entity_type: entity_type.to_string(),
            attribute_name: attribute.to_string(),
            entity_count,
            unique_values,
        };

        self.indexes.insert(index_key, attribute_index);

        Ok(IndexStats {
            entity_count,
            unique_values,
        })
    }

    /// Query indexed attribute with a predicate function
    ///
    /// # Arguments
    /// * `entity_type` - Entity type (e.g., "User")
    /// * `attribute` - Attribute name (e.g., "role")
    /// * `predicate` - Function to test attribute values
    ///
    /// # Returns
    /// Vector of entity IDs matching the predicate
    ///
    /// # Example
    /// ```ignore
    /// // Find all devices with trustscore >= 75
    /// let high_trust = index_manager.query("Device", "trustscore", |v| {
    ///     matches!(v, AttributeValue::Int(score) if *score >= 75)
    /// });
    ///
    /// // Find all admins
    /// let admins = index_manager.query("User", "role", |v| {
    ///     matches!(v, AttributeValue::String(s) if
    ///              interner.resolve(*s).unwrap().as_ref() == "admin")
    /// });
    /// ```
    pub fn query<F>(&self, entity_type: &str, attribute: &str, predicate: F) -> Vec<EntityId>
    where
        F: Fn(&AttributeValue) -> bool,
    {
        let index_key = format!("{}.{}", entity_type, attribute);

        self.indexes
            .get(&index_key)
            .map(|idx| {
                let mut result = Vec::new();
                for entry in idx.index.iter() {
                    if predicate(entry.key()) {
                        result.extend(entry.value().iter().copied());
                    }
                }
                result
            })
            .unwrap_or_default()
    }

    /// Query for exact equality match (optimized)
    ///
    /// This is faster than using query() with a predicate for equality checks.
    ///
    /// # Example
    /// ```ignore
    /// let admins = index_manager.query_equals("User", "role", &admin_value)?;
    /// ```
    pub fn query_equals(
        &self,
        entity_type: &str,
        attribute: &str,
        value: &AttributeValue,
    ) -> Vec<EntityId> {
        let index_key = format!("{}.{}", entity_type, attribute);

        self.indexes
            .get(&index_key)
            .and_then(|idx| {
                idx.index
                    .get(value)
                    .map(|ids| ids.iter().copied().collect())
            })
            .unwrap_or_default()
    }

    /// Get index statistics
    pub fn get_index_stats(&self, entity_type: &str, attribute: &str) -> Option<IndexStats> {
        let index_key = format!("{}.{}", entity_type, attribute);

        self.indexes.get(&index_key).map(|idx| IndexStats {
            entity_count: idx.entity_count,
            unique_values: idx.unique_values,
        })
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<String> {
        self.indexes
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Remove an index
    pub fn remove_index(&self, entity_type: &str, attribute: &str) -> bool {
        let index_key = format!("{}.{}", entity_type, attribute);
        self.indexes.remove(&index_key).is_some()
    }

    /// Clear all indexes
    pub fn clear(&self) {
        self.indexes.clear();
    }

    /// Get total number of indexes
    pub fn index_count(&self) -> usize {
        self.indexes.len()
    }
}

/// Statistics about an attribute index
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// Number of entities indexed
    pub entity_count: usize,

    /// Number of unique attribute values
    pub unique_values: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::entity::EntityBuilder;
    use crate::data::DataStore;

    fn setup_test_store() -> Arc<DataStore> {
        let store = DataStore::new();
        let interner = store.interner();

        // Create some test entities
        let user_type = interner.intern("User");
        let device_type = interner.intern("Device");

        let role_key = interner.intern("role");
        let dept_key = interner.intern("department");
        let trustscore_key = interner.intern("trustscore");
        let os_key = interner.intern("os");

        // Users
        for i in 0..100 {
            let user_id = interner.intern(&format!("user_{}", i));
            let role = if i < 10 {
                "admin"
            } else if i < 30 {
                "analyst"
            } else {
                "viewer"
            };
            let dept = if i % 3 == 0 {
                "engineering"
            } else if i % 3 == 1 {
                "security"
            } else {
                "hr"
            };

            let role_id = interner.intern(role);
            let dept_id = interner.intern(dept);

            store.insert(
                EntityBuilder::new(user_id, user_type)
                    .with_string(role_key, role_id)
                    .with_string(dept_key, dept_id)
                    .build(),
            );
        }

        // Devices
        for i in 0..50 {
            let device_id = interner.intern(&format!("device_{}", i));
            let trustscore = 50 + (i as i64 % 50); // 50-99
            let os = if i % 2 == 0 { "Linux" } else { "Windows" };
            let os_id = interner.intern(os);

            store.insert(
                EntityBuilder::new(device_id, device_type)
                    .with_attribute(trustscore_key, AttributeValue::Int(trustscore))
                    .with_string(os_key, os_id)
                    .build(),
            );
        }

        Arc::new(store)
    }

    #[test]
    fn test_create_index() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        let stats = index_manager.create_index("User", "role").unwrap();

        assert_eq!(stats.entity_count, 100);
        assert_eq!(stats.unique_values, 3); // admin, analyst, viewer
    }

    #[test]
    fn test_create_index_duplicate() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();

        // Try to create same index again
        let result = index_manager.create_index("User", "role");
        assert!(result.is_err());
    }

    #[test]
    fn test_query_equality() {
        let store = setup_test_store();
        let interner = store.interner();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();

        // Query for admins
        let admin_id = interner.intern("admin");
        let admins = index_manager.query(
            "User",
            "role",
            |v| matches!(v, AttributeValue::String(s) if *s == admin_id),
        );

        assert_eq!(admins.len(), 10);
    }

    #[test]
    fn test_query_equals() {
        let store = setup_test_store();
        let interner = store.interner();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();

        // Query for admins using optimized equals
        let admin_id = interner.intern("admin");
        let admins = index_manager.query_equals("User", "role", &AttributeValue::String(admin_id));

        assert_eq!(admins.len(), 10);
    }

    #[test]
    fn test_query_range() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("Device", "trustscore").unwrap();

        // Find devices with trustscore >= 75
        let high_trust = index_manager.query(
            "Device",
            "trustscore",
            |v| matches!(v, AttributeValue::Int(score) if *score >= 75),
        );

        // Trustscores are 50-99, so >= 75 should be 25 devices
        assert_eq!(high_trust.len(), 25);
    }

    #[test]
    fn test_query_range_multiple() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("Device", "trustscore").unwrap();

        // Find devices with 60 <= trustscore < 80
        let medium_trust = index_manager.query(
            "Device",
            "trustscore",
            |v| matches!(v, AttributeValue::Int(score) if *score >= 60 && *score < 80),
        );

        // Should be 20 devices (60-79)
        assert_eq!(medium_trust.len(), 20);
    }

    #[test]
    fn test_multiple_indexes() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        // Create multiple indexes
        index_manager.create_index("User", "role").unwrap();
        index_manager.create_index("User", "department").unwrap();
        index_manager.create_index("Device", "trustscore").unwrap();

        assert_eq!(index_manager.index_count(), 3);

        let indexes = index_manager.list_indexes();
        assert!(indexes.contains(&"User.role".to_string()));
        assert!(indexes.contains(&"User.department".to_string()));
        assert!(indexes.contains(&"Device.trustscore".to_string()));
    }

    #[test]
    fn test_query_multiple_attributes() {
        let store = setup_test_store();
        let interner = store.interner();
        let index_manager = IndexManager::new(store.clone());

        // Create indexes
        index_manager.create_index("User", "role").unwrap();
        index_manager.create_index("User", "department").unwrap();

        // Query role
        let analyst_id = interner.intern("analyst");
        let analysts =
            index_manager.query_equals("User", "role", &AttributeValue::String(analyst_id));
        assert_eq!(analysts.len(), 20);

        // Query department
        let eng_id = interner.intern("engineering");
        let engineers =
            index_manager.query_equals("User", "department", &AttributeValue::String(eng_id));
        assert_eq!(engineers.len(), 34); // 100 users / 3 departments ≈ 33-34
    }

    #[test]
    fn test_get_index_stats() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();

        let stats = index_manager.get_index_stats("User", "role").unwrap();
        assert_eq!(stats.entity_count, 100);
        assert_eq!(stats.unique_values, 3);

        // Non-existent index
        let no_stats = index_manager.get_index_stats("User", "nonexistent");
        assert!(no_stats.is_none());
    }

    #[test]
    fn test_remove_index() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();
        assert_eq!(index_manager.index_count(), 1);

        let removed = index_manager.remove_index("User", "role");
        assert!(removed);
        assert_eq!(index_manager.index_count(), 0);

        // Try to remove again
        let removed_again = index_manager.remove_index("User", "role");
        assert!(!removed_again);
    }

    #[test]
    fn test_clear_indexes() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        index_manager.create_index("User", "role").unwrap();
        index_manager.create_index("Device", "trustscore").unwrap();
        assert_eq!(index_manager.index_count(), 2);

        index_manager.clear();
        assert_eq!(index_manager.index_count(), 0);
    }

    #[test]
    fn test_query_nonexistent_index() {
        let store = setup_test_store();
        let index_manager = IndexManager::new(store.clone());

        // Query without creating index - should return empty
        let results = index_manager.query("User", "role", |_| true);
        assert_eq!(results.len(), 0);
    }
}
