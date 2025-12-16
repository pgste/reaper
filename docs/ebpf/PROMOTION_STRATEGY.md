# Policy & Data Promotion Strategy

This document summarizes the complete approach for promoting policies and data to eBPF kernel space.

## Executive Summary

We're implementing a **3-tier adaptive data layer** with **intelligent policy promotion** that:

✅ **Supports 10K to 1M entities** with adaptive scaling
✅ **Handles JWT, RBAC, ABAC, and ReBAC** with unified schema
✅ **Auto-promotes hot paths** with <100ns latency
✅ **Validates everything** via CLI before deployment
✅ **Provides clear feedback** on what can/cannot be promoted

## Key Design Decisions

### 1. Adaptive Multi-Tier Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Tier 1: Direct Maps (< 10K entities)    ~50ns lookup   │
│  • Single map per entity type (USERS, ROLES, etc.)      │
│  • Always promoted                                       │
│  • Memory: ~20MB                                         │
├─────────────────────────────────────────────────────────┤
│  Tier 2: Sharded Maps (10K-100K)         ~100ns lookup  │
│  • 16 shards per entity type                             │
│  • Auto-promoted if >80% hit rate                        │
│  • Memory: ~200MB                                        │
├─────────────────────────────────────────────────────────┤
│  Tier 3: Bloom + LRU (100K-1M)           ~150ns lookup  │
│  • Bloom filter for negative lookups                     │
│  • 10K LRU cache in userspace                            │
│  • Selective promotion (hot entities only)               │
│  • Memory: ~2GB                                           │
└─────────────────────────────────────────────────────────┘
```

### 2. Unified Entity Schema

**Single schema supports all use cases:**

```rust
struct Entity {
    id: [u8; 64],              // "user:alice", "role:admin"
    entity_type: EntityType,   // User, Role, Group, Resource

    // ABAC support
    string_attrs: [StringAttr; 8],   // "dept": "eng", "role": "admin"
    numeric_attrs: [NumericAttr; 8], // "age": 30, "clearance": 7

    // ReBAC support
    relationships: [Relationship; 16], // "member_of:group:eng"

    // Flags (boolean attrs)
    flags: u64,  // is_active, is_verified, etc.
}
```

**This supports:**
- **JWT**: Claims as attributes (`sub`, `exp`, `role`)
- **RBAC**: Users → Roles → Permissions via relationships
- **ABAC**: Attribute comparisons (`clearance >= 5`)
- **ReBAC**: Graph traversal (`user member_of group can_access resource`)

### 3. Promotion Decision Tree

```
Policy Condition:
│
├─ Condition::True
│  └─ ✅ PROMOTE (wildcard, ~10ns)
│
├─ user.role == "admin"
│  ├─ Is "role" a known attribute?
│  │  └─ ✅ PROMOTE (1 lookup, ~50ns)
│  └─ Else: Generic KV lookup
│     └─ ✅ PROMOTE (1 lookup, ~70ns)
│
├─ user.clearance >= resource.min_clearance
│  └─ ✅ PROMOTE (2 lookups, ~100ns)
│
├─ user has_role "admin" with has_permission "write"
│  └─ ✅ PROMOTE (2-hop traversal, ~120ns)
│
├─ user.role in ["admin", "manager"]
│  ├─ ≤ 64 values?
│  │  └─ ✅ PROMOTE (bitmask check, ~60ns)
│  └─ Else
│     └─ ❌ USERSPACE
│
├─ {p | p := user.perms[_]}.count() > 0
│  └─ ❌ USERSPACE (comprehension)
│
└─ email matches ".*@company.com"
   └─ ❌ USERSPACE (regex)
```

## What Can Be Promoted

### ✅ Always Promotable

| Pattern | Example | Latency |
|---------|---------|---------|
| Constants | `true`, `false` | 10ns |
| Exact match | `user.role == "admin"` | 50ns |
| Numeric compare | `user.age >= 18` | 50ns |
| Multi-field AND | `user.role == "admin" && user.dept == "eng"` | 100ns |
| Small IN checks | `role in ["admin", "manager"]` | 60ns |
| Relationship check | `user has_role "admin"` | 50ns |
| Two-hop traversal | `user → role → permission` | 120ns |

### ⚠️ Conditionally Promotable

| Pattern | Condition | Fallback |
|---------|-----------|----------|
| Large IN checks | ≤ 64 values | Userspace |
| Deep traversal | ≤ 3 hops | Userspace |
| String operations | Simple prefix/suffix only | Userspace |
| Wildcards | Pattern matching only | Userspace |

### ❌ Never Promotable

| Pattern | Reason |
|---------|--------|
| Comprehensions | Dynamic iteration, unbounded loops |
| Regex | No regex engine in eBPF |
| Variable assignments | Stack limitations |
| Complex expressions | Limited helper functions |
| Deep nesting | Pointer chasing limits |

## Data Loading Workflow

```
1. Developer creates JSON dataset:
   {
     "entities": {
       "user:alice": {
         "type": "user",
         "string_attrs": {"role": "admin"},
         "numeric_attrs": {"clearance": 7},
         "relationships": [
           {"type": "member_of", "target": "group:eng"}
         ]
       }
     }
   }

2. Validation (CLI):
   $ reaper-cli validate-data --file users.json --check-ebpf

   ✓ Schema valid
   ✓ 1,234 entities
   ✓ Tier 1 (Direct maps)
   ✓ Estimated memory: 2.4 MB
   ⚠ Entity 'user:bob' has 9 string attrs (max 8)

3. Load into Reaper:
   $ curl -X POST http://localhost:8080/api/v1/data \
       --data-binary @users.json

4. Auto-promotion:
   - System monitors access patterns
   - Promotes hot paths to eBPF
   - Logs promotion events

5. Verification:
   $ reaper-cli stats
   Fast Path: 85% (1,234 hot entities in eBPF)
   Slow Path: 15%
```

## Policy Analysis Workflow

```
1. Developer writes policy:
   policy example {
     default: deny,
     rule admin_access {
       allow if user.role == "admin"
     },
     rule clearance_check {
       allow if user.clearance >= resource.min_clearance
     },
     rule complex {
       allow if {p | p := user.perms[_]}.count() > 0
     }
   }

2. Analysis (CLI):
   $ reaper-cli analyze-policy --file policy.reap --check-ebpf

   === Policy Analysis ===
   Total Rules: 3
   ✓ 2 eBPF-ready (66%)
   ✗ 1 userspace (34%)

   eBPF-Ready:
   ✓ admin_access (complexity: 1, ~50ns)
   ✓ clearance_check (complexity: 2, ~100ns)

   Userspace:
   ✗ complex (reason: comprehension)

   Recommendations:
   • Simplify 'complex' rule to avoid comprehension
   • Expected fast path: 66%

3. Deploy policy:
   $ reaper-cli policy deploy --file policy.reap

   Deploying 2 rules to eBPF...
   Keeping 1 rule in userspace...
   ✓ Deployment complete

4. Monitor:
   $ reaper-cli stats --watch

   Fast Path: 85% (↑ from 66% - auto-promotion working!)
   eBPF Policies: 5 (3 auto-promoted)
```

## CLI Commands Summary

```bash
# Data validation
reaper-cli validate-data --file users.json
reaper-cli validate-data --file users.json --check-ebpf
reaper-cli validate-data --file users.json --format table

# Policy analysis
reaper-cli analyze-policy --file policy.reap
reaper-cli analyze-policy --file policy.reap --check-ebpf
reaper-cli analyze-policy --file policy.reap --show-recommendations

# Deployment
reaper-cli data load --file users.json
reaper-cli policy deploy --file policy.reap --auto-promote

# Monitoring
reaper-cli stats                    # One-time snapshot
reaper-cli stats --watch           # Live updates
reaper-cli promotion-log           # Show auto-promotion events
reaper-cli analyze-workload        # Analyze access patterns
```

## Performance Expectations

### Data Lookups

| Tier | Size | Lookup Latency (p99) | Memory |
|------|------|----------------------|--------|
| 1 | < 10K | 50ns | ~20MB |
| 2 | 10K-100K | 100ns | ~200MB |
| 3 | 100K-1M | 150ns | ~2GB |

### Policy Evaluation

| Complexity | Example | Latency (p99) |
|------------|---------|---------------|
| Trivial | `true` | 10ns |
| Simple | `user.role == "admin"` | 50ns |
| Multi-field | `role == "admin" && dept == "eng"` | 100ns |
| Relationship | `user → role → permission` | 120ns |
| Userspace | Comprehensions, regex | 10-50µs |

### Throughput

- **Fast Path**: > 1M evaluations/second
- **Slow Path**: > 100K evaluations/second
- **Mixed (80/20)**: > 800K evaluations/second

## Implementation Status

| Phase | Status | Deliverables |
|-------|--------|--------------|
| Phase 1: Data Structures | 📝 Planned | Core entity types, eBPF maps |
| Phase 2: Data Ingestion | 📝 Planned | Validation, loading, conversion |
| Phase 3: Policy Analysis | 📝 Planned | Promotability analyzer |
| Phase 4: CLI Tools | 📝 Planned | validate-data, analyze-policy |
| Phase 5: Auto-Promotion | 📝 Planned | Learning engine integration |

## Next Steps

**Recommended Implementation Order:**

1. **Week 1**: Phase 1 + 2 (Data layer foundation)
   - Implement entity structures
   - Build validation engine
   - Create data loader
   - Test with sample datasets

2. **Week 2**: Phase 3 (Intelligence layer)
   - Build policy analyzer
   - Implement promotion decision logic
   - Create complexity estimator

3. **Week 3**: Phase 4 + 5 (User experience)
   - Build CLI validation tools
   - Integrate auto-promotion
   - Add monitoring/feedback

**Ready to proceed?**

Let me know if you want me to:
- **Option A**: Start implementing Phase 1 + 2 (data layer)
- **Option B**: Create working prototypes/demos first
- **Option C**: Adjust the design based on your feedback

All documentation is in `docs/ebpf/`:
- `DATA_SCHEMA.md` - Complete schema specification
- `IMPLEMENTATION_PLAN.md` - Detailed code-level plan
- `PROMOTION_STRATEGY.md` - This document (high-level overview)
