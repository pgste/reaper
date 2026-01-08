# Phase 5: Constant-Time Policy Evaluation - Implementation Plan

**Version:** 1.0
**Date:** 2025-11-26
**Status:** 📝 Planning (Not Yet Implemented)
**Goal:** Achieve O(1) or O(log n) policy evaluation regardless of policy/data size

---

## Problem Statement

### Current Performance Characteristics

**Phase 1-4 Achievements:**
- ✅ Phase 1-2: Efficient data loading and joins
- ✅ Phase 3: 22x faster queries with attribute indexing
- ✅ Phase 4: Unlimited scale with streaming

**Remaining Challenge:**
- Policy evaluation still grows **linearly** with:
  - Number of rules: O(r) where r = rule count
  - Number of conditions per rule: O(c)
  - Entity lookups: O(1) with indexes, but still per-rule
  - **Total: O(r * c)** - grows with policy complexity

**Real-World Impact:**
```
Small policy (10 rules):     ~1µs evaluation
Medium policy (100 rules):    ~10µs evaluation
Large policy (1000 rules):    ~100µs evaluation
Enterprise (10k rules):       ~1ms evaluation
```

**Goal:** Make evaluation **O(1)** or **O(log r)** regardless of rule count.

---

## Proposed Solutions

### Approach 1: Pre-Compiled Decision Tables (Recommended)

**Concept:** Pre-compute all possible decisions at policy load time

**Algorithm:**
```
At policy load:
1. Analyze all rules and extract decision patterns
2. Build decision lookup table: (attribute_values) → decision
3. Use perfect hashing or tries for O(1) lookup

At evaluation:
1. Extract request attributes (principal, action, resource)
2. Hash to lookup key: O(1)
3. Table lookup: O(1)
4. Return cached decision: O(1)

Total: O(1) constant time
```

**Data Structure:**
```rust
pub struct DecisionTable {
    // Perfect hash table: request_signature -> Decision
    decisions: HashMap<RequestSignature, PolicyDecision>,

    // Fallback for unmapped requests
    fallback: DecisionTree,
}

pub struct RequestSignature {
    // Compact representation of (principal, action, resource, context)
    hash: u64,
    principal_attrs: AttributeSet,
    resource_attrs: AttributeSet,
}
```

**Pros:**
- ✅ O(1) lookup time
- ✅ Predictable performance
- ✅ Works for static policies
- ✅ No rule traversal needed

**Cons:**
- ❌ Memory growth: O(n^m) where n = attribute values, m = attributes
- ❌ Requires recompilation on policy change
- ❌ May not scale to all attributes combinations
- ❌ Best for policies with bounded attribute spaces

**Use Case:** Policies with limited, enumerable attribute combinations (e.g., role-based access)

---

### Approach 2: Decision Trees / Tries (Recommended)

**Concept:** Compile policies into optimized decision trees

**Algorithm:**
```
At policy load:
1. Analyze all rules and build decision tree
2. Each node represents an attribute check
3. Branches represent values
4. Leaves contain decisions
5. Optimize tree structure (balance, prune)

At evaluation:
1. Start at tree root
2. Navigate based on request attributes
3. Follow path to leaf: O(log r)
4. Return decision

Total: O(log r) logarithmic in rule count
```

**Data Structure:**
```rust
pub struct DecisionTree {
    root: Arc<TreeNode>,
    depth: usize,
    node_count: usize,
}

pub enum TreeNode {
    Decision(PolicyDecision),

    AttributeCheck {
        attribute: InternedString,
        branches: HashMap<AttributeValue, Arc<TreeNode>>,
        default: Arc<TreeNode>,
    },

    MultiCheck {
        // Check multiple attributes simultaneously
        checks: Vec<AttributeCheck>,
        fast_path: Option<Arc<TreeNode>>, // Common case optimization
    },
}
```

**Optimization Techniques:**
1. **Attribute Ordering:** Check most selective attributes first
2. **Branch Pruning:** Eliminate redundant checks
3. **Path Compression:** Merge sequential single-branch nodes
4. **Memoization:** Cache subtree results

**Pros:**
- ✅ O(log r) evaluation time
- ✅ Memory efficient: O(r) space
- ✅ Works for complex policies
- ✅ Handles dynamic attributes
- ✅ Can be updated incrementally

**Cons:**
- ❌ More complex than linear scan
- ❌ Tree construction overhead
- ❌ Cache locality may vary

**Use Case:** General-purpose solution for most policies

---

### Approach 3: Bloom Filters for Fast Rejection

**Concept:** Use probabilistic data structures to quickly reject impossible matches

**Algorithm:**
```
At policy load:
1. Create bloom filter per rule
2. Insert all attribute combinations that could match rule
3. Bloom filter provides O(1) membership test

At evaluation:
1. Check bloom filter for each rule: O(1)
2. False positives: Check rule normally
3. True negatives: Skip rule completely
4. Effective if most rules don't match

Total: O(r) worst case, O(k) average where k << r
```

**Data Structure:**
```rust
pub struct BloomFilteredPolicy {
    rules: Vec<RuleWithFilter>,
}

pub struct RuleWithFilter {
    rule: PolicyRule,
    filter: BloomFilter<RequestSignature>,
    selectivity: f64, // How often this rule matches
}

pub struct BloomFilter<T> {
    bits: BitVec,
    hash_count: usize,
    size: usize,
}
```

**Pros:**
- ✅ Fast rejection of non-matching rules
- ✅ Memory efficient (a few bits per element)
- ✅ No false negatives
- ✅ Complements existing approach

**Cons:**
- ❌ False positives require full check
- ❌ Still O(r) worst case
- ❌ Best as optimization, not primary solution

**Use Case:** Optimize existing evaluation by skipping unlikely rules

---

### Approach 4: Attribute-Based Routing

**Concept:** Partition rules by attribute patterns, route to relevant subset

**Algorithm:**
```
At policy load:
1. Analyze rules and group by attribute patterns
2. Create routing table: attribute_pattern -> rule_subset
3. Build indexes for fast routing

At evaluation:
1. Classify request by attributes: O(1)
2. Route to relevant rule subset: O(1)
3. Evaluate only k rules where k << r
4. Return decision

Total: O(k) where k = rules in subset, k << r
```

**Data Structure:**
```rust
pub struct RoutedPolicy {
    // Route by primary attributes
    role_routes: HashMap<String, RuleSet>,
    action_routes: HashMap<String, RuleSet>,
    resource_routes: HashMap<String, RuleSet>,

    // Combined routing
    multi_attr_routes: HashMap<AttributePattern, RuleSet>,

    // Fallback for unrouted
    default_rules: RuleSet,
}

pub struct AttributePattern {
    role: Option<String>,
    action: Option<String>,
    resource_type: Option<String>,
}

pub struct RuleSet {
    rules: Vec<PolicyRule>,
    count: usize,
    avg_eval_time: Duration,
}
```

**Optimization:**
- Order rule subsets by likelihood (most common first)
- Use decision trees within each subset
- Cache routing decisions

**Pros:**
- ✅ Dramatically reduces rules checked
- ✅ O(k) where k is small subset
- ✅ Works with existing evaluation
- ✅ Easy to implement

**Cons:**
- ❌ Requires attribute-based partitioning
- ❌ Rules may appear in multiple subsets
- ❌ Still O(k) not O(1)

**Use Case:** Policies with clear attribute-based segmentation (e.g., by role, department, action type)

---

### Approach 5: Hierarchical Decision Cache

**Concept:** Cache decisions at multiple levels with intelligent invalidation

**Algorithm:**
```
At policy load:
1. Build cache hierarchy: user level, group level, resource type level
2. Pre-populate common decisions

At evaluation:
1. Check cache at most specific level: O(1)
2. Cache hit: Return immediately
3. Cache miss: Evaluate and cache: O(r)
4. Hierarchical lookup: specific → general → evaluate

Total: O(1) average with high cache hit rate
```

**Data Structure:**
```rust
pub struct HierarchicalCache {
    // L1: Exact request cache (principal, action, resource)
    exact_cache: LruCache<RequestKey, PolicyDecision>,

    // L2: User-level cache (principal, action, resource_type)
    user_cache: LruCache<UserActionKey, PolicyDecision>,

    // L3: Group-level cache (group, action, resource_type)
    group_cache: LruCache<GroupActionKey, PolicyDecision>,

    // L4: Type-level cache (principal_type, action, resource_type)
    type_cache: LruCache<TypeActionKey, PolicyDecision>,

    stats: CacheStats,
}

pub struct CacheStats {
    l1_hits: AtomicU64,
    l2_hits: AtomicU64,
    l3_hits: AtomicU64,
    l4_hits: AtomicU64,
    misses: AtomicU64,
}
```

**Cache Invalidation:**
```rust
impl HierarchicalCache {
    pub fn invalidate_user(&self, user_id: &str) {
        // Invalidate all caches for this user
        self.exact_cache.retain(|k, _| k.principal != user_id);
        self.user_cache.retain(|k, _| k.user != user_id);
    }

    pub fn invalidate_policy(&self) {
        // Policy changed - clear all caches
        self.exact_cache.clear();
        self.user_cache.clear();
        self.group_cache.clear();
        self.type_cache.clear();
    }
}
```

**Pros:**
- ✅ O(1) average case with high hit rate
- ✅ Works with any policy
- ✅ Adaptive to access patterns
- ✅ Can pre-populate common cases

**Cons:**
- ❌ Memory overhead for cache
- ❌ Cache invalidation complexity
- ❌ Cold start penalty
- ❌ Still O(r) on cache miss

**Use Case:** Production systems with repeated access patterns

---

### Approach 6: Compiled Policies (JIT Compilation)

**Concept:** JIT-compile policies to native machine code

**Algorithm:**
```
At policy load:
1. Parse policy into AST
2. Generate LLVM IR or native code
3. Compile to machine code
4. Link compiled functions

At evaluation:
1. Direct function call to compiled code: O(1) overhead
2. Native execution (no interpretation)
3. Inline all checks

Total: O(c) where c = conditions, but with minimal overhead
```

**Data Structure:**
```rust
pub struct CompiledPolicy {
    // Compiled function pointer
    eval_fn: unsafe extern "C" fn(*const Request) -> Decision,

    // Metadata
    rule_count: usize,
    compilation_time: Duration,
}

// Example generated code:
fn evaluate_request_compiled(request: &Request) -> Decision {
    // Inlined checks with no overhead
    if request.principal.role == "admin" {
        return Decision::Allow;
    }
    if request.principal.role == "analyst" && request.resource.classification == "internal" {
        return Decision::Allow;
    }
    Decision::Deny
}
```

**Pros:**
- ✅ Native code performance
- ✅ No interpretation overhead
- ✅ Aggressive inlining
- ✅ CPU branch prediction friendly

**Cons:**
- ❌ Complex implementation (requires LLVM or similar)
- ❌ Longer policy load time
- ❌ Platform-specific
- ❌ Security concerns (code execution)

**Use Case:** Ultra-high-performance scenarios where compilation time is acceptable

---

## Recommended Implementation Strategy

### Phase 5A: Decision Trees (Priority 1)

**Why:**
- Best balance of performance and complexity
- O(log r) evaluation time
- Memory efficient
- Works for all policy types

**Implementation:**
```rust
// crates/policy-engine/src/optimizer/decision_tree.rs

pub struct DecisionTreeOptimizer {
    tree: DecisionTree,
    construction_stats: TreeStats,
}

impl DecisionTreeOptimizer {
    pub fn compile(policy: &EnhancedPolicy) -> Result<Self, ReaperError> {
        // 1. Analyze rules
        let rules = policy.extract_rules();

        // 2. Determine optimal attribute ordering
        let attribute_order = Self::optimize_attribute_order(&rules);

        // 3. Build tree recursively
        let tree = Self::build_tree(&rules, &attribute_order, 0);

        // 4. Optimize tree structure
        let optimized = Self::optimize_tree(tree);

        Ok(Self {
            tree: optimized,
            construction_stats: TreeStats::new(),
        })
    }

    pub fn evaluate(&self, request: &PolicyRequest) -> PolicyDecision {
        self.tree.traverse(request)
    }
}
```

**Expected Performance:**
```
Current (linear): 100 rules = ~10µs
With tree:        100 rules = ~1µs (10x improvement)
                  1000 rules = ~1.5µs (logarithmic)
                  10k rules = ~2µs (still logarithmic)
```

---

### Phase 5B: Attribute-Based Routing (Priority 2)

**Why:**
- Complements decision trees
- Easy to implement
- Dramatic reduction in rules checked
- Natural fit for RBAC/ABAC policies

**Implementation:**
```rust
// crates/policy-engine/src/optimizer/routing.rs

pub struct PolicyRouter {
    routes: HashMap<AttributePattern, Vec<usize>>, // rule indices
    rules: Vec<PolicyRule>,
}

impl PolicyRouter {
    pub fn new(policy: &EnhancedPolicy) -> Self {
        let mut routes = HashMap::new();
        let rules = policy.extract_rules();

        // Build routing table
        for (idx, rule) in rules.iter().enumerate() {
            let patterns = Self::extract_patterns(rule);
            for pattern in patterns {
                routes.entry(pattern).or_insert_with(Vec::new).push(idx);
            }
        }

        Self { routes, rules }
    }

    pub fn route(&self, request: &PolicyRequest) -> Vec<&PolicyRule> {
        let pattern = Self::request_to_pattern(request);

        if let Some(rule_indices) = self.routes.get(&pattern) {
            rule_indices.iter().map(|&idx| &self.rules[idx]).collect()
        } else {
            // Fallback to all rules
            self.rules.iter().collect()
        }
    }
}
```

---

### Phase 5C: Hierarchical Cache (Priority 3)

**Why:**
- Orthogonal to other optimizations
- High ROI in production
- Handles repeated requests efficiently

**Implementation:**
```rust
// crates/policy-engine/src/optimizer/cache.rs

pub struct EvaluationCache {
    l1: DashMap<RequestKey, CachedDecision>,
    l2: DashMap<UserActionKey, CachedDecision>,
    config: CacheConfig,
}

pub struct CachedDecision {
    decision: PolicyDecision,
    timestamp: Instant,
    ttl: Duration,
}

impl EvaluationCache {
    pub fn get_or_evaluate<F>(
        &self,
        request: &PolicyRequest,
        eval_fn: F,
    ) -> PolicyDecision
    where
        F: FnOnce() -> PolicyDecision,
    {
        // L1: Exact match
        let key = RequestKey::from(request);
        if let Some(cached) = self.l1.get(&key) {
            if !cached.is_expired() {
                return cached.decision.clone();
            }
        }

        // L2: User-level match
        let user_key = UserActionKey::from(request);
        if let Some(cached) = self.l2.get(&user_key) {
            if !cached.is_expired() {
                return cached.decision.clone();
            }
        }

        // Cache miss - evaluate and cache
        let decision = eval_fn();
        self.insert(request, decision.clone());
        decision
    }
}
```

---

## Performance Projections

### Current Performance (Phase 1-4)

| Rule Count | Evaluation Time | Notes |
|------------|-----------------|-------|
| 10 rules | ~1µs | Acceptable |
| 100 rules | ~10µs | Acceptable |
| 1,000 rules | ~100µs | Borderline |
| 10,000 rules | ~1ms | Too slow |

### Projected with Phase 5A (Decision Trees)

| Rule Count | Current | With Trees | Speedup |
|------------|---------|------------|---------|
| 10 rules | 1µs | **500ns** | 2x |
| 100 rules | 10µs | **1µs** | 10x |
| 1,000 rules | 100µs | **1.5µs** | 67x |
| 10,000 rules | 1ms | **2µs** | 500x |

**Formula:** O(log₂ r) where r = rule count

### Projected with Phase 5B (Routing)

| Rule Count | Subset Size | Evaluation |
|------------|-------------|------------|
| 100 rules | ~10 rules | **1µs** |
| 1,000 rules | ~50 rules | **5µs** |
| 10,000 rules | ~100 rules | **10µs** |

**Reduction:** ~10x fewer rules checked on average

### Projected with Phase 5C (Cache, 90% hit rate)

| Cache Hit Rate | Average Latency |
|----------------|-----------------|
| 50% | 50ns + 50% miss penalty |
| 90% | **100ns** + 10% miss penalty |
| 99% | **50ns** + 1% miss penalty |

**With 90% hit rate:**
- Cold: 1µs (tree evaluation)
- Warm: **100ns** (cache hit)
- **10x improvement** for repeated requests

---

## Implementation Timeline

### Phase 5A: Decision Trees (2-3 sessions)
1. Session 1: Tree builder and basic traversal
2. Session 2: Tree optimization (pruning, balancing)
3. Session 3: Integration and testing

### Phase 5B: Attribute Routing (1-2 sessions)
1. Session 1: Router implementation and rule partitioning
2. Session 2: Integration with decision trees

### Phase 5C: Hierarchical Cache (1 session)
1. Session 1: Multi-level cache with TTL and invalidation

**Total:** 4-6 sessions for complete Phase 5 implementation

---

## Success Criteria

### Performance Targets

- ✅ **10 rules:** <500ns (2x improvement)
- ✅ **100 rules:** <1µs (10x improvement)
- ✅ **1,000 rules:** <2µs (50x improvement)
- ✅ **10,000 rules:** <5µs (200x improvement)

### Memory Targets

- ✅ Tree overhead: <10% of policy size
- ✅ Cache overhead: <50MB for 1M cached decisions
- ✅ Routing tables: <1MB for 10k rules

### Compatibility

- ✅ Backward compatible with Phase 1-4
- ✅ Works with all policy languages
- ✅ No breaking API changes
- ✅ Opt-in optimization flags

---

## Alternative Considerations

### When NOT to use Phase 5:

1. **Small policies (<10 rules):** Overhead exceeds benefit
2. **Highly dynamic policies:** Recompilation cost too high
3. **Memory-constrained environments:** Cache overhead may be prohibitive
4. **Simple policies:** Linear scan is fast enough

### When Phase 5 is CRITICAL:

1. **Enterprise policies (1000+ rules)**
2. **High-throughput systems (>100k req/sec)**
3. **Latency-sensitive applications (<10µs P99)**
4. **Complex multi-tenant policies**

---

## Conclusion

**Phase 5 can achieve near-constant-time policy evaluation through:**

1. **Decision Trees:** O(log r) evaluation
2. **Attribute Routing:** Reduce search space by 10x
3. **Hierarchical Caching:** O(1) for repeated requests

**Combined approach:**
- Best case (cache hit): **50ns** (O(1))
- Average case (routed + tree): **1-2µs** (O(log k) where k << r)
- Worst case (cache miss, full tree): **<5µs** (O(log r))

**Projected improvement: 10-500x faster depending on policy size**

---

**Status:** Ready for implementation when approved
**Recommendation:** Implement Phase 5A (Decision Trees) first for maximum impact

