//! Entity data structures for userspace
//!
//! This module defines the JSON input format and conversion logic for entities
//! that will be stored in eBPF maps.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Entity constants (must match kernel definitions)
pub const MAX_ENTITY_ID_LEN: usize = 64;
pub const MAX_ATTR_KEY_LEN: usize = 32;
pub const MAX_STRING_VALUE_LEN: usize = 64;
pub const MAX_STRING_ATTRS: usize = 8;
pub const MAX_NUMERIC_ATTRS: usize = 8;
pub const MAX_RELATIONSHIPS: usize = 8;

/// Complete entity dataset (top-level JSON structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDataset {
    /// Dataset name/identifier
    pub dataset: String,

    /// Dataset version
    pub version: String,

    /// All entities in the dataset
    pub entities: HashMap<String, EntityData>,

    /// Optional metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Single entity data (JSON input format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    /// Entity type
    #[serde(rename = "type")]
    pub entity_type: String,

    /// String attributes (for ABAC, JWT string claims)
    #[serde(default)]
    pub string_attrs: HashMap<String, String>,

    /// Numeric attributes (for ABAC, JWT numeric claims like exp/iat)
    #[serde(default)]
    pub numeric_attrs: HashMap<String, i64>,

    /// Relationships (for ReBAC, RBAC)
    #[serde(default)]
    pub relationships: Vec<RelationshipData>,

    /// Boolean flags
    #[serde(default)]
    pub flags: HashMap<String, bool>,

    /// Optional metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Relationship data (JSON input format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipData {
    /// Relationship type (e.g., "has_role", "member_of", "owns")
    #[serde(rename = "type")]
    pub rel_type: String,

    /// Target entity ID
    pub target: String,
}

/// Entity type enumeration (matches kernel definition)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    User,
    Role,
    Group,
    Resource,
    Permission,
    #[serde(rename = "jwt_session")]
    JwtSession,
    Custom,
}

impl EntityType {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(EntityType::User),
            "role" => Some(EntityType::Role),
            "group" => Some(EntityType::Group),
            "resource" => Some(EntityType::Resource),
            "permission" => Some(EntityType::Permission),
            "jwt_session" | "jwt" | "session" => Some(EntityType::JwtSession),
            "custom" => Some(EntityType::Custom),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            EntityType::User => 0,
            EntityType::Role => 1,
            EntityType::Group => 2,
            EntityType::Resource => 3,
            EntityType::Permission => 4,
            EntityType::JwtSession => 5,
            EntityType::Custom => 255,
        }
    }
}

/// Data tier selection based on dataset size
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataTier {
    /// Tier 1: Direct maps (< 10K entities)
    Tier1Direct,
    /// Tier 2: Sharded maps (10K-100K entities)
    Tier2Sharded,
    /// Tier 3: Partitioned with bloom filter (100K-1M entities)
    Tier3Partitioned,
}

impl DataTier {
    /// Determine tier based on entity count
    pub fn from_count(count: usize) -> Self {
        match count {
            0..=10_000 => DataTier::Tier1Direct,
            10_001..=100_000 => DataTier::Tier2Sharded,
            _ => DataTier::Tier3Partitioned,
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            DataTier::Tier1Direct => "Tier 1: Direct Maps",
            DataTier::Tier2Sharded => "Tier 2: Sharded Maps",
            DataTier::Tier3Partitioned => "Tier 3: Partitioned + Bloom",
        }
    }

    /// Get expected lookup latency
    pub fn latency_ns(&self) -> u32 {
        match self {
            DataTier::Tier1Direct => 50,
            DataTier::Tier2Sharded => 100,
            DataTier::Tier3Partitioned => 150,
        }
    }
}

/// Validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Overall validation status
    pub valid: bool,

    /// Validation errors (must be empty for valid dataset)
    pub errors: Vec<String>,

    /// Validation warnings (dataset is valid but has issues)
    pub warnings: Vec<String>,

    /// Recommended tier for this dataset
    pub tier: DataTier,

    /// Estimated memory usage in bytes
    pub estimated_memory: usize,

    /// Number of entities in dataset
    pub entity_count: usize,

    /// Breakdown by entity type
    pub entity_types: HashMap<String, usize>,
}

impl ValidationResult {
    pub fn new() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            tier: DataTier::Tier1Direct,
            estimated_memory: 0,
            entity_count: 0,
            entity_types: HashMap::new(),
        }
    }

    /// Add an error (makes validation fail)
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.valid = false;
    }

    /// Add a warning (doesn't fail validation)
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics from loading entities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadStats {
    pub users: usize,
    pub roles: usize,
    pub groups: usize,
    pub resources: usize,
    pub permissions: usize,
    pub jwt_sessions: usize,
    pub custom: usize,
    pub total: usize,
    pub errors: usize,
}

impl LoadStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn increment(&mut self, entity_type: EntityType) {
        match entity_type {
            EntityType::User => self.users += 1,
            EntityType::Role => self.roles += 1,
            EntityType::Group => self.groups += 1,
            EntityType::Resource => self.resources += 1,
            EntityType::Permission => self.permissions += 1,
            EntityType::JwtSession => self.jwt_sessions += 1,
            EntityType::Custom => self.custom += 1,
        }
        self.total += 1;
    }
}

/// JWT-specific helpers
pub mod jwt {
    use super::*;

    /// Standard JWT claims
    pub const CLAIM_SUB: &str = "sub"; // Subject
    pub const CLAIM_ISS: &str = "iss"; // Issuer
    pub const CLAIM_AUD: &str = "aud"; // Audience
    pub const CLAIM_EXP: &str = "exp"; // Expiration time
    pub const CLAIM_NBF: &str = "nbf"; // Not before
    pub const CLAIM_IAT: &str = "iat"; // Issued at
    pub const CLAIM_JTI: &str = "jti"; // JWT ID

    /// Common custom claims
    pub const CLAIM_EMAIL: &str = "email";
    pub const CLAIM_NAME: &str = "name";
    pub const CLAIM_ROLE: &str = "role";
    pub const CLAIM_ROLES: &str = "roles";

    /// Create a JWT session entity from claims
    pub fn create_jwt_entity(
        _session_id: &str,
        claims: HashMap<String, serde_json::Value>,
    ) -> EntityData {
        let mut entity = EntityData {
            entity_type: "jwt_session".to_string(),
            string_attrs: HashMap::new(),
            numeric_attrs: HashMap::new(),
            relationships: Vec::new(),
            flags: HashMap::new(),
            metadata: HashMap::new(),
        };

        // Parse claims
        for (key, value) in claims {
            match value {
                serde_json::Value::String(s) => {
                    entity.string_attrs.insert(key, s);
                }
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        entity.numeric_attrs.insert(key, i);
                    }
                }
                serde_json::Value::Bool(b) => {
                    entity.flags.insert(key, b);
                }
                serde_json::Value::Array(arr) => {
                    // Convert array to space-separated string
                    let joined = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !joined.is_empty() {
                        entity.string_attrs.insert(key, joined);
                    }
                }
                _ => {
                    // Skip complex types
                }
            }
        }

        entity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_type_parsing() {
        assert_eq!(EntityType::parse("user"), Some(EntityType::User));
        assert_eq!(EntityType::parse("role"), Some(EntityType::Role));
        assert_eq!(
            EntityType::parse("jwt_session"),
            Some(EntityType::JwtSession)
        );
        assert_eq!(EntityType::parse("jwt"), Some(EntityType::JwtSession));
        assert_eq!(EntityType::parse("invalid"), None);
    }

    #[test]
    fn test_data_tier_selection() {
        assert_eq!(DataTier::from_count(100), DataTier::Tier1Direct);
        assert_eq!(DataTier::from_count(5_000), DataTier::Tier1Direct);
        assert_eq!(DataTier::from_count(10_000), DataTier::Tier1Direct);
        assert_eq!(DataTier::from_count(10_001), DataTier::Tier2Sharded);
        assert_eq!(DataTier::from_count(50_000), DataTier::Tier2Sharded);
        assert_eq!(DataTier::from_count(100_001), DataTier::Tier3Partitioned);
    }

    #[test]
    fn test_jwt_entity_creation() {
        let mut claims = HashMap::new();
        claims.insert(
            "sub".to_string(),
            serde_json::Value::String("user123".to_string()),
        );
        claims.insert(
            "exp".to_string(),
            serde_json::Value::Number(1735689600.into()),
        );
        claims.insert("email_verified".to_string(), serde_json::Value::Bool(true));

        let entity = jwt::create_jwt_entity("session_abc", claims);

        assert_eq!(entity.entity_type, "jwt_session");
        assert_eq!(entity.string_attrs.get("sub"), Some(&"user123".to_string()));
        assert_eq!(entity.numeric_attrs.get("exp"), Some(&1735689600));
        assert_eq!(entity.flags.get("email_verified"), Some(&true));
    }
}
