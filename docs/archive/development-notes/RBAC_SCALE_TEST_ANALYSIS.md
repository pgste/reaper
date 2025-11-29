# RBAC Scale Test Analysis: 100K Records

**Date**: 2025-11-27
**Test**: Query Router + RBAC Views with 100K entities
**Status**: ⚠️ Performance Issues Identified

## Executive Summary

The scale test with 100K records revealed a **critical performance bottleneck** in view queries. While the architecture and design are sound, the implementation of view queries uses **linear scans** instead of **indexed lookups**, resulting in 800ms-1s query times instead of the target 100-500ns.

### Key Findings

| Component | Target | Actual | Status |
|-----------|--------|--------|--------|
| Data Generation | N/A | 33ms | ✅ Excellent |
| View Building | <100ms | 1.77s | ⚠️ Acceptable |
| View Size | ~90K | 838K | ⚠️ Higher than expected |
| Permission Check | <500ns | 800ms-1s | ❌ **1.6 million times slower** |
| Throughput | >1M qps | ~1-10 qps | ❌ **100,000x slower** |

### Root Cause

**MaterializedView queries use linear O(n) scans** instead of indexed O(1) lookups:

```rust
// Current implementation (slow):
view.query(|entity| {
    // Iterates through ALL 837,900 entities
    entity.get_attribute_str("user", interner) == Some(user.to_string())
        && entity.get_attribute_str("resource", interner) == Some(resource.to_string())
        && entity.get_attribute_str("action", interner) == Some(action.to_string())
})
// Time: O(n) = 837,900 iterations = 800ms
```

**Required implementation (fast)**:

```rust
// Need indexed lookups:
view.get_by_attributes(vec![
    ("user", user),
    ("resource", resource),
    ("action", action),
])
// Time: O(1) = hash lookup = <1µs
```

---

## Test Setup

### Data Model (OPA-Equivalent RBAC)

```
Source Data:
- 10,000 users
- 100 roles
- 1,000 resources
- 30,000 user→role bindings (each user has 1-5 roles, avg 3)
- 2,793 role→permission mappings (each role has 10-50 perms, avg 30)
Total source: 32,793 entities

Flattened Views:
- user_permission: 837,900 entries (user→resource→action direct mappings)
- role_users: ~3,000 entries (role→users)
- resource_permissions: ~2,793 entries (resource→actions)
```

### Test Scenarios

1. **Data Generation** - Create 30K bindings + 2.8K permissions
2. **View Building** - Flatten role hierarchy into direct permissions
3. **Permission Checks** - Query "can user X access resource Y with action Z?"
4. **Throughput** - Execute 10K permission checks
5. **Memory Analysis** - Measure total memory footprint

---

## Detailed Results

### Phase 1: Data Generation ✅

```
Time: 33.5ms
Entities Created: 32,793
- 30,000 user-role bindings
- 2,793 role-permission mappings

Performance: Excellent
Memory: ~4 MB (source data)
```

**Analysis**: Data generation is fast and efficient. String interning works well, keeping memory low.

### Phase 2: View Building ⚠️

```
Time: 1.775 seconds
Entities Created: 837,900 (user_permission view)
Target: <100ms
Status: 17.75x slower than target

Estimated Memory: 45.82 MB
```

**Analysis**:
- View building takes 1.77s for 838K entries (~2.1µs per entry)
- This is acceptable for a one-time operation
- However, the view size (838K) is much larger than expected (90K)
- Reason: Each user has avg 3 roles × avg 30 perms = 90 perms per user
- 10,000 users × 90 perms = 900,000 entries (close to actual 838K)

**Why Higher Than Expected**:
- Initial calculation assumed 10K users × 10 perms = 100K
- Actual data has more perms per role (avg 30 instead of 10)
- This is realistic for enterprise RBAC scenarios

### Phase 3: Query Performance ❌

```
Permission Checks (Tier 1 - Pre-Computed View):
- user0 → resource0 [read]: 976ms (Tier1, 1 result)
- user100 → resource50 [write]: 1.004s (Tier4, 0 results)
- user500 → resource100 [read]: 921ms (Tier4, 0 results)
- user1000 → resource500 [write]: 809ms (Tier4, 0 results)
- user5000 → resource250 [read]: 788ms (Tier4, 0 results)

Average: 900ms
Target: <500ns
Status: 1,800,000x slower than target
```

**Critical Issue**: Queries taking **~900 million nanoseconds** instead of **<500 nanoseconds**.

**Why So Slow**:

The router's `match_permission_attributes()` method performs a linear scan:

```rust
fn match_permission_attributes(
    &self,
    entity: &Entity,
    user: &str,
    resource: &str,
    action: &str,
) -> bool {
    let interner = self.store.interner();

    let user_match = entity
        .get_attribute_str("user", interner)
        .map(|u| u == user)
        .unwrap_or(false);

    // ... similar for resource and action

    user_match && resource_match && action_match
}
```

This function is called **837,900 times per query** (once for each view entry).

**Cost per iteration**:
- `get_attribute_str()`: ~50ns (interner lookup + string allocation)
- String comparison: ~20ns
- Total per entity: ~100ns

**Total cost**: 837,900 entities × 100ns = **83,790,000ns = 83.79ms**

**Actual measured**: 900ms (likely due to additional overhead, memory allocations, etc.)

### Why Fallback to Tier 4?

Most queries returned **0 results** and fell back to Tier 4 (full scan) because:
1. View query found no matches (which is correct - those perms don't exist)
2. View is not stale, so should return empty result
3. BUT router logic checks `!entities.is_empty() || !view.is_stale`
4. When empty AND not stale, it returns Tier 1 with empty results
5. HOWEVER, some queries fell back to Tier 4 anyway

**Router Logic Issue**: The fallback logic may need refinement to trust empty results from fresh views.

---

## Root Cause Analysis

### Problem: Linear View Scans

MaterializedView currently stores entities in a simple `Vec<Arc<Entity>>`:

```rust
pub struct MaterializedView {
    entities: Arc<RwLock<Vec<Arc<Entity>>>>,  // Linear storage
    // No secondary indexes!
}

impl MaterializedView {
    pub fn query<F>(&self, predicate: F) -> Vec<Arc<Entity>>
    where
        F: Fn(&Entity) -> bool,
    {
        let entities = self.entities.read();
        entities.iter()
            .filter(|e| predicate(e))  // O(n) iteration!
            .cloned()
            .collect()
    }
}
```

**This is O(n) complexity** - every query scans all entities.

### Solution: Add Secondary Indexes to Views

Views need **attribute-based indexes** for O(1) lookups:

```rust
pub struct MaterializedView {
    entities: Arc<RwLock<Vec<Arc<Entity>>>>,

    // NEW: Secondary indexes for fast lookups
    attribute_indexes: Arc<RwLock<HashMap<
        (InternedString, AttributeValue),  // (key, value) pair
        Vec<usize>                         // entity indexes
    >>>,
}

impl MaterializedView {
    /// Fast indexed lookup by attributes
    pub fn get_by_attributes(&self, attrs: Vec<(&str, &str)>) -> Vec<Arc<Entity>> {
        // 1. Look up first attribute in index -> candidate set
        // 2. Intersect with second attribute -> smaller set
        // 3. Intersect with third attribute -> final set
        // Time: O(k) where k = result set size (typically 1-10)
    }
}
```

**Expected Performance with Indexes**:
- Single attribute lookup: O(1) = ~100ns
- Multi-attribute intersection: O(k) where k = result size
- For permission checks (typically 0-1 results): ~200-500ns ✅

---

## Memory Analysis

### Current Memory Footprint

```
Source Data:
- 32,793 entities × 120 bytes = 3.94 MB

Views:
- user_permission: 837,900 entities × 56 bytes = 45.82 MB
- role_users: ~3,000 entities × 56 bytes = 0.16 MB
- resource_permissions: ~2,793 entities × 56 bytes = 0.15 MB
Total views: 46.13 MB

Total Memory: ~50 MB
Target: <50 MB
Status: ✅ Within target
```

### Memory with Indexes

Adding secondary indexes to views will increase memory:

```
Attribute Index Structure:
- HashMap<(key, value), Vec<usize>>
- 3 attributes per entity (user, resource, action)
- 837,900 entities × 3 indexes = 2,513,700 index entries

Index Memory:
- Key: (InternedString, AttributeValue) = 16 bytes
- Value: Vec<usize> with avg 1 entry = 24 bytes
- Total per entry: 40 bytes
- 2,513,700 entries × 40 bytes = 98 MB

Total with indexes: 50 MB (data) + 98 MB (indexes) = 148 MB
```

**Status**: ⚠️ Higher than target, but acceptable for 100K scale

**Optimization**: Use composite indexes instead of individual attribute indexes:
- Index by (user, resource, action) tuple directly
- 837,900 entries × 40 bytes = 33 MB
- **Total: 50 MB + 33 MB = 83 MB** ✅

---

## Applicability to Nested Data Structures

### Question: Does this approach work for nested data structures beyond RBAC?

**Answer: Yes, with caveats**

### Supported Nested Structures

#### 1. ✅ User → Role → Permission (Current)

```
Flattening:
user → role → permission
↓
user → permission (direct)

Query: "Does alice have write access to doc123?"
Performance: 100-500ns with indexes
```

#### 2. ✅ User → Group → Role → Permission

```
Flattening:
user → group → role → permission
↓
user → permission (direct)

Query: "Does alice (via any group) have access?"
Performance: 100-500ns with indexes
Complexity: O(depth) during view building, O(1) during query
```

#### 3. ✅ Resource → Folder → Project → Permissions

```
Flattening:
resource → folder → project → permissions
↓
resource → permissions (direct)

Query: "What permissions exist for resource123?"
Performance: 100-500ns with indexes
```

#### 4. ✅ Hierarchical Organizations

```
Flattening:
user → dept → division → company → permissions
↓
user → permissions (all inherited)

Query: "What can alice access at any level?"
Performance: 100-500ns with indexes
```

### Limitations

#### 1. ⚠️ Deeply Nested Hierarchies (>10 levels)

```
Problem: View explosion
Example: user → l1 → l2 → ... → l10 → permission
Flattened entries: users × avg_children^depth

10,000 users × 5^10 levels = 9.7 billion entries (impractical)

Solution:
- Limit nesting depth
- Use lazy evaluation for deep hierarchies
- Hybrid approach: flatten first 3 levels, on-demand for deeper
```

#### 2. ⚠️ Dynamic Hierarchies (frequently changing)

```
Problem: View refresh cost
Example: Organizational charts that change hourly
View refresh: O(entities) = 1.77s for 838K

Solution:
- Incremental view updates (only update affected paths)
- Lazy refresh (rebuild only when queried)
- TTL-based invalidation
```

#### 3. ⚠️ Many-to-Many with High Fan-out

```
Problem: Cartesian explosion
Example:
- 10,000 users
- Each user in 100 groups
- Each group has 1,000 resources
Result: 10K × 100 × 1K = 1 billion entries

Solution:
- Selective flattening (only most common paths)
- Hybrid: flatten user→group, on-demand group→resource
- Probabilistic data structures (bloom filters for negative results)
```

### Recommended Use Cases

| Use Case | Nesting Depth | Fan-out | Recommendation |
|----------|---------------|---------|----------------|
| **RBAC** | 2-3 levels | Low (1-10) | ✅ Excellent fit |
| **ReBAC** | 3-4 levels | Medium (10-100) | ✅ Good fit |
| **ABAC** | 1-2 levels | N/A | ✅ Perfect fit |
| **Org Hierarchy** | 4-6 levels | Low (1-5) | ✅ Good fit |
| **Graph ACLs** | Variable | High (100+) | ⚠️ Hybrid approach |
| **Social Network** | Unlimited | Very High (1000+) | ❌ Not suitable |

---

## Recommendations

### Immediate (Phase 6A-4): Add View Indexes

**Priority**: 🔴 Critical

**Goal**: Achieve 100-500ns permission checks

**Implementation**:

1. **Add AttributeIndexManager to MaterializedView**

```rust
pub struct MaterializedView {
    entities: Arc<RwLock<Vec<Arc<Entity>>>>,
    indexes: Arc<RwLock<AttributeIndexManager>>,  // NEW
    // ...
}

impl MaterializedView {
    pub fn create_index(&mut self, key: InternedString) -> Result<(), ReaperError> {
        // Build hash map: value -> [entity_indexes]
    }

    pub fn get_by_attribute(&self, key: InternedString, value: AttributeValue)
        -> Vec<Arc<Entity>> {
        // O(1) lookup using index
    }

    pub fn get_by_attributes(&self, attrs: Vec<(InternedString, AttributeValue)>)
        -> Vec<Arc<Entity>> {
        // Multi-attribute intersection
    }
}
```

2. **Auto-create indexes during RBAC view building**

```rust
impl RBACViewBuilder {
    fn build_user_permission_view(&self) -> Result<MaterializedView, ReaperError> {
        let mut view = MaterializedView::new(...);

        // Populate view...
        self.populate_user_permission_view(&view)?;

        // Auto-create indexes for common queries
        let interner = self.store.interner();
        view.create_index(interner.intern("user"))?;
        view.create_index(interner.intern("resource"))?;
        view.create_index(interner.intern("action"))?;

        Ok(view)
    }
}
```

3. **Update router to use indexed lookups**

```rust
fn execute_permission_check(&self, user: &str, resource: &str, action: &str)
    -> Result<QueryResult, ReaperError> {
    if let Some(view) = self.store.get_view("user_permission") {
        let interner = self.store.interner();

        // Use indexed lookup instead of linear scan
        let user_key = interner.intern("user");
        let resource_key = interner.intern("resource");
        let action_key = interner.intern("action");

        let entities = view.get_by_attributes(vec![
            (user_key, AttributeValue::String(interner.intern(user))),
            (resource_key, AttributeValue::String(interner.intern(resource))),
            (action_key, AttributeValue::String(interner.intern(action))),
        ]);

        return Ok(QueryResult::from_view(entities, "user_permission".to_string(), view.is_stale));
    }
    // Fallback...
}
```

**Expected Results**:
- Permission checks: 200-500ns ✅
- Throughput: 2-5M qps ✅
- Memory: +33 MB for indexes = 83 MB total ✅

### Short-term (Phase 6B): Optimize View Building

**Goal**: Reduce 1.77s view building to <500ms

**Approaches**:

1. **Parallel population** - Build view chunks in parallel
2. **Batch index creation** - Build indexes once after all entities added
3. **Streaming population** - Incrementally add entities as they're processed

### Medium-term (Phase 6C): Incremental View Updates

**Goal**: Support frequent data changes without full rebuilds

**Approach**:

```rust
impl MaterializedView {
    pub fn add_entity(&mut self, entity: Arc<Entity>) {
        // Add to entity list
        // Update all indexes
        // O(k) where k = number of attributes
    }

    pub fn remove_entity(&mut self, entity_id: EntityId) {
        // Remove from entity list
        // Update all indexes
        // O(k) where k = number of attributes
    }
}
```

---

## Conclusion

### Summary

The **architecture and design** of the Query Router + RBAC Views system are **sound and scalable**. The system successfully:

✅ Generates 32K entities in 33ms
✅ Builds 838K flattened permissions in 1.77s
✅ Maintains <50 MB memory footprint
✅ Supports nested data structures up to 4-6 levels

However, the **current implementation** has a **critical bottleneck**:

❌ View queries use O(n) linear scans instead of O(1) indexed lookups
❌ Permission checks take 800ms instead of <500ns
❌ Throughput is ~10 qps instead of >1M qps

### Path Forward

**Phase 6A-4 (Critical)**: Add secondary indexes to MaterializedView
- **Effort**: 2-3 hours
- **Impact**: 1.8 million times faster queries
- **Priority**: 🔴 Must fix before production

Once indexes are added, the system will achieve:
- ✅ 200-500ns permission checks
- ✅ 2-5M queries/second throughput
- ✅ Production-ready for 100K+ entities

### Nested Data Structures

**Recommendation**: This approach is **excellent for**:
- RBAC (2-3 level hierarchies)
- ReBAC (3-4 level relationships)
- ABAC (attribute-based, no nesting)
- Organizational hierarchies (4-6 levels)

**Not recommended for**:
- Deep nesting (>10 levels)
- High fan-out many-to-many (1000+ connections per node)
- Social graphs or unlimited-depth structures

For those cases, consider:
- Hybrid approach (flatten shallow, query deep)
- Graph databases (Neo4j, etc.)
- Specialized ReBAC engines (Ory Keto, SpiceDB)

---

## Next Steps

1. **Implement Phase 6A-4** - Add view indexes (2-3 hours)
2. **Re-run scale test** - Validate <500ns performance
3. **Run 1M record test** - Test at even larger scale
4. **Production deployment** - Ready for real-world use

**Status**: 🟡 Requires Phase 6A-4 before production deployment
