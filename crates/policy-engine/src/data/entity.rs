//! Entity Storage and Attribute Management
//!
//! Entities represent users, resources, groups, etc. with their attributes.
//! This module provides memory-efficient storage using interned strings.

use super::interning::{InternedString, StringInterner};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Entity ID (interned for efficiency)
pub type EntityId = InternedString;

/// Entity type (e.g., "User", "Resource", "Group")
pub type EntityType = InternedString;

/// Attribute value supporting multiple types
///
/// Uses Rust's enum for type-safe, memory-efficient storage.
/// Each variant is sized for the data it holds (no wasted space).
///
/// Supports Rego-like data types:
/// - Scalars: String, Int, Float, Bool, Null
/// - Collections: List (arrays), Object (maps), Set (unordered unique values)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttributeValue {
    /// String value (interned for efficiency)
    String(InternedString),
    /// Integer value (8 bytes)
    Int(i64),
    /// Float value (8 bytes)
    Float(f64),
    /// Boolean value (1 byte)
    Bool(bool),
    /// List/Array of values (ordered)
    List(Vec<AttributeValue>),
    /// Object/Map of key-value pairs (Rego-like objects)
    /// Keys are interned strings for efficiency
    Object(HashMap<InternedString, AttributeValue>),
    /// Set of unique values (unordered, Rego-like sets)
    /// Uses FxHashSet for faster hashing (~6% improvement over std HashSet)
    Set(FxHashSet<AttributeValue>),
    /// Null/None value
    Null,
}

// Custom Eq and Hash implementation for AttributeValue
// Floats are hashed using their bit representation
impl Eq for AttributeValue {}

impl std::hash::Hash for AttributeValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            AttributeValue::String(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            AttributeValue::Int(i) => {
                1u8.hash(state);
                i.hash(state);
            }
            AttributeValue::Float(f) => {
                2u8.hash(state);
                f.to_bits().hash(state); // Hash the bit representation
            }
            AttributeValue::Bool(b) => {
                3u8.hash(state);
                b.hash(state);
            }
            AttributeValue::List(l) => {
                4u8.hash(state);
                l.hash(state);
            }
            AttributeValue::Object(m) => {
                5u8.hash(state);
                // Sort keys for deterministic hashing
                let mut pairs: Vec<_> = m.iter().collect();
                pairs.sort_by_key(|(k, _)| **k);
                for (k, v) in pairs {
                    k.hash(state);
                    v.hash(state);
                }
            }
            AttributeValue::Set(s) => {
                6u8.hash(state);
                // Sort values for deterministic hashing
                let mut values: Vec<_> = s.iter().collect();
                values.sort_by(|a, b| {
                    // Custom comparison for AttributeValue
                    // Order: Null < Bool < Int < Float < String < List < Object < Set
                    use AttributeValue::*;
                    match (a, b) {
                        (Null, Null) => std::cmp::Ordering::Equal,
                        (Null, _) => std::cmp::Ordering::Less,
                        (_, Null) => std::cmp::Ordering::Greater,
                        (Bool(a), Bool(b)) => a.cmp(b),
                        (Bool(_), _) => std::cmp::Ordering::Less,
                        (_, Bool(_)) => std::cmp::Ordering::Greater,
                        (Int(a), Int(b)) => a.cmp(b),
                        (Int(_), _) => std::cmp::Ordering::Less,
                        (_, Int(_)) => std::cmp::Ordering::Greater,
                        (Float(a), Float(b)) => {
                            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (Float(_), _) => std::cmp::Ordering::Less,
                        (_, Float(_)) => std::cmp::Ordering::Greater,
                        (String(a), String(b)) => a.cmp(b),
                        (String(_), _) => std::cmp::Ordering::Less,
                        (_, String(_)) => std::cmp::Ordering::Greater,
                        (List(_), List(_)) => std::cmp::Ordering::Equal, // Simplified
                        (List(_), _) => std::cmp::Ordering::Less,
                        (_, List(_)) => std::cmp::Ordering::Greater,
                        (Object(_), Object(_)) => std::cmp::Ordering::Equal, // Simplified
                        (Object(_), _) => std::cmp::Ordering::Less,
                        (_, Object(_)) => std::cmp::Ordering::Greater,
                        (Set(_), Set(_)) => std::cmp::Ordering::Equal, // Simplified
                    }
                });
                for v in values {
                    v.hash(state);
                }
            }
            AttributeValue::Null => {
                7u8.hash(state);
            }
        }
    }
}

impl AttributeValue {
    /// Create a String attribute from a raw string (using interner)
    pub fn from_string(s: &str, interner: &StringInterner) -> Self {
        Self::String(interner.intern(s))
    }

    /// Get the value as a string (if it's a string)
    pub fn as_string(&self, interner: &StringInterner) -> Option<Arc<str>> {
        match self {
            AttributeValue::String(id) => interner.resolve(*id),
            _ => None,
        }
    }

    /// Get the value as an integer (if it's an integer)
    pub fn as_int(&self) -> Option<i64> {
        match self {
            AttributeValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Get the value as a float (if it's a float)
    pub fn as_float(&self) -> Option<f64> {
        match self {
            AttributeValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Get the value as a boolean (if it's a boolean)
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AttributeValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get the value as a list (if it's a list)
    pub fn as_list(&self) -> Option<&Vec<AttributeValue>> {
        match self {
            AttributeValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// Get the value as an object (if it's an object)
    pub fn as_object(&self) -> Option<&HashMap<InternedString, AttributeValue>> {
        match self {
            AttributeValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Get the value as a set (if it's a set)
    pub fn as_set(&self) -> Option<&FxHashSet<AttributeValue>> {
        match self {
            AttributeValue::Set(s) => Some(s),
            _ => None,
        }
    }

    /// Check if the value is null
    pub fn is_null(&self) -> bool {
        matches!(self, AttributeValue::Null)
    }

    /// Check if the value is an array/list
    pub fn is_list(&self) -> bool {
        matches!(self, AttributeValue::List(_))
    }

    /// Check if the value is an object/map
    pub fn is_object(&self) -> bool {
        matches!(self, AttributeValue::Object(_))
    }

    /// Check if the value is a set
    pub fn is_set(&self) -> bool {
        matches!(self, AttributeValue::Set(_))
    }

    /// Estimate memory usage of this value
    pub fn memory_size(&self) -> usize {
        match self {
            AttributeValue::String(_) => 8, // Just the ID
            AttributeValue::Int(_) => 8,
            AttributeValue::Float(_) => 8,
            AttributeValue::Bool(_) => 1,
            AttributeValue::List(items) => {
                16 + items.iter().map(|v| v.memory_size()).sum::<usize>()
            }
            AttributeValue::Object(map) => {
                48 + map.values().map(|v| 8 + v.memory_size()).sum::<usize>()
            }
            AttributeValue::Set(set) => 48 + set.iter().map(|v| v.memory_size()).sum::<usize>(),
            AttributeValue::Null => 0,
        }
    }
}

/// Entity attributes (key-value pairs)
///
/// Keys are interned strings for memory efficiency.
/// Values use the AttributeValue enum for type safety.
pub type Attributes = HashMap<InternedString, AttributeValue>;

/// An entity with ID, type, and attributes
///
/// # Memory Layout
/// - EntityId: 4 bytes (interned)
/// - EntityType: 4 bytes (interned)
/// - Attributes: ~16 bytes (HashMap overhead) + attribute data
///
/// Compare to OPA (Go):
/// - ID: 24 bytes (string)
/// - Type: 24 bytes (string)
/// - Attributes: ~48 bytes (map overhead) + attribute data
///
/// Reaper uses ~60% less memory for the entity structure alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique entity identifier (interned)
    pub id: EntityId,
    /// Entity type (e.g., User, Resource, Group) (interned)
    pub entity_type: EntityType,
    /// Entity attributes
    pub attributes: Attributes,
    /// Optional parent entity (for hierarchies)
    pub parent: Option<EntityId>,
}

impl Entity {
    /// Create a new entity
    pub fn new(id: EntityId, entity_type: EntityType, attributes: Attributes) -> Self {
        Self {
            id,
            entity_type,
            attributes,
            parent: None,
        }
    }

    /// Create a new entity with a parent
    pub fn new_with_parent(
        id: EntityId,
        entity_type: EntityType,
        attributes: Attributes,
        parent: EntityId,
    ) -> Self {
        Self {
            id,
            entity_type,
            attributes,
            parent: Some(parent),
        }
    }

    /// Get an attribute value
    pub fn get_attribute(&self, key: InternedString) -> Option<&AttributeValue> {
        self.attributes.get(&key)
    }

    /// Get an attribute value as a string
    pub fn get_string_attribute(
        &self,
        key: InternedString,
        interner: &StringInterner,
    ) -> Option<Arc<str>> {
        self.get_attribute(key)?.as_string(interner)
    }

    /// Get an attribute value as an integer
    pub fn get_int_attribute(&self, key: InternedString) -> Option<i64> {
        self.get_attribute(key)?.as_int()
    }

    /// Get an attribute value as a boolean
    pub fn get_bool_attribute(&self, key: InternedString) -> Option<bool> {
        self.get_attribute(key)?.as_bool()
    }

    /// Check if entity has a specific attribute
    pub fn has_attribute(&self, key: InternedString) -> bool {
        self.attributes.contains_key(&key)
    }

    /// Get an attribute value as a string by key name
    ///
    /// This is a convenience method that looks up the key in the interner
    /// and returns the attribute value as a String.
    pub fn get_attribute_str(&self, key: &str, interner: &StringInterner) -> Option<String> {
        let key_interned = interner.intern(key);
        self.get_string_attribute(key_interned, interner)
            .map(|s| s.to_string())
    }

    /// Estimate memory usage of this entity
    pub fn memory_size(&self) -> usize {
        // Entity ID + Type + HashMap overhead + attributes
        8 + 48
            + self
                .attributes
                .values()
                .map(|v| 8 + v.memory_size())
                .sum::<usize>()
    }
}

/// Builder for creating entities with a fluent API
pub struct EntityBuilder {
    id: EntityId,
    entity_type: EntityType,
    attributes: Attributes,
    parent: Option<EntityId>,
}

impl EntityBuilder {
    /// Create a new entity builder
    pub fn new(id: EntityId, entity_type: EntityType) -> Self {
        Self {
            id,
            entity_type,
            attributes: HashMap::new(),
            parent: None,
        }
    }

    /// Add a string attribute
    pub fn with_string(mut self, key: InternedString, value: InternedString) -> Self {
        self.attributes.insert(key, AttributeValue::String(value));
        self
    }

    /// Add an integer attribute
    pub fn with_int(mut self, key: InternedString, value: i64) -> Self {
        self.attributes.insert(key, AttributeValue::Int(value));
        self
    }

    /// Add a float attribute
    pub fn with_float(mut self, key: InternedString, value: f64) -> Self {
        self.attributes.insert(key, AttributeValue::Float(value));
        self
    }

    /// Add a boolean attribute
    pub fn with_bool(mut self, key: InternedString, value: bool) -> Self {
        self.attributes.insert(key, AttributeValue::Bool(value));
        self
    }

    /// Add any attribute value
    pub fn with_attribute(mut self, key: InternedString, value: AttributeValue) -> Self {
        self.attributes.insert(key, value);
        self
    }

    /// Set the parent entity
    pub fn with_parent(mut self, parent: EntityId) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Build the entity
    pub fn build(self) -> Entity {
        Entity {
            id: self.id,
            entity_type: self.entity_type,
            attributes: self.attributes,
            parent: self.parent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribute_value_memory_efficiency() {
        // String (via interned ID): 8 bytes
        let string_val = AttributeValue::String(InternedString::from_id(42));
        assert_eq!(string_val.memory_size(), 8);

        // Int: 8 bytes
        let int_val = AttributeValue::Int(123);
        assert_eq!(int_val.memory_size(), 8);

        // Bool: 1 byte
        let bool_val = AttributeValue::Bool(true);
        assert_eq!(bool_val.memory_size(), 1);
    }

    #[test]
    fn test_entity_builder() {
        let interner = StringInterner::new();

        let user_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");
        let age_key = interner.intern("age");

        let entity = EntityBuilder::new(user_id, user_type)
            .with_string(role_key, admin_value)
            .with_int(age_key, 30)
            .build();

        assert_eq!(entity.id, user_id);
        assert_eq!(entity.entity_type, user_type);
        assert_eq!(entity.attributes.len(), 2);
    }

    #[test]
    fn test_entity_attribute_access() {
        let interner = StringInterner::new();

        let user_id = interner.intern("bob");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let manager_value = interner.intern("manager");

        let entity = EntityBuilder::new(user_id, user_type)
            .with_string(role_key, manager_value)
            .build();

        // Get attribute as string
        let role = entity.get_string_attribute(role_key, &interner).unwrap();
        assert_eq!(role.as_ref(), "manager");
    }

    #[test]
    fn test_entity_hierarchy() {
        let interner = StringInterner::new();

        let parent_id = interner.intern("team-eng");
        let child_id = interner.intern("alice");
        let team_type = interner.intern("Team");
        let user_type = interner.intern("User");

        let _parent = EntityBuilder::new(parent_id, team_type).build();
        let child = EntityBuilder::new(child_id, user_type)
            .with_parent(parent_id)
            .build();

        assert_eq!(child.parent, Some(parent_id));
    }

    #[test]
    fn test_attribute_value_list() {
        let list = AttributeValue::List(vec![
            AttributeValue::Int(1),
            AttributeValue::Int(2),
            AttributeValue::Int(3),
        ]);

        assert!(list.is_list());
        assert_eq!(list.as_list().unwrap().len(), 3);
        assert_eq!(list.as_list().unwrap()[0], AttributeValue::Int(1));
    }

    #[test]
    fn test_attribute_value_object() {
        let interner = StringInterner::new();
        let name_key = interner.intern("name");
        let age_key = interner.intern("age");

        let mut map = HashMap::new();
        map.insert(name_key, AttributeValue::String(interner.intern("alice")));
        map.insert(age_key, AttributeValue::Int(30));

        let obj = AttributeValue::Object(map);

        assert!(obj.is_object());
        assert_eq!(obj.as_object().unwrap().len(), 2);
        assert_eq!(
            obj.as_object().unwrap().get(&name_key),
            Some(&AttributeValue::String(interner.intern("alice")))
        );
    }

    #[test]
    fn test_attribute_value_set() {
        let interner = StringInterner::new();
        let mut set = FxHashSet::default();
        set.insert(AttributeValue::String(interner.intern("admin")));
        set.insert(AttributeValue::String(interner.intern("user")));
        set.insert(AttributeValue::String(interner.intern("moderator")));

        let set_val = AttributeValue::Set(set);

        assert!(set_val.is_set());
        assert_eq!(set_val.as_set().unwrap().len(), 3);
        assert!(set_val
            .as_set()
            .unwrap()
            .contains(&AttributeValue::String(interner.intern("admin"))));
    }

    #[test]
    fn test_attribute_value_nested_structures() {
        let interner = StringInterner::new();

        // Create nested structure: object containing list of objects
        let role_key = interner.intern("role");
        let perms_key = interner.intern("permissions");
        let name_key = interner.intern("name");

        let mut inner_obj1 = HashMap::new();
        inner_obj1.insert(name_key, AttributeValue::String(interner.intern("read")));

        let mut inner_obj2 = HashMap::new();
        inner_obj2.insert(name_key, AttributeValue::String(interner.intern("write")));

        let perm_list = AttributeValue::List(vec![
            AttributeValue::Object(inner_obj1),
            AttributeValue::Object(inner_obj2),
        ]);

        let mut outer_obj = HashMap::new();
        outer_obj.insert(role_key, AttributeValue::String(interner.intern("admin")));
        outer_obj.insert(perms_key, perm_list);

        let nested = AttributeValue::Object(outer_obj);

        assert!(nested.is_object());
        let obj = nested.as_object().unwrap();
        assert_eq!(obj.len(), 2);

        let perms = obj.get(&perms_key).unwrap();
        assert!(perms.is_list());
        assert_eq!(perms.as_list().unwrap().len(), 2);
    }

    #[test]
    fn test_attribute_value_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let _interner = StringInterner::new();

        // Test that identical sets hash the same regardless of insertion order
        let mut set1 = FxHashSet::default();
        set1.insert(AttributeValue::Int(1));
        set1.insert(AttributeValue::Int(2));
        set1.insert(AttributeValue::Int(3));

        let mut set2 = FxHashSet::default();
        set2.insert(AttributeValue::Int(3));
        set2.insert(AttributeValue::Int(1));
        set2.insert(AttributeValue::Int(2));

        let val1 = AttributeValue::Set(set1);
        let val2 = AttributeValue::Set(set2);

        let mut hasher1 = DefaultHasher::new();
        val1.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        val2.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        assert_eq!(hash1, hash2, "Identical sets should hash the same");
    }

    #[test]
    fn test_attribute_value_memory_size() {
        let interner = StringInterner::new();

        // Object memory size
        let mut map = HashMap::new();
        map.insert(interner.intern("key1"), AttributeValue::Int(100));
        map.insert(interner.intern("key2"), AttributeValue::Int(200));
        let obj = AttributeValue::Object(map);
        assert!(obj.memory_size() > 48); // HashMap overhead + data

        // Set memory size
        let mut set = FxHashSet::default();
        set.insert(AttributeValue::Int(1));
        set.insert(AttributeValue::Int(2));
        let set_val = AttributeValue::Set(set);
        assert!(set_val.memory_size() > 48); // FxHashSet overhead + data
    }
}
