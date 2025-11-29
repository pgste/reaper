# Multi-Entity Type Policy Architecture

**Status:** Design Proposal
**Author:** Claude
**Date:** 2025-11-25
**Version:** 1.0

---

## Executive Summary

This document proposes an architecture for extending Reaper's policy engine to support **multiple entity types** in policy evaluation beyond the current `principal` and `resource` model. This enables policies that reference arbitrary entities like `user`, `machine`, `department`, `location`, etc.

**Key Benefits:**
- **Flexible Entity Model**: Support any number of entity types in policies
- **Zero Trust Architecture**: Enable device trust, network segmentation, location-based policies
- **Multi-Party Authorization**: Support approval workflows, delegation, multi-factor checks
- **Backwards Compatible**: Existing policies continue to work unchanged
- **Performance Optimized**: Target <1µs for 5-entity policies

---

## Table of Contents

1. [Current Architecture](#current-architecture)
2. [Problem Statement](#problem-statement)
3. [Proposed Solution](#proposed-solution)
4. [Implementation Details](#implementation-details)
5. [Performance Considerations](#performance-considerations)
6. [Migration Path](#migration-path)
7. [Use Cases](#use-cases)
8. [Timeline & Roadmap](#timeline--roadmap)

---

## Current Architecture

### Request Structure

```rust
pub struct PolicyRequest {
    pub resource: String,
    pub action: String,
    pub context: HashMap<String, String>,
}
```

**Context contains:**
- `principal`: User/service identifier

### DSL Syntax

```reap
policy current_example {
    rule simple_access {
        allow if {
            user.is_active == true &&
            resource.classification == "public"
        }
    }
}
```

### Limitations

1. **Fixed Entity Types**: Can only reference `user` (principal) and `resource`
2. **No Device Context**: Cannot check device trust, OS version, encryption status
3. **No Network Context**: Cannot enforce network segmentation policies
4. **No Multi-Entity Rules**: Cannot check relationships between >2 entities
5. **Context Limitations**: Context is flat string map, not entity-based

---

## Problem Statement

### Real-World Scenarios Requiring Multi-Entity Policies

**Scenario 1: Zero Trust Device Check**
```
Allow user Alice to access document D only if:
- Alice is active
- Alice's device has trustscore >= 75
- Device is encrypted and up-to-date
- Document classification <= device clearance
```

**Scenario 2: Network Segmentation**
```
Allow resource access only if:
- User and resource are in same network segment
- Access originates from trusted location
- Network firewall policy allows connection
```

**Scenario 3: Multi-Party Authorization**
```
Allow sensitive operation if:
- User requests action
- Manager approves request
- Both user and manager in same department
- Approval is recent (<24 hours)
```

**Current Workaround:**
Flatten all entity data into user context → **Poor separation of concerns, data duplication, stale data issues**

---

## Proposed Solution

### 1. Request Structure - Entity Map Approach

```rust
pub struct PolicyRequest {
    /// Core action being requested
    pub action: String,

    /// Entity references (flexible, supports any entity type)
    /// Maps entity_type -> entity_id
    pub entities: HashMap<String, String>,

    /// Additional context (non-entity data like timestamps, IPs)
    pub context: HashMap<String, AttributeValue>,

    /// Backwards compatibility aliases
    pub principal: String,   // Alias for entities["user"]
    pub resource: String,    // Alias for entities["resource"]
}
```

**Example Request:**

```rust
PolicyRequest {
    action: "read",
    entities: hashmap! {
        "user" => "alice",
        "resource" => "doc_123",
        "machine" => "laptop_456",
        "department" => "engineering",
        "location" => "office_sf",
    },
    context: hashmap! {
        "ip_address" => AttributeValue::String("192.168.1.1"),
        "time_of_day" => AttributeValue::String("business_hours"),
        "request_timestamp" => AttributeValue::Int(1700000000),
    },
    principal: "alice",      // Backwards compat
    resource: "doc_123",     // Backwards compat
}
```

---

### 2. DSL Syntax - Reserved Entity Keywords

**Approach:** Use dot notation with reserved keywords mapping to entity types

```reap
policy machine_trust_policy {
    version: "1.0.0",
    description: "Multi-entity policy with user, machine, and resource",

    // Optional: Declare required entities for validation
    requires: ["user", "machine", "resource"],

    default: deny,

    // Rule 1: Check attributes from 3 different entities
    rule trusted_machine_access {
        allow if {
            // User entity attributes
            user.is_active == true &&
            user.clearance >= 3 &&

            // Machine entity attributes
            machine.trustscore > 50 &&
            machine.status == "compliant" &&
            machine.encryption_enabled == true &&

            // Resource entity attributes
            resource.classification != "secret" &&
            resource.requires_trusted_device == true
        }
    }

    // Rule 2: Cross-entity attribute matching
    rule department_alignment {
        allow if {
            user.department == machine.assigned_department &&
            user.department == resource.owner_department
        }
    }

    // Rule 3: Context variables (non-entity data)
    rule time_and_location {
        allow if {
            context.time_of_day == "business_hours" &&
            context.ip_range == "corporate" &&
            location.security_zone == "trusted"
        }
    }
}
```

**Reserved Entity Keywords:**

| Keyword | Description | Example Attributes |
|---------|-------------|-------------------|
| `user` / `principal` | User or service account | `is_active`, `role`, `department`, `clearance` |
| `resource` | Target resource | `classification`, `owner`, `type`, `sensitivity` |
| `machine` / `device` / `endpoint` | Device used for access | `trustscore`, `os_version`, `encryption_enabled` |
| `department` / `organization` | Organizational unit | `budget`, `location`, `security_level` |
| `location` / `network` | Physical/network location | `security_zone`, `country`, `ip_range` |
| `role` / `group` | Role or group entity | `permissions`, `level`, `scope` |
| `approver` / `delegator` | Authorization entities | `approval_time`, `delegation_scope` |

**Context Variables:** Use `context.` prefix for non-entity data

```reap
rule contextual_checks {
    allow if {
        context.ip_address.startsWith("10.0.") &&
        context.request_timestamp < 1700000000 &&
        context.mfa_verified == true
    }
}
```

---

### 3. Evaluator Architecture

#### Current Flow (2 entities)

```
PolicyRequest
    ↓
Lookup principal → Entity (user)
    ↓
Lookup resource → Entity (resource)
    ↓
Evaluate policy with 2 entities
    ↓
PolicyDecision
```

#### New Flow (N entities)

```rust
impl Evaluator {
    pub fn evaluate(
        &self,
        policy: &Policy,
        request: &PolicyRequest,
        store: &DataStore,
    ) -> Result<PolicyDecision> {
        // PHASE 1: Resolve all entity references
        let mut resolved_entities = HashMap::new();

        for (entity_type, entity_id) in &request.entities {
            let entity_id_interned = store.interner().intern(entity_id);

            match store.get(entity_id_interned) {
                Some(entity) => {
                    // Validate entity type matches expectation
                    let expected_type = store.interner().intern(entity_type);
                    if entity.entity_type != expected_type {
                        return Err(ReaperError::EntityTypeMismatch {
                            expected: entity_type.clone(),
                            found: store.interner().resolve(entity.entity_type).to_string(),
                        });
                    }
                    resolved_entities.insert(entity_type.clone(), entity);
                }
                None => {
                    return Err(ReaperError::EntityNotFound {
                        entity_type: entity_type.clone(),
                        entity_id: entity_id.clone(),
                    });
                }
            }
        }

        // PHASE 2: Build evaluation context
        let eval_context = EvaluationContext {
            entities: resolved_entities,
            context: request.context.clone(),
            action: request.action.clone(),
        };

        // PHASE 3: Validate required entities
        if let Some(required) = &policy.requires_entities {
            for required_type in required {
                if !eval_context.entities.contains_key(required_type) {
                    return Err(ReaperError::MissingRequiredEntity {
                        entity_type: required_type.clone(),
                    });
                }
            }
        }

        // PHASE 4: Evaluate policy rules
        self.evaluate_rules(policy, &eval_context, store)
    }
}
```

**Evaluation Context:**

```rust
pub struct EvaluationContext<'a> {
    /// Resolved entities by type
    pub entities: HashMap<String, &'a Entity>,

    /// Context variables (non-entity data)
    pub context: HashMap<String, AttributeValue>,

    /// Action being performed
    pub action: String,
}
```

---

### 4. DSL Parser Changes

#### Identifier Resolution

```rust
#[derive(Debug, Clone)]
enum Identifier {
    /// Entity attribute: user.is_active, machine.trustscore
    EntityAttribute {
        entity_type: String,
        attribute: String,
    },

    /// Context variable: context.ip_address
    ContextVariable {
        key: String,
    },
}

impl Parser {
    fn parse_identifier(&mut self, expr: &str) -> Result<Identifier> {
        let parts: Vec<&str> = expr.split('.').collect();

        if parts.len() != 2 {
            return Err(ParseError::InvalidIdentifier {
                expr: expr.to_string(),
                reason: "Expected format: entity.attribute or context.key".to_string(),
            });
        }

        let prefix = parts[0];
        let suffix = parts[1];

        if prefix == "context" {
            Ok(Identifier::ContextVariable {
                key: suffix.to_string(),
            })
        } else if ENTITY_TYPE_KEYWORDS.contains(&prefix) {
            Ok(Identifier::EntityAttribute {
                entity_type: prefix.to_string(),
                attribute: suffix.to_string(),
            })
        } else {
            Err(ParseError::UnknownEntityType {
                entity_type: prefix.to_string(),
                valid_types: ENTITY_TYPE_KEYWORDS.to_vec(),
            })
        }
    }
}

const ENTITY_TYPE_KEYWORDS: &[&str] = &[
    "user", "principal",
    "resource",
    "machine", "device", "endpoint",
    "department", "organization", "team",
    "location", "network",
    "role", "group",
    "approver", "delegator",
];
```

#### AST Representation

```rust
#[derive(Debug, Clone)]
enum Expression {
    EntityAttribute {
        entity_type: InternedString,
        attribute: InternedString,
    },
    ContextVariable {
        key: InternedString,
    },
    Comparison {
        left: Box<Expression>,
        op: ComparisonOp,
        right: Box<Expression>,
    },
    // ... other expression types
}
```

---

### 5. DataStore Query Optimization

#### Problem
With N entity types, we need N lookups per policy evaluation. This could become expensive.

#### Solution 1: Batch Entity Loading

```rust
impl DataStore {
    /// Load multiple entities in a single pass
    pub fn get_entities_batch(
        &self,
        entity_ids: &[InternedString],
    ) -> HashMap<InternedString, &Entity> {
        let mut result = HashMap::new();

        // Single pass through entity map
        for id in entity_ids {
            if let Some(entity) = self.entities.get(id) {
                result.insert(*id, entity);
            }
        }

        result
    }

    /// Validate entity types and load in one operation
    pub fn get_entities_typed(
        &self,
        requests: &[(String, String)],  // (type, id) pairs
    ) -> Result<HashMap<String, &Entity>> {
        // Validates types and loads entities efficiently
    }
}
```

**Performance:** O(N) where N = number of entities, single map lookup per entity

#### Solution 2: Entity Caching

```rust
use lru::LruCache;

pub struct EntityCache {
    cache: LruCache<InternedString, Arc<Entity>>,
    ttl: Duration,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
}

impl EntityCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            cache: LruCache::new(capacity),
            ttl,
            hit_count: AtomicU64::new(0),
            miss_count: AtomicU64::new(0),
        }
    }

    pub fn get_or_load(
        &mut self,
        entity_id: InternedString,
        store: &DataStore,
    ) -> Option<Arc<Entity>> {
        // Check cache first
        if let Some(cached) = self.cache.get(&entity_id) {
            self.hit_count.fetch_add(1, Ordering::Relaxed);
            return Some(Arc::clone(cached));
        }

        // Load from store
        if let Some(entity) = store.get(entity_id) {
            let entity_arc = Arc::new(entity.clone());
            self.cache.put(entity_id, Arc::clone(&entity_arc));
            self.miss_count.fetch_add(1, Ordering::Relaxed);
            Some(entity_arc)
        } else {
            None
        }
    }
}
```

**Performance:** O(1) cache hit, O(1) cache miss + store lookup

#### Solution 3: Pre-Joined Entity Views

```rust
pub struct JoinedEntityView {
    /// All entities involved in evaluation
    entities: HashMap<String, Entity>,

    /// Flattened attribute map for fast lookups
    /// Format: "entity_type.attribute" -> AttributeValue
    flat_attributes: HashMap<String, AttributeValue>,
}

impl DataStore {
    pub fn create_view(
        &self,
        entity_requests: &HashMap<String, String>,
    ) -> Result<JoinedEntityView> {
        let mut entities = HashMap::new();
        let mut flat_attributes = HashMap::new();

        // Load all entities
        for (entity_type, entity_id) in entity_requests {
            let entity_id_interned = self.interner.intern(entity_id);
            if let Some(entity) = self.get(entity_id_interned) {
                // Flatten attributes
                for (attr_key, attr_value) in &entity.attributes {
                    let flat_key = format!("{}.{}",
                        entity_type,
                        self.interner.resolve(*attr_key)
                    );
                    flat_attributes.insert(flat_key, attr_value.clone());
                }

                entities.insert(entity_type.clone(), entity.clone());
            }
        }

        Ok(JoinedEntityView {
            entities,
            flat_attributes,
        })
    }
}
```

**Performance:** O(N*M) where N = entities, M = avg attributes per entity. Amortized across multiple policy evaluations.

---

## Performance Considerations

### Benchmark Targets

| Entity Count | Current | Target | Max Acceptable |
|--------------|---------|--------|----------------|
| 2 entities (user, resource) | 300-400ns | 300-400ns | 500ns |
| 3 entities (+ machine) | N/A | <600ns | 800ns |
| 5 entities (full zero-trust) | N/A | <1µs | 1.5µs |
| 10 entities (complex workflows) | N/A | <2µs | 3µs |

### Memory Targets

| Dataset Size | Entities | Target Memory | Max Memory |
|--------------|----------|---------------|------------|
| Small | 100k users + 100k devices + 200k resources | <200MB | 300MB |
| Medium | 1M users + 1M devices + 2M resources | <1.5GB | 2GB |
| Large | 10M users + 10M devices + 20M resources | <12GB | 16GB |

**Memory Optimizations:**
1. **String Interning**: Critical for attribute names, entity types, common values
2. **Arc Sharing**: Share immutable entities across multiple evaluations
3. **Attribute Compression**: Use enums for common attribute values
4. **Entity Caching**: LRU cache for frequently accessed entities (80/20 rule)

### Optimization Techniques

#### 1. Lazy Entity Loading

```rust
// Only load entities actually referenced in the policy
impl Evaluator {
    fn get_referenced_entities(&self, policy: &Policy) -> HashSet<String> {
        // Parse policy AST and extract entity types
        // Return only those actually used in conditions
    }

    fn evaluate_lazy(&self, policy: &Policy, request: &PolicyRequest) -> Result<Decision> {
        let referenced = self.get_referenced_entities(policy);

        // Only load entities that are referenced
        let entities_to_load: HashMap<_, _> = request.entities
            .iter()
            .filter(|(entity_type, _)| referenced.contains(*entity_type))
            .collect();

        // Load and evaluate
    }
}
```

#### 2. Attribute Indexing

```rust
pub struct DataStore {
    entities: DashMap<InternedString, Entity>,

    // Index frequently queried attributes
    attribute_indexes: HashMap<String, AttributeIndex>,
}

pub struct AttributeIndex {
    // attribute_value -> entity_ids with that value
    index: DashMap<AttributeValue, Vec<InternedString>>,
}

impl DataStore {
    pub fn query_by_attribute(
        &self,
        entity_type: &str,
        attribute: &str,
        value: AttributeValue,
    ) -> Vec<InternedString> {
        let index_key = format!("{}.{}", entity_type, attribute);

        if let Some(index) = self.attribute_indexes.get(&index_key) {
            index.get(&value).unwrap_or_default()
        } else {
            // Fall back to full scan
            self.scan_for_attribute(entity_type, attribute, value)
        }
    }
}
```

#### 3. Query Planning & Short-Circuit Evaluation

```rust
impl Evaluator {
    fn optimize_rule(&self, rule: &Rule) -> OptimizedRule {
        // Reorder conditions by estimated cost
        let mut conditions = rule.conditions.clone();

        conditions.sort_by_key(|cond| self.estimate_cost(cond));

        OptimizedRule {
            conditions,
            short_circuit: true, // Stop at first false condition
        }
    }

    fn estimate_cost(&self, condition: &Condition) -> u32 {
        match condition {
            // Cheap: boolean checks
            Condition::BooleanCheck(..) => 1,

            // Medium: equality checks
            Condition::Equality(..) => 5,

            // Expensive: string operations
            Condition::StringMatch(..) => 20,

            // Very expensive: cross-entity comparisons
            Condition::CrossEntityCheck(..) => 50,
        }
    }
}
```

#### 4. Entity Pre-Fetching

```rust
impl Evaluator {
    pub fn evaluate_batch(
        &self,
        policy: &Policy,
        requests: &[PolicyRequest],
        store: &DataStore,
    ) -> Vec<Result<PolicyDecision>> {
        // Pre-fetch all entities needed for all requests
        let all_entity_ids: HashSet<_> = requests
            .iter()
            .flat_map(|req| req.entities.values())
            .collect();

        // Batch load
        let entity_cache = store.get_entities_batch(&all_entity_ids);

        // Evaluate all requests with cached entities
        requests
            .iter()
            .map(|req| self.evaluate_with_cache(policy, req, &entity_cache))
            .collect()
    }
}
```

---

## Migration Path

### Phase 1: Backwards Compatible Extension

**Goal:** Add multi-entity support without breaking existing code

**Changes:**
1. Add `entities` HashMap to `PolicyRequest`
2. Keep `principal` and `resource` fields (deprecated but functional)
3. Internal mapping: `entities["user"]` = `principal`, `entities["resource"]` = `resource`
4. DSL continues to support `user.` and `resource.` syntax

**Code:**
```rust
impl PolicyRequest {
    pub fn new_legacy(principal: String, resource: String, action: String) -> Self {
        let mut entities = HashMap::new();
        entities.insert("user".to_string(), principal.clone());
        entities.insert("resource".to_string(), resource.clone());

        Self {
            entities,
            context: HashMap::new(),
            action,
            principal,  // Backwards compat
            resource,   // Backwards compat
        }
    }

    pub fn new_multi_entity(
        entities: HashMap<String, String>,
        action: String,
        context: HashMap<String, AttributeValue>,
    ) -> Self {
        // Extract principal/resource for backwards compat
        let principal = entities.get("user").cloned().unwrap_or_default();
        let resource = entities.get("resource").cloned().unwrap_or_default();

        Self {
            entities,
            context,
            action,
            principal,
            resource,
        }
    }
}
```

**Timeline:** Sprint 1 (2 weeks)

---

### Phase 2: Extended Entity Support

**Goal:** Add support for common entity types

**New Entity Types:**
- `machine` / `device` / `endpoint`
- `location` / `network`
- `department` / `organization`

**Changes:**
1. Update DSL parser to recognize new keywords
2. Add validation for entity type mismatches
3. Implement batch entity loading
4. Add multi-entity test suite
5. Document new capabilities

**Example Policy:**
```reap
policy phase2_example {
    requires: ["user", "device", "resource"],

    rule device_trust {
        allow if {
            user.is_active &&
            device.trustscore > 70 &&
            resource.classification != "secret"
        }
    }
}
```

**Timeline:** Sprint 2-3 (4 weeks)

---

### Phase 3: Full Schema Support

**Goal:** Add schema validation and type safety

**Features:**
1. Entity schema definition (YAML/JSON)
2. Policy compile-time validation
3. Attribute type checking
4. Required entity validation
5. Schema evolution support

**Entity Schema Example:**
```yaml
entity_types:
  user:
    attributes:
      - name: is_active
        type: boolean
        required: true
      - name: clearance
        type: integer
        range: [0, 10]
      - name: department
        type: string

  machine:
    attributes:
      - name: trustscore
        type: integer
        range: [0, 100]
        required: true
      - name: status
        type: enum
        values: [compliant, non_compliant, unknown]
```

**Validation:**
```rust
impl Policy {
    pub fn validate(&self, schema: &EntitySchema) -> Result<ValidationReport> {
        let mut report = ValidationReport::new();

        // Check all referenced entity types exist
        for entity_type in self.get_referenced_entity_types() {
            if !schema.has_entity_type(entity_type) {
                report.add_error(ValidationError::UnknownEntityType {
                    entity_type: entity_type.clone(),
                });
            }
        }

        // Check all attribute accesses are valid
        for attr_access in self.get_attribute_accesses() {
            if !schema.has_attribute(&attr_access.entity_type, &attr_access.attribute) {
                report.add_error(ValidationError::UnknownAttribute {
                    entity_type: attr_access.entity_type.clone(),
                    attribute: attr_access.attribute.clone(),
                });
            }
        }

        // Check type compatibility
        for comparison in self.get_comparisons() {
            let left_type = self.infer_type(&comparison.left, schema)?;
            let right_type = self.infer_type(&comparison.right, schema)?;

            if !types_compatible(&left_type, &right_type, &comparison.op) {
                report.add_error(ValidationError::TypeMismatch {
                    left: left_type,
                    right: right_type,
                    operation: comparison.op,
                });
            }
        }

        Ok(report)
    }
}
```

**Timeline:** Sprint 4-6 (6 weeks)

---

### Phase 4: Performance Optimization

**Goal:** Achieve target performance metrics

**Optimizations:**
1. Entity caching (LRU)
2. Query planning and reordering
3. Compiled policy caching
4. Hot path optimizations (inlining, SIMD)
5. Benchmark suite and regression testing

**Timeline:** Sprint 7-8 (4 weeks)

---

## Use Cases

### Use Case 1: Zero Trust Device Check

**Requirement:**
- Only allow access from trusted, compliant devices
- Check device encryption, OS version, last security scan
- Match device clearance to resource sensitivity

**Policy:**
```reap
policy zero_trust {
    version: "1.0.0",
    description: "Zero trust device verification",

    requires: ["user", "device", "resource"],

    default: deny,

    // Deny non-compliant devices immediately
    rule deny_non_compliant {
        deny if device.compliance_status != "compliant"
    }

    // Check device trust score
    rule device_trust_check {
        allow if {
            user.is_active == true &&
            device.trustscore >= 75 &&
            device.encryption_enabled == true &&
            device.os_up_to_date == true &&
            device.last_scan_hours < 24 &&
            resource.sensitivity_level <= device.clearance_level
        }
    }
}
```

**Request:**
```rust
PolicyRequest {
    action: "read",
    entities: hashmap! {
        "user" => "alice",
        "device" => "laptop_456",
        "resource" => "doc_secret_123",
    },
    context: hashmap! {
        "ip_address" => "192.168.1.50",
        "timestamp" => 1700000000,
    },
}
```

---

### Use Case 2: Network Segmentation

**Requirement:**
- Enforce network segmentation policies
- Check user and resource are in same network segment
- Verify access from trusted locations
- Validate firewall policies

**Policy:**
```reap
policy network_segmentation {
    version: "1.0.0",
    description: "Network segmentation enforcement",

    requires: ["user", "resource", "network", "location"],

    default: deny,

    // Same segment access
    rule same_segment {
        allow if {
            user.network_segment == resource.network_segment &&
            user.network_segment != "restricted" &&
            location.security_zone == "trusted" &&
            network.firewall_policy == "internal"
        }
    }

    // Cross-segment requires elevated privilege
    rule cross_segment {
        allow if {
            user.privilege_level == "admin" &&
            network.cross_segment_allowed == true &&
            location.security_zone != "untrusted"
        }
    }
}
```

---

### Use Case 3: Multi-Party Authorization

**Requirement:**
- Sensitive operations require manager approval
- Approver must be from same department
- Approval must be recent (<24 hours)
- Track audit trail

**Policy:**
```reap
policy multi_party_authorization {
    version: "1.0.0",
    description: "Require manager approval for sensitive operations",

    requires: ["user", "approver", "resource"],

    default: deny,

    rule requires_approval {
        allow if {
            // User is active
            user.is_active == true &&

            // Approver is manager in same department
            approver.role == "manager" &&
            approver.department == user.department &&
            approver.is_active == true &&

            // Resource requires approval
            resource.requires_approval == true &&

            // Approval is recent
            context.approval_age_hours < 24 &&
            context.approval_granted == true
        }
    }
}
```

**Request:**
```rust
PolicyRequest {
    action: "delete",
    entities: hashmap! {
        "user" => "alice",
        "approver" => "bob_manager",
        "resource" => "production_database",
    },
    context: hashmap! {
        "approval_granted" => AttributeValue::Bool(true),
        "approval_age_hours" => AttributeValue::Int(2),
        "audit_id" => AttributeValue::String("audit_789"),
    },
}
```

---

### Use Case 4: Geolocation & Compliance

**Requirement:**
- EU users can only access EU-hosted resources (GDPR)
- Check user location, resource location, data residency
- Allow exceptions for global admins

**Policy:**
```reap
policy data_residency {
    version: "1.0.0",
    description: "GDPR data residency enforcement",

    requires: ["user", "resource", "user_location", "resource_location"],

    default: deny,

    // EU data must stay in EU
    rule eu_data_residency {
        allow if {
            user_location.region == "EU" &&
            resource_location.region == "EU" &&
            resource.contains_pii == true &&
            user.gdpr_training_completed == true
        }
    }

    // Global admin override
    rule global_admin {
        allow if {
            user.role == "global_admin" &&
            user.compliance_certified == true &&
            context.justification_provided == true
        }
    }

    // Non-PII data can cross borders
    rule non_pii {
        allow if {
            resource.contains_pii == false &&
            user.is_active == true
        }
    }
}
```

---

## Timeline & Roadmap

### Sprint 1-2: Foundation (4 weeks)
- [ ] Design review and approval
- [ ] Add `entities` HashMap to PolicyRequest
- [ ] Backwards compatibility layer
- [ ] Update DSL parser for entity keywords
- [ ] Basic multi-entity evaluation
- [ ] Unit tests

### Sprint 3-4: Core Features (4 weeks)
- [ ] Implement batch entity loading
- [ ] Add validation for required entities
- [ ] Extend DSL for machine, device, location entities
- [ ] Integration tests with 3+ entities
- [ ] Documentation updates

### Sprint 5-6: Schema & Validation (4 weeks)
- [ ] Entity schema definition format
- [ ] Policy compile-time validation
- [ ] Attribute type checking
- [ ] Schema evolution support
- [ ] Error reporting improvements

### Sprint 7-8: Performance (4 weeks)
- [ ] Entity caching (LRU)
- [ ] Query optimization
- [ ] Benchmark suite
- [ ] Performance regression tests
- [ ] Memory profiling

### Sprint 9-10: Production Ready (4 weeks)
- [ ] Production testing
- [ ] Migration guide
- [ ] API documentation
- [ ] Example policies
- [ ] Security audit

**Total Timeline:** 20 weeks (5 months)

---

## Success Metrics

### Performance
- ✅ 2 entity evaluation: <500ns (maintain current performance)
- ✅ 3 entity evaluation: <800ns
- ✅ 5 entity evaluation: <1.5µs
- ✅ Memory: <2GB for 1M users + 1M devices + 2M resources

### Functionality
- ✅ Support 10+ entity types
- ✅ Backwards compatible with existing policies
- ✅ Schema validation with helpful error messages
- ✅ Cross-entity attribute comparisons

### Quality
- ✅ 95%+ test coverage
- ✅ Zero security regressions
- ✅ Comprehensive documentation
- ✅ Migration path for existing deployments

---

## Risks & Mitigation

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Performance degradation with many entities | High | Medium | Implement caching, optimize lookups, benchmark continuously |
| Memory bloat with large datasets | High | Medium | Use string interning, Arc sharing, LRU cache |
| Breaking changes to existing APIs | High | Low | Maintain backwards compat layer, deprecation path |
| Complex policies become hard to reason about | Medium | Medium | Schema validation, policy testing framework, linting |
| Security vulnerabilities in entity resolution | Critical | Low | Security audit, fuzz testing, formal verification |

---

## Open Questions

1. **Entity Aliasing**: Should we support aliases like `principal` = `user`, `machine` = `device`?
2. **Dynamic Entities**: Should policies support dynamic entity loading (e.g., "load all approvers for user")?
3. **Entity Relationships**: Should we support relationship traversal (e.g., `user.manager.department`)?
4. **Performance Tradeoffs**: What's the acceptable latency increase for 10+ entities?
5. **Schema Evolution**: How do we handle schema changes without breaking existing policies?

---

## Appendix A: API Reference

### PolicyRequest

```rust
pub struct PolicyRequest {
    /// Action being requested (e.g., "read", "write", "delete")
    pub action: String,

    /// Entity references: entity_type -> entity_id
    pub entities: HashMap<String, String>,

    /// Additional context (non-entity data)
    pub context: HashMap<String, AttributeValue>,

    /// [Deprecated] Use entities["user"] instead
    #[deprecated(since = "2.0.0", note = "Use entities map instead")]
    pub principal: String,

    /// [Deprecated] Use entities["resource"] instead
    #[deprecated(since = "2.0.0", note = "Use entities map instead")]
    pub resource: String,
}

impl PolicyRequest {
    /// Create a new multi-entity request
    pub fn new(
        action: impl Into<String>,
        entities: HashMap<String, String>,
        context: HashMap<String, AttributeValue>,
    ) -> Self;

    /// Add an entity to the request
    pub fn with_entity(
        mut self,
        entity_type: impl Into<String>,
        entity_id: impl Into<String>,
    ) -> Self;

    /// Add context variable
    pub fn with_context(
        mut self,
        key: impl Into<String>,
        value: AttributeValue,
    ) -> Self;
}
```

### EntitySchema

```rust
pub struct EntitySchema {
    entity_types: HashMap<String, EntityTypeSchema>,
}

pub struct EntityTypeSchema {
    name: String,
    attributes: Vec<AttributeSchema>,
    required: bool,
}

pub struct AttributeSchema {
    name: String,
    attr_type: AttributeType,
    required: bool,
    default: Option<AttributeValue>,
}

impl EntitySchema {
    pub fn from_yaml(yaml: &str) -> Result<Self>;
    pub fn from_json(json: &str) -> Result<Self>;

    pub fn validate_entity(&self, entity: &Entity) -> Result<ValidationReport>;
    pub fn validate_policy(&self, policy: &Policy) -> Result<ValidationReport>;
}
```

---

## Appendix B: Entity Type Registry

| Entity Type | Common Attributes | Use Cases |
|-------------|-------------------|-----------|
| `user` / `principal` | id, name, email, role, department, is_active, clearance | All access control |
| `resource` | id, name, type, classification, owner, sensitivity | All access control |
| `machine` / `device` | id, trustscore, os_version, encryption_enabled, last_scan | Zero trust, device compliance |
| `location` | id, region, country, security_zone, ip_range | Geofencing, compliance |
| `network` | id, segment, firewall_policy, cross_segment_allowed | Network segmentation |
| `department` | id, name, budget, security_level | Organizational policies |
| `approver` | id, name, role, department | Multi-party authorization |
| `session` | id, mfa_verified, start_time, ip_address | Session-based policies |
| `application` | id, name, version, security_rating | Application-based access |

---

**End of Document**
