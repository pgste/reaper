//! Binary Data Bundle Format (.rdb)
//!
//! Provides efficient serialization of DataStore contents for:
//! - Fast loading (pre-interned strings, binary format)
//! - Atomic data updates (swap entire dataset at once)
//! - Network transfer (compact binary representation)
//!
//! # Format
//! ```text
//! Magic: "REDB" (4 bytes)
//! Version: u32
//! Metadata (postcard):
//!   - name
//!   - version
//!   - created_at
//!   - entity_count
//! String Table (postcard):
//!   - Vec<String> indexed by InternedString ID
//! Entities (postcard):
//!   - Vec<SerializedEntity>
//! Checksum: SHA256 (32 bytes)
//! ```

use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::entity::{AttributeValue, Entity};
use super::interning::InternedString;
use super::store::DataStore;

/// Magic bytes for data bundle format
pub const DATA_BUNDLE_MAGIC: &[u8; 4] = b"REDB";

/// Current bundle format version (v2: postcard serialization, replaces bincode — RUSTSEC-2025-0141)
pub const DATA_BUNDLE_VERSION: u32 = 2;

/// Metadata for a data bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBundleMetadata {
    /// Bundle name
    pub name: String,
    /// Bundle version
    pub version: String,
    /// Creation timestamp (RFC 3339)
    pub created_at: String,
    /// Number of entities in the bundle
    pub entity_count: usize,
    /// Original source (e.g., "api", "file", "sync")
    pub source: Option<String>,
    /// Additional metadata
    pub extra: HashMap<String, String>,
}

/// String table for efficient storage
///
/// All strings in the bundle are stored once in this table.
/// Entities reference strings by their index in this table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringTable {
    /// All unique strings, indexed by their table position
    pub strings: Vec<String>,
}

impl StringTable {
    /// Create a new empty string table
    pub fn new() -> Self {
        Self {
            strings: Vec::new(),
        }
    }

    /// Add a string and return its index
    pub fn add(&mut self, s: &str) -> u32 {
        // Check if already exists
        if let Some(idx) = self.strings.iter().position(|existing| existing == s) {
            return idx as u32;
        }
        // Add new string
        let idx = self.strings.len() as u32;
        self.strings.push(s.to_string());
        idx
    }

    /// Get a string by index
    pub fn get(&self, idx: u32) -> Option<&str> {
        self.strings.get(idx as usize).map(|s| s.as_str())
    }

    /// Get the number of strings
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

impl Default for StringTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Serialized attribute value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedAttributeValue {
    /// String value (index into string table)
    String(u32),
    /// Integer value
    Int(i64),
    /// Float value
    Float(f64),
    /// Boolean value
    Bool(bool),
    /// List of values
    List(Vec<SerializedAttributeValue>),
    /// Object/Map of key-value pairs
    Object(Vec<(u32, SerializedAttributeValue)>),
    /// Set of unique values
    Set(Vec<SerializedAttributeValue>),
    /// Null value
    Null,
}

/// Serialized entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedEntity {
    /// Entity ID (index into string table)
    pub id: u32,
    /// Entity type (index into string table)
    pub entity_type: u32,
    /// Attributes: key (string table index) -> value
    pub attributes: Vec<(u32, SerializedAttributeValue)>,
    /// Parent entity ID (index into string table, if any)
    pub parent: Option<u32>,
}

/// Binary data bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBundle {
    /// Bundle metadata
    pub metadata: DataBundleMetadata,
    /// String table for deduplication
    pub string_table: StringTable,
    /// Serialized entities
    pub entities: Vec<SerializedEntity>,
}

impl DataBundle {
    /// Create a new empty data bundle
    pub fn new(name: String, version: String) -> Self {
        Self {
            metadata: DataBundleMetadata {
                name,
                version,
                created_at: chrono::Utc::now().to_rfc3339(),
                entity_count: 0,
                source: None,
                extra: HashMap::new(),
            },
            string_table: StringTable::new(),
            entities: Vec::new(),
        }
    }

    /// Create a data bundle from a DataStore
    ///
    /// Extracts all entities and interns all strings into the string table.
    pub fn from_store(store: &DataStore, name: String, version: String) -> Self {
        let interner = store.interner();
        let mut bundle = Self::new(name, version);

        // Build string table and serialize entities
        let all_entities = store.all();
        bundle.metadata.entity_count = all_entities.len();

        for entity in all_entities {
            let serialized = bundle.serialize_entity(&entity, interner);
            bundle.entities.push(serialized);
        }

        bundle
    }

    /// Serialize a single entity
    fn serialize_entity(
        &mut self,
        entity: &Entity,
        interner: &super::interning::StringInterner,
    ) -> SerializedEntity {
        // Add entity ID and type to string table
        let id = self.add_interned_string(entity.id, interner);
        let entity_type = self.add_interned_string(entity.entity_type, interner);

        // Serialize attributes
        let mut attributes = Vec::with_capacity(entity.attributes.len());
        for (key, value) in &entity.attributes {
            let key_idx = self.add_interned_string(*key, interner);
            let serialized_value = self.serialize_attribute_value(value, interner);
            attributes.push((key_idx, serialized_value));
        }

        // Serialize parent if present
        let parent = entity.parent.map(|p| self.add_interned_string(p, interner));

        SerializedEntity {
            id,
            entity_type,
            attributes,
            parent,
        }
    }

    /// Add an interned string to the string table and return its index
    fn add_interned_string(
        &mut self,
        interned: InternedString,
        interner: &super::interning::StringInterner,
    ) -> u32 {
        if let Some(s) = interner.resolve(interned) {
            self.string_table.add(s.as_ref())
        } else {
            // Fallback for unknown strings
            self.string_table.add("")
        }
    }

    /// Serialize an attribute value
    fn serialize_attribute_value(
        &mut self,
        value: &AttributeValue,
        interner: &super::interning::StringInterner,
    ) -> SerializedAttributeValue {
        match value {
            AttributeValue::String(s) => {
                SerializedAttributeValue::String(self.add_interned_string(*s, interner))
            }
            AttributeValue::Int(i) => SerializedAttributeValue::Int(*i),
            AttributeValue::Float(f) => SerializedAttributeValue::Float(*f),
            AttributeValue::Bool(b) => SerializedAttributeValue::Bool(*b),
            AttributeValue::List(list) => {
                let serialized: Vec<SerializedAttributeValue> = list
                    .iter()
                    .map(|v| self.serialize_attribute_value(v, interner))
                    .collect();
                SerializedAttributeValue::List(serialized)
            }
            AttributeValue::Object(map) => {
                let serialized: Vec<(u32, SerializedAttributeValue)> = map
                    .iter()
                    .map(|(k, v)| {
                        let key_idx = self.add_interned_string(*k, interner);
                        let value = self.serialize_attribute_value(v, interner);
                        (key_idx, value)
                    })
                    .collect();
                SerializedAttributeValue::Object(serialized)
            }
            AttributeValue::Set(set) => {
                let serialized: Vec<SerializedAttributeValue> = set
                    .iter()
                    .map(|v| self.serialize_attribute_value(v, interner))
                    .collect();
                SerializedAttributeValue::Set(serialized)
            }
            AttributeValue::Null => SerializedAttributeValue::Null,
        }
    }

    /// Serialize to binary format
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();

        // Magic bytes
        bytes.extend_from_slice(DATA_BUNDLE_MAGIC);

        // Version
        bytes.extend_from_slice(&DATA_BUNDLE_VERSION.to_le_bytes());

        // Serialize metadata with postcard (replaces bincode — RUSTSEC-2025-0141)
        let metadata_bytes = postcard::to_allocvec(&self.metadata).map_err(|e| {
            ReaperError::BinarySerializationError(format!("Failed to serialize metadata: {}", e))
        })?;
        bytes.extend_from_slice(&(metadata_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&metadata_bytes);

        // Serialize string table
        let string_table_bytes = postcard::to_allocvec(&self.string_table).map_err(|e| {
            ReaperError::BinarySerializationError(format!(
                "Failed to serialize string table: {}",
                e
            ))
        })?;
        bytes.extend_from_slice(&(string_table_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&string_table_bytes);

        // Serialize entities
        let entities_bytes = postcard::to_allocvec(&self.entities).map_err(|e| {
            ReaperError::BinarySerializationError(format!("Failed to serialize entities: {}", e))
        })?;
        bytes.extend_from_slice(&(entities_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&entities_bytes);

        // Compute and append checksum
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let checksum = hasher.finalize();
        bytes.extend_from_slice(&checksum);

        Ok(bytes)
    }

    /// Deserialize from binary format
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        let mut offset = 0;

        // Verify magic bytes
        if bytes.len() < 4 || &bytes[0..4] != DATA_BUNDLE_MAGIC {
            return Err(ReaperError::ParseError(
                "Invalid data bundle magic bytes".to_string(),
            ));
        }
        offset += 4;

        // Read version
        if bytes.len() < offset + 4 {
            return Err(ReaperError::ParseError(
                "Data bundle too short for version".to_string(),
            ));
        }
        let version = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        offset += 4;

        if version != DATA_BUNDLE_VERSION {
            return Err(ReaperError::ParseError(format!(
                "Unsupported data bundle version: {} (expected {})",
                version, DATA_BUNDLE_VERSION
            )));
        }

        // Read metadata
        if bytes.len() < offset + 4 {
            return Err(ReaperError::ParseError(
                "Data bundle too short for metadata length".to_string(),
            ));
        }
        let metadata_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        if bytes.len() < offset + metadata_len {
            return Err(ReaperError::ParseError(
                "Data bundle too short for metadata".to_string(),
            ));
        }
        let metadata: DataBundleMetadata =
            postcard::from_bytes(&bytes[offset..offset + metadata_len])
                .map_err(|e| ReaperError::ParseError(format!("Failed to parse metadata: {}", e)))?;
        offset += metadata_len;

        // Read string table
        if bytes.len() < offset + 4 {
            return Err(ReaperError::ParseError(
                "Data bundle too short for string table length".to_string(),
            ));
        }
        let string_table_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        if bytes.len() < offset + string_table_len {
            return Err(ReaperError::ParseError(
                "Data bundle too short for string table".to_string(),
            ));
        }
        let string_table: StringTable =
            postcard::from_bytes(&bytes[offset..offset + string_table_len]).map_err(|e| {
                ReaperError::ParseError(format!("Failed to parse string table: {}", e))
            })?;
        offset += string_table_len;

        // Read entities
        if bytes.len() < offset + 4 {
            return Err(ReaperError::ParseError(
                "Data bundle too short for entities length".to_string(),
            ));
        }
        let entities_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        if bytes.len() < offset + entities_len {
            return Err(ReaperError::ParseError(
                "Data bundle too short for entities".to_string(),
            ));
        }
        let entities: Vec<SerializedEntity> =
            postcard::from_bytes(&bytes[offset..offset + entities_len])
                .map_err(|e| ReaperError::ParseError(format!("Failed to parse entities: {}", e)))?;
        offset += entities_len;

        // Verify checksum
        if bytes.len() < offset + 32 {
            return Err(ReaperError::ParseError(
                "Data bundle missing checksum".to_string(),
            ));
        }
        let stored_checksum = &bytes[offset..offset + 32];
        let mut hasher = Sha256::new();
        hasher.update(&bytes[..offset]);
        let computed_checksum = hasher.finalize();

        if stored_checksum != computed_checksum.as_slice() {
            return Err(ReaperError::ParseError(
                "Data bundle checksum mismatch".to_string(),
            ));
        }

        Ok(Self {
            metadata,
            string_table,
            entities,
        })
    }

    /// Load into a new DataStore
    pub fn load_into_store(&self) -> Result<DataStore, ReaperError> {
        let store = DataStore::new();
        self.load_into_existing_store(&store)?;
        Ok(store)
    }

    /// Load into an existing DataStore (appending entities)
    pub fn load_into_existing_store(&self, store: &DataStore) -> Result<(), ReaperError> {
        let interner = store.interner();

        // Pre-intern all strings from the string table
        let mut string_map: Vec<InternedString> = Vec::with_capacity(self.string_table.len());
        for s in &self.string_table.strings {
            string_map.push(interner.intern(s));
        }

        // Load entities
        for serialized in &self.entities {
            let entity = self.deserialize_entity(serialized, &string_map)?;
            store.insert(entity);
        }

        Ok(())
    }

    /// Replace all data in a DataStore with this bundle's data
    pub fn replace_store(&self, store: &DataStore) -> Result<(), ReaperError> {
        store.clear();
        self.load_into_existing_store(store)
    }

    /// Deserialize a single entity
    fn deserialize_entity(
        &self,
        serialized: &SerializedEntity,
        string_map: &[InternedString],
    ) -> Result<Entity, ReaperError> {
        let id = *string_map.get(serialized.id as usize).ok_or_else(|| {
            ReaperError::ParseError(format!("Invalid entity ID index: {}", serialized.id))
        })?;

        let entity_type = *string_map
            .get(serialized.entity_type as usize)
            .ok_or_else(|| {
                ReaperError::ParseError(format!(
                    "Invalid entity type index: {}",
                    serialized.entity_type
                ))
            })?;

        let mut attributes = HashMap::new();
        for (key_idx, value) in &serialized.attributes {
            let key = *string_map.get(*key_idx as usize).ok_or_else(|| {
                ReaperError::ParseError(format!("Invalid attribute key index: {}", key_idx))
            })?;

            let attr_value = Self::deserialize_attribute_value(value, string_map)?;
            attributes.insert(key, attr_value);
        }

        let parent = if let Some(parent_idx) = serialized.parent {
            Some(*string_map.get(parent_idx as usize).ok_or_else(|| {
                ReaperError::ParseError(format!("Invalid parent index: {}", parent_idx))
            })?)
        } else {
            None
        };

        Ok(Entity {
            id,
            entity_type,
            attributes,
            parent,
        })
    }

    /// Deserialize an attribute value
    fn deserialize_attribute_value(
        value: &SerializedAttributeValue,
        string_map: &[InternedString],
    ) -> Result<AttributeValue, ReaperError> {
        match value {
            SerializedAttributeValue::String(idx) => {
                let s = *string_map.get(*idx as usize).ok_or_else(|| {
                    ReaperError::ParseError(format!("Invalid string value index: {}", idx))
                })?;
                Ok(AttributeValue::String(s))
            }
            SerializedAttributeValue::Int(i) => Ok(AttributeValue::Int(*i)),
            SerializedAttributeValue::Float(f) => Ok(AttributeValue::Float(*f)),
            SerializedAttributeValue::Bool(b) => Ok(AttributeValue::Bool(*b)),
            SerializedAttributeValue::List(list) => {
                let mut deserialized = Vec::with_capacity(list.len());
                for v in list {
                    deserialized.push(Self::deserialize_attribute_value(v, string_map)?);
                }
                Ok(AttributeValue::List(deserialized))
            }
            SerializedAttributeValue::Object(entries) => {
                let mut map = std::collections::HashMap::new();
                for (key_idx, v) in entries {
                    let key = *string_map.get(*key_idx as usize).ok_or_else(|| {
                        ReaperError::ParseError(format!("Invalid object key index: {}", key_idx))
                    })?;
                    let value = Self::deserialize_attribute_value(v, string_map)?;
                    map.insert(key, value);
                }
                Ok(AttributeValue::Object(map))
            }
            SerializedAttributeValue::Set(items) => {
                let mut set = rustc_hash::FxHashSet::default();
                for v in items {
                    set.insert(Self::deserialize_attribute_value(v, string_map)?);
                }
                Ok(AttributeValue::Set(set))
            }
            SerializedAttributeValue::Null => Ok(AttributeValue::Null),
        }
    }
}

// ============================================================================
// DataStore extension methods
// ============================================================================

impl DataStore {
    /// Create a data bundle from this store
    pub fn to_bundle(&self, name: String, version: String) -> DataBundle {
        DataBundle::from_store(self, name, version)
    }

    /// Create a DataStore from a data bundle
    pub fn from_bundle(bundle: &DataBundle) -> Result<Self, ReaperError> {
        bundle.load_into_store()
    }

    /// Replace all data with data from a bundle
    pub fn replace_with_bundle(&self, bundle: &DataBundle) -> Result<(), ReaperError> {
        bundle.replace_store(self)
    }

    /// Serialize this store to binary format
    pub fn to_bytes(&self, name: String, version: String) -> Result<Vec<u8>, ReaperError> {
        let bundle = self.to_bundle(name, version);
        bundle.to_bytes()
    }

    /// Create a DataStore from binary data
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        let bundle = DataBundle::from_bytes(bytes)?;
        bundle.load_into_store()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::entity::EntityBuilder;

    #[test]
    fn test_string_table() {
        let mut table = StringTable::new();

        let idx1 = table.add("hello");
        let idx2 = table.add("world");
        let idx3 = table.add("hello"); // Duplicate

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(idx3, 0); // Should return existing index
        assert_eq!(table.len(), 2);

        assert_eq!(table.get(0), Some("hello"));
        assert_eq!(table.get(1), Some("world"));
        assert_eq!(table.get(2), None);
    }

    #[test]
    fn test_bundle_roundtrip() {
        let store = DataStore::new();
        let interner = store.interner();

        // Create some entities
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");
        let viewer_value = interner.intern("viewer");

        let alice_id = interner.intern("alice");
        let bob_id = interner.intern("bob");

        store.insert(
            EntityBuilder::new(alice_id, user_type)
                .with_string(role_key, admin_value)
                .build(),
        );
        store.insert(
            EntityBuilder::new(bob_id, user_type)
                .with_string(role_key, viewer_value)
                .build(),
        );

        // Create bundle
        let bundle = store.to_bundle("test".to_string(), "1.0.0".to_string());
        assert_eq!(bundle.metadata.entity_count, 2);

        // Serialize to bytes
        let bytes = bundle.to_bytes().unwrap();
        assert!(!bytes.is_empty());

        // Deserialize
        let bundle2 = DataBundle::from_bytes(&bytes).unwrap();
        assert_eq!(bundle2.metadata.name, "test");
        assert_eq!(bundle2.metadata.version, "1.0.0");
        assert_eq!(bundle2.entities.len(), 2);

        // Load into new store
        let store2 = bundle2.load_into_store().unwrap();
        assert_eq!(store2.stats().total_entities, 2);

        // Verify entities
        let interner2 = store2.interner();
        let alice_id2 = interner2.intern("alice");
        let alice = store2.get(alice_id2).unwrap();
        assert_eq!(
            alice.get_attribute_str("role", interner2),
            Some("admin".into())
        );
    }

    #[test]
    fn test_bundle_with_various_attributes() {
        let store = DataStore::new();
        let interner = store.interner();

        let entity_type = interner.intern("TestEntity");
        let entity_id = interner.intern("test1");
        let str_key = interner.intern("str_attr");
        let str_val = interner.intern("string_value");
        let int_key = interner.intern("int_attr");
        let bool_key = interner.intern("bool_attr");
        let float_key = interner.intern("float_attr");

        let entity = EntityBuilder::new(entity_id, entity_type)
            .with_string(str_key, str_val)
            .with_int(int_key, 42)
            .with_bool(bool_key, true)
            .with_float(float_key, std::f64::consts::PI)
            .build();

        store.insert(entity);

        // Roundtrip
        let bytes = store
            .to_bytes("attr_test".to_string(), "1.0.0".to_string())
            .unwrap();
        let store2 = DataStore::from_bytes(&bytes).unwrap();

        let interner2 = store2.interner();
        let entity_id2 = interner2.intern("test1");
        let retrieved = store2.get(entity_id2).unwrap();

        // Verify string attribute
        let str_attr = retrieved.get_attribute(interner2.intern("str_attr"));
        assert!(matches!(str_attr, Some(AttributeValue::String(_))));

        // Verify int attribute
        let int_attr = retrieved.get_attribute(interner2.intern("int_attr"));
        assert_eq!(int_attr, Some(&AttributeValue::Int(42)));

        // Verify bool attribute
        let bool_attr = retrieved.get_attribute(interner2.intern("bool_attr"));
        assert_eq!(bool_attr, Some(&AttributeValue::Bool(true)));

        // Verify float attribute
        let float_attr = retrieved.get_attribute(interner2.intern("float_attr"));
        assert_eq!(
            float_attr,
            Some(&AttributeValue::Float(std::f64::consts::PI))
        );
    }

    #[test]
    fn test_replace_store() {
        let store = DataStore::new();
        let interner = store.interner();

        // Initial data
        let user_type = interner.intern("User");
        let alice_id = interner.intern("alice");
        store.insert(EntityBuilder::new(alice_id, user_type).build());
        assert_eq!(store.stats().total_entities, 1);

        // Create bundle with different data
        let bundle = DataBundle::new("replacement".to_string(), "2.0.0".to_string());

        // Replace store data
        bundle.replace_store(&store).unwrap();
        assert_eq!(store.stats().total_entities, 0);
    }

    #[test]
    fn test_invalid_bundle_magic() {
        let bytes = b"BADM\x01\x00\x00\x00"; // Wrong magic
        let result = DataBundle::from_bytes(bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_bundle_checksum_verification() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let alice_id = interner.intern("alice");
        store.insert(EntityBuilder::new(alice_id, user_type).build());

        let mut bytes = store
            .to_bytes("test".to_string(), "1.0.0".to_string())
            .unwrap();

        // Corrupt the data (modify a byte before the checksum)
        let corrupt_idx = bytes.len().saturating_sub(40);
        if corrupt_idx > 0 {
            bytes[corrupt_idx] ^= 0xFF;
        }

        // Should fail checksum verification
        let result = DataBundle::from_bytes(&bytes);
        assert!(result.is_err());
    }
}
