//! High-Performance Data Store with Multi-Index Support
//!
//! The DataStore provides fast entity lookups optimized for policy evaluation.
//! Multiple index strategies enable sub-microsecond queries.

use super::entity::{AttributeValue, Entity, EntityId, EntityType};
use super::interning::{InternedString, StringInterner};
use super::router::{QueryPattern, QueryResult, QueryRouter};
use super::views::ViewManager;
use dashmap::DashMap;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Index strategy for optimizing different query patterns
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStrategy {
    /// Index by entity ID only (default, always enabled)
    ById,
    /// Index by entity type (e.g., all Users, all Resources)
    ByType,
    /// Index by specific attribute (e.g., all entities with role=admin)
    ByAttribute { attribute_key: InternedString },
    /// Composite index (e.g., type=User AND role=admin)
    Composite {
        entity_type: EntityType,
        attribute_key: InternedString,
    },
}

/// Controls which secondary indexes are built during entity insertion.
/// Disabling unused indexes reduces memory ~40-45% for enforcement-only workloads.
#[derive(Debug, Clone)]
pub struct DataStoreConfig {
    /// Build (AttrKey, AttrValue) → EntityId index. Default: true.
    pub index_attributes: bool,
    /// Build (Type, AttrKey, AttrValue) → EntityId index. Default: true.
    pub index_composite: bool,
}

impl Default for DataStoreConfig {
    fn default() -> Self {
        Self {
            index_attributes: true,
            index_composite: true,
        }
    }
}

/// High-performance, multi-indexed data store
///
/// # Memory Layout
/// - Uses Arc<Entity> for zero-copy sharing
/// - DashMap for lock-free concurrent access
/// - String interning for ~60% memory savings
/// - Multiple indexes for fast queries
///
/// # Performance
/// - ID lookup: ~20-50 ns
/// - Type lookup: ~100-200 ns
/// - Attribute lookup: ~100-300 ns (indexed)
/// - Update: ~1-2 µs (atomic)
#[derive(Clone, Debug)]
pub struct DataStore {
    /// Configuration controlling which secondary indexes are built
    config: DataStoreConfig,

    /// String interner shared across all data
    interner: Arc<StringInterner>,

    /// Primary index: ID -> Entity
    entities: Arc<DashMap<EntityId, Arc<Entity>>>,

    /// Type index: EntityType -> Set<EntityId>
    type_index: Arc<DashMap<EntityType, HashSet<EntityId>>>,

    /// Attribute index: (AttrKey, AttrValue) -> Set<EntityId>
    /// Only built for attributes marked for indexing
    attribute_index: Arc<DashMap<(InternedString, InternedString), HashSet<EntityId>>>,

    /// Composite index: (Type, AttrKey, AttrValue) -> Set<EntityId>
    composite_index: Arc<DashMap<(EntityType, InternedString, InternedString), HashSet<EntityId>>>,

    /// Materialized view manager (Phase 6A-1)
    view_manager: Arc<ViewManager>,

    /// ReBAC relationship graph (named directed edges, forward+reverse indexed)
    relationships: Arc<crate::data::relationships::RelationshipGraph>,
}

impl DataStore {
    /// Create a new data store
    pub fn new() -> Self {
        let interner = Arc::new(StringInterner::new());
        Self {
            config: DataStoreConfig::default(),
            entities: Arc::new(DashMap::new()),
            type_index: Arc::new(DashMap::new()),
            attribute_index: Arc::new(DashMap::new()),
            composite_index: Arc::new(DashMap::new()),
            view_manager: Arc::new(ViewManager::new()),
            relationships: Arc::new(crate::data::relationships::RelationshipGraph::new(
                (*interner).clone(),
            )),
            interner,
        }
    }

    /// Create a new data store with custom configuration
    pub fn with_config(config: DataStoreConfig) -> Self {
        let interner = Arc::new(StringInterner::new());
        Self {
            config,
            entities: Arc::new(DashMap::new()),
            type_index: Arc::new(DashMap::new()),
            attribute_index: Arc::new(DashMap::new()),
            composite_index: Arc::new(DashMap::new()),
            view_manager: Arc::new(ViewManager::new()),
            relationships: Arc::new(crate::data::relationships::RelationshipGraph::new(
                (*interner).clone(),
            )),
            interner,
        }
    }

    /// Create a new data store with pre-warmed strings
    pub fn with_prewarm(common_strings: &[&str]) -> Self {
        let interner = StringInterner::new();
        interner.prewarm(common_strings);
        let interner = Arc::new(interner);

        Self {
            config: DataStoreConfig::default(),
            entities: Arc::new(DashMap::new()),
            type_index: Arc::new(DashMap::new()),
            attribute_index: Arc::new(DashMap::new()),
            composite_index: Arc::new(DashMap::new()),
            view_manager: Arc::new(ViewManager::new()),
            relationships: Arc::new(crate::data::relationships::RelationshipGraph::new(
                (*interner).clone(),
            )),
            interner,
        }
    }

    /// Get the string interner
    /// ReBAC relationship graph (edges declared in entity `relationships`).
    pub fn relationships(&self) -> &crate::data::relationships::RelationshipGraph {
        &self.relationships
    }

    /// Record `from #relation @to` in the relationship graph.
    pub fn add_relationship(&self, from: EntityId, relation: InternedString, to: EntityId) {
        self.relationships.add_edge(from, relation, to);
    }

    pub fn interner(&self) -> &StringInterner {
        &self.interner
    }

    /// Insert or update an entity
    ///
    /// # Performance
    /// - ~1-2 µs (includes index updates)
    pub fn insert(&self, entity: Entity) {
        let entity_id = entity.id;
        let entity_type = entity.entity_type;
        let entity_arc = Arc::new(entity);

        // Insert into primary index
        self.entities.insert(entity_id, entity_arc.clone());

        // Update type index
        self.type_index
            .entry(entity_type)
            .or_default()
            .insert(entity_id);

        // Update secondary indexes for string attributes (only if configured)
        if self.config.index_attributes {
            for (attr_key, attr_value) in &entity_arc.attributes {
                if let AttributeValue::String(value_id) = attr_value {
                    self.attribute_index
                        .entry((*attr_key, *value_id))
                        .or_default()
                        .insert(entity_id);
                }
            }
        }

        if self.config.index_composite {
            for (attr_key, attr_value) in &entity_arc.attributes {
                if let AttributeValue::String(value_id) = attr_value {
                    self.composite_index
                        .entry((entity_type, *attr_key, *value_id))
                        .or_default()
                        .insert(entity_id);
                }
            }
        }
    }

    /// Batch insert entities with reduced per-entity overhead.
    /// Separates primary insertion from index building for better locality.
    pub fn insert_batch(&self, entities: Vec<Entity>) {
        // Phase 1: Primary store insertion + type index
        let arcs: Vec<Arc<Entity>> = entities
            .into_iter()
            .map(|entity| {
                let arc = Arc::new(entity);
                self.entities.insert(arc.id, arc.clone());
                self.type_index
                    .entry(arc.entity_type)
                    .or_default()
                    .insert(arc.id);
                arc
            })
            .collect();

        // Phase 2: Secondary indexes (only if configured)
        if self.config.index_attributes {
            for entity in &arcs {
                for (attr_key, attr_value) in &entity.attributes {
                    if let AttributeValue::String(value_id) = attr_value {
                        self.attribute_index
                            .entry((*attr_key, *value_id))
                            .or_default()
                            .insert(entity.id);
                    }
                }
            }
        }

        if self.config.index_composite {
            for entity in &arcs {
                for (attr_key, attr_value) in &entity.attributes {
                    if let AttributeValue::String(value_id) = attr_value {
                        self.composite_index
                            .entry((entity.entity_type, *attr_key, *value_id))
                            .or_default()
                            .insert(entity.id);
                    }
                }
            }
        }
    }

    /// Get an entity by ID
    ///
    /// # Performance
    /// - ~20-50 ns (single hash lookup)
    pub fn get(&self, id: EntityId) -> Option<Arc<Entity>> {
        self.entities.get(&id).map(|entry| entry.value().clone())
    }

    /// Snapshot an entity's attributes as a JSON object, resolving interned
    /// strings. Read-only: uses a lookup that does NOT intern `id`, so it never
    /// pollutes the interner with transient request strings.
    ///
    /// This backs the decision-log "explain" tier — capturing the resolved
    /// entity attributes a decision branched on, so a denial is reproducible. It
    /// runs on the LOG path (after evaluation), never inside the sub-µs eval loop.
    /// Returns `None` if `id` is not a known entity.
    pub fn entity_attributes_json(&self, id: &str) -> Option<serde_json::Value> {
        let interned = self.interner().lookup(id)?;
        let entity = self.get(interned)?;
        let mut map = serde_json::Map::with_capacity(entity.attributes.len());
        for (k, v) in entity.attributes.iter() {
            if let Some(key) = self.interner().resolve(*k) {
                map.insert(key.to_string(), v.to_json(self.interner()));
            }
        }
        Some(serde_json::Value::Object(map))
    }

    /// Get all entities of a specific type
    ///
    /// # Performance
    /// - ~100-200 ns + (n * 50 ns) where n = number of entities
    pub fn get_by_type(&self, entity_type: EntityType) -> Vec<Arc<Entity>> {
        self.type_index
            .get(&entity_type)
            .map(|entry| {
                entry
                    .value()
                    .iter()
                    .filter_map(|id| self.get(*id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get entities by attribute value
    ///
    /// # Performance
    /// - ~100-300 ns + (n * 50 ns) where n = number of matching entities
    pub fn get_by_attribute(
        &self,
        attr_key: InternedString,
        attr_value: InternedString,
    ) -> Vec<Arc<Entity>> {
        self.attribute_index
            .get(&(attr_key, attr_value))
            .map(|entry| {
                entry
                    .value()
                    .iter()
                    .filter_map(|id| self.get(*id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get entities by type and attribute (composite index)
    ///
    /// # Performance
    /// - ~100-200 ns + (n * 50 ns) where n = number of matching entities
    pub fn get_by_type_and_attribute(
        &self,
        entity_type: EntityType,
        attr_key: InternedString,
        attr_value: InternedString,
    ) -> Vec<Arc<Entity>> {
        self.composite_index
            .get(&(entity_type, attr_key, attr_value))
            .map(|entry| {
                entry
                    .value()
                    .iter()
                    .filter_map(|id| self.get(*id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remove an entity by ID
    /// UPSERT: replace an entity, cleaning the indexes its OLD attribute
    /// values occupied (a plain insert leaves stale attribute/composite
    /// index entries pointing at the previous values) and dropping the
    /// relationship edges it previously carried. The delta-sync primitive —
    /// applying the same upsert twice converges (idempotent).
    pub fn upsert(&self, entity: Entity) {
        let id = entity.id;
        self.remove(id);
        self.relationships().detach_carried(id);
        self.insert(entity);
    }

    /// DELETE with cascade: the entity leaves the store AND the graph —
    /// both edges it carried and edges pointing at it. A deleted entity
    /// must not linger as anyone's owner/viewer/member (fail closed).
    pub fn remove_entity(&self, id: EntityId) -> Option<Arc<Entity>> {
        self.relationships().detach(id);
        self.remove(id)
    }

    pub fn remove(&self, id: EntityId) -> Option<Arc<Entity>> {
        let entity = self.entities.remove(&id).map(|(_, e)| e)?;

        // Remove from type index, pruning the entry if it becomes empty so a
        // churn of short-lived types cannot grow the index map without bound.
        if let Some(mut type_set) = self.type_index.get_mut(&entity.entity_type) {
            type_set.remove(&id);
        }
        self.type_index
            .remove_if(&entity.entity_type, |_, set| set.is_empty());

        // Remove from secondary indexes (only if configured). Emptied entries
        // are pruned: without this, a high-cardinality delta stream (unique
        // attribute values) would grow attribute_index/composite_index with
        // empty sets forever — the same unbounded-growth class the interner
        // refcounting fixes, one layer down.
        if self.config.index_attributes {
            for (attr_key, attr_value) in &entity.attributes {
                if let AttributeValue::String(value_id) = attr_value {
                    let key = (*attr_key, *value_id);
                    if let Some(mut attr_set) = self.attribute_index.get_mut(&key) {
                        attr_set.remove(&id);
                    }
                    self.attribute_index
                        .remove_if(&key, |_, set| set.is_empty());
                }
            }
        }

        if self.config.index_composite {
            for (attr_key, attr_value) in &entity.attributes {
                if let AttributeValue::String(value_id) = attr_value {
                    let composite_key = (entity.entity_type, *attr_key, *value_id);
                    if let Some(mut comp_set) = self.composite_index.get_mut(&composite_key) {
                        comp_set.remove(&id);
                    }
                    self.composite_index
                        .remove_if(&composite_key, |_, set| set.is_empty());
                }
            }
        }

        // Release the counted interned strings this entity owned. Indexes above
        // are cleared first, so nothing references these ids by the time the
        // last reference is released and the string is evicted. Balances the
        // DataLoader's `intern_counted` on id/parent/string-values; pinned
        // strings (type, keys, relations) are no-ops in `release`.
        self.release_entity_strings(&entity);

        Some(entity)
    }

    /// Release the interned strings an entity owns, mirroring exactly what the
    /// DataLoader counts (`intern_counted`): the id, the parent, and string
    /// attribute values (recursively through lists/objects/sets). Object keys,
    /// entity types, attribute keys, and relation strings are pinned, so they
    /// are intentionally NOT released here — `release` is a no-op on them anyway.
    fn release_entity_strings(&self, entity: &Entity) {
        self.interner.release(entity.id);
        if let Some(parent) = entity.parent {
            self.interner.release(parent);
        }
        for value in entity.attributes.values() {
            Self::release_attr_value(&self.interner, value);
        }
    }

    fn release_attr_value(interner: &StringInterner, value: &AttributeValue) {
        match value {
            AttributeValue::String(id) => interner.release(*id),
            AttributeValue::List(items) => {
                for v in items {
                    Self::release_attr_value(interner, v);
                }
            }
            // Object keys are pinned (bounded schema vocabulary); release values.
            AttributeValue::Object(map) => {
                for v in map.values() {
                    Self::release_attr_value(interner, v);
                }
            }
            AttributeValue::Set(set) => {
                for v in set {
                    Self::release_attr_value(interner, v);
                }
            }
            AttributeValue::Int(_)
            | AttributeValue::Float(_)
            | AttributeValue::Bool(_)
            | AttributeValue::Null => {}
        }
    }

    /// Get all entities
    pub fn all(&self) -> Vec<Arc<Entity>> {
        self.entities
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Clear all data
    pub fn clear(&self) {
        self.entities.clear();
        self.type_index.clear();
        if self.config.index_attributes {
            self.attribute_index.clear();
        }
        if self.config.index_composite {
            self.composite_index.clear();
        }
        // Drop the relationship graph too: its edges reference entity ids that
        // are about to be evicted, and a stale edge must never outlive its
        // entity (fail closed) or leave a counted subject orphaned.
        self.relationships.clear();
        // After clearing all entities, no counted string is referenced — drop
        // them so a clear()+reload (snapshot deploy) doesn't accumulate stale
        // interned strings. Pinned strings (policy literals, types) survive.
        self.interner.reset_counted();
    }

    /// Get entity counts by type
    ///
    /// Returns a map of entity type name -> count
    /// Useful for understanding dataset composition
    ///
    /// # Example
    /// ```
    /// use policy_engine::DataStore;
    ///
    /// let store = DataStore::new();
    /// let stats = store.get_entity_type_stats();
    /// // Returns empty map for new store: {}
    /// // After loading data: {"User": 1000, "Device": 500, "Resource": 2000}
    /// assert!(stats.is_empty());
    /// ```
    pub fn get_entity_type_stats(&self) -> HashMap<String, usize> {
        self.type_index
            .iter()
            .map(|entry| {
                let type_name = self
                    .interner
                    .resolve(*entry.key())
                    .map(|s| s.as_ref().to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                let count = entry.value().len();
                (type_name, count)
            })
            .collect()
    }

    /// Get statistics about the data store
    pub fn stats(&self) -> DataStoreStats {
        DataStoreStats {
            total_entities: self.entities.len(),
            unique_types: self.type_index.len(),
            indexed_attributes: if self.config.index_attributes {
                self.attribute_index.len()
            } else {
                0
            },
            composite_indexes: if self.config.index_composite {
                self.composite_index.len()
            } else {
                0
            },
            interner_stats: self.interner.stats(),
            estimated_memory_bytes: self.estimate_memory(),
        }
    }

    /// Estimate total memory usage
    fn estimate_memory(&self) -> usize {
        let entity_memory: usize = self
            .entities
            .iter()
            .map(|entry| entry.value().memory_size())
            .sum();

        let attr_index_overhead = if self.config.index_attributes {
            self.attribute_index.len() * 64
        } else {
            0
        };
        let composite_index_overhead = if self.config.index_composite {
            self.composite_index.len() * 64
        } else {
            0
        };
        let index_overhead =
            (self.type_index.len() * 64) + attr_index_overhead + composite_index_overhead;

        entity_memory + index_overhead + self.interner.stats().estimated_memory_bytes
    }

    // ========================================================================
    // Materialized View Management (Phase 6A-1)
    // ========================================================================

    /// Get access to the view manager
    pub fn view_manager(&self) -> &ViewManager {
        &self.view_manager
    }

    /// Add a materialized view
    ///
    /// # Example
    /// ```text
    /// use policy_engine::data::{DataStore, MaterializedView, ViewQuery, ViewStrategy};
    ///
    /// let store = DataStore::new();
    /// let view = MaterializedView::new(
    ///     "user_permission".to_string(),
    ///     ViewQuery::UserPermission {
    ///         binding_type: "user_role_binding".to_string(),
    ///         permission_type: "role_permission".to_string(),
    ///         join_key: "role".to_string(),
    ///     },
    ///     ViewStrategy::Eager,
    /// );
    /// store.add_view(view)?;
    /// ```
    pub fn add_view(&self, view: super::views::MaterializedView) -> Result<(), ReaperError> {
        self.view_manager.add_view(view)
    }

    /// Get a materialized view by name
    pub fn get_view(&self, name: &str) -> Option<super::views::MaterializedView> {
        self.view_manager.get_view(name)
    }

    /// Remove a materialized view
    pub fn remove_view(&self, name: &str) -> Option<super::views::MaterializedView> {
        self.view_manager.remove_view(name)
    }

    /// List all materialized view names
    pub fn list_views(&self) -> Vec<String> {
        self.view_manager.list_views()
    }

    /// Invalidate views that depend on the given entity type
    ///
    /// This should be called when entities of a specific type are modified
    pub fn invalidate_views_by_type(&self, entity_type: &str) {
        self.view_manager.invalidate_by_type(entity_type);
    }

    /// Invalidate a specific view by name
    pub fn invalidate_view(&self, name: &str) -> Result<(), ReaperError> {
        self.view_manager.invalidate_view(name)
    }

    /// Get statistics for all views
    pub fn view_stats(&self) -> Vec<super::views::ViewStats> {
        self.view_manager.stats()
    }

    /// Query a materialized view
    ///
    /// Returns entities from the view. If the view is stale, it should be
    /// recomputed first (Phase 6A-2 will add automatic recomputation).
    ///
    /// # Example
    /// ```text
    /// let results = store.query_view("user_permission", |entity| {
    ///     // Filter by user and resource
    ///     entity.get_attribute_str("user") == Some("alice") &&
    ///     entity.get_attribute_str("resource") == Some("foo123")
    /// });
    /// ```
    pub fn query_view<F>(&self, name: &str, predicate: F) -> Result<Vec<Arc<Entity>>, ReaperError>
    where
        F: Fn(&Entity) -> bool,
    {
        let view = self
            .get_view(name)
            .ok_or_else(|| ReaperError::ViewError(format!("View '{}' not found", name)))?;

        Ok(view.query(predicate))
    }

    /// Get all entities from a materialized view
    pub fn get_view_entities(&self, name: &str) -> Result<Vec<Arc<Entity>>, ReaperError> {
        let view = self
            .get_view(name)
            .ok_or_else(|| ReaperError::ViewError(format!("View '{}' not found", name)))?;

        Ok(view.all())
    }

    // ========================================================================
    // Query Router (Phase 6A-2)
    // ========================================================================

    /// Execute a query using the intelligent query router
    ///
    /// The router automatically selects the optimal execution strategy:
    /// - Tier 1 (100-500ns): Use pre-computed materialized views
    /// - Tier 2 (1-3µs): Use indexed joins
    /// - Tier 3 (3-5µs): Partial scan with some indexes
    /// - Tier 4 (5-10µs): Full scan with filtering
    ///
    /// # Example
    /// ```text
    /// use policy_engine::data::{DataStore, QueryPattern};
    ///
    /// let store = DataStore::new();
    ///
    /// // Check if alice can write to foo123
    /// let result = store.query(QueryPattern::PermissionCheck {
    ///     user: "alice".to_string(),
    ///     resource: "foo123".to_string(),
    ///     action: "write".to_string(),
    /// })?;
    ///
    /// println!("Found {} results using {}",
    ///     result.entities.len(),
    ///     result.tier.description()
    /// );
    /// ```
    pub fn query(&self, pattern: QueryPattern) -> Result<QueryResult, ReaperError> {
        // Create router on-demand (no need to store it)
        let router = QueryRouter::new(Arc::new(self.clone()));
        router.execute(pattern)
    }

    /// Create a query router for this data store
    ///
    /// Use this if you need to execute multiple queries with the same router.
    /// Otherwise, use `query()` which creates a router on-demand.
    pub fn create_router(&self) -> QueryRouter {
        QueryRouter::new(Arc::new(self.clone()))
    }
}

impl Default for DataStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the data store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataStoreStats {
    pub total_entities: usize,
    pub unique_types: usize,
    pub indexed_attributes: usize,
    pub composite_indexes: usize,
    pub interner_stats: super::interning::InternerStats,
    pub estimated_memory_bytes: usize,
}

/// Query builder for complex queries
pub struct QueryBuilder<'a> {
    store: &'a DataStore,
    entity_type: Option<EntityType>,
    attribute_filters: Vec<(InternedString, InternedString)>,
}

impl<'a> QueryBuilder<'a> {
    /// Create a new query builder
    pub fn new(store: &'a DataStore) -> Self {
        Self {
            store,
            entity_type: None,
            attribute_filters: Vec::new(),
        }
    }

    /// Filter by entity type
    pub fn with_type(mut self, entity_type: EntityType) -> Self {
        self.entity_type = Some(entity_type);
        self
    }

    /// Add an attribute filter
    pub fn with_attribute(mut self, key: InternedString, value: InternedString) -> Self {
        self.attribute_filters.push((key, value));
        self
    }

    /// Execute the query
    pub fn execute(self) -> Vec<Arc<Entity>> {
        // Optimize based on available indexes
        if let (Some(entity_type), Some((key, value))) =
            (self.entity_type, self.attribute_filters.first())
        {
            // Use composite index if available
            let mut results = self
                .store
                .get_by_type_and_attribute(entity_type, *key, *value);

            // Apply remaining filters
            for (filter_key, filter_value) in self.attribute_filters.iter().skip(1) {
                results.retain(|entity| {
                    entity
                        .get_attribute(*filter_key)
                        .and_then(|v| v.as_string(self.store.interner()))
                        .map(|s| {
                            s.as_ref()
                                == self
                                    .store
                                    .interner()
                                    .resolve(*filter_value)
                                    .unwrap()
                                    .as_ref()
                        })
                        .unwrap_or(false)
                });
            }

            results
        } else if let Some(entity_type) = self.entity_type {
            // Just type filter
            let mut results = self.store.get_by_type(entity_type);

            // Apply attribute filters
            for (key, value) in self.attribute_filters {
                results.retain(|entity| {
                    entity
                        .get_attribute(key)
                        .and_then(|v| v.as_string(self.store.interner()))
                        .map(|s| {
                            s.as_ref() == self.store.interner().resolve(value).unwrap().as_ref()
                        })
                        .unwrap_or(false)
                });
            }

            results
        } else if let Some((key, value)) = self.attribute_filters.first() {
            // Just attribute filter
            self.store.get_by_attribute(*key, *value)
        } else {
            // No filters - return all
            self.store.all()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::entity::EntityBuilder;

    #[test]
    fn test_insert_and_get() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_id = interner.intern("alice");
        let user_type = interner.intern("User");

        let entity = EntityBuilder::new(user_id, user_type).build();
        store.insert(entity);

        let retrieved = store.get(user_id).unwrap();
        assert_eq!(retrieved.id, user_id);
    }

    #[test]
    fn test_entity_attributes_json_snapshot() {
        let store = DataStore::new();
        let interner = store.interner();

        let alice = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let clearance_key = interner.intern("clearance_level");
        store.insert(
            EntityBuilder::new(alice, user_type)
                .with_attribute(role_key, AttributeValue::from_string("admin", interner))
                .with_attribute(clearance_key, AttributeValue::Int(5))
                .build(),
        );

        // Known entity -> resolved attributes as JSON.
        let json = store
            .entity_attributes_json("alice")
            .expect("entity exists");
        assert_eq!(json["role"], serde_json::json!("admin"));
        assert_eq!(json["clearance_level"], serde_json::json!(5));

        // Unknown id -> None, and (crucially) the lookup did NOT intern it.
        assert!(store.entity_attributes_json("ghost").is_none());
        assert!(
            interner.lookup("ghost").is_none(),
            "explain snapshot must not pollute the interner"
        );
    }

    #[test]
    fn test_get_by_type() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let alice_id = interner.intern("alice");
        let bob_id = interner.intern("bob");

        store.insert(EntityBuilder::new(alice_id, user_type).build());
        store.insert(EntityBuilder::new(bob_id, user_type).build());

        let users = store.get_by_type(user_type);
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn test_get_by_attribute() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let alice_id = interner.intern("alice");
        let bob_id = interner.intern("bob");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");
        let user_value = interner.intern("user");

        store.insert(
            EntityBuilder::new(alice_id, user_type)
                .with_string(role_key, admin_value)
                .build(),
        );
        store.insert(
            EntityBuilder::new(bob_id, user_type)
                .with_string(role_key, user_value)
                .build(),
        );

        let admins = store.get_by_attribute(role_key, admin_value);
        assert_eq!(admins.len(), 1);
        assert_eq!(admins[0].id, alice_id);
    }

    #[test]
    fn test_composite_index() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let resource_type = interner.intern("Resource");
        let alice_id = interner.intern("alice");
        let doc_id = interner.intern("doc1");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");

        store.insert(
            EntityBuilder::new(alice_id, user_type)
                .with_string(role_key, admin_value)
                .build(),
        );
        store.insert(
            EntityBuilder::new(doc_id, resource_type)
                .with_string(role_key, admin_value)
                .build(),
        );

        // Should only get Users with role=admin
        let admin_users = store.get_by_type_and_attribute(user_type, role_key, admin_value);
        assert_eq!(admin_users.len(), 1);
        assert_eq!(admin_users[0].id, alice_id);
    }

    #[test]
    fn test_query_builder() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let alice_id = interner.intern("alice");
        let role_key = interner.intern("role");
        let dept_key = interner.intern("department");
        let admin_value = interner.intern("admin");
        let eng_value = interner.intern("engineering");

        store.insert(
            EntityBuilder::new(alice_id, user_type)
                .with_string(role_key, admin_value)
                .with_string(dept_key, eng_value)
                .build(),
        );

        let results = QueryBuilder::new(&store)
            .with_type(user_type)
            .with_attribute(role_key, admin_value)
            .with_attribute(dept_key, eng_value)
            .execute();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, alice_id);
    }

    #[test]
    fn test_memory_efficiency() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let department_key = interner.intern("department");
        let admin_value = interner.intern("admin");
        let engineering_value = interner.intern("engineering");

        // Insert 1000 users, all with:
        // - type: "User" (used 1000x, stored 1x)
        // - role: "admin" (used 1000x, stored 1x)
        // - department: "engineering" (used 1000x, stored 1x)
        for i in 0..1000 {
            let user_id = interner.intern(&format!("user{}", i));
            store.insert(
                EntityBuilder::new(user_id, user_type)
                    .with_string(role_key, admin_value)
                    .with_string(department_key, engineering_value)
                    .build(),
            );
        }

        let stats = store.stats();

        // Memory efficiency verification:
        // The key benefit of string interning is that repeated strings are stored only ONCE:
        // - "User" is used 1000 times (entity type) but stored 1x
        // - "role" is used 1000 times (attribute key) but stored 1x
        // - "department" is used 1000 times (attribute key) but stored 1x
        // - "admin" is used 1000 times (attribute value) but stored 1x
        // - "engineering" is used 1000 times (attribute value) but stored 1x
        //
        // We also have 1000 unique user IDs ("user0" through "user999")
        //
        // Total unique strings = 1000 user IDs + 5 shared strings = 1005
        assert_eq!(stats.interner_stats.unique_strings, 1005);

        // Should have 1000 entities
        assert_eq!(stats.total_entities, 1000);

        // Memory is ~80KB with Arc overhead and DashMap entries
        // This is still efficient: each entity references shared strings via 4-byte IDs
        // instead of storing full String copies
        assert!(stats.interner_stats.estimated_memory_bytes < 100000);
    }

    #[test]
    fn test_get_entity_type_stats() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let device_type = interner.intern("Device");
        let resource_type = interner.intern("Resource");

        // Insert multiple entity types
        for i in 0..100 {
            let user_id = interner.intern(&format!("user_{}", i));
            store.insert(EntityBuilder::new(user_id, user_type).build());
        }

        for i in 0..50 {
            let device_id = interner.intern(&format!("device_{}", i));
            store.insert(EntityBuilder::new(device_id, device_type).build());
        }

        for i in 0..200 {
            let resource_id = interner.intern(&format!("doc_{}", i));
            store.insert(EntityBuilder::new(resource_id, resource_type).build());
        }

        // Get type stats
        let type_stats = store.get_entity_type_stats();

        assert_eq!(type_stats.get("User"), Some(&100));
        assert_eq!(type_stats.get("Device"), Some(&50));
        assert_eq!(type_stats.get("Resource"), Some(&200));
        assert_eq!(type_stats.len(), 3);
    }

    // ========================================================================
    // Materialized View Tests (Phase 6A-1)
    // ========================================================================

    #[test]
    fn test_add_and_get_view() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();

        let view = MaterializedView::new(
            "test_view".to_string(),
            ViewQuery::Custom {
                description: "Test view".to_string(),
            },
            ViewStrategy::Eager,
        );

        // Add view
        store.add_view(view).unwrap();

        // Get view back
        let retrieved = store.get_view("test_view").unwrap();
        assert_eq!(retrieved.name, "test_view");

        // List views
        let views = store.list_views();
        assert_eq!(views.len(), 1);
        assert!(views.contains(&"test_view".to_string()));
    }

    #[test]
    fn test_remove_view() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();

        let view = MaterializedView::new(
            "removable_view".to_string(),
            ViewQuery::Custom {
                description: "Test view".to_string(),
            },
            ViewStrategy::Lazy,
        );

        store.add_view(view).unwrap();
        assert_eq!(store.list_views().len(), 1);

        // Remove view
        let removed = store.remove_view("removable_view");
        assert!(removed.is_some());
        assert_eq!(store.list_views().len(), 0);
    }

    #[test]
    fn test_invalidate_view() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();

        let view = MaterializedView::new(
            "staleable_view".to_string(),
            ViewQuery::Custom {
                description: "Test view".to_string(),
            },
            ViewStrategy::Eager,
        );

        store.add_view(view).unwrap();

        // Mark view as fresh
        if let Some(mut manager_view) = store.view_manager().get_view("staleable_view") {
            manager_view.mark_fresh();
            // Re-add the updated view
            store.remove_view("staleable_view");
            store.add_view(manager_view).unwrap();
        }

        // Invalidate view
        store.invalidate_view("staleable_view").unwrap();

        // Check if stale
        let view = store.get_view("staleable_view").unwrap();
        assert!(view.is_stale);
    }

    #[test]
    fn test_invalidate_views_by_type() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();

        // Create views with dependencies
        let view1 = MaterializedView::new(
            "user_perm_view".to_string(),
            ViewQuery::UserPermission {
                binding_type: "user_role_binding".to_string(),
                permission_type: "role_permission".to_string(),
                join_key: "role".to_string(),
            },
            ViewStrategy::Eager,
        );

        let view2 = MaterializedView::new(
            "role_users_view".to_string(),
            ViewQuery::RoleUsers {
                binding_type: "user_role_binding".to_string(),
            },
            ViewStrategy::Lazy,
        );

        store.add_view(view1).unwrap();
        store.add_view(view2).unwrap();

        // Invalidate all views depending on "user_role_binding"
        store.invalidate_views_by_type("user_role_binding");

        // Both views should be stale
        let view1_after = store.get_view("user_perm_view").unwrap();
        let view2_after = store.get_view("role_users_view").unwrap();
        assert!(view1_after.is_stale);
        assert!(view2_after.is_stale);
    }

    #[test]
    fn test_view_with_entities() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();
        let interner = store.interner();

        // Create a view
        let view = MaterializedView::new(
            "entity_view".to_string(),
            ViewQuery::Custom {
                description: "Test view with entities".to_string(),
            },
            ViewStrategy::Eager,
        );

        // Add some entities to the view
        let entity_id = interner.intern("test_entity");
        let entity_type = interner.intern("TestType");
        let entity = Arc::new(EntityBuilder::new(entity_id, entity_type).build());

        view.insert("key1".to_string(), entity.clone());

        // Add view to store
        store.add_view(view).unwrap();

        // Query view entities
        let entities = store.get_view_entities("entity_view").unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].id, entity_id);
    }

    #[test]
    fn test_query_view_with_predicate() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();
        let interner = store.interner();

        // Create a view
        let view = MaterializedView::new(
            "filtered_view".to_string(),
            ViewQuery::Custom {
                description: "Test view for filtering".to_string(),
            },
            ViewStrategy::Lazy,
        );

        // Add entities of different types
        let type_a = interner.intern("TypeA");
        let type_b = interner.intern("TypeB");

        for i in 0..5 {
            let id = interner.intern(&format!("entity_{}", i));
            let entity_type = if i % 2 == 0 { type_a } else { type_b };
            let entity = Arc::new(EntityBuilder::new(id, entity_type).build());
            view.insert(format!("key_{}", i), entity);
        }

        store.add_view(view).unwrap();

        // Query for TypeA entities
        let type_a_entities = store
            .query_view("filtered_view", |e| e.entity_type == type_a)
            .unwrap();
        assert_eq!(type_a_entities.len(), 3); // 0, 2, 4

        // Query for TypeB entities
        let type_b_entities = store
            .query_view("filtered_view", |e| e.entity_type == type_b)
            .unwrap();
        assert_eq!(type_b_entities.len(), 2); // 1, 3
    }

    #[test]
    fn test_view_stats() {
        use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};

        let store = DataStore::new();

        let view1 = MaterializedView::new(
            "stats_view_1".to_string(),
            ViewQuery::Custom {
                description: "First view".to_string(),
            },
            ViewStrategy::Eager,
        );

        let view2 = MaterializedView::new(
            "stats_view_2".to_string(),
            ViewQuery::Custom {
                description: "Second view".to_string(),
            },
            ViewStrategy::Lazy,
        );

        store.add_view(view1).unwrap();
        store.add_view(view2).unwrap();

        // Get stats for all views
        let stats = store.view_stats();
        assert_eq!(stats.len(), 2);

        // Verify stats contain both views
        let view_names: Vec<String> = stats.iter().map(|s| s.name.clone()).collect();
        assert!(view_names.contains(&"stats_view_1".to_string()));
        assert!(view_names.contains(&"stats_view_2".to_string()));
    }
}
