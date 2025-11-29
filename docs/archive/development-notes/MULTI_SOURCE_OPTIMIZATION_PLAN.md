# Multi-Source Data Loading Optimization - Implementation Plan

**Version:** 1.0
**Date:** 2025-11-25
**Status:** Phase 1 In Progress
**Target:** Future-proof multi-entity support per MULTI_ENTITY_POLICY_ARCHITECTURE.md

---

## Overview

This document provides detailed implementation guidance for optimizing multi-source data loading with support for future multi-entity policy architecture. The optimization eliminates JSON re-serialization bottlenecks and adds foundation for arbitrary entity types.

---

## Phase 1: Foundation & Entity Type Indexing

**Status:** 🟡 In Progress
**Timeline:** 1 session
**Goal:** Enable 100k entities with generic, entity-type agnostic design

### 1.1 DataStore: Add Entity Type Indexing

**File:** `crates/policy-engine/src/data/store.rs`

**Changes:**

```rust
// Add to DataStore struct
pub struct DataStore {
    entities: DashMap<InternedString, Entity>,
    interner: Arc<StringInterner>,

    // NEW: Index entities by type for O(1) type-based queries
    // entity_type -> Set<entity_id>
    entity_type_index: DashMap<InternedString, DashSet<InternedString>>,
}

// Add to impl DataStore
impl DataStore {
    pub fn new() -> Self {
        Self {
            entities: DashMap::new(),
            interner: Arc::new(StringInterner::new()),
            entity_type_index: DashMap::new(),  // NEW
        }
    }

    /// Insert entity and update type index
    pub fn insert(&self, entity: Entity) {
        let entity_id = entity.id;
        let entity_type = entity.entity_type;

        // Insert entity
        self.entities.insert(entity_id, entity);

        // Update type index
        self.entity_type_index
            .entry(entity_type)
            .or_insert_with(DashSet::new)
            .insert(entity_id);
    }

    /// Get all entity IDs of a specific type
    /// Returns empty vec if type doesn't exist
    pub fn get_entities_by_type(
        &self,
        entity_type: InternedString,
    ) -> Vec<InternedString> {
        self.entity_type_index
            .get(&entity_type)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get entity type statistics
    pub fn get_entity_type_stats(&self) -> HashMap<String, usize> {
        self.entity_type_index
            .iter()
            .map(|entry| {
                let type_str = self.interner.resolve(*entry.key()).to_string();
                let count = entry.value().len();
                (type_str, count)
            })
            .collect()
    }
}
```

**Unit Tests:**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_entity_type_index() {
        let store = DataStore::new();
        let interner = store.interner();

        // Insert multiple entity types
        let user_type = interner.intern("User");
        let device_type = interner.intern("Device");

        let user1 = Entity::new(interner.intern("user_1"), user_type);
        let user2 = Entity::new(interner.intern("user_2"), user_type);
        let device1 = Entity::new(interner.intern("device_1"), device_type);

        store.insert(user1);
        store.insert(user2);
        store.insert(device1);

        // Query by type
        let users = store.get_entities_by_type(user_type);
        assert_eq!(users.len(), 2);

        let devices = store.get_entities_by_type(device_type);
        assert_eq!(devices.len(), 1);

        // Stats
        let stats = store.get_entity_type_stats();
        assert_eq!(stats.get("User"), Some(&2));
        assert_eq!(stats.get("Device"), Some(&1));
    }
}
```

---

### 1.2 DataLoader: Generic JSON Value Loading

**File:** `crates/policy-engine/src/data/loader.rs`

**Changes:**

```rust
// Add LoadStats struct
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

// Add to impl DataLoader
impl DataLoader {
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
    /// let entities = vec![
    ///     json!({"id": "device_1", "type": "Device", "attributes": {"trustscore": 85}}),
    ///     json!({"id": "user_1", "type": "User", "attributes": {"active": true}}),
    /// ];
    /// let stats = loader.load_json_values(entities)?;
    /// println!("Loaded {} entities", stats.total);
    /// ```
    pub fn load_json_values(
        &self,
        entities: Vec<JsonValue>,
    ) -> Result<LoadStats, ReaperError> {
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
    fn parse_entity_from_value(
        &self,
        value: &JsonValue,
    ) -> Result<EntityDocument, ReaperError> {
        serde_json::from_value(value.clone())
            .map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to parse entity: {}", e),
            })
    }

    /// Build an entity from a document (entity-type agnostic)
    fn build_entity_from_doc(
        &self,
        doc: EntityDocument,
        interner: &StringInterner,
    ) -> Result<Entity, ReaperError> {
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
}
```

**Unit Tests:**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_load_json_values_multi_type() {
        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        let entities = vec![
            json!({
                "id": "user_1",
                "type": "User",
                "attributes": {"name": "Alice", "active": true}
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

        assert_eq!(stats.total, 3);
        assert_eq!(stats.by_type.get("User"), Some(&1));
        assert_eq!(stats.by_type.get("Device"), Some(&1));
        assert_eq!(stats.by_type.get("Resource"), Some(&1));
        assert_eq!(stats.total_attributes, 5);
    }

    #[test]
    fn test_load_json_values_vs_load_json() {
        // Verify both methods produce same result
        let store1 = DataStore::new();
        let store2 = DataStore::new();
        let loader1 = DataLoader::new(store1.clone());
        let loader2 = DataLoader::new(store2.clone());

        let json_str = r#"{"entities": [{"id": "user_1", "type": "User", "attributes": {"active": true}}]}"#;
        let json_val = serde_json::from_str::<serde_json::Value>(json_str).unwrap();
        let entities = json_val["entities"].as_array().unwrap().clone();

        loader1.load_json(json_str).unwrap();
        loader2.load_json_values(entities).unwrap();

        // Both should produce identical entities
        let interner = store1.interner();
        let user_id = interner.intern("user_1");

        assert!(store1.get(user_id).is_some());
        assert!(store2.get(user_id).is_some());
    }
}
```

---

### 1.3 Update Test Harness

**File:** `crates/policy-engine/examples/test_dualsource_scale.rs`

**Changes to PHASE 3:**

```rust
// BEFORE (lines 196-213):
let convert_start = Instant::now();

// Create merged JSON document
let merged_json = serde_json::json!({
    "entities": all_entities
});
let merged_json_str = serde_json::to_string(&merged_json)?;

// Load into DataStore
let store = DataStore::new();
let loader = DataLoader::new(store.clone());
let entity_count = loader.load_json(&merged_json_str)?;
let store = Arc::new(store);

let convert_time = convert_start.elapsed();
println!("   ✓ Built DataStore in {:?}", convert_time);
println!("   Total entities: {}", entity_count);

// AFTER:
let convert_start = Instant::now();

// Load into DataStore (direct from JSON values, no serialization)
let store = DataStore::new();
let loader = DataLoader::new(store.clone());
let stats = loader.load_json_values(all_entities)?;
let store = Arc::new(store);

let convert_time = convert_start.elapsed();
println!("   ✓ Built DataStore in {:?}", convert_time);
println!("   Total entities: {}", stats.total);
println!("   Entity types:");
for (entity_type, count) in stats.by_type.iter() {
    println!("      {}: {}", entity_type, count);
}
```

**Add new PHASE 3.5: Entity Type Validation**

```rust
// NEW: Verify entity type index is working
println!("\n🔍 PHASE 3.5: Entity type validation...\n");

let entity_stats = store.get_entity_type_stats();
for (entity_type, count) in entity_stats.iter() {
    println!("   {} entities: {}", entity_type, count);
}
println!();
```

---

### 1.4 Remove Old Approach

**Files to update:**
1. Mark `DataLoader::load_json()` as deprecated (don't remove yet for backwards compat)
2. Update all examples to use `load_json_values()`
3. Update documentation

**Deprecation marker:**

```rust
impl DataLoader {
    /// Load data from a JSON string
    ///
    /// **DEPRECATED:** Use `load_json_values()` instead for better memory efficiency
    /// This method will be removed in version 2.0
    #[deprecated(since = "1.1.0", note = "Use load_json_values() instead")]
    pub fn load_json(&self, json: &str) -> Result<usize, ReaperError> {
        // Keep implementation for backwards compatibility
    }
}
```

---

### 1.5 Integration Tests

**File:** `crates/policy-engine/tests/multi_source_test.rs` (NEW)

```rust
//! Integration tests for multi-source data loading

use policy_engine::*;

#[test]
fn test_multi_source_100_entities() {
    // Small scale test: 100 entities
    // ... (similar to current small test)
}

#[test]
fn test_multi_source_entity_types() {
    // Test multiple entity types in single load
    let entities = vec![
        // Users
        json!({"id": "user_1", "type": "User", "attributes": {...}}),
        // Devices
        json!({"id": "device_1", "type": "Device", "attributes": {...}}),
        // Locations
        json!({"id": "location_1", "type": "Location", "attributes": {...}}),
    ];

    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let stats = loader.load_json_values(entities).unwrap();

    assert_eq!(stats.by_type.len(), 3);
}

#[test]
fn test_entity_type_queries() {
    // Test get_entities_by_type()
    let store = DataStore::new();
    // ... load entities ...

    let user_type = store.interner().intern("User");
    let users = store.get_entities_by_type(user_type);
    assert_eq!(users.len(), expected_user_count);
}
```

---

## Phase 2: Generic Join Framework

**Status:** 📝 Planned
**Timeline:** 1-2 sessions
**Goal:** Support N-way joins for arbitrary entity types

### 2.1 Join Configuration

**File:** `crates/policy-engine/src/data/join.rs` (NEW)

```rust
/// Configuration for joining entities from multiple sources
#[derive(Debug, Clone)]
pub struct JoinConfig {
    /// Primary entity source (will be enriched with secondary data)
    pub primary: EntitySource,

    /// Secondary sources to join with primary
    /// Map: entity_type -> (source, join_key)
    pub secondary: HashMap<String, SecondarySource>,
}

#[derive(Debug, Clone)]
pub struct EntitySource {
    /// File path to JSON data
    pub file_path: String,

    /// Entity type name ("User", "Device", etc.)
    pub entity_type: String,
}

#[derive(Debug, Clone)]
pub struct SecondarySource {
    /// Source file and entity type
    pub source: EntitySource,

    /// How to join
    pub join_key: JoinKey,
}

#[derive(Debug, Clone)]
pub struct JoinKey {
    /// Field in primary entity (e.g., "device_id")
    pub primary_field: String,

    /// Field in secondary entity (e.g., "id")
    pub secondary_field: String,
}
```

### 2.2 Join Engine Implementation

```rust
pub struct JoinEngine {
    loader: DataLoader,
}

impl JoinEngine {
    pub fn new(loader: DataLoader) -> Self {
        Self { loader }
    }

    /// Execute a multi-source join
    pub fn join_and_load(
        &self,
        config: JoinConfig,
    ) -> Result<JoinResult, ReaperError> {
        // 1. Load primary entities
        let primary_entities = self.load_source(&config.primary)?;

        // 2. Build indexes for all secondary sources
        let mut secondary_indexes = HashMap::new();
        for (entity_type, sec_source) in &config.secondary {
            let index = self.build_join_index(&sec_source.source, &sec_source.join_key)?;
            secondary_indexes.insert(entity_type.clone(), (index, sec_source.join_key.clone()));
        }

        // 3. Join and load
        let mut joined_entities = Vec::new();
        for mut primary in primary_entities {
            // Join with each secondary source
            for (_, (index, join_key)) in &secondary_indexes {
                if let Some(join_value) = self.extract_join_value(&primary, &join_key.primary_field) {
                    if let Some(secondary) = index.get(&join_value) {
                        self.merge_attributes(&mut primary, secondary);
                    }
                }
            }
            joined_entities.push(primary);
        }

        // 4. Load all joined entities
        let stats = self.loader.load_json_values(joined_entities)?;

        Ok(JoinResult { stats })
    }

    /// Build join index: join_value -> entity
    fn build_join_index(
        &self,
        source: &EntitySource,
        join_key: &JoinKey,
    ) -> Result<HashMap<String, JsonValue>, ReaperError> {
        let entities = self.load_source(source)?;
        let mut index = HashMap::new();

        for entity in entities {
            if let Some(join_value) = self.extract_join_value(&entity, &join_key.secondary_field) {
                index.insert(join_value, entity);
            }
        }

        Ok(index)
    }

    /// Merge attributes from secondary into primary
    fn merge_attributes(&self, primary: &mut JsonValue, secondary: &JsonValue) {
        if let (Some(p_attrs), Some(s_attrs)) = (
            primary["attributes"].as_object_mut(),
            secondary["attributes"].as_object(),
        ) {
            for (key, value) in s_attrs {
                p_attrs.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
}
```

### 2.3 Usage Examples

```rust
// Example 1: Join users with their devices
let config = JoinConfig {
    primary: EntitySource {
        file_path: "users.json".to_string(),
        entity_type: "User".to_string(),
    },
    secondary: hashmap! {
        "Device".to_string() => SecondarySource {
            source: EntitySource {
                file_path: "devices.json".to_string(),
                entity_type: "Device".to_string(),
            },
            join_key: JoinKey {
                primary_field: "device_id".to_string(),
                secondary_field: "id".to_string(),
            },
        },
    },
};

let engine = JoinEngine::new(loader);
let result = engine.join_and_load(config)?;

// Example 2: Multi-way join (user + device + location)
let config = JoinConfig {
    primary: EntitySource { /* users */ },
    secondary: hashmap! {
        "Device" => SecondarySource { /* devices */ },
        "Location" => SecondarySource { /* locations */ },
    },
};
```

---

## Phase 3: Attribute Indexing

**Status:** 📝 Planned
**Timeline:** 1-2 sessions
**Goal:** Enable fast attribute-based queries

### 3.1 Index Manager

**File:** `crates/policy-engine/src/data/indexes.rs` (NEW)

```rust
/// Manages attribute-based indexes for fast queries
pub struct IndexManager {
    store: Arc<DataStore>,
    indexes: DashMap<String, AttributeIndex>,
}

pub struct AttributeIndex {
    /// value -> Set<entity_ids>
    index: HashMap<AttributeValue, DashSet<InternedString>>,
    entity_type: String,
    attribute_name: String,
}

impl IndexManager {
    pub fn new(store: Arc<DataStore>) -> Self {
        Self {
            store,
            indexes: DashMap::new(),
        }
    }

    /// Create an index for fast lookups
    ///
    /// # Example
    /// ```
    /// // Index devices by trustscore
    /// index_manager.create_index("Device", "trustscore")?;
    ///
    /// // Later: fast query
    /// let high_trust = index_manager.query(
    ///     "Device",
    ///     "trustscore",
    ///     |score| matches!(score, AttributeValue::Int(s) if *s >= 75)
    /// );
    /// ```
    pub fn create_index(
        &self,
        entity_type: &str,
        attribute: &str,
    ) -> Result<(), ReaperError> {
        let index_key = format!("{}.{}", entity_type, attribute);
        let interner = self.store.interner();
        let entity_type_id = interner.intern(entity_type);
        let attr_id = interner.intern(attribute);

        // Build index
        let mut index = HashMap::new();
        for entity_id in self.store.get_entities_by_type(entity_type_id) {
            if let Some(entity) = self.store.get(entity_id) {
                if let Some(attr_value) = entity.get_attribute(attr_id) {
                    index
                        .entry(attr_value.clone())
                        .or_insert_with(DashSet::new)
                        .insert(entity_id);
                }
            }
        }

        self.indexes.insert(
            index_key,
            AttributeIndex {
                index,
                entity_type: entity_type.to_string(),
                attribute_name: attribute.to_string(),
            },
        );

        Ok(())
    }

    /// Query indexed attribute with predicate
    pub fn query<F>(
        &self,
        entity_type: &str,
        attribute: &str,
        predicate: F,
    ) -> Vec<InternedString>
    where
        F: Fn(&AttributeValue) -> bool,
    {
        let index_key = format!("{}.{}", entity_type, attribute);

        self.indexes
            .get(&index_key)
            .map(|idx| {
                idx.index
                    .iter()
                    .filter(|(value, _)| predicate(value))
                    .flat_map(|(_, ids)| ids.iter().copied())
                    .collect()
            })
            .unwrap_or_default()
    }
}
```

### 3.2 Common Index Patterns

```rust
// Pattern 1: Range queries
let high_score_devices = index_manager.query("Device", "trustscore", |v| {
    matches!(v, AttributeValue::Int(score) if *score >= 75)
});

// Pattern 2: Equality
let eu_locations = index_manager.query("Location", "region", |v| {
    matches!(v, AttributeValue::String(r) if r == "EU")
});

// Pattern 3: Set membership
let active_users = index_manager.query("User", "status", |v| {
    matches!(v, AttributeValue::String(s) if s == "active")
});
```

---

## Phase 4: Streaming Support

**Status:** 📝 Planned
**Timeline:** 2-3 sessions
**Goal:** Unlimited scale with constant memory

### 4.1 Streaming JSON Reader

**File:** `crates/policy-engine/src/data/streaming.rs` (NEW)

```rust
/// Streaming JSON reader that processes entities incrementally
pub struct JsonStreamReader {
    file: BufReader<File>,
    buffer: String,
    state: ReaderState,
}

enum ReaderState {
    Start,
    InArray,
    Done,
}

impl JsonStreamReader {
    pub fn new(file_path: &str) -> Result<Self, ReaperError> {
        let file = File::open(file_path)?;
        Ok(Self {
            file: BufReader::new(file),
            buffer: String::new(),
            state: ReaderState::Start,
        })
    }

    /// Read next entity from stream
    pub fn next(&mut self) -> Result<Option<JsonValue>, ReaperError> {
        // Parse one entity at a time
        // Return None when done
    }
}
```

### 4.2 Streaming Loader

```rust
pub struct StreamingLoader {
    loader: DataLoader,
    chunk_size: usize,  // Process in chunks for I/O efficiency
}

impl StreamingLoader {
    /// Stream and load from multiple sources
    /// Memory: O(chunk_size) regardless of total dataset size
    pub fn load_multi_source(
        &self,
        sources: Vec<EntitySource>,
    ) -> Result<StreamingResult, ReaperError> {
        let mut total = 0;
        let mut stats_by_type = HashMap::new();

        for source in sources {
            let count = self.stream_and_load(&source)?;
            total += count;
            stats_by_type.insert(source.entity_type.clone(), count);
        }

        Ok(StreamingResult { total, stats_by_type })
    }

    fn stream_and_load(&self, source: &EntitySource) -> Result<usize, ReaperError> {
        let mut reader = JsonStreamReader::new(&source.file_path)?;
        let mut chunk = Vec::with_capacity(self.chunk_size);
        let mut loaded = 0;

        while let Some(entity) = reader.next()? {
            chunk.push(entity);

            if chunk.len() >= self.chunk_size {
                self.loader.load_json_values(chunk.clone())?;
                loaded += chunk.len();
                chunk.clear();
            }
        }

        // Load remaining
        if !chunk.is_empty() {
            self.loader.load_json_values(chunk)?;
            loaded += chunk.len();
        }

        Ok(loaded)
    }
}
```

### 4.3 Streaming Join

```rust
/// Join with streaming for unlimited scale
pub fn streaming_join(
    primary_source: EntitySource,
    secondary_sources: Vec<SecondarySource>,
    loader: &DataLoader,
) -> Result<usize, ReaperError> {
    // 1. Build compact indexes for secondary sources
    let mut indexes = Vec::new();
    for sec_source in secondary_sources {
        let index = build_compact_index(&sec_source)?;
        indexes.push((index, sec_source.join_key));
    }

    // 2. Stream primary and join incrementally
    let mut reader = JsonStreamReader::new(&primary_source.file_path)?;
    let mut joined_count = 0;

    while let Some(mut primary) = reader.next()? {
        // Join with all secondary sources
        for (index, join_key) in &indexes {
            if let Some(join_val) = extract_join_value(&primary, &join_key.primary_field) {
                if let Some(secondary) = index.get(&join_val) {
                    merge_attributes(&mut primary, secondary);
                }
            }
        }

        // Load immediately (no accumulation)
        loader.load_json_values(vec![primary])?;
        joined_count += 1;
    }

    Ok(joined_count)
}
```

---

## Testing Strategy

### Unit Tests (Per Component)

**Phase 1:**
- `DataStore::insert()` updates type index
- `DataStore::get_entities_by_type()` returns correct IDs
- `DataStore::get_entity_type_stats()` accurate counts
- `DataLoader::load_json_values()` handles multiple types
- `DataLoader::load_json_values()` produces same result as `load_json()`
- LoadStats tracks counts correctly

**Phase 2:**
- JoinConfig construction
- JoinEngine builds indexes correctly
- Join with 1 secondary source
- Join with 2+ secondary sources
- Attribute merge preserves primary values

**Phase 3:**
- IndexManager creates indexes
- Query with range predicates
- Query with equality predicates
- Index updates when new entities added

**Phase 4:**
- JsonStreamReader reads entities sequentially
- StreamingLoader processes chunks
- Memory usage stays constant
- Streaming join produces same result as in-memory

### Integration Tests

**Phase 1:**
- 100 entities across 3 types
- 10k entities across 5 types
- 100k entities (if memory allows)

**Phase 2:**
- 2-way join (user + attributes)
- 3-way join (user + device + location)
- Join with missing data

**Phase 3:**
- Index + query 100k entities
- Multiple indexes on same entity type
- Query performance vs full scan

**Phase 4:**
- Stream 1M entities
- Streaming join 1M entities
- Memory profiling

---

## Performance Targets

| Phase | Dataset | Memory | Load Time | Throughput |
|-------|---------|--------|-----------|------------|
| **1** | 100k entities | <300MB | <10s | 10k entities/sec |
| **2** | 100k (joined) | <350MB | <15s | 7k entities/sec |
| **3** | 100k (indexed) | <400MB | <20s | Index: <5s |
| **4** | 1M entities | <100MB | <120s | Stream: 8k/sec |

---

## Migration Guide

### From Current Approach

**Before (JSON re-serialization):**
```rust
let merged_json = serde_json::json!({"entities": all_entities});
let json_str = serde_json::to_string(&merged_json)?;
let count = loader.load_json(&json_str)?;
```

**After (Direct loading):**
```rust
let stats = loader.load_json_values(all_entities)?;
println!("Loaded {} entities", stats.total);
```

### Future: Multi-Entity Join

**Before (Manual merge):**
```rust
// Manually join two sources
for role in roles {
    if let Some(attrs) = attributes_map.get(&role.user_id) {
        // Manual merge logic
    }
}
```

**After (Join framework):**
```rust
let config = JoinConfig {
    primary: EntitySource { ... },
    secondary: hashmap! { ... },
};
let result = join_engine.join_and_load(config)?;
```

---

## Success Criteria

### Phase 1
- ✅ 100k entities load successfully
- ✅ Memory < 300MB
- ✅ Entity type index working
- ✅ LoadStats shows type distribution
- ✅ All unit tests pass
- ✅ Integration test passes

### Phase 2
- ✅ Generic join supports any entity types
- ✅ N-way joins work
- ✅ Join performance acceptable
- ✅ Documentation complete

### Phase 3
- ✅ Indexes created successfully
- ✅ Queries faster than full scan
- ✅ Multiple indexes supported
- ✅ Index maintenance correct

### Phase 4
- ✅ 1M entities in constant memory
- ✅ Streaming performance acceptable
- ✅ Memory < 100MB regardless of size
- ✅ Production ready

---

**End of Implementation Plan**
