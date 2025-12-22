//! Custom data source support
//!
//! Allows users to define custom entity types with enforced schemas for eBPF loading.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Custom data source definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomDataSource {
    /// Unique name for this data source (e.g., "organizations", "projects")
    pub name: String,

    /// Schema definition for validation
    pub schema: CustomSchema,

    /// Optional description
    #[serde(default)]
    pub description: String,

    /// Optional metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Schema definition for custom data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomSchema {
    /// Required string attributes (must be present)
    #[serde(default)]
    pub required_string_attrs: Vec<String>,

    /// Optional string attributes (may be present)
    #[serde(default)]
    pub optional_string_attrs: Vec<String>,

    /// Required numeric attributes (must be present)
    #[serde(default)]
    pub required_numeric_attrs: Vec<String>,

    /// Optional numeric attributes (may be present)
    #[serde(default)]
    pub optional_numeric_attrs: Vec<String>,

    /// Required relationships (must be present)
    #[serde(default)]
    pub required_relationships: Vec<RelationshipSchema>,

    /// Optional relationships (may be present)
    #[serde(default)]
    pub optional_relationships: Vec<RelationshipSchema>,

    /// Required boolean flags (must be present)
    #[serde(default)]
    pub required_flags: Vec<String>,

    /// Optional boolean flags (may be present)
    #[serde(default)]
    pub optional_flags: Vec<String>,

    /// Maximum entities expected (for capacity planning)
    #[serde(default)]
    pub max_entities: Option<usize>,
}

/// Relationship schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipSchema {
    /// Relationship type name
    pub rel_type: String,

    /// Expected target entity type (optional constraint)
    #[serde(default)]
    pub target_type: Option<String>,

    /// Description of this relationship
    #[serde(default)]
    pub description: String,
}

impl CustomDataSource {
    /// Create a new custom data source
    pub fn new(name: String, schema: CustomSchema) -> Self {
        Self {
            name,
            schema,
            description: String::new(),
            metadata: HashMap::new(),
        }
    }

    /// Validate that the total schema fits within eBPF constraints
    pub fn validate_schema(&self) -> Result<(), String> {
        let total_string =
            self.schema.required_string_attrs.len() + self.schema.optional_string_attrs.len();
        let total_numeric =
            self.schema.required_numeric_attrs.len() + self.schema.optional_numeric_attrs.len();
        let total_relationships =
            self.schema.required_relationships.len() + self.schema.optional_relationships.len();
        let total_flags = self.schema.required_flags.len() + self.schema.optional_flags.len();

        if total_string > 8 {
            return Err(format!(
                "Custom schema '{}' has {} string attributes (max 8)",
                self.name, total_string
            ));
        }

        if total_numeric > 8 {
            return Err(format!(
                "Custom schema '{}' has {} numeric attributes (max 8)",
                self.name, total_numeric
            ));
        }

        if total_relationships > 8 {
            return Err(format!(
                "Custom schema '{}' has {} relationships (max 8)",
                self.name, total_relationships
            ));
        }

        if total_flags > 64 {
            return Err(format!(
                "Custom schema '{}' has {} flags (max 64)",
                self.name, total_flags
            ));
        }

        Ok(())
    }

    /// Check if an attribute name is valid according to schema
    pub fn is_valid_string_attr(&self, attr: &str) -> bool {
        self.schema
            .required_string_attrs
            .contains(&attr.to_string())
            || self
                .schema
                .optional_string_attrs
                .contains(&attr.to_string())
    }

    /// Check if a numeric attribute is valid according to schema
    pub fn is_valid_numeric_attr(&self, attr: &str) -> bool {
        self.schema
            .required_numeric_attrs
            .contains(&attr.to_string())
            || self
                .schema
                .optional_numeric_attrs
                .contains(&attr.to_string())
    }

    /// Check if a relationship type is valid according to schema
    pub fn is_valid_relationship(&self, rel_type: &str) -> bool {
        self.schema
            .required_relationships
            .iter()
            .any(|r| r.rel_type == rel_type)
            || self
                .schema
                .optional_relationships
                .iter()
                .any(|r| r.rel_type == rel_type)
    }

    /// Check if a flag is valid according to schema
    pub fn is_valid_flag(&self, flag: &str) -> bool {
        self.schema.required_flags.contains(&flag.to_string())
            || self.schema.optional_flags.contains(&flag.to_string())
    }

    /// Validate an entity against this schema
    pub fn validate_entity(&self, entity_data: &crate::entity::EntityData) -> Vec<String> {
        let mut errors = Vec::new();

        // Check required string attributes
        for required in &self.schema.required_string_attrs {
            if !entity_data.string_attrs.contains_key(required) {
                errors.push(format!("Missing required string attribute '{}'", required));
            }
        }

        // Check required numeric attributes
        for required in &self.schema.required_numeric_attrs {
            if !entity_data.numeric_attrs.contains_key(required) {
                errors.push(format!("Missing required numeric attribute '{}'", required));
            }
        }

        // Check required relationships
        for required in &self.schema.required_relationships {
            let has_rel = entity_data
                .relationships
                .iter()
                .any(|r| r.rel_type == required.rel_type);
            if !has_rel {
                errors.push(format!(
                    "Missing required relationship '{}'",
                    required.rel_type
                ));
            }
        }

        // Check required flags
        for required in &self.schema.required_flags {
            if !entity_data.flags.contains_key(required) {
                errors.push(format!("Missing required flag '{}'", required));
            }
        }

        // Validate all string attributes are in schema
        for attr in entity_data.string_attrs.keys() {
            if !self.is_valid_string_attr(attr) {
                errors.push(format!("Unknown string attribute '{}' not in schema", attr));
            }
        }

        // Validate all numeric attributes are in schema
        for attr in entity_data.numeric_attrs.keys() {
            if !self.is_valid_numeric_attr(attr) {
                errors.push(format!(
                    "Unknown numeric attribute '{}' not in schema",
                    attr
                ));
            }
        }

        // Validate all relationships are in schema
        for rel in &entity_data.relationships {
            if !self.is_valid_relationship(&rel.rel_type) {
                errors.push(format!(
                    "Unknown relationship type '{}' not in schema",
                    rel.rel_type
                ));
            }
        }

        // Validate all flags are in schema
        for flag in entity_data.flags.keys() {
            if !self.is_valid_flag(flag) {
                errors.push(format!("Unknown flag '{}' not in schema", flag));
            }
        }

        errors
    }
}

/// Registry for managing custom data sources
#[derive(Debug, Clone, Default)]
pub struct CustomDataRegistry {
    sources: HashMap<String, CustomDataSource>,
}

impl CustomDataRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    /// Register a custom data source
    pub fn register(&mut self, source: CustomDataSource) -> Result<(), String> {
        // Validate schema first
        source.validate_schema()?;

        if self.sources.contains_key(&source.name) {
            return Err(format!(
                "Custom data source '{}' already registered",
                source.name
            ));
        }

        self.sources.insert(source.name.clone(), source);
        Ok(())
    }

    /// Get a custom data source by name
    pub fn get(&self, name: &str) -> Option<&CustomDataSource> {
        self.sources.get(name)
    }

    /// Remove a custom data source
    pub fn unregister(&mut self, name: &str) -> Option<CustomDataSource> {
        self.sources.remove(name)
    }

    /// List all registered custom data sources
    pub fn list(&self) -> Vec<&str> {
        self.sources.keys().map(|s| s.as_str()).collect()
    }

    /// Validate an entity against a registered schema
    pub fn validate_entity(
        &self,
        source_name: &str,
        entity_data: &crate::entity::EntityData,
    ) -> Result<(), Vec<String>> {
        let source = self.get(source_name).ok_or_else(|| {
            vec![format!(
                "Custom data source '{}' not registered",
                source_name
            )]
        })?;

        let errors = source.validate_entity(entity_data);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityData, RelationshipData};

    #[test]
    fn test_custom_schema_validation() {
        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec!["description".to_string()],
            required_numeric_attrs: vec!["id".to_string()],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: Some(1000),
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);
        assert!(source.validate_schema().is_ok());
    }

    #[test]
    fn test_schema_exceeds_limits() {
        let schema = CustomSchema {
            required_string_attrs: (0..9).map(|i| format!("attr{}", i)).collect(),
            optional_string_attrs: vec![],
            required_numeric_attrs: vec![],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("test".to_string(), schema);
        assert!(source.validate_schema().is_err());
    }

    #[test]
    fn test_entity_validation_success() {
        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec!["description".to_string()],
            required_numeric_attrs: vec!["id".to_string()],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);

        let mut entity_data = EntityData {
            entity_type: "organizations".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: vec![],
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        entity_data
            .string_attrs
            .insert("name".to_string(), "Acme Corp".to_string());
        entity_data.numeric_attrs.insert("id".to_string(), 123);

        let errors = source.validate_entity(&entity_data);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_entity_validation_missing_required() {
        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec![],
            required_numeric_attrs: vec!["id".to_string()],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);

        let entity_data = EntityData {
            entity_type: "organizations".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: vec![],
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        let errors = source.validate_entity(&entity_data);
        assert_eq!(errors.len(), 2); // Missing name and id
        assert!(errors.iter().any(|e| e.contains("name")));
        assert!(errors.iter().any(|e| e.contains("id")));
    }

    #[test]
    fn test_entity_validation_unknown_attribute() {
        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec![],
            required_numeric_attrs: vec![],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);

        let mut entity_data = EntityData {
            entity_type: "organizations".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: vec![],
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        entity_data
            .string_attrs
            .insert("name".to_string(), "Acme".to_string());
        entity_data
            .string_attrs
            .insert("unknown".to_string(), "value".to_string());

        let errors = source.validate_entity(&entity_data);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown"));
    }

    #[test]
    fn test_registry_operations() {
        let mut registry = CustomDataRegistry::new();

        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec![],
            required_numeric_attrs: vec![],
            optional_numeric_attrs: vec![],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);
        assert!(registry.register(source).is_ok());

        // Check it's registered
        assert!(registry.get("organizations").is_some());
        assert_eq!(registry.list().len(), 1);

        // Try to register duplicate
        let schema2 = CustomSchema::default();
        let source2 = CustomDataSource::new("organizations".to_string(), schema2);
        assert!(registry.register(source2).is_err());

        // Unregister
        assert!(registry.unregister("organizations").is_some());
        assert!(registry.get("organizations").is_none());
    }

    #[test]
    fn test_relationship_schema() {
        let schema = CustomSchema {
            required_string_attrs: vec![],
            optional_string_attrs: vec![],
            required_numeric_attrs: vec![],
            optional_numeric_attrs: vec![],
            required_relationships: vec![RelationshipSchema {
                rel_type: "belongs_to".to_string(),
                target_type: Some("department".to_string()),
                description: "Organization belongs to department".to_string(),
            }],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec![],
            max_entities: None,
        };

        let source = CustomDataSource::new("organizations".to_string(), schema);

        let mut entity_data = EntityData {
            entity_type: "organizations".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: vec![RelationshipData {
                rel_type: "belongs_to".to_string(),
                target: "dept:engineering".to_string(),
            }],
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        let errors = source.validate_entity(&entity_data);
        assert!(errors.is_empty());

        // Remove required relationship
        entity_data.relationships.clear();
        let errors = source.validate_entity(&entity_data);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("belongs_to"));
    }
}
