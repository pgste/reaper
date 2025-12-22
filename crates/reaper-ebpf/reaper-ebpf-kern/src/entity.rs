//! Entity data structures for eBPF kernel space
//!
//! This module defines the core entity types that can be stored in eBPF maps
//! and used for policy evaluation in the kernel.
//!
//! Supports:
//! - JWT claims validation
//! - ReBAC (relationship-based access control)
//! - RBAC (role-based access control)
//! - ABAC (attribute-based access control)

#![no_std]

/// Maximum length of entity ID
pub const MAX_ENTITY_ID_LEN: usize = 64;

/// Maximum length of attribute key
pub const MAX_ATTR_KEY_LEN: usize = 32;

/// Maximum length of string attribute value
pub const MAX_STRING_VALUE_LEN: usize = 64;

/// Maximum number of string attributes per entity
pub const MAX_STRING_ATTRS: usize = 8;

/// Maximum number of numeric attributes per entity
pub const MAX_NUMERIC_ATTRS: usize = 8;

/// Maximum number of relationships per entity
pub const MAX_RELATIONSHIPS: usize = 8;

/// Entity type enumeration
///
/// Determines how the entity is used in policy evaluation
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    /// User entity (for authentication and authorization)
    User = 0,
    /// Role entity (for RBAC)
    Role = 1,
    /// Group entity (for hierarchical access)
    Group = 2,
    /// Resource entity (protected objects)
    Resource = 3,
    /// Permission entity (specific access rights)
    Permission = 4,
    /// JWT session entity (temporary authentication tokens)
    JwtSession = 5,
    /// Custom entity type
    Custom = 255,
}

/// String attribute (key-value pair)
///
/// Used for storing textual data like:
/// - JWT claims: "sub", "email", "name"
/// - ABAC attributes: "role", "dept", "location"
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StringAttr {
    /// Attribute key (null-terminated)
    pub key: [u8; MAX_ATTR_KEY_LEN],
    /// Attribute value (null-terminated)
    pub value: [u8; MAX_STRING_VALUE_LEN],
}

impl StringAttr {
    /// Create a new empty string attribute
    pub const fn new() -> Self {
        Self {
            key: [0u8; MAX_ATTR_KEY_LEN],
            value: [0u8; MAX_STRING_VALUE_LEN],
        }
    }

    /// Check if this attribute matches the given key
    #[inline(always)]
    pub fn matches_key(&self, key: &[u8]) -> bool {
        let mut i = 0;
        while i < MAX_ATTR_KEY_LEN && i < key.len() {
            if self.key[i] != key[i] {
                return false;
            }
            if self.key[i] == 0 {
                // Null terminator - key ends here
                return key.get(i) == Some(&0) || key.len() == i;
            }
            i += 1;
        }
        true
    }

    /// Check if this attribute's value matches the given value
    #[inline(always)]
    pub fn matches_value(&self, value: &[u8]) -> bool {
        let mut i = 0;
        while i < MAX_STRING_VALUE_LEN && i < value.len() {
            if self.value[i] != value[i] {
                return false;
            }
            if self.value[i] == 0 {
                // Null terminator - value ends here
                return value.get(i) == Some(&0) || value.len() == i;
            }
            i += 1;
        }
        true
    }
}

/// Numeric attribute (key-value pair)
///
/// Used for storing numeric data like:
/// - JWT timestamps: "exp", "iat", "nbf"
/// - ABAC numeric attributes: "age", "clearance_level"
/// - Counters and thresholds
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NumericAttr {
    /// Attribute key (null-terminated)
    pub key: [u8; MAX_ATTR_KEY_LEN],
    /// Numeric value (signed 64-bit integer)
    pub value: i64,
}

impl NumericAttr {
    /// Create a new empty numeric attribute
    pub const fn new() -> Self {
        Self {
            key: [0u8; MAX_ATTR_KEY_LEN],
            value: 0,
        }
    }

    /// Check if this attribute matches the given key
    #[inline(always)]
    pub fn matches_key(&self, key: &[u8]) -> bool {
        let mut i = 0;
        while i < MAX_ATTR_KEY_LEN && i < key.len() {
            if self.key[i] != key[i] {
                return false;
            }
            if self.key[i] == 0 {
                return key.get(i) == Some(&0) || key.len() == i;
            }
            i += 1;
        }
        true
    }
}

/// Relationship between entities (for ReBAC)
///
/// Used for representing graph-based access control:
/// - User → Role: "has_role"
/// - Role → Permission: "has_permission"
/// - User → Group: "member_of"
/// - Resource → Owner: "owned_by"
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Relationship {
    /// Relationship type (e.g., "has_role", "member_of", "owns")
    pub rel_type: [u8; MAX_ATTR_KEY_LEN],
    /// Target entity ID (what this relationship points to)
    pub target_id: [u8; MAX_ENTITY_ID_LEN],
}

impl Relationship {
    /// Create a new empty relationship
    pub const fn new() -> Self {
        Self {
            rel_type: [0u8; MAX_ATTR_KEY_LEN],
            target_id: [0u8; MAX_ENTITY_ID_LEN],
        }
    }

    /// Check if this relationship matches the given type
    #[inline(always)]
    pub fn matches_type(&self, rel_type: &[u8]) -> bool {
        let mut i = 0;
        while i < MAX_ATTR_KEY_LEN && i < rel_type.len() {
            if self.rel_type[i] != rel_type[i] {
                return false;
            }
            if self.rel_type[i] == 0 {
                return rel_type.get(i) == Some(&0) || rel_type.len() == i;
            }
            i += 1;
        }
        true
    }

    /// Check if this relationship points to the given target
    #[inline(always)]
    pub fn matches_target(&self, target: &[u8]) -> bool {
        let mut i = 0;
        while i < MAX_ENTITY_ID_LEN && i < target.len() {
            if self.target_id[i] != target[i] {
                return false;
            }
            if self.target_id[i] == 0 {
                return target.get(i) == Some(&0) || target.len() == i;
            }
            i += 1;
        }
        true
    }
}

/// Core entity structure
///
/// This unified structure supports JWT, RBAC, ABAC, and ReBAC.
/// Total size: ~2KB (fits in eBPF stack)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entity {
    /// Entity ID (e.g., "user:alice", "role:admin", "jwt:session_abc123")
    pub id: [u8; MAX_ENTITY_ID_LEN],

    /// Entity type
    pub entity_type: EntityType,

    /// String attributes (for ABAC and JWT string claims)
    pub string_attrs: [StringAttr; MAX_STRING_ATTRS],
    /// Number of valid string attributes
    pub string_count: u8,

    /// Numeric attributes (for ABAC and JWT numeric claims)
    pub numeric_attrs: [NumericAttr; MAX_NUMERIC_ATTRS],
    /// Number of valid numeric attributes
    pub numeric_count: u8,

    /// Relationships (for ReBAC and RBAC)
    pub relationships: [Relationship; MAX_RELATIONSHIPS],
    /// Number of valid relationships
    pub relationship_count: u8,

    /// Boolean flags (up to 64 boolean attributes)
    /// Bit 0: is_active
    /// Bit 1: is_verified
    /// Bit 2-63: custom flags
    pub flags: u64,

    /// Entity version (for updates)
    pub version: u32,

    /// Creation timestamp (nanoseconds since epoch)
    pub created_at: u64,

    /// Last update timestamp (nanoseconds since epoch)
    pub updated_at: u64,
}

impl Entity {
    /// Create a new empty entity
    pub const fn new() -> Self {
        Self {
            id: [0u8; MAX_ENTITY_ID_LEN],
            entity_type: EntityType::Custom,
            string_attrs: [StringAttr::new(); MAX_STRING_ATTRS],
            string_count: 0,
            numeric_attrs: [NumericAttr::new(); MAX_NUMERIC_ATTRS],
            numeric_count: 0,
            relationships: [Relationship::new(); MAX_RELATIONSHIPS],
            relationship_count: 0,
            flags: 0,
            version: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Find a string attribute by key
    #[inline(always)]
    pub fn get_string_attr(&self, key: &[u8]) -> Option<&StringAttr> {
        for i in 0..self.string_count {
            let attr = &self.string_attrs[i as usize];
            if attr.matches_key(key) {
                return Some(attr);
            }
        }
        None
    }

    /// Find a numeric attribute by key
    #[inline(always)]
    pub fn get_numeric_attr(&self, key: &[u8]) -> Option<&NumericAttr> {
        for i in 0..self.numeric_count {
            let attr = &self.numeric_attrs[i as usize];
            if attr.matches_key(key) {
                return Some(attr);
            }
        }
        None
    }

    /// Check if a string attribute equals a value
    #[inline(always)]
    pub fn string_attr_equals(&self, key: &[u8], value: &[u8]) -> bool {
        if let Some(attr) = self.get_string_attr(key) {
            attr.matches_value(value)
        } else {
            false
        }
    }

    /// Check if a numeric attribute equals a value
    #[inline(always)]
    pub fn numeric_attr_equals(&self, key: &[u8], value: i64) -> bool {
        if let Some(attr) = self.get_numeric_attr(key) {
            attr.value == value
        } else {
            false
        }
    }

    /// Check if a numeric attribute is greater than or equal to a value
    #[inline(always)]
    pub fn numeric_attr_gte(&self, key: &[u8], value: i64) -> bool {
        if let Some(attr) = self.get_numeric_attr(key) {
            attr.value >= value
        } else {
            false
        }
    }

    /// Check if a numeric attribute is less than or equal to a value
    #[inline(always)]
    pub fn numeric_attr_lte(&self, key: &[u8], value: i64) -> bool {
        if let Some(attr) = self.get_numeric_attr(key) {
            attr.value <= value
        } else {
            false
        }
    }

    /// Find a relationship by type and target
    #[inline(always)]
    pub fn has_relationship(&self, rel_type: &[u8], target: &[u8]) -> bool {
        for i in 0..self.relationship_count {
            let rel = &self.relationships[i as usize];
            if rel.matches_type(rel_type) && rel.matches_target(target) {
                return true;
            }
        }
        false
    }

    /// Find any relationship by type (returns first match)
    #[inline(always)]
    pub fn get_relationship(&self, rel_type: &[u8]) -> Option<&Relationship> {
        for i in 0..self.relationship_count {
            let rel = &self.relationships[i as usize];
            if rel.matches_type(rel_type) {
                return Some(rel);
            }
        }
        None
    }

    /// Check if a flag bit is set
    #[inline(always)]
    pub fn has_flag(&self, bit: u8) -> bool {
        if bit >= 64 {
            return false;
        }
        (self.flags & (1u64 << bit)) != 0
    }

    /// JWT-specific: Check if token is expired
    /// Compares "exp" claim against current time
    #[inline(always)]
    pub fn is_jwt_expired(&self, current_time_ns: u64) -> bool {
        if let Some(exp_attr) = self.get_numeric_attr(b"exp\0") {
            // exp is in seconds, convert current_time from ns to seconds
            let current_time_s = current_time_ns / 1_000_000_000;
            exp_attr.value < (current_time_s as i64)
        } else {
            // No exp claim - consider it expired
            true
        }
    }

    /// JWT-specific: Check if token is not yet valid
    /// Compares "nbf" (not before) claim against current time
    #[inline(always)]
    pub fn is_jwt_not_yet_valid(&self, current_time_ns: u64) -> bool {
        if let Some(nbf_attr) = self.get_numeric_attr(b"nbf\0") {
            let current_time_s = current_time_ns / 1_000_000_000;
            nbf_attr.value > (current_time_s as i64)
        } else {
            // No nbf claim - consider it valid
            false
        }
    }
}

// Ensure Entity fits in reasonable memory
const _: () = {
    const ENTITY_SIZE: usize = core::mem::size_of::<Entity>();
    // Entity should be <= 2KB to fit in eBPF stack
    const _: [(); 1] = [(); (ENTITY_SIZE <= 2048) as usize];
};
