//! Entity validation module
//!
//! Validates entity datasets against eBPF schema constraints before loading into kernel maps.
//! Ensures data fits within eBPF stack and map size limitations.

use crate::custom::CustomDataRegistry;
use crate::entity::{
    DataTier, EntityData, EntityDataset, EntityType, ValidationResult, MAX_ATTR_KEY_LEN,
    MAX_ENTITY_ID_LEN, MAX_NUMERIC_ATTRS, MAX_RELATIONSHIPS, MAX_STRING_ATTRS,
    MAX_STRING_VALUE_LEN,
};
use std::collections::HashSet;

/// Entity validator for eBPF compatibility checking
pub struct EntityValidator {
    /// Whether to perform strict validation (fail on warnings)
    strict: bool,
    /// Maximum entities per tier (overrides for testing)
    tier1_max: usize,
    tier2_max: usize,
    /// Optional custom data source registry
    custom_registry: Option<CustomDataRegistry>,
}

impl EntityValidator {
    /// Create a new validator with default settings
    pub fn new() -> Self {
        Self {
            strict: false,
            tier1_max: 10_000,
            tier2_max: 100_000,
            custom_registry: None,
        }
    }

    /// Enable strict validation mode (warnings become errors)
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Set custom tier thresholds (for testing)
    pub fn with_tier_limits(mut self, tier1_max: usize, tier2_max: usize) -> Self {
        self.tier1_max = tier1_max;
        self.tier2_max = tier2_max;
        self
    }

    /// Add custom data source registry for schema validation
    pub fn with_custom_registry(mut self, registry: CustomDataRegistry) -> Self {
        self.custom_registry = Some(registry);
        self
    }

    /// Validate a complete entity dataset
    pub fn validate(&self, dataset: &EntityDataset) -> ValidationResult {
        let mut result = ValidationResult::new();

        // Validate dataset metadata
        if dataset.dataset.is_empty() {
            result.add_error("Dataset name cannot be empty".to_string());
        }

        if dataset.version.is_empty() {
            result.add_warning("Dataset version not specified".to_string());
        }

        // Count entities
        result.entity_count = dataset.entities.len();

        // Determine tier based on entity count
        result.tier = if result.entity_count <= self.tier1_max {
            DataTier::Tier1Direct
        } else if result.entity_count <= self.tier2_max {
            DataTier::Tier2Sharded
        } else {
            DataTier::Tier3Partitioned
        };

        // Collect all entity IDs for relationship validation
        let entity_ids: HashSet<&String> = dataset.entities.keys().collect();

        // Validate each entity
        for (entity_id, entity_data) in &dataset.entities {
            self.validate_entity(entity_id, entity_data, &entity_ids, &mut result);
        }

        // Estimate memory usage
        result.estimated_memory = self.estimate_memory(&result);

        // Convert warnings to errors in strict mode
        if self.strict && !result.warnings.is_empty() {
            result.errors.extend(result.warnings.clone());
            result.warnings.clear();
            result.valid = false;
        }

        result
    }

    /// Validate a single entity
    fn validate_entity(
        &self,
        entity_id: &str,
        entity: &EntityData,
        all_entity_ids: &HashSet<&String>,
        result: &mut ValidationResult,
    ) {
        // Validate entity ID length
        if entity_id.len() > MAX_ENTITY_ID_LEN {
            result.add_error(format!(
                "Entity ID '{}' exceeds max length {} (got {})",
                entity_id,
                MAX_ENTITY_ID_LEN,
                entity_id.len()
            ));
        }

        if entity_id.is_empty() {
            result.add_error("Entity ID cannot be empty".to_string());
        }

        // Validate entity type
        if let Some(parsed_type) = EntityType::parse(&entity.entity_type) {
            // Count entity types
            *result
                .entity_types
                .entry(entity.entity_type.clone())
                .or_insert(0) += 1;

            // Type-specific validation
            match parsed_type {
                EntityType::JwtSession => {
                    self.validate_jwt_entity(entity_id, entity, result);
                }
                EntityType::User | EntityType::Role | EntityType::Group => {
                    self.validate_rbac_entity(entity_id, entity, result);
                }
                _ => {}
            }
        } else {
            // Not a built-in type - check if it's a registered custom type
            if let Some(ref registry) = self.custom_registry {
                if let Some(custom_source) = registry.get(&entity.entity_type) {
                    // Validate against custom schema
                    let custom_errors = custom_source.validate_entity(entity);
                    for error in custom_errors {
                        result.add_error(format!("Entity '{}': {}", entity_id, error));
                    }
                    *result
                        .entity_types
                        .entry(entity.entity_type.clone())
                        .or_insert(0) += 1;
                } else {
                    result.add_error(format!(
                        "Entity '{}' has unregistered custom type '{}'",
                        entity_id, entity.entity_type
                    ));
                }
            } else {
                result.add_error(format!(
                    "Entity '{}' has invalid type '{}' (no custom registry provided)",
                    entity_id, entity.entity_type
                ));
            }
        }

        // Validate string attributes
        if entity.string_attrs.len() > MAX_STRING_ATTRS {
            result.add_error(format!(
                "Entity '{}' has {} string attributes (max {})",
                entity_id,
                entity.string_attrs.len(),
                MAX_STRING_ATTRS
            ));
        }

        for (key, value) in &entity.string_attrs {
            if key.len() > MAX_ATTR_KEY_LEN {
                result.add_error(format!(
                    "Entity '{}' string attribute key '{}' exceeds max length {} (got {})",
                    entity_id,
                    key,
                    MAX_ATTR_KEY_LEN,
                    key.len()
                ));
            }

            if value.len() > MAX_STRING_VALUE_LEN {
                result.add_error(format!(
                    "Entity '{}' string attribute '{}' value exceeds max length {} (got {})",
                    entity_id,
                    key,
                    MAX_STRING_VALUE_LEN,
                    value.len()
                ));
            }

            if key.is_empty() {
                result.add_error(format!("Entity '{}' has empty attribute key", entity_id));
            }
        }

        // Validate numeric attributes
        if entity.numeric_attrs.len() > MAX_NUMERIC_ATTRS {
            result.add_error(format!(
                "Entity '{}' has {} numeric attributes (max {})",
                entity_id,
                entity.numeric_attrs.len(),
                MAX_NUMERIC_ATTRS
            ));
        }

        for key in entity.numeric_attrs.keys() {
            if key.len() > MAX_ATTR_KEY_LEN {
                result.add_error(format!(
                    "Entity '{}' numeric attribute key '{}' exceeds max length {} (got {})",
                    entity_id,
                    key,
                    MAX_ATTR_KEY_LEN,
                    key.len()
                ));
            }

            if key.is_empty() {
                result.add_error(format!(
                    "Entity '{}' has empty numeric attribute key",
                    entity_id
                ));
            }
        }

        // Validate relationships
        if entity.relationships.len() > MAX_RELATIONSHIPS {
            result.add_error(format!(
                "Entity '{}' has {} relationships (max {})",
                entity_id,
                entity.relationships.len(),
                MAX_RELATIONSHIPS
            ));
        }

        for rel in &entity.relationships {
            if rel.rel_type.len() > MAX_ATTR_KEY_LEN {
                result.add_error(format!(
                    "Entity '{}' relationship type '{}' exceeds max length {} (got {})",
                    entity_id,
                    rel.rel_type,
                    MAX_ATTR_KEY_LEN,
                    rel.rel_type.len()
                ));
            }

            if rel.target.len() > MAX_ENTITY_ID_LEN {
                result.add_error(format!(
                    "Entity '{}' relationship target '{}' exceeds max length {} (got {})",
                    entity_id,
                    rel.target,
                    MAX_ENTITY_ID_LEN,
                    rel.target.len()
                ));
            }

            // Check if target exists
            if !all_entity_ids.contains(&rel.target) {
                result.add_warning(format!(
                    "Entity '{}' relationship points to non-existent target '{}'",
                    entity_id, rel.target
                ));
            }

            if rel.rel_type.is_empty() {
                result.add_error(format!(
                    "Entity '{}' has relationship with empty type",
                    entity_id
                ));
            }

            if rel.target.is_empty() {
                result.add_error(format!(
                    "Entity '{}' has relationship with empty target",
                    entity_id
                ));
            }
        }

        // Validate flags (max 64 boolean flags)
        if entity.flags.len() > 64 {
            result.add_error(format!(
                "Entity '{}' has {} flags (max 64)",
                entity_id,
                entity.flags.len()
            ));
        }

        for key in entity.flags.keys() {
            if key.len() > MAX_ATTR_KEY_LEN {
                result.add_error(format!(
                    "Entity '{}' flag key '{}' exceeds max length {} (got {})",
                    entity_id,
                    key,
                    MAX_ATTR_KEY_LEN,
                    key.len()
                ));
            }
        }
    }

    /// JWT-specific validation
    fn validate_jwt_entity(
        &self,
        entity_id: &str,
        entity: &EntityData,
        result: &mut ValidationResult,
    ) {
        // Check for required JWT claims
        let required_claims = ["sub", "exp"];
        for claim in &required_claims {
            let has_string = entity.string_attrs.contains_key(*claim);
            let has_numeric = entity.numeric_attrs.contains_key(*claim);

            if !has_string && !has_numeric {
                result.add_warning(format!(
                    "JWT entity '{}' missing recommended claim '{}'",
                    entity_id, claim
                ));
            }
        }

        // Validate exp claim is numeric
        if entity.string_attrs.contains_key("exp") {
            result.add_warning(format!(
                "JWT entity '{}' has 'exp' claim as string (should be numeric timestamp)",
                entity_id
            ));
        }

        // Validate iat claim is numeric
        if entity.string_attrs.contains_key("iat") {
            result.add_warning(format!(
                "JWT entity '{}' has 'iat' claim as string (should be numeric timestamp)",
                entity_id
            ));
        }

        // Validate nbf claim is numeric
        if entity.string_attrs.contains_key("nbf") {
            result.add_warning(format!(
                "JWT entity '{}' has 'nbf' claim as string (should be numeric timestamp)",
                entity_id
            ));
        }
    }

    /// RBAC-specific validation
    fn validate_rbac_entity(
        &self,
        entity_id: &str,
        entity: &EntityData,
        result: &mut ValidationResult,
    ) {
        // For users, check if they have role relationships
        if entity.entity_type == "user" && entity.relationships.is_empty() {
            result.add_warning(format!(
                "User entity '{}' has no role relationships (RBAC may not work)",
                entity_id
            ));
        }

        // For roles, check if they have permission relationships
        if entity.entity_type == "role" && entity.relationships.is_empty() {
            result.add_warning(format!(
                "Role entity '{}' has no permission relationships (RBAC may not work)",
                entity_id
            ));
        }
    }

    /// Estimate memory usage for the dataset
    fn estimate_memory(&self, result: &ValidationResult) -> usize {
        // Base entity size: ~2KB per entity (from Entity struct)
        const BASE_ENTITY_SIZE: usize = 2048;

        // Additional overhead for maps and metadata
        match result.tier {
            DataTier::Tier1Direct => {
                // Single map per entity type (5 maps)
                result.entity_count * BASE_ENTITY_SIZE
            }
            DataTier::Tier2Sharded => {
                // 16 shards per entity type (5 types * 16 shards)
                // Plus shard routing overhead
                result.entity_count * BASE_ENTITY_SIZE + (5 * 16 * 1024)
            }
            DataTier::Tier3Partitioned => {
                // Bloom filter + LRU cache overhead
                // Bloom: 10 bits per entity
                // LRU: 10K entries cached
                let bloom_size = (result.entity_count * 10) / 8; // bits to bytes
                let lru_size = 10_000 * BASE_ENTITY_SIZE;
                bloom_size + lru_size
            }
        }
    }
}

impl Default for EntityValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::RelationshipData;
    use std::collections::HashMap;

    #[test]
    fn test_valid_small_dataset() {
        let mut entities = HashMap::new();
        entities.insert(
            "user:alice".to_string(),
            EntityData {
                entity_type: "user".to_string(),
                string_attrs: {
                    let mut attrs = HashMap::new();
                    attrs.insert("email".to_string(), "alice@example.com".to_string());
                    attrs
                },
                numeric_attrs: HashMap::new(),
                relationships: vec![RelationshipData {
                    rel_type: "has_role".to_string(),
                    target: "role:admin".to_string(),
                }],
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );
        entities.insert(
            "role:admin".to_string(),
            EntityData {
                entity_type: "role".to_string(),
                string_attrs: HashMap::new(),
                numeric_attrs: HashMap::new(),
                relationships: Vec::new(),
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);

        assert!(result.valid, "Validation should pass: {:?}", result.errors);
        assert_eq!(result.entity_count, 2);
        assert_eq!(result.tier, DataTier::Tier1Direct);
    }

    #[test]
    fn test_entity_id_too_long() {
        let long_id = "a".repeat(MAX_ENTITY_ID_LEN + 1);
        let mut entities = HashMap::new();
        entities.insert(
            long_id.clone(),
            EntityData {
                entity_type: "user".to_string(),
                string_attrs: HashMap::new(),
                numeric_attrs: HashMap::new(),
                relationships: Vec::new(),
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);

        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("exceeds max length")));
    }

    #[test]
    fn test_too_many_string_attrs() {
        let mut string_attrs = HashMap::new();
        for i in 0..=MAX_STRING_ATTRS {
            string_attrs.insert(format!("attr{}", i), "value".to_string());
        }

        let mut entities = HashMap::new();
        entities.insert(
            "user:bob".to_string(),
            EntityData {
                entity_type: "user".to_string(),
                string_attrs,
                numeric_attrs: HashMap::new(),
                relationships: Vec::new(),
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);

        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("string attributes")));
    }

    #[test]
    fn test_jwt_validation() {
        let mut numeric_attrs = HashMap::new();
        numeric_attrs.insert("exp".to_string(), 1735689600);
        numeric_attrs.insert("iat".to_string(), 1735603200);

        let mut string_attrs = HashMap::new();
        string_attrs.insert("sub".to_string(), "user123".to_string());

        let mut entities = HashMap::new();
        entities.insert(
            "jwt:session_abc".to_string(),
            EntityData {
                entity_type: "jwt_session".to_string(),
                string_attrs,
                numeric_attrs,
                relationships: Vec::new(),
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);

        assert!(result.valid);
        assert_eq!(result.entity_types.get("jwt_session"), Some(&1));
    }

    #[test]
    fn test_tier_selection() {
        let validator = EntityValidator::new().with_tier_limits(10, 100);

        // Tier 1: 5 entities
        let mut entities = HashMap::new();
        for i in 0..5 {
            entities.insert(
                format!("user:{}", i),
                EntityData {
                    entity_type: "user".to_string(),
                    string_attrs: HashMap::new(),
                    numeric_attrs: HashMap::new(),
                    relationships: Vec::new(),
                    flags: HashMap::new(),
                    metadata: HashMap::new(),
                },
            );
        }

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities: entities.clone(),
            metadata: HashMap::new(),
        };

        let result = validator.validate(&dataset);
        assert_eq!(result.tier, DataTier::Tier1Direct);

        // Tier 2: 50 entities
        for i in 5..50 {
            entities.insert(
                format!("user:{}", i),
                EntityData {
                    entity_type: "user".to_string(),
                    string_attrs: HashMap::new(),
                    numeric_attrs: HashMap::new(),
                    relationships: Vec::new(),
                    flags: HashMap::new(),
                    metadata: HashMap::new(),
                },
            );
        }

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities: entities.clone(),
            metadata: HashMap::new(),
        };

        let result = validator.validate(&dataset);
        assert_eq!(result.tier, DataTier::Tier2Sharded);

        // Tier 3: 150 entities
        for i in 50..150 {
            entities.insert(
                format!("user:{}", i),
                EntityData {
                    entity_type: "user".to_string(),
                    string_attrs: HashMap::new(),
                    numeric_attrs: HashMap::new(),
                    relationships: Vec::new(),
                    flags: HashMap::new(),
                    metadata: HashMap::new(),
                },
            );
        }

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let result = validator.validate(&dataset);
        assert_eq!(result.tier, DataTier::Tier3Partitioned);
    }

    #[test]
    fn test_relationship_validation() {
        let mut entities = HashMap::new();
        entities.insert(
            "user:charlie".to_string(),
            EntityData {
                entity_type: "user".to_string(),
                string_attrs: HashMap::new(),
                numeric_attrs: HashMap::new(),
                relationships: vec![RelationshipData {
                    rel_type: "has_role".to_string(),
                    target: "role:nonexistent".to_string(), // Target doesn't exist
                }],
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);

        // Should be valid but have warnings
        assert!(result.valid);
        assert!(result.warnings.iter().any(|w| w.contains("non-existent")));
    }

    #[test]
    fn test_strict_mode() {
        let mut entities = HashMap::new();
        entities.insert(
            "user:dave".to_string(),
            EntityData {
                entity_type: "user".to_string(),
                string_attrs: HashMap::new(),
                numeric_attrs: HashMap::new(),
                relationships: Vec::new(), // No relationships - will trigger warning
                flags: HashMap::new(),
                metadata: HashMap::new(),
            },
        );

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        // Non-strict mode: warnings don't fail validation
        let validator = EntityValidator::new();
        let result = validator.validate(&dataset);
        assert!(result.valid);
        assert!(!result.warnings.is_empty());

        // Strict mode: warnings become errors
        let validator = EntityValidator::new().strict();
        let result = validator.validate(&dataset);
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_custom_data_source_validation() {
        use crate::custom::{CustomDataRegistry, CustomDataSource, CustomSchema};

        // Create a custom schema for "organizations"
        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            optional_string_attrs: vec!["description".to_string()],
            required_numeric_attrs: vec!["id".to_string()],
            optional_numeric_attrs: vec!["employee_count".to_string()],
            required_relationships: vec![],
            optional_relationships: vec![],
            required_flags: vec![],
            optional_flags: vec!["is_active".to_string()],
            max_entities: Some(1000),
        };

        let mut registry = CustomDataRegistry::new();
        let source = CustomDataSource::new("organizations".to_string(), schema);
        registry.register(source).unwrap();

        // Create a valid organization entity
        let mut entities = HashMap::new();
        let mut entity_data = EntityData {
            entity_type: "organizations".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };
        entity_data
            .string_attrs
            .insert("name".to_string(), "Acme Corp".to_string());
        entity_data.numeric_attrs.insert("id".to_string(), 123);
        entity_data.flags.insert("is_active".to_string(), true);
        entities.insert("org:acme".to_string(), entity_data);

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        // Validate with custom registry
        let validator = EntityValidator::new().with_custom_registry(registry);
        let result = validator.validate(&dataset);

        assert!(result.valid, "Validation errors: {:?}", result.errors);
        assert_eq!(result.entity_count, 1);
        assert_eq!(result.entity_types.get("organizations"), Some(&1));
    }

    #[test]
    fn test_custom_data_source_missing_required() {
        use crate::custom::{CustomDataRegistry, CustomDataSource, CustomSchema};

        let schema = CustomSchema {
            required_string_attrs: vec!["name".to_string()],
            required_numeric_attrs: vec!["id".to_string()],
            ..Default::default()
        };

        let mut registry = CustomDataRegistry::new();
        let source = CustomDataSource::new("projects".to_string(), schema);
        registry.register(source).unwrap();

        // Create entity missing required fields
        let mut entities = HashMap::new();
        let entity_data = EntityData {
            entity_type: "projects".to_string(),
            string_attrs: HashMap::new(),  // Missing "name"
            numeric_attrs: HashMap::new(), // Missing "id"
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };
        entities.insert("project:1".to_string(), entity_data);

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new().with_custom_registry(registry);
        let result = validator.validate(&dataset);

        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("name")));
        assert!(result.errors.iter().any(|e| e.contains("id")));
    }

    #[test]
    fn test_unregistered_custom_type() {
        use crate::custom::CustomDataRegistry;

        let registry = CustomDataRegistry::new(); // Empty registry

        let mut entities = HashMap::new();
        let entity_data = EntityData {
            entity_type: "unknown_type".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };
        entities.insert("unknown:1".to_string(), entity_data);

        let dataset = EntityDataset {
            dataset: "test".to_string(),
            version: "1.0".to_string(),
            entities,
            metadata: HashMap::new(),
        };

        let validator = EntityValidator::new().with_custom_registry(registry);
        let result = validator.validate(&dataset);

        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("unregistered custom type")));
    }
}
