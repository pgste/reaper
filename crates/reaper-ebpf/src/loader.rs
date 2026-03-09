//! Entity data loader for eBPF maps
//!
//! Converts validated JSON entity data into kernel Entity structs and loads them into eBPF maps.

use crate::entity::{EntityData, EntityDataset, EntityType, LoadStats, RelationshipData};
use anyhow::{anyhow, Context, Result};
use aya::maps::HashMap as BpfHashMap;
use aya::Pod;
use std::time::SystemTime;

/// Kernel entity structure (must match reaper-ebpf-kern/src/entity.rs)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entity {
    pub id: [u8; 64],
    pub entity_type: u8,
    pub string_attrs: [StringAttr; 8],
    pub string_count: u8,
    pub numeric_attrs: [NumericAttr; 8],
    pub numeric_count: u8,
    pub relationships: [Relationship; 8],
    pub relationship_count: u8,
    pub flags: u64,
    pub version: u32,
    pub created_at: u64,
    pub updated_at: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StringAttr {
    pub key: [u8; 32],
    pub value: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NumericAttr {
    pub key: [u8; 32],
    pub value: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Relationship {
    pub rel_type: [u8; 32],
    pub target_id: [u8; 64],
}

// Safety: These types are plain old data (POD) types with repr(C)
unsafe impl Pod for Entity {}
unsafe impl Pod for StringAttr {}
unsafe impl Pod for NumericAttr {}
unsafe impl Pod for Relationship {}

impl Entity {
    /// Create a new empty entity
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            id: [0u8; 64],
            entity_type: 255, // Custom
            string_attrs: [StringAttr::new(); 8],
            string_count: 0,
            numeric_attrs: [NumericAttr::new(); 8],
            numeric_count: 0,
            relationships: [Relationship::new(); 8],
            relationship_count: 0,
            flags: 0,
            version: 0,
            created_at: 0,
            updated_at: 0,
        }
    }
}

impl StringAttr {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            key: [0u8; 32],
            value: [0u8; 64],
        }
    }
}

impl NumericAttr {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            key: [0u8; 32],
            value: 0,
        }
    }
}

impl Relationship {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            rel_type: [0u8; 32],
            target_id: [0u8; 64],
        }
    }
}

/// Entity data loader
pub struct EntityLoader {
    /// Current timestamp for created_at/updated_at
    timestamp: u64,
}

impl EntityLoader {
    /// Create a new loader
    pub fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        Self { timestamp }
    }

    /// Load entities from dataset into eBPF maps
    pub fn load_into_maps(
        &self,
        dataset: &EntityDataset,
        users_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<LoadStats> {
        let mut stats = LoadStats::new();

        for (entity_id, entity_data) in &dataset.entities {
            match self.load_entity(
                entity_id,
                entity_data,
                users_map,
                roles_map,
                groups_map,
                resources_map,
                jwt_sessions_map,
            ) {
                Ok(entity_type) => {
                    stats.increment(entity_type);
                }
                Err(e) => {
                    eprintln!("Failed to load entity '{}': {}", entity_id, e);
                    stats.errors += 1;
                }
            }
        }

        Ok(stats)
    }

    /// Load a single entity into the appropriate map
    #[allow(clippy::too_many_arguments)]
    fn load_entity(
        &self,
        entity_id: &str,
        entity_data: &EntityData,
        users_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<EntityType> {
        // Parse entity type
        let entity_type = EntityType::parse(&entity_data.entity_type)
            .ok_or_else(|| anyhow!("Invalid entity type: {}", entity_data.entity_type))?;

        // Convert to kernel entity
        let entity = self.convert_to_kernel_entity(entity_id, entity_data, entity_type)?;

        // Create entity ID key
        let mut id_key = [0u8; 64];
        let id_bytes = entity_id.as_bytes();
        let copy_len = id_bytes.len().min(64);
        id_key[..copy_len].copy_from_slice(&id_bytes[..copy_len]);

        // Insert into appropriate map
        match entity_type {
            EntityType::User => {
                users_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert user into map")?;
            }
            EntityType::Role => {
                roles_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert role into map")?;
            }
            EntityType::Group => {
                groups_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert group into map")?;
            }
            EntityType::Resource | EntityType::Permission => {
                resources_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert resource into map")?;
            }
            EntityType::JwtSession => {
                jwt_sessions_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert JWT session into map")?;
            }
            EntityType::Custom => {
                // For now, custom entities go into resources map
                resources_map
                    .insert(id_key, entity, 0)
                    .context("Failed to insert custom entity into map")?;
            }
        }

        Ok(entity_type)
    }

    /// Convert JSON EntityData to kernel Entity struct
    fn convert_to_kernel_entity(
        &self,
        entity_id: &str,
        data: &EntityData,
        entity_type: EntityType,
    ) -> Result<Entity> {
        let mut entity = Entity::new();

        // Set entity ID (null-terminated)
        let id_bytes = entity_id.as_bytes();
        let copy_len = id_bytes.len().min(63); // Leave room for null terminator
        entity.id[..copy_len].copy_from_slice(&id_bytes[..copy_len]);
        entity.id[copy_len] = 0; // Null terminator

        // Set entity type
        entity.entity_type = entity_type.to_u8();

        // Convert string attributes
        entity.string_count = data.string_attrs.len().min(8) as u8;
        for (i, (key, value)) in data.string_attrs.iter().take(8).enumerate() {
            entity.string_attrs[i] = self.convert_string_attr(key, value)?;
        }

        // Convert numeric attributes
        entity.numeric_count = data.numeric_attrs.len().min(8) as u8;
        for (i, (key, value)) in data.numeric_attrs.iter().take(8).enumerate() {
            entity.numeric_attrs[i] = self.convert_numeric_attr(key, *value)?;
        }

        // Convert relationships
        entity.relationship_count = data.relationships.len().min(8) as u8;
        for (i, rel) in data.relationships.iter().take(8).enumerate() {
            entity.relationships[i] = self.convert_relationship(&rel.rel_type, &rel.target)?;
        }

        // Convert flags
        entity.flags = self.convert_flags(&data.flags);

        // Set timestamps
        entity.created_at = self.timestamp;
        entity.updated_at = self.timestamp;
        entity.version = 1;

        Ok(entity)
    }

    /// Convert a string attribute to kernel format
    fn convert_string_attr(&self, key: &str, value: &str) -> Result<StringAttr> {
        let mut attr = StringAttr::new();

        // Copy key (null-terminated)
        let key_bytes = key.as_bytes();
        if key_bytes.len() >= 32 {
            return Err(anyhow!("Attribute key too long: {}", key));
        }
        attr.key[..key_bytes.len()].copy_from_slice(key_bytes);
        attr.key[key_bytes.len()] = 0;

        // Copy value (null-terminated)
        let value_bytes = value.as_bytes();
        if value_bytes.len() >= 64 {
            return Err(anyhow!("Attribute value too long: {}", value));
        }
        attr.value[..value_bytes.len()].copy_from_slice(value_bytes);
        attr.value[value_bytes.len()] = 0;

        Ok(attr)
    }

    /// Convert a numeric attribute to kernel format
    fn convert_numeric_attr(&self, key: &str, value: i64) -> Result<NumericAttr> {
        let mut attr = NumericAttr::new();

        // Copy key (null-terminated)
        let key_bytes = key.as_bytes();
        if key_bytes.len() >= 32 {
            return Err(anyhow!("Attribute key too long: {}", key));
        }
        attr.key[..key_bytes.len()].copy_from_slice(key_bytes);
        attr.key[key_bytes.len()] = 0;

        attr.value = value;

        Ok(attr)
    }

    /// Convert a relationship to kernel format
    fn convert_relationship(&self, rel_type: &str, target: &str) -> Result<Relationship> {
        let mut rel = Relationship::new();

        // Copy relationship type (null-terminated)
        let type_bytes = rel_type.as_bytes();
        if type_bytes.len() >= 32 {
            return Err(anyhow!("Relationship type too long: {}", rel_type));
        }
        rel.rel_type[..type_bytes.len()].copy_from_slice(type_bytes);
        rel.rel_type[type_bytes.len()] = 0;

        // Copy target ID (null-terminated)
        let target_bytes = target.as_bytes();
        if target_bytes.len() >= 64 {
            return Err(anyhow!("Relationship target too long: {}", target));
        }
        rel.target_id[..target_bytes.len()].copy_from_slice(target_bytes);
        rel.target_id[target_bytes.len()] = 0;

        Ok(rel)
    }

    /// Convert boolean flags to bitfield
    fn convert_flags(&self, flags: &std::collections::HashMap<String, bool>) -> u64 {
        let mut bitfield = 0u64;

        // Standard flags (bit positions 0-7)
        if flags.get("is_active").copied().unwrap_or(false) {
            bitfield |= 1 << 0;
        }
        if flags.get("is_verified").copied().unwrap_or(false) {
            bitfield |= 1 << 1;
        }
        if flags.get("is_admin").copied().unwrap_or(false) {
            bitfield |= 1 << 2;
        }
        if flags.get("is_locked").copied().unwrap_or(false) {
            bitfield |= 1 << 3;
        }

        // Custom flags (bit positions 8-63)
        // For custom flags, we'll hash the flag name to a bit position
        for (flag_name, &value) in flags {
            if value
                && !["is_active", "is_verified", "is_admin", "is_locked"]
                    .contains(&flag_name.as_str())
            {
                // Simple hash to bit position (8-63)
                let hash = flag_name.bytes().fold(0u8, |acc, b| acc.wrapping_add(b));
                let bit_pos = 8 + (hash % 56); // 56 custom bits available
                bitfield |= 1u64 << bit_pos;
            }
        }

        bitfield
    }

    // ===== CRUD Operations for Runtime Entity Updates =====

    /// Insert or update a single entity (CREATE/UPDATE)
    ///
    /// This method allows runtime updates to entities in eBPF maps.
    /// If the entity exists, it will be updated; otherwise, it will be created.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_entity(
        &self,
        entity_id: &str,
        entity_data: &EntityData,
        users_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<EntityType> {
        // Reuse load_entity logic
        self.load_entity(
            entity_id,
            entity_data,
            users_map,
            roles_map,
            groups_map,
            resources_map,
            jwt_sessions_map,
        )
    }

    /// Delete a single entity (DELETE)
    ///
    /// Returns true if the entity existed and was deleted, false otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn delete_entity(
        &self,
        entity_id: &str,
        entity_type: EntityType,
        users_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<bool> {
        // Create entity ID key
        let mut id_key = [0u8; 64];
        let id_bytes = entity_id.as_bytes();
        let copy_len = id_bytes.len().min(64);
        id_key[..copy_len].copy_from_slice(&id_bytes[..copy_len]);

        // Delete from appropriate map
        let result = match entity_type {
            EntityType::User => users_map.remove(&id_key),
            EntityType::Role => roles_map.remove(&id_key),
            EntityType::Group => groups_map.remove(&id_key),
            EntityType::Resource | EntityType::Permission => resources_map.remove(&id_key),
            EntityType::JwtSession => jwt_sessions_map.remove(&id_key),
            EntityType::Custom => {
                return Err(anyhow!(
                    "Custom entity type not supported for CRUD operations"
                ));
            }
        };

        match result {
            Ok(_) => Ok(true),
            Err(aya::maps::MapError::KeyNotFound) => Ok(false),
            Err(e) => Err(anyhow!("Failed to delete entity: {}", e)),
        }
    }

    /// Get a single entity (READ)
    ///
    /// Returns the entity data if found, None otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn get_entity(
        &self,
        entity_id: &str,
        entity_type: EntityType,
        users_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<Option<EntityData>> {
        // Create entity ID key
        let mut id_key = [0u8; 64];
        let id_bytes = entity_id.as_bytes();
        let copy_len = id_bytes.len().min(64);
        id_key[..copy_len].copy_from_slice(&id_bytes[..copy_len]);

        // Lookup in appropriate map
        let entity_result = match entity_type {
            EntityType::User => users_map.get(&id_key, 0),
            EntityType::Role => roles_map.get(&id_key, 0),
            EntityType::Group => groups_map.get(&id_key, 0),
            EntityType::Resource | EntityType::Permission => resources_map.get(&id_key, 0),
            EntityType::JwtSession => jwt_sessions_map.get(&id_key, 0),
            EntityType::Custom => {
                return Err(anyhow!(
                    "Custom entity type not supported for CRUD operations"
                ));
            }
        };

        match entity_result {
            Ok(entity) => Ok(Some(self.kernel_entity_to_data(&entity)?)),
            Err(aya::maps::MapError::KeyNotFound) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get entity: {}", e)),
        }
    }

    /// List all entities of a type
    ///
    /// Returns up to `limit` entities of the specified type.
    #[allow(clippy::too_many_arguments)]
    pub fn list_entities(
        &self,
        entity_type: EntityType,
        limit: usize,
        users_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<Vec<EntityData>> {
        let target_map: &BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity> = match entity_type {
            EntityType::User => users_map,
            EntityType::Role => roles_map,
            EntityType::Group => groups_map,
            EntityType::Resource | EntityType::Permission => resources_map,
            EntityType::JwtSession => jwt_sessions_map,
            EntityType::Custom => {
                return Err(anyhow!(
                    "Custom entity type not supported for CRUD operations"
                ));
            }
        };

        let mut result = Vec::new();
        let mut count = 0;

        // Iterate over map entries
        for item in target_map.iter() {
            if count >= limit {
                break;
            }

            match item {
                Ok((_key, entity)) => match self.kernel_entity_to_data(&entity) {
                    Ok(entity_data) => {
                        result.push(entity_data);
                        count += 1;
                    }
                    Err(e) => {
                        eprintln!("Failed to convert entity: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to read map entry: {}", e);
                }
            }
        }

        Ok(result)
    }

    /// Batch upsert for efficiency
    ///
    /// Updates multiple entities in one call.
    /// Returns success/failure count and errors per entity.
    pub fn batch_upsert(
        &self,
        entities: &[(String, EntityData)], // (entity_id, entity_data)
        users_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        roles_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        groups_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        resources_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
        jwt_sessions_map: &mut BpfHashMap<&mut aya::maps::MapData, [u8; 64], Entity>,
    ) -> Result<BatchResult> {
        let mut succeeded = 0;
        let mut failed = 0;
        let mut errors = Vec::new();

        for (entity_id, entity_data) in entities {
            match self.upsert_entity(
                entity_id,
                entity_data,
                users_map,
                roles_map,
                groups_map,
                resources_map,
                jwt_sessions_map,
            ) {
                Ok(_) => succeeded += 1,
                Err(e) => {
                    failed += 1;
                    errors.push((entity_id.clone(), e.to_string()));
                }
            }
        }

        Ok(BatchResult {
            succeeded,
            failed,
            errors,
        })
    }

    /// Convert kernel Entity to EntityData (reverse of convert_to_kernel_entity)
    fn kernel_entity_to_data(&self, entity: &Entity) -> Result<EntityData> {
        // Extract entity type string
        let entity_type = match entity.entity_type {
            0 => "user".to_string(),
            1 => "role".to_string(),
            2 => "group".to_string(),
            3 => "resource".to_string(),
            4 => "permission".to_string(),
            5 => "jwt_session".to_string(),
            _ => return Err(anyhow!("Unknown entity type: {}", entity.entity_type)),
        };

        // Extract string attributes
        let mut string_attrs = std::collections::HashMap::new();
        for i in 0..entity.string_count as usize {
            let attr = &entity.string_attrs[i];
            let key = self.extract_cstring(&attr.key)?;
            let value = self.extract_cstring(&attr.value)?;
            string_attrs.insert(key, value);
        }

        // Extract numeric attributes
        let mut numeric_attrs = std::collections::HashMap::new();
        for i in 0..entity.numeric_count as usize {
            let attr = &entity.numeric_attrs[i];
            let key = self.extract_cstring(&attr.key)?;
            numeric_attrs.insert(key, attr.value);
        }

        // Extract relationships
        let mut relationships = Vec::new();
        for i in 0..entity.relationship_count as usize {
            let rel = &entity.relationships[i];
            relationships.push(RelationshipData {
                rel_type: self.extract_cstring(&rel.rel_type)?,
                target: self.extract_cstring(&rel.target_id)?,
            });
        }

        // Extract flags from bitfield
        let mut flags = std::collections::HashMap::new();
        if entity.flags & (1 << 0) != 0 {
            flags.insert("is_active".to_string(), true);
        }
        if entity.flags & (1 << 1) != 0 {
            flags.insert("is_verified".to_string(), true);
        }
        if entity.flags & (1 << 2) != 0 {
            flags.insert("is_admin".to_string(), true);
        }
        if entity.flags & (1 << 3) != 0 {
            flags.insert("is_locked".to_string(), true);
        }

        Ok(EntityData {
            entity_type,
            string_attrs,
            numeric_attrs,
            relationships,
            flags,
            metadata: std::collections::HashMap::new(),
        })
    }

    /// Extract null-terminated C string from byte array
    fn extract_cstring(&self, bytes: &[u8]) -> Result<String> {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec())
            .map_err(|e| anyhow!("Invalid UTF-8 in C string: {}", e))
    }
}

/// Result of batch upsert operation
#[derive(Debug)]
pub struct BatchResult {
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<(String, String)>, // (entity_id, error)
}

impl Default for EntityLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::RelationshipData;

    #[test]
    fn test_convert_string_attr() {
        let loader = EntityLoader::new();
        let attr = loader
            .convert_string_attr("email", "alice@example.com")
            .unwrap();

        // Check key
        assert_eq!(&attr.key[..5], b"email");
        assert_eq!(attr.key[5], 0); // Null terminator

        // Check value
        assert_eq!(&attr.value[..17], b"alice@example.com");
        assert_eq!(attr.value[17], 0); // Null terminator
    }

    #[test]
    fn test_convert_numeric_attr() {
        let loader = EntityLoader::new();
        let attr = loader.convert_numeric_attr("age", 30).unwrap();

        assert_eq!(&attr.key[..3], b"age");
        assert_eq!(attr.key[3], 0);
        assert_eq!(attr.value, 30);
    }

    #[test]
    fn test_convert_relationship() {
        let loader = EntityLoader::new();
        let rel = loader
            .convert_relationship("has_role", "role:admin")
            .unwrap();

        assert_eq!(&rel.rel_type[..8], b"has_role");
        assert_eq!(rel.rel_type[8], 0);
        assert_eq!(&rel.target_id[..10], b"role:admin");
        assert_eq!(rel.target_id[10], 0);
    }

    #[test]
    fn test_convert_flags() {
        let loader = EntityLoader::new();
        let mut flags = std::collections::HashMap::new();
        flags.insert("is_active".to_string(), true);
        flags.insert("is_verified".to_string(), true);
        flags.insert("is_admin".to_string(), false);

        let bitfield = loader.convert_flags(&flags);

        // Bit 0 (is_active) should be set
        assert_eq!(bitfield & (1 << 0), 1 << 0);
        // Bit 1 (is_verified) should be set
        assert_eq!(bitfield & (1 << 1), 1 << 1);
        // Bit 2 (is_admin) should NOT be set
        assert_eq!(bitfield & (1 << 2), 0);
    }

    #[test]
    fn test_convert_to_kernel_entity() {
        let loader = EntityLoader::new();

        let mut string_attrs = std::collections::HashMap::new();
        string_attrs.insert("email".to_string(), "bob@example.com".to_string());

        let mut numeric_attrs = std::collections::HashMap::new();
        numeric_attrs.insert("age".to_string(), 25);

        let relationships = vec![RelationshipData {
            rel_type: "has_role".to_string(),
            target: "role:user".to_string(),
        }];

        let mut flags = std::collections::HashMap::new();
        flags.insert("is_active".to_string(), true);

        let entity_data = EntityData {
            entity_type: "user".to_string(),
            string_attrs,
            numeric_attrs,
            relationships,
            flags,
            metadata: std::collections::HashMap::new(),
        };

        let entity = loader
            .convert_to_kernel_entity("user:bob", &entity_data, EntityType::User)
            .unwrap();

        // Check entity ID
        assert_eq!(&entity.id[..8], b"user:bob");
        assert_eq!(entity.id[8], 0);

        // Check entity type
        assert_eq!(entity.entity_type, 0); // User = 0

        // Check counts
        assert_eq!(entity.string_count, 1);
        assert_eq!(entity.numeric_count, 1);
        assert_eq!(entity.relationship_count, 1);

        // Check string attribute
        assert_eq!(&entity.string_attrs[0].key[..5], b"email");
        assert_eq!(&entity.string_attrs[0].value[..15], b"bob@example.com");

        // Check numeric attribute
        assert_eq!(&entity.numeric_attrs[0].key[..3], b"age");
        assert_eq!(entity.numeric_attrs[0].value, 25);

        // Check relationship
        assert_eq!(&entity.relationships[0].rel_type[..8], b"has_role");
        assert_eq!(&entity.relationships[0].target_id[..9], b"role:user");

        // Check flags
        assert_eq!(entity.flags & 1, 1); // is_active set

        // Check timestamps
        assert!(entity.created_at > 0);
        assert_eq!(entity.created_at, entity.updated_at);
        assert_eq!(entity.version, 1);
    }

    #[test]
    fn test_entity_size() {
        // Verify Entity struct size is reasonable for eBPF
        let size = std::mem::size_of::<Entity>();
        println!("Entity size: {} bytes", size);

        // Should be <= 2KB to fit in eBPF stack
        assert!(size <= 2048, "Entity size {} exceeds 2KB limit", size);
    }

    // ===== Tests for CRUD Operations =====

    #[test]
    fn test_kernel_entity_to_data_conversion() {
        // Test the reverse conversion (kernel Entity -> EntityData)
        let loader = EntityLoader::new();

        // Create a kernel entity
        let mut string_attrs = std::collections::HashMap::new();
        string_attrs.insert("email".to_string(), "test@example.com".to_string());
        string_attrs.insert("name".to_string(), "Test User".to_string());

        let mut numeric_attrs = std::collections::HashMap::new();
        numeric_attrs.insert("age".to_string(), 42);
        numeric_attrs.insert("level".to_string(), 5);

        let relationships = vec![
            RelationshipData {
                rel_type: "has_role".to_string(),
                target: "role:admin".to_string(),
            },
            RelationshipData {
                rel_type: "member_of".to_string(),
                target: "group:engineers".to_string(),
            },
        ];

        let mut flags = std::collections::HashMap::new();
        flags.insert("is_active".to_string(), true);
        flags.insert("is_verified".to_string(), true);
        flags.insert("is_admin".to_string(), true);

        let original_data = EntityData {
            entity_type: "user".to_string(),
            string_attrs: string_attrs.clone(),
            numeric_attrs: numeric_attrs.clone(),
            relationships: relationships.clone(),
            flags: flags.clone(),
            metadata: std::collections::HashMap::new(),
        };

        // Convert to kernel entity
        let kernel_entity = loader
            .convert_to_kernel_entity("user:testuser", &original_data, EntityType::User)
            .unwrap();

        // Convert back to EntityData
        let recovered_data = loader.kernel_entity_to_data(&kernel_entity).unwrap();

        // Verify all fields match
        assert_eq!(recovered_data.entity_type, "user");
        assert_eq!(recovered_data.string_attrs.len(), 2);
        assert_eq!(
            recovered_data.string_attrs.get("email"),
            Some(&"test@example.com".to_string())
        );
        assert_eq!(
            recovered_data.string_attrs.get("name"),
            Some(&"Test User".to_string())
        );

        assert_eq!(recovered_data.numeric_attrs.len(), 2);
        assert_eq!(recovered_data.numeric_attrs.get("age"), Some(&42));
        assert_eq!(recovered_data.numeric_attrs.get("level"), Some(&5));

        assert_eq!(recovered_data.relationships.len(), 2);
        assert!(recovered_data
            .relationships
            .iter()
            .any(|r| r.rel_type == "has_role" && r.target == "role:admin"));
        assert!(recovered_data
            .relationships
            .iter()
            .any(|r| r.rel_type == "member_of" && r.target == "group:engineers"));

        // Check flags
        assert_eq!(recovered_data.flags.get("is_active"), Some(&true));
        assert_eq!(recovered_data.flags.get("is_verified"), Some(&true));
        assert_eq!(recovered_data.flags.get("is_admin"), Some(&true));
    }

    #[test]
    fn test_kernel_entity_to_data_empty() {
        // Test conversion with minimal data
        let loader = EntityLoader::new();

        let entity_data = EntityData {
            entity_type: "role".to_string(),
            string_attrs: std::collections::HashMap::new(),
            numeric_attrs: std::collections::HashMap::new(),
            relationships: vec![],
            flags: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
        };

        let kernel_entity = loader
            .convert_to_kernel_entity("role:viewer", &entity_data, EntityType::Role)
            .unwrap();

        let recovered_data = loader.kernel_entity_to_data(&kernel_entity).unwrap();

        assert_eq!(recovered_data.entity_type, "role");
        assert!(recovered_data.string_attrs.is_empty());
        assert!(recovered_data.numeric_attrs.is_empty());
        assert!(recovered_data.relationships.is_empty());
        assert!(recovered_data.flags.is_empty());
    }

    #[test]
    fn test_extract_cstring() {
        let loader = EntityLoader::new();

        // Test normal null-terminated string
        let bytes = b"hello\0world";
        let result = loader.extract_cstring(bytes).unwrap();
        assert_eq!(result, "hello");

        // Test string without null terminator (should read to end)
        let bytes = b"hello";
        let result = loader.extract_cstring(bytes).unwrap();
        assert_eq!(result, "hello");

        // Test empty string
        let bytes = b"\0";
        let result = loader.extract_cstring(bytes).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_batch_result_creation() {
        // Test BatchResult struct
        let result = BatchResult {
            succeeded: 10,
            failed: 2,
            errors: vec![
                ("entity1".to_string(), "Error 1".to_string()),
                ("entity2".to_string(), "Error 2".to_string()),
            ],
        };

        assert_eq!(result.succeeded, 10);
        assert_eq!(result.failed, 2);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn test_entity_type_to_u8_values() {
        // Verify EntityType::to_u8() returns correct kernel values
        assert_eq!(EntityType::User.to_u8(), 0);
        assert_eq!(EntityType::Role.to_u8(), 1);
        assert_eq!(EntityType::Group.to_u8(), 2);
        assert_eq!(EntityType::Resource.to_u8(), 3);
        assert_eq!(EntityType::Permission.to_u8(), 4);
        assert_eq!(EntityType::JwtSession.to_u8(), 5);
        assert_eq!(EntityType::Custom.to_u8(), 255);
    }

    #[test]
    fn test_round_trip_conversion_all_entity_types() {
        // Test conversion for all entity types (except Custom)
        let loader = EntityLoader::new();

        let entity_types = vec![
            (EntityType::User, "user"),
            (EntityType::Role, "role"),
            (EntityType::Group, "group"),
            (EntityType::Resource, "resource"),
            (EntityType::Permission, "permission"),
            (EntityType::JwtSession, "jwt_session"),
        ];

        for (entity_type, type_str) in entity_types {
            let mut string_attrs = std::collections::HashMap::new();
            string_attrs.insert("test_key".to_string(), "test_value".to_string());

            let entity_data = EntityData {
                entity_type: type_str.to_string(),
                string_attrs,
                numeric_attrs: std::collections::HashMap::new(),
                relationships: vec![],
                flags: std::collections::HashMap::new(),
                metadata: std::collections::HashMap::new(),
            };

            let kernel_entity = loader
                .convert_to_kernel_entity(&format!("{}:test", type_str), &entity_data, entity_type)
                .unwrap();

            let recovered_data = loader.kernel_entity_to_data(&kernel_entity).unwrap();

            assert_eq!(recovered_data.entity_type, type_str);
            assert_eq!(
                recovered_data.string_attrs.get("test_key"),
                Some(&"test_value".to_string())
            );
        }
    }

    // NOTE: Full CRUD integration tests (upsert_entity, delete_entity, get_entity, list_entities)
    // require actual eBPF maps which aren't available in unit tests.
    // These methods will be tested in integration tests when eBPF is initialized.
}
