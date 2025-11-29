# Hybrid Approach: Pre-Computed Matrices + General Queries

**Date:** 2025-11-26
**Status:** 📋 Planning Phase - NO IMPLEMENTATION YET
**Goal:** Combine Option A speed with general query flexibility

---

## The Question

> "Can we go with option A but is it possible to make it general query like?"

**Translation:** Can we get 100-500ns pre-computed performance while still supporting arbitrary query patterns?

**Short Answer:** Yes, with materialized views + smart invalidation!

---

## Deep Analysis

### Option A (Pre-Computed Matrix) - Current Design

```rust
// RBAC-specific: Pre-flatten user→role→permission
// alice → dev → write foo123
// Becomes:
// alice → write foo123 (direct permission entity)

// Ultra-fast: Single O(1) lookup
let has_perm = store.get("user_permission")
    .where("user", alice)
    .where("resource", foo123)
    .where("action", write)
    .exists(); // 100-500ns
```

**Pros:**
- ✅ Fastest possible (10-270x faster than Rego)
- ✅ Simple queries
- ✅ Works with decision trees

**Cons:**
- ❌ RBAC-specific (not general)
- ❌ Denormalized data
- ❌ Rebuild on changes
- ❌ Can't query intermediate relationships

### What "General Query Like" Could Mean

**Interpretation 1: Support Any Query Pattern**
```rust
// Not just RBAC, but also:
// - ABAC: user attributes + resource attributes
// - ReBAC: resource hierarchies, org charts
// - Temporal: time-based permissions
// - Multi-factor: combine multiple conditions
```

**Interpretation 2: Query Intermediate Data**
```rust
// Not just "does user have permission?"
// But also:
// - "what roles does user have?"
// - "what permissions does role grant?"
// - "who has access to resource X?"
```

**Interpretation 3: Flexible Computation**
```rust
// Not just pre-computed paths
// But also:
// - Compute aggregations (count, sum)
// - Apply functions (inheritance, delegation)
// - Handle edge cases (deny overrides, conflicts)
```

---

## Three Hybrid Approaches

### Approach 1: Materialized Views (RECOMMENDED)

**Concept:** Pre-compute common queries as "views", keep source data for ad-hoc queries.

```rust
// Store BOTH normalized and denormalized data
DataStore {
    // Source data (normalized)
    entities: {
        user_role_binding: [...],
        role_permission: [...],
    },

    // Materialized views (denormalized)
    views: {
        "user_permission": [...],      // Pre-computed matrix
        "role_users": [...],           // Inverse index
        "resource_permissions": [...], // Resource-centric view
    },
}

// Fast path: Use materialized view
if let Some(view) = store.get_view("user_permission") {
    return view.query(user, resource, action); // 100-500ns
}

// Slow path: Query source data
return store.query_chain()
    .from("user_role_binding")
    .join("role_permission", "role")
    .where("user", user)
    .where("resource", resource)
    .where("action", action)
    .exists(); // 5-10µs
```

**Architecture:**

```
┌─────────────────────────────────────────────────────────┐
│                    DataStore                            │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Source Data (Normalized)                              │
│  ├── user_role_binding: [alice→dev, alice→test, ...]  │
│  ├── role_permission: [dev→write foo123, ...]          │
│  └── user_attributes: [alice.dept=eng, ...]            │
│                                                         │
│  Materialized Views (Denormalized)                     │
│  ├── user_permission: [alice→write foo123, ...]        │
│  │   └── Strategy: PRECOMPUTE (updated on source change)│
│  ├── role_users: [dev→[alice, bob], ...]               │
│  │   └── Strategy: LAZY (computed on first query)       │
│  └── resource_grants: [foo123→[alice, bob], ...]       │
│      └── Strategy: INCREMENTAL (updated per change)     │
│                                                         │
│  Query Router                                           │
│  ├── Pattern Match: "user + resource + action"         │
│  │   └── Use: user_permission view (100-500ns)         │
│  ├── Pattern Match: "role + *"                         │
│  │   └── Use: role_users view (1-2µs)                  │
│  └── Fallback: Query source data (5-10µs)              │
└─────────────────────────────────────────────────────────┘
```

**Pros:**
- ✅ Fast common queries (100-500ns)
- ✅ Supports arbitrary queries (5-10µs fallback)
- ✅ Keeps source data for auditability
- ✅ Can add new views without breaking existing

**Cons:**
- ❌ More memory (2-3x storage)
- ❌ View invalidation complexity
- ❌ Need to choose which views to materialize

**Implementation Complexity:** Medium (2-3 sessions)

---

### Approach 2: Query Compilation with Caching

**Concept:** Compile query plans, cache results aggressively.

```rust
// Define query patterns
let query = Query::new()
    .from("user_role_binding")
    .join("role_permission", on: "role")
    .where("user", Param::User)
    .where("resource", Param::Resource)
    .where("action", Param::Action)
    .project("exists");

// First execution: Compile + execute (5-10µs)
let plan = query.compile()?;
let result = plan.execute(&store, params)?;

// Cache compiled plan + results
cache.insert(query_hash, (plan, result));

// Subsequent executions: Cache hit (100-500ns)
if let Some((plan, cached)) = cache.get(query_hash) {
    if !cached.is_stale() {
        return cached.result; // 100-500ns
    }
}
```

**Architecture:**

```
┌─────────────────────────────────────────────────────────┐
│                  Query Engine                           │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Query Parser                                           │
│  └── Parse DSL → AST                                    │
│                                                         │
│  Query Optimizer                                        │
│  ├── Detect pattern: user→role→permission              │
│  ├── Choose strategy: FLATTEN (pre-compute)            │
│  └── Generate plan: single table scan                  │
│                                                         │
│  Query Compiler                                         │
│  └── Compile plan → executable code                    │
│                                                         │
│  Execution Cache                                        │
│  ├── Key: (query_hash, params)                         │
│  ├── Value: (result, computed_at, dependencies)        │
│  └── Invalidation: Track data dependencies             │
│                                                         │
│  Executor                                               │
│  ├── Fast path: Cache hit (100-500ns)                  │
│  ├── Medium path: Compiled plan (1-5µs)                │
│  └── Slow path: Interpret query (5-10µs)               │
└─────────────────────────────────────────────────────────┘
```

**Pros:**
- ✅ General-purpose (any query pattern)
- ✅ Fast after warm-up (100-500ns)
- ✅ Automatic optimization
- ✅ No manual view management

**Cons:**
- ❌ Cold start penalty (5-10µs first query)
- ❌ Cache invalidation hard to get right
- ❌ Complex implementation
- ❌ Memory overhead for cache

**Implementation Complexity:** High (4-5 sessions)

---

### Approach 3: Stratified Evaluation

**Concept:** Detect query type, route to specialized handler.

```rust
enum QueryStrategy {
    PreComputed,     // 100-500ns - Use materialized view
    Indexed,         // 1-3µs - Use indexes + joins
    FullScan,        // 5-10µs - Scan all data
}

impl QueryRouter {
    fn route(&self, query: &Query) -> QueryStrategy {
        // Detect pattern
        match query.pattern() {
            // RBAC: user + resource + action
            Pattern::RBACPermissionCheck => {
                if self.has_view("user_permission") {
                    QueryStrategy::PreComputed
                } else {
                    QueryStrategy::Indexed
                }
            }

            // ABAC: attributes + conditions
            Pattern::ABACConditions => {
                QueryStrategy::Indexed
            }

            // Ad-hoc: Unknown pattern
            Pattern::Unknown => {
                QueryStrategy::FullScan
            }
        }
    }
}

// Execution
let strategy = router.route(&query);
match strategy {
    PreComputed => view.get(params),        // 100-500ns
    Indexed => indexed_join(params),         // 1-3µs
    FullScan => full_scan_filter(params),   // 5-10µs
}
```

**Architecture:**

```
┌─────────────────────────────────────────────────────────┐
│                Query Router                             │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Pattern Detector                                       │
│  ├── RBAC: (user, resource, action) → PreComputed      │
│  ├── ABAC: (attributes, conditions) → Indexed          │
│  ├── ReBAC: (relationships) → Graph                    │
│  └── Unknown: → FullScan                                │
│                                                         │
│  Strategy Executors                                     │
│  ├── PreComputed Handler (100-500ns)                   │
│  │   └── Lookup: materialized_views[pattern][params]   │
│  ├── Indexed Handler (1-3µs)                           │
│  │   └── Join: indexes[attr1] ⋈ indexes[attr2]         │
│  ├── Graph Handler (3-10µs)                            │
│  │   └── Traverse: relationships[src]→[dst]            │
│  └── FullScan Handler (5-10µs)                         │
│      └── Filter: entities.iter().filter(predicate)     │
└─────────────────────────────────────────────────────────┘
```

**Pros:**
- ✅ Best performance for each pattern
- ✅ Extensible (add new strategies)
- ✅ Graceful degradation (fallback to slower path)
- ✅ Predictable performance

**Cons:**
- ❌ Need to maintain multiple code paths
- ❌ Pattern detection heuristics
- ❌ May miss optimization opportunities

**Implementation Complexity:** Medium (3-4 sessions)

---

## Detailed Comparison

| Aspect | Materialized Views | Query Compilation | Stratified Evaluation |
|--------|-------------------|-------------------|----------------------|
| **Performance** | | | |
| - Common queries | 100-500ns ✅ | 100-500ns ✅ | 100-500ns ✅ |
| - Ad-hoc queries | 5-10µs ✅ | 5-10µs ✅ | 5-10µs ✅ |
| - Cold start | Instant ✅ | 5-10µs ❌ | Instant ✅ |
| **Flexibility** | | | |
| - Query patterns | Manual views ⚠️ | Any pattern ✅ | Detect patterns ✅ |
| - Extensibility | Add views ✅ | Automatic ✅ | Add strategies ✅ |
| - Transparency | Explicit ✅ | Opaque ❌ | Clear ✅ |
| **Complexity** | | | |
| - Implementation | Medium | High | Medium |
| - Maintenance | Medium | High | Medium |
| - Debugging | Easy ✅ | Hard ❌ | Medium |
| **Memory** | | | |
| - Storage | 2-3x ❌ | 1.5-2x ⚠️ | 2-3x ❌ |
| - Predictable | Yes ✅ | No ❌ | Yes ✅ |
| **Data Consistency** | | | |
| - Invalidation | Manual ❌ | Auto ✅ | Manual ❌ |
| - Staleness | Can control ✅ | Unpredictable ❌ | Can control ✅ |

---

## Recommended Hybrid: Materialized Views + Smart Router

**Why:** Best balance of speed, flexibility, and maintainability.

### Core Design

```rust
pub struct DataStore {
    // Source data (always up-to-date)
    entities: HashMap<EntityType, Vec<Entity>>,
    indexes: HashMap<String, Index>,

    // Materialized views (pre-computed)
    views: HashMap<String, MaterializedView>,

    // Query router (intelligent dispatch)
    router: QueryRouter,
}

pub struct MaterializedView {
    name: String,
    source_query: Query,
    data: Vec<Entity>,
    strategy: ViewStrategy,
    last_updated: Instant,
    dependencies: Vec<EntityType>,
}

pub enum ViewStrategy {
    Eager,        // Update immediately on source change
    Lazy,         // Compute on first query
    Incremental,  // Update only affected rows
    Periodic,     // Refresh every N seconds
}

impl DataStore {
    // High-level query API
    pub fn query(&self, pattern: QueryPattern) -> QueryResult {
        // Route to best execution strategy
        self.router.execute(pattern)
    }

    // Low-level view management
    pub fn create_view(&mut self, def: ViewDefinition) -> Result<()> {
        // Create materialized view from query
    }

    pub fn invalidate_view(&mut self, view_name: &str) -> Result<()> {
        // Mark view as stale, recompute if needed
    }
}
```

### Example Usage

```rust
// Define common query patterns as views
let store = DataStore::new();

// View 1: RBAC permission check (Eager strategy)
store.create_view(ViewDefinition {
    name: "user_permission",
    source: Query::new()
        .from("user_role_binding")
        .join("role_permission", "role")
        .project(["user", "resource", "action"]),
    strategy: ViewStrategy::Eager,
})?;

// View 2: Role membership (Lazy strategy)
store.create_view(ViewDefinition {
    name: "role_users",
    source: Query::new()
        .from("user_role_binding")
        .group_by("role")
        .aggregate("users", Collect),
    strategy: ViewStrategy::Lazy,
})?;

// Fast query: Uses user_permission view (100-500ns)
let has_perm = store.query(
    QueryPattern::PermissionCheck {
        user: "alice",
        resource: "foo123",
        action: "write",
    }
)?;

// Flexible query: Uses role_users view or falls back to source (1-10µs)
let users = store.query(
    QueryPattern::RoleMembers {
        role: "dev",
    }
)?;

// Ad-hoc query: No view available, query source (5-10µs)
let resources = store.query(
    QueryPattern::Custom(Query::new()
        .from("role_permission")
        .where("action", "write")
        .group_by("resource")
    )
)?;
```

### Performance Tiers

```
┌──────────────────────────────────────────────────────────┐
│  Performance Tier System                                 │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  Tier 1: Pre-Computed Views (100-500ns)                 │
│  ├── Pattern: Exact match to materialized view          │
│  ├── Example: RBAC permission check                     │
│  └── Strategy: Single index lookup                      │
│                                                          │
│  Tier 2: Indexed Joins (1-3µs)                          │
│  ├── Pattern: Multi-table with indexed attributes       │
│  ├── Example: Find user's roles + permissions           │
│  └── Strategy: Index scan + hash join                   │
│                                                          │
│  Tier 3: Partial Scan (3-5µs)                           │
│  ├── Pattern: Some indexed, some filtered               │
│  ├── Example: Users with attribute X in resource Y      │
│  └── Strategy: Index scan + filter                      │
│                                                          │
│  Tier 4: Full Scan (5-10µs)                             │
│  ├── Pattern: No indexes, complex conditions            │
│  ├── Example: Find all users matching complex predicate │
│  └── Strategy: Full entity scan + filter                │
└──────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 6A-1: View Foundation (Session 1)

**Goal:** Basic materialized view infrastructure

**Tasks:**
1. Design MaterializedView struct
2. Implement ViewStrategy trait
3. Add view storage to DataStore
4. Build simple view updater

**Deliverables:**
```rust
// Can create and query views
let view = MaterializedView::new("user_permission", source_query);
store.add_view(view)?;
let result = store.query_view("user_permission", params)?;
```

**Time:** 1 session
**Risk:** Low
**Dependencies:** None

---

### Phase 6A-2: Query Router (Session 2)

**Goal:** Intelligent query routing

**Tasks:**
1. Design QueryPattern enum
2. Implement pattern detection
3. Build router with fallback logic
4. Add performance tier system

**Deliverables:**
```rust
// Router automatically selects best strategy
let result = store.query(QueryPattern::PermissionCheck {...})?;
// Uses view if available (100-500ns)
// Falls back to indexed join (1-3µs)
// Or full scan if needed (5-10µs)
```

**Time:** 1 session
**Risk:** Low
**Dependencies:** Phase 6A-1

---

### Phase 6A-3: RBAC Views (Session 3)

**Goal:** Pre-built views for common RBAC patterns

**Tasks:**
1. Implement user_permission view
2. Implement role_users view
3. Implement resource_grants view
4. Add view invalidation logic

**Deliverables:**
```rust
// One-liner RBAC setup
let store = DataStore::with_rbac_views()?;
// Automatic 100-500ns permission checks
```

**Time:** 1 session
**Risk:** Low
**Dependencies:** Phase 6A-1, 6A-2

---

### Phase 6A-4: General Query API (Session 4 - Optional)

**Goal:** Flexible query API for custom patterns

**Tasks:**
1. Design Query builder API
2. Implement query compilation
3. Add custom view creation
4. Document query patterns

**Deliverables:**
```rust
// Define custom views
store.create_view("my_pattern", Query::new()
    .from("entity_type")
    .where("attr", value)
    .join("other_type", "key")
)?;
```

**Time:** 1 session
**Risk:** Medium
**Dependencies:** Phase 6A-1, 6A-2

---

## Migration Path

### Stage 1: Foundation (Week 1)
- Implement materialized views
- Add query router
- NO breaking changes

### Stage 2: RBAC Views (Week 1-2)
- Add pre-built RBAC views
- Integrate with PolicyEngine
- Benchmark vs Rego

### Stage 3: General Queries (Week 2-3)
- Add custom view API
- Document query patterns
- Performance tuning

### Stage 4: Production (Week 3+)
- Deploy to production
- Monitor performance
- Iterate based on feedback

---

## Expected Performance

### RBAC Permission Check

| Implementation | Time | vs Rego (5-27µs) |
|----------------|------|------------------|
| Rego | 5-27µs | 1x baseline |
| **Pre-computed view** | **100-500ns** | **10-270x faster** ✅ |
| Indexed join | 1-3µs | 2-27x faster |
| Full scan | 5-10µs | ~same |

### Complex Queries

| Query Type | Time | Strategy |
|------------|------|----------|
| User's permissions | 100-500ns | Pre-computed view |
| Role members | 1-2µs | Indexed scan |
| Resource ACL | 2-3µs | Indexed join |
| Attribute matching | 3-5µs | Partial scan |
| Complex predicate | 5-10µs | Full scan |

---

## Tradeoffs Analysis

### Memory vs Speed

**Option A (Pure Pre-Compute):**
- Memory: 3-4x base data
- Speed: 100-500ns always
- Flexibility: Low

**Hybrid (Views + Router):**
- Memory: 2-3x base data
- Speed: 100-500ns common, 1-10µs others
- Flexibility: High ✅

**Option C (Pure Query Engine):**
- Memory: 1-1.5x base data
- Speed: 5-10µs always (+ cache)
- Flexibility: Highest

**Recommendation:** Hybrid - best balance

### Complexity vs Maintainability

**Simple (Option A):**
- Lines of code: ~500
- Maintenance: Easy
- Extensibility: Hard

**Hybrid (Recommended):**
- Lines of code: ~1500
- Maintenance: Medium ✅
- Extensibility: Easy

**Complex (Option C):**
- Lines of code: ~3000
- Maintenance: Hard
- Extensibility: Easy

**Recommendation:** Hybrid - manageable complexity

---

## Risks & Mitigation

### Risk 1: View Invalidation Bugs

**Problem:** Stale views return wrong answers

**Mitigation:**
- Track dependencies explicitly
- Add staleness checks
- Provide manual invalidation API
- Test thoroughly with concurrent updates

### Risk 2: Memory Bloat

**Problem:** Too many views = OOM

**Mitigation:**
- Limit total view count
- Use Lazy strategy for rare queries
- Add memory budgets
- Implement LRU eviction

### Risk 3: Performance Regression

**Problem:** Router overhead slower than direct query

**Mitigation:**
- Benchmark routing overhead (<10ns target)
- Profile hot paths
- Optimize pattern matching
- Add performance tests

### Risk 4: API Complexity

**Problem:** Too many options confuse users

**Mitigation:**
- Provide sensible defaults
- Pre-build common views (RBAC)
- Hide complexity behind simple API
- Document clearly

---

## Success Criteria

### Must Have
- ✅ 100-500ns for RBAC permission checks
- ✅ Support ad-hoc queries (fallback to 5-10µs)
- ✅ No breaking changes to existing API
- ✅ 91+ tests still passing

### Should Have
- ✅ 1-3µs for common query patterns
- ✅ Automatic view invalidation
- ✅ Simple API for custom views
- ✅ Memory < 3x base data

### Nice to Have
- ⭐ Query optimization hints
- ⭐ View statistics/monitoring
- ⭐ Automatic view selection
- ⭐ Distributed view updates

---

## Next Steps Decision Tree

```
Do you need RBAC + general queries?
├─ YES → Hybrid Approach (Recommended)
│  ├─ Timeline: 3-4 sessions
│  ├─ Performance: 100-500ns common, 1-10µs others
│  └─ Flexibility: High
│
├─ RBAC only, max speed → Pure Pre-Compute (Option A)
│  ├─ Timeline: 1-2 sessions
│  ├─ Performance: 100-500ns always
│  └─ Flexibility: Low
│
└─ General queries only → Query Engine (Option C)
   ├─ Timeline: 4-5 sessions
   ├─ Performance: 5-10µs (+ cache)
   └─ Flexibility: Highest
```

---

## Recommendation

**Go with Hybrid Approach (Materialized Views + Router)**

**Why:**
1. ✅ Achieves 100-500ns for RBAC (your primary use case)
2. ✅ Supports general queries when needed
3. ✅ Reasonable complexity (3-4 sessions)
4. ✅ Extensible for future patterns
5. ✅ Graceful performance degradation

**Implementation Order:**
1. Session 1: View foundation
2. Session 2: Query router
3. Session 3: RBAC views + benchmarks
4. Session 4 (optional): General query API

**Performance Target:**
- RBAC: 100-500ns (10-270x faster than Rego) ✅
- Other queries: 1-10µs depending on pattern ✅
- Memory: 2-3x base data (acceptable) ✅

---

## Final Decision Required

**Before implementing, please confirm:**

1. ✅ Hybrid approach acceptable?
2. ✅ 2-3x memory overhead OK?
3. ✅ 3-4 session timeline works?
4. ✅ Start with RBAC views first?

**Once confirmed, I'll proceed with implementation.**

---

**Status:** 📋 PLANNING COMPLETE - AWAITING APPROVAL
