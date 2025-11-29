//! Generic Join Framework for Multi-Source Data Loading
//!
//! Enables joining entities from multiple data sources on common keys.
//! Supports N-way joins for arbitrary entity types.

use super::loader::{DataLoader, LoadStats};
use reaper_core::ReaperError;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

/// Configuration for joining entities from multiple sources
///
/// # Example
/// ```ignore
/// let config = JoinConfig {
///     primary: EntitySource {
///         file_path: "users.json".to_string(),
///         entity_type: "User".to_string(),
///     },
///     secondary: HashMap::from([
///         ("Device".to_string(), SecondarySource {
///             source: EntitySource {
///                 file_path: "devices.json".to_string(),
///                 entity_type: "Device".to_string(),
///             },
///             join_key: JoinKey {
///                 primary_field: "device_id".to_string(),
///                 secondary_field: "id".to_string(),
///             },
///         }),
///     ]),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct JoinConfig {
    /// Primary entity source (will be enriched with secondary data)
    pub primary: EntitySource,

    /// Secondary sources to join with primary
    /// Map: entity_type -> (source, join_key)
    pub secondary: HashMap<String, SecondarySource>,
}

/// Entity source specification
#[derive(Debug, Clone)]
pub struct EntitySource {
    /// File path to JSON data
    pub file_path: String,

    /// Entity type name ("User", "Device", etc.)
    pub entity_type: String,
}

/// Secondary source configuration
#[derive(Debug, Clone)]
pub struct SecondarySource {
    /// Source file and entity type
    pub source: EntitySource,

    /// How to join with primary source
    pub join_key: JoinKey,
}

/// Join key specification
#[derive(Debug, Clone)]
pub struct JoinKey {
    /// Field in primary entity (e.g., "device_id", "user_id")
    pub primary_field: String,

    /// Field in secondary entity (e.g., "id")
    pub secondary_field: String,
}

/// Result of a join operation
#[derive(Debug)]
pub struct JoinResult {
    /// Statistics from loading joined entities
    pub stats: LoadStats,

    /// Number of primary entities processed
    pub primary_count: usize,

    /// Number of successful joins per secondary source
    pub join_counts: HashMap<String, usize>,

    /// Number of primary entities with missing secondary data
    pub missing_counts: HashMap<String, usize>,

    /// Total join time
    pub join_duration: std::time::Duration,
}

/// Engine for executing multi-source joins
pub struct JoinEngine {
    loader: DataLoader,
}

impl JoinEngine {
    /// Create a new join engine
    pub fn new(loader: DataLoader) -> Self {
        Self { loader }
    }

    /// Execute a multi-source join and load into DataStore
    ///
    /// # Process
    /// 1. Load primary entities from primary source
    /// 2. Build indexes for all secondary sources
    /// 3. Join primary with each secondary source
    /// 4. Load all joined entities into DataStore
    ///
    /// # Returns
    /// JoinResult with statistics and entity counts
    pub fn join_and_load(&self, config: JoinConfig) -> Result<JoinResult, ReaperError> {
        let join_start = Instant::now();

        // Load primary entities
        let primary_entities = load_entities_from_file(&config.primary.file_path)?;
        let primary_count = primary_entities.len();

        // Build indexes for all secondary sources
        let mut secondary_indexes = HashMap::new();
        for (entity_type, sec_source) in &config.secondary {
            let index = self.build_join_index(&sec_source.source, &sec_source.join_key)?;
            secondary_indexes.insert(entity_type.clone(), (index, sec_source.join_key.clone()));
        }

        // Join and load
        let mut joined_entities = Vec::new();
        let mut join_counts: HashMap<String, usize> = HashMap::new();
        let mut missing_counts: HashMap<String, usize> = HashMap::new();

        for mut primary in primary_entities {
            // Join with each secondary source
            for (entity_type, (index, join_key)) in &secondary_indexes {
                if let Some(join_value) = extract_join_value(&primary, &join_key.primary_field) {
                    if let Some(secondary) = index.get(&join_value) {
                        merge_attributes(&mut primary, secondary);
                        *join_counts.entry(entity_type.clone()).or_insert(0) += 1;
                    } else {
                        *missing_counts.entry(entity_type.clone()).or_insert(0) += 1;
                    }
                } else {
                    *missing_counts.entry(entity_type.clone()).or_insert(0) += 1;
                }
            }
            joined_entities.push(primary);
        }

        let join_duration = join_start.elapsed();

        // Load all joined entities
        let stats = self.loader.load_json_values(joined_entities)?;

        Ok(JoinResult {
            stats,
            primary_count,
            join_counts,
            missing_counts,
            join_duration,
        })
    }

    /// Build join index: join_value -> entity
    fn build_join_index(
        &self,
        source: &EntitySource,
        join_key: &JoinKey,
    ) -> Result<HashMap<String, JsonValue>, ReaperError> {
        let entities = load_entities_from_file(&source.file_path)?;
        let mut index = HashMap::new();

        for entity in entities {
            if let Some(join_value) = extract_join_value(&entity, &join_key.secondary_field) {
                index.insert(join_value, entity);
            }
        }

        Ok(index)
    }
}

/// Load entities from a JSON file
fn load_entities_from_file(filename: &str) -> Result<Vec<JsonValue>, ReaperError> {
    let content = fs::read_to_string(filename).map_err(|e| ReaperError::InvalidPolicy {
        reason: format!("Failed to read file {}: {}", filename, e),
    })?;

    let data: JsonValue =
        serde_json::from_str(&content).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to parse JSON from {}: {}", filename, e),
        })?;

    let entities = data["entities"]
        .as_array()
        .ok_or_else(|| ReaperError::InvalidPolicy {
            reason: format!("Missing 'entities' array in {}", filename),
        })?
        .clone();

    Ok(entities)
}

/// Extract join value from entity at specified field path
fn extract_join_value(entity: &JsonValue, field_path: &str) -> Option<String> {
    // Support nested paths like "attributes.id"
    let parts: Vec<&str> = field_path.split('.').collect();

    let mut current = entity;
    for part in &parts {
        current = current.get(part)?;
    }

    current.as_str().map(|s| s.to_string())
}

/// Merge attributes from secondary into primary
fn merge_attributes(primary: &mut JsonValue, secondary: &JsonValue) {
    if let (Some(p_attrs), Some(s_attrs)) = (
        primary["attributes"].as_object_mut(),
        secondary["attributes"].as_object(),
    ) {
        for (key, value) in s_attrs {
            // Don't overwrite existing attributes in primary
            p_attrs.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{DataLoader, DataStore};
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Helper to create a temporary JSON file
    fn create_temp_json_file(entities: Vec<JsonValue>) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        let data = json!({ "entities": entities });
        write!(file, "{}", serde_json::to_string(&data).unwrap()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_extract_join_value_simple() {
        let entity = json!({
            "id": "user_1",
            "attributes": {
                "id": "user_1",
                "name": "Alice"
            }
        });

        assert_eq!(
            extract_join_value(&entity, "attributes.id"),
            Some("user_1".to_string())
        );
        assert_eq!(
            extract_join_value(&entity, "id"),
            Some("user_1".to_string())
        );
    }

    #[test]
    fn test_extract_join_value_missing() {
        let entity = json!({
            "id": "user_1",
            "attributes": {}
        });

        assert_eq!(extract_join_value(&entity, "attributes.missing"), None);
    }

    #[test]
    fn test_merge_attributes() {
        let mut primary = json!({
            "id": "user_1",
            "type": "User",
            "attributes": {
                "id": "user_1",
                "role": "admin"
            }
        });

        let secondary = json!({
            "id": "user_1",
            "type": "User",
            "attributes": {
                "id": "user_1",
                "department": "engineering",
                "role": "viewer"  // Should NOT overwrite
            }
        });

        merge_attributes(&mut primary, &secondary);

        let attrs = primary["attributes"].as_object().unwrap();
        assert_eq!(attrs.get("role").unwrap().as_str().unwrap(), "admin"); // Not overwritten
        assert_eq!(
            attrs.get("department").unwrap().as_str().unwrap(),
            "engineering"
        );
    }

    #[test]
    fn test_join_two_sources() {
        // Create temporary files
        let users_file = create_temp_json_file(vec![
            json!({
                "id": "user_1",
                "type": "User",
                "attributes": {
                    "id": "user_1",
                    "role": "admin"
                }
            }),
            json!({
                "id": "user_2",
                "type": "User",
                "attributes": {
                    "id": "user_2",
                    "role": "viewer"
                }
            }),
        ]);

        let details_file = create_temp_json_file(vec![
            json!({
                "id": "user_1",
                "type": "UserDetail",
                "attributes": {
                    "id": "user_1",
                    "department": "engineering",
                    "clearance": 3
                }
            }),
            json!({
                "id": "user_2",
                "type": "UserDetail",
                "attributes": {
                    "id": "user_2",
                    "department": "hr",
                    "clearance": 1
                }
            }),
        ]);

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());
        let engine = JoinEngine::new(loader);

        let config = JoinConfig {
            primary: EntitySource {
                file_path: users_file.path().to_str().unwrap().to_string(),
                entity_type: "User".to_string(),
            },
            secondary: HashMap::from([(
                "UserDetail".to_string(),
                SecondarySource {
                    source: EntitySource {
                        file_path: details_file.path().to_str().unwrap().to_string(),
                        entity_type: "UserDetail".to_string(),
                    },
                    join_key: JoinKey {
                        primary_field: "attributes.id".to_string(),
                        secondary_field: "attributes.id".to_string(),
                    },
                },
            )]),
        };

        let result = engine.join_and_load(config).unwrap();

        assert_eq!(result.primary_count, 2);
        assert_eq!(result.stats.total, 2);
        assert_eq!(result.join_counts.get("UserDetail"), Some(&2));
        assert_eq!(result.missing_counts.get("UserDetail"), None);

        // Verify merged data
        let interner = store.interner();
        let user1_id = interner.intern("user_1");
        let user1 = store.get(user1_id).unwrap();

        let role_key = interner.intern("role");
        let dept_key = interner.intern("department");
        let clearance_key = interner.intern("clearance");

        assert_eq!(
            user1
                .get_string_attribute(role_key, interner)
                .unwrap()
                .as_ref(),
            "admin"
        );
        assert_eq!(
            user1
                .get_string_attribute(dept_key, interner)
                .unwrap()
                .as_ref(),
            "engineering"
        );
        assert_eq!(user1.get_int_attribute(clearance_key), Some(3));
    }

    #[test]
    fn test_join_three_sources() {
        // Create temporary files
        let users_file = create_temp_json_file(vec![json!({
            "id": "user_1",
            "type": "User",
            "attributes": {
                "id": "user_1",
                "name": "Alice",
                "device_id": "device_1",
                "location_id": "loc_1"
            }
        })]);

        let devices_file = create_temp_json_file(vec![json!({
            "id": "device_1",
            "type": "Device",
            "attributes": {
                "id": "device_1",
                "trustscore": 85,
                "os": "Linux"
            }
        })]);

        let locations_file = create_temp_json_file(vec![json!({
            "id": "loc_1",
            "type": "Location",
            "attributes": {
                "id": "loc_1",
                "region": "US",
                "secure": true
            }
        })]);

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());
        let engine = JoinEngine::new(loader);

        let config = JoinConfig {
            primary: EntitySource {
                file_path: users_file.path().to_str().unwrap().to_string(),
                entity_type: "User".to_string(),
            },
            secondary: HashMap::from([
                (
                    "Device".to_string(),
                    SecondarySource {
                        source: EntitySource {
                            file_path: devices_file.path().to_str().unwrap().to_string(),
                            entity_type: "Device".to_string(),
                        },
                        join_key: JoinKey {
                            primary_field: "attributes.device_id".to_string(),
                            secondary_field: "attributes.id".to_string(),
                        },
                    },
                ),
                (
                    "Location".to_string(),
                    SecondarySource {
                        source: EntitySource {
                            file_path: locations_file.path().to_str().unwrap().to_string(),
                            entity_type: "Location".to_string(),
                        },
                        join_key: JoinKey {
                            primary_field: "attributes.location_id".to_string(),
                            secondary_field: "attributes.id".to_string(),
                        },
                    },
                ),
            ]),
        };

        let result = engine.join_and_load(config).unwrap();

        assert_eq!(result.primary_count, 1);
        assert_eq!(result.stats.total, 1);
        assert_eq!(result.join_counts.get("Device"), Some(&1));
        assert_eq!(result.join_counts.get("Location"), Some(&1));

        // Verify all data merged
        let interner = store.interner();
        let user1_id = interner.intern("user_1");
        let user1 = store.get(user1_id).unwrap();

        let trustscore_key = interner.intern("trustscore");
        let region_key = interner.intern("region");
        let secure_key = interner.intern("secure");

        assert_eq!(user1.get_int_attribute(trustscore_key), Some(85));
        assert_eq!(
            user1
                .get_string_attribute(region_key, interner)
                .unwrap()
                .as_ref(),
            "US"
        );
        assert_eq!(user1.get_bool_attribute(secure_key), Some(true));
    }

    #[test]
    fn test_join_with_missing_secondary() {
        let users_file = create_temp_json_file(vec![
            json!({
                "id": "user_1",
                "type": "User",
                "attributes": {
                    "id": "user_1",
                    "role": "admin"
                }
            }),
            json!({
                "id": "user_2",
                "type": "User",
                "attributes": {
                    "id": "user_2",
                    "role": "viewer"
                }
            }),
        ]);

        let details_file = create_temp_json_file(vec![json!({
            "id": "user_1",
            "type": "UserDetail",
            "attributes": {
                "id": "user_1",
                "department": "engineering"
            }
        })]);

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());
        let engine = JoinEngine::new(loader);

        let config = JoinConfig {
            primary: EntitySource {
                file_path: users_file.path().to_str().unwrap().to_string(),
                entity_type: "User".to_string(),
            },
            secondary: HashMap::from([(
                "UserDetail".to_string(),
                SecondarySource {
                    source: EntitySource {
                        file_path: details_file.path().to_str().unwrap().to_string(),
                        entity_type: "UserDetail".to_string(),
                    },
                    join_key: JoinKey {
                        primary_field: "attributes.id".to_string(),
                        secondary_field: "attributes.id".to_string(),
                    },
                },
            )]),
        };

        let result = engine.join_and_load(config).unwrap();

        assert_eq!(result.primary_count, 2);
        assert_eq!(result.stats.total, 2);
        assert_eq!(result.join_counts.get("UserDetail"), Some(&1)); // Only 1 matched
        assert_eq!(result.missing_counts.get("UserDetail"), Some(&1)); // 1 missing
    }
}
