# Reaper eBPF Data Schema

This document defines the unified schema for storing entities in eBPF maps, supporting JWT claims, RBAC, ABAC, and ReBAC models.

## Core Entity Schema

All entities (users, roles, groups, resources, etc.) follow this unified structure:

```rust
/// Maximum string attribute length
const MAX_STRING_LEN: usize = 64;

/// Maximum number of string attributes per entity
const MAX_STRING_ATTRS: usize = 8;

/// Maximum number of numeric attributes per entity
const MAX_NUMERIC_ATTRS: usize = 8;

/// Maximum number of relationships per entity
const MAX_RELATIONSHIPS: usize = 16;

/// Entity type enumeration
#[repr(u8)]
enum EntityType {
    User = 0,
    Role = 1,
    Group = 2,
    Resource = 3,
    Permission = 4,
    Custom = 255,
}

/// String attribute (key-value pair)
#[repr(C)]
struct StringAttr {
    key: [u8; 32],      // Attribute name
    value: [u8; 64],    // Attribute value
}

/// Numeric attribute (key-value pair)
#[repr(C)]
struct NumericAttr {
    key: [u8; 32],      // Attribute name
    value: i64,         // Numeric value (supports negative)
}

/// Relationship (for ReBAC)
#[repr(C)]
struct Relationship {
    rel_type: [u8; 32], // Relationship type: "member_of", "owns", "manages"
    target_id: [u8; 64], // Target entity ID
}

/// Unified entity structure
#[repr(C)]
struct Entity {
    // Identity
    id: [u8; 64],
    entity_type: EntityType,

    // Attributes (for ABAC)
    string_attrs: [StringAttr; MAX_STRING_ATTRS],
    string_count: u8,

    numeric_attrs: [NumericAttr; MAX_NUMERIC_ATTRS],
    numeric_count: u8,

    // Relationships (for ReBAC)
    relationships: [Relationship; MAX_RELATIONSHIPS],
    relationship_count: u8,

    // Flags (for boolean attributes, up to 64)
    flags: u64,

    // Metadata
    version: u32,
    created_at: u64,
    updated_at: u64,
}
```

## Supported Use Cases

### 1. JWT Claims

JWT tokens parsed into entity attributes:

```json
{
  "entities": {
    "jwt:session_abc123": {
      "type": "user",
      "string_attrs": {
        "sub": "user@example.com",
        "name": "Alice Smith",
        "email": "alice@example.com",
        "role": "admin"
      },
      "numeric_attrs": {
        "exp": 1735689600,
        "iat": 1735603200,
        "nbf": 1735603200
      },
      "flags": {
        "email_verified": true,
        "active": true
      }
    }
  }
}
```

eBPF can check:
- `jwt.exp > current_time` (token not expired)
- `jwt.role == "admin"` (RBAC check)
- `jwt.email_verified == true` (flag check)

### 2. RBAC (Role-Based Access Control)

```json
{
  "entities": {
    "user:alice": {
      "type": "user",
      "string_attrs": {
        "email": "alice@example.com",
        "dept": "engineering"
      },
      "relationships": [
        {"type": "has_role", "target": "role:admin"},
        {"type": "member_of", "target": "group:eng-team"}
      ]
    },
    "role:admin": {
      "type": "role",
      "relationships": [
        {"type": "has_permission", "target": "perm:users.write"},
        {"type": "has_permission", "target": "perm:users.read"},
        {"type": "has_permission", "target": "perm:resources.delete"}
      ]
    },
    "perm:users.write": {
      "type": "permission",
      "string_attrs": {
        "resource": "users",
        "action": "write"
      }
    }
  }
}
```

eBPF can check:
- Direct: `user.relationships[].target == "role:admin"`
- Two-hop: User → Role → Permission (requires 2 lookups)

### 3. ABAC (Attribute-Based Access Control)

```json
{
  "entities": {
    "user:bob": {
      "type": "user",
      "string_attrs": {
        "dept": "engineering",
        "location": "us-west"
      },
      "numeric_attrs": {
        "clearance_level": 7,
        "years_of_service": 5
      },
      "flags": {
        "is_contractor": false,
        "background_check_passed": true
      }
    },
    "resource:doc123": {
      "type": "resource",
      "string_attrs": {
        "classification": "confidential",
        "dept": "engineering",
        "owner": "bob"
      },
      "numeric_attrs": {
        "required_clearance": 5
      }
    }
  }
}
```

eBPF can check:
- `user.clearance_level >= resource.required_clearance`
- `user.dept == resource.dept`
- `user.background_check_passed == true`

### 4. ReBAC (Relationship-Based Access Control)

```json
{
  "entities": {
    "user:charlie": {
      "type": "user",
      "relationships": [
        {"type": "member_of", "target": "group:eng-managers"},
        {"type": "reports_to", "target": "user:alice"}
      ]
    },
    "group:eng-managers": {
      "type": "group",
      "relationships": [
        {"type": "can_access", "target": "resource:sensitive-docs"},
        {"type": "parent_group", "target": "group:all-managers"}
      ]
    },
    "resource:sensitive-docs": {
      "type": "resource",
      "string_attrs": {
        "classification": "restricted"
      },
      "relationships": [
        {"type": "owned_by", "target": "group:eng-managers"}
      ]
    }
  }
}
```

eBPF can check:
- Direct membership: `user.relationships[] contains "member_of:group:eng-managers"`
- Resource ownership: `resource.relationships[] contains "owned_by:group:eng-managers"`
- Transitive relationships (limited depth in eBPF)

## Schema Validation Rules

When ingesting data, the userspace component validates:

1. **Type Safety**
   - Entity type must be valid
   - Attribute counts must not exceed maximums
   - Relationship targets must exist

2. **Size Constraints**
   - String attributes ≤ 64 bytes
   - Attribute names ≤ 32 bytes
   - IDs ≤ 64 bytes
   - Total entity size ≤ 2KB (eBPF stack limit)

3. **Relationship Integrity**
   - Relationship targets must reference existing entities
   - Circular relationships allowed but tracked
   - Relationship types must be consistent

4. **Flag Consistency**
   - Flag names must be registered
   - Boolean values only
   - Max 64 flags per entity

## eBPF Map Structures

### Tier 1: Small Datasets (< 10K)

Single map per entity type:

```rust
#[map]
static USERS: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(10000, 0);

#[map]
static ROLES: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(1000, 0);

#[map]
static RESOURCES: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(10000, 0);
```

### Tier 2: Medium Datasets (10K-100K)

Sharded maps using map-in-map:

```rust
// Outer map: shard index → inner map FD
#[map]
static ENTITY_SHARDS: HashMap<u32, u32> =
    HashMap::with_max_entries(16, 0);

// Each inner map holds ~6250 entities (100K / 16 shards)
// Shard selection: hash(entity_id) % 16
```

### Tier 3: Large Datasets (100K-1M)

Bloom filter + partitioned maps + userspace LRU:

```rust
// Bloom filter for negative lookups
#[map]
static ENTITY_BLOOM: HashMap<u32, u64> =
    HashMap::with_max_entries(1024, 0);  // 8KB bloom filter

// Partitioned maps (32 partitions)
#[map]
static ENTITY_PARTITION_0: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(31250, 0);
// ... ENTITY_PARTITION_1 through ENTITY_PARTITION_31

// Userspace LRU cache (10K hottest entities)
```

## Lookup Patterns

### Simple Attribute Lookup

Policy: `user.role == "admin"`

```rust
// 1. Lookup entity
let user = USERS.get(user_id)?;

// 2. Find string attribute "role"
for i in 0..user.string_count {
    if user.string_attrs[i].key == b"role\0" {
        if user.string_attrs[i].value == b"admin\0" {
            return PolicyAction::Allow;
        }
    }
}
```

### Relationship Traversal

Policy: `user has_role "admin"`

```rust
// 1. Lookup user
let user = USERS.get(user_id)?;

// 2. Find relationship "has_role"
for i in 0..user.relationship_count {
    if user.relationships[i].rel_type == b"has_role\0" {
        if user.relationships[i].target_id == b"role:admin\0" {
            return PolicyAction::Allow;
        }
    }
}
```

### Two-Hop Traversal (RBAC)

Policy: `user has_role with has_permission "users.write"`

```rust
// 1. Lookup user
let user = USERS.get(user_id)?;

// 2. Find roles
for i in 0..user.relationship_count {
    if user.relationships[i].rel_type == b"has_role\0" {
        let role_id = &user.relationships[i].target_id;

        // 3. Lookup role
        if let Some(role) = ROLES.get(role_id) {
            // 4. Check permissions
            for j in 0..role.relationship_count {
                if role.relationships[j].rel_type == b"has_permission\0" {
                    if role.relationships[j].target_id == b"perm:users.write\0" {
                        return PolicyAction::Allow;
                    }
                }
            }
        }
    }
}
```

**Note**: eBPF limits traversal depth to ~3 hops due to stack constraints.

## JSON Input Format

Userspace accepts this JSON format:

```json
{
  "dataset": "production",
  "version": "1.0",
  "entities": {
    "user:alice": {
      "type": "user",
      "string_attrs": {
        "email": "alice@example.com",
        "dept": "engineering",
        "role": "admin"
      },
      "numeric_attrs": {
        "age": 30,
        "clearance": 7
      },
      "relationships": [
        {"type": "member_of", "target": "group:eng-team"},
        {"type": "has_role", "target": "role:admin"}
      ],
      "flags": {
        "active": true,
        "verified": true
      }
    }
  }
}
```

## Validation CLI

```bash
# Validate dataset schema
reaper-cli validate-data --file users.json --schema entity

# Check if data fits in eBPF constraints
reaper-cli validate-data --file users.json --check-ebpf

# Output:
# ✓ Schema valid (entity v1.0)
# ✓ 1,234 entities
# ✓ Fits in eBPF (Tier 1: Direct maps)
# ✓ Estimated memory: 2.4 MB
# ⚠ Entity 'user:bob' has 9 string attrs (max 8)
# ✗ Entity 'user:charlie' string attr 'description' exceeds 64 bytes
```

## Performance Characteristics

| Tier | Size | Lookup | Memory | Promotion |
|------|------|--------|--------|-----------|
| 1 | < 10K | 50ns | ~20MB | Always |
| 2 | 10K-100K | 100ns | ~200MB | Auto (>80% hit) |
| 3 | 100K-1M | 150ns | ~2GB | Selective (LRU) |

## Best Practices

1. **Keep entities small**: Use < 8 string attrs, < 8 numeric attrs
2. **Limit relationship depth**: Max 2-3 hops in eBPF
3. **Use flags for booleans**: Much faster than string comparisons
4. **Normalize IDs**: Consistent format (e.g., "user:alice", "role:admin")
5. **Index hot paths**: Auto-promotion will optimize based on access patterns
6. **Pre-validate data**: Use CLI validation before deployment

## Next Steps

- [Policy Compilation](./POLICY_COMPILATION.md)
- [Deployment Guide](./EBPF_DEPLOYMENT.md)
- [Performance Tuning](./PERFORMANCE.md)
