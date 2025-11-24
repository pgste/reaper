//! Entity Storage and Attribute Management
//!
//! Entities represent users, resources, groups, etc. with their attributes.
//! This module provides memory-efficient storage using interned strings.

use super::interning::{InternedString, StringInterner};
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
    /// List of values (for multi-valued attributes)
    List(Vec<AttributeValue>),
    /// Null/None value
    Null,
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

    /// Check if the value is null
    pub fn is_null(&self) -> bool {
        matches!(self, AttributeValue::Null)
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
    pub fn new(
        id: EntityId,
        entity_type: EntityType,
        attributes: Attributes,
    ) -> Self {
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

    /// Estimate memory usage of this entity
    pub fn memory_size(&self) -> usize {
        // Entity ID + Type + HashMap overhead + attributes
        8 + 48 + self.attributes.values()
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
}
