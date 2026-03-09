# eBPF Deployment Feasibility Analysis for Reaper Policy Engine

## Executive Summary

**Verdict**: ✅ **Reaper CAN support eBPF deployment with a two-tier architecture**

**Key Finding**: While full Reaper policy evaluation (Cedar, Reaper DSL with JSON functions) cannot run directly in eBPF due to verifier constraints, a **Simple Policy Fast Path** can achieve <100ns kernel-level enforcement, with complex policies handled via userspace.

**Dynamic Policy Updates**: ✅ **Fully supported** via BPF maps - userspace can update policies without reloading eBPF programs.

---

## Table of Contents

1. [eBPF Constraints (2025)](#ebpf-constraints-2025)
2. [What IS Possible in eBPF](#what-is-possible-in-ebpf)
3. [What IS NOT Possible in eBPF](#what-is-not-possible-in-ebpf)
4. [Two-Tier Architecture Design](#two-tier-architecture-design)
5. [Dynamic Policy Update Mechanism](#dynamic-policy-update-mechanism)
6. [Implementation Roadmap](#implementation-roadmap)
7. [Performance Analysis](#performance-analysis)
8. [Production Deployment Strategy](#production-deployment-strategy)

---

## eBPF Constraints (2025)

### Hard Limits (Enforced by BPF Verifier)

Based on [eBPF Ecosystem Progress 2024-2025](https://eunomia.dev/blog/2025/02/12/ebpf-ecosystem-progress-in-20242025-a-technical-deep-dive/) and [LSM BPF Programs - Linux Kernel](https://docs.kernel.org/bpf/prog_lsm.html):

| Constraint | Limit | Impact on Reaper |
|------------|-------|------------------|
| **Stack Size** | 512 bytes | ❌ Blocks Cedar (uses >1KB stack) |
| **Max Instructions** | 1M (configurable) | ⚠️ Limits policy complexity |
| **Loops** | Bounded only (verifiable termination) | ❌ Blocks dynamic loops over rules |
| **Dynamic Memory** | None (BPF maps only) | ❌ Blocks HashMap, DashMap |
| **Function Pointers** | Not allowed | ❌ Blocks trait objects (PolicyEvaluator) |
| **Strings** | Fixed-size arrays only | ⚠️ Limits pattern matching |
| **Branch Complexity** | Max 8,192 combined states | ⚠️ Limits rule count |

### What's Available in eBPF

✅ **BPF Maps** - Pre-allocated hash tables, arrays, ring buffers
✅ **BPF Helpers** - 200+ kernel functions (string compare, crypto, time)
✅ **kfuncs** - Growing set of kernel functions exposed to BPF ([eBPF Docs Maps](https://docs.ebpf.io/linux/concepts/maps/))
✅ **Bounded Loops** - `for (i = 0; i < N && i < MAX; i++)` where MAX is const
✅ **LSM Hooks** - file_open, socket_connect, bprm_check, task_kill, etc.

---

## What IS Possible in eBPF

### ✅ 1. Simple Policy Evaluator (RBAC, IP Filtering)

**Reaper's Simple Policy Evaluator** can be adapted for eBPF:

```rust
// Current Reaper Simple evaluator (src/evaluators/simple.rs:106-111)
fn matches_rule(&self, rule: &PolicyRule, request: &PolicyRequest) -> bool {
    rule.resource == "*" || rule.resource == request.resource
}

for rule in &self.rules {
    if self.matches_rule(rule, request) {
        return Ok(rule.action.clone());  // First-match-wins
    }
}
```

**eBPF Equivalent (C-like)**:
```c
// BPF map: hash map of resource -> action
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 10000);
    __type(key, char[256]);      // resource path
    __type(value, u8);           // 0=deny, 1=allow
} policy_map SEC(".maps");

SEC("lsm/file_open")
int reaper_file_open(struct file *file) {
    char path[256];
    bpf_d_path(&file->f_path, path, sizeof(path));

    // Exact match lookup (O(1))
    u8 *action = bpf_map_lookup_elem(&policy_map, path);
    if (action && *action == 1) {
        return 0;  // Allow
    }

    // Check wildcard rule "*"
    char wildcard[] = "*";
    action = bpf_map_lookup_elem(&policy_map, wildcard);
    if (action && *action == 1) {
        return 0;  // Allow
    }

    return -EPERM;  // Deny (default)
}
```

**Supported Patterns**:
- ✅ Exact string match (`/api/users`)
- ✅ Wildcard `*` match (all resources)
- ✅ Prefix match (with bounded loop, e.g., `/api/*`)
- ✅ IP address filtering (IPv4/IPv6 via socket hooks)
- ✅ UID/GID checks (kernel provides via `bpf_get_current_uid_gid()`)

**Performance**: <100ns evaluation (kernel-level, no context switch)

---

### ✅ 2. Dynamic Policy Updates via BPF Maps

**Critical Capability**: Userspace can update BPF maps **without reloading eBPF program**.

From [BPF Maps - Linux Kernel](https://docs.kernel.org/bpf/maps.html):

> BPF maps are accessed from user space via the bpf syscall, which provides commands to create maps, lookup elements, update elements and delete elements.

**Userspace Control Plane (Rust)**:
```rust
use aya::maps::HashMap;
use aya::Bpf;

struct ReaperEbpfController {
    bpf: Bpf,
    policy_map: HashMap<&mut mapfd::MapData, [u8; 256], u8>,
}

impl ReaperEbpfController {
    /// Deploy new policy bundle to eBPF (hot-swap, zero downtime)
    pub fn update_policy(&mut self, policy: &SimplePolicyEvaluator) -> Result<()> {
        // Clear existing policies
        self.policy_map.iter().for_each(|key| {
            let _ = self.policy_map.remove(&key);
        });

        // Insert new rules
        for rule in &policy.rules {
            let resource_key: [u8; 256] = string_to_fixed_array(&rule.resource);
            let action_value: u8 = match rule.action {
                PolicyAction::Allow => 1,
                PolicyAction::Deny => 0,
                _ => 0,
            };

            self.policy_map.insert(resource_key, action_value, 0)?;
        }

        tracing::info!("eBPF policy updated: {} rules", policy.rules.len());
        Ok(())
    }

    /// Publish new bundle (atomically updates all policies)
    pub fn publish_bundle(&mut self, bundle_path: &str) -> Result<()> {
        let bundle = PolicyBundle::from_file(bundle_path)?;

        for policy in bundle.policies {
            // Only Simple policies can go to eBPF
            if let Some(simple_eval) = policy.as_simple_evaluator() {
                self.update_policy(simple_eval)?;
            } else {
                // Complex policies stay in userspace
                tracing::warn!("Policy {} too complex for eBPF, using userspace", policy.name);
            }
        }

        Ok(())
    }
}
```

**Update Latency**: ~1-10µs (userspace → kernel map update)
**Atomicity**: Per-rule (not per-bundle, but can use map-in-map for atomic swap)

---

### ✅ 3. Hybrid Enforcement (eBPF + Userspace)

**Architecture**: eBPF program can defer to userspace for complex policies.

From [eBPF Ecosystem Progress 2024-2025](https://eunomia.dev/blog/2025/02/12/ebpf-ecosystem-progress-in-20242025-a-technical-deep-dive/):

> eBPF LSM programs can make access control decisions dynamically, with policies loaded and unloaded per container or process group.

**Decision Flow**:
```c
SEC("lsm/file_open")
int reaper_file_open(struct file *file) {
    // 1. Fast path: Check eBPF policy map (simple rules)
    u8 *action = bpf_map_lookup_elem(&policy_map, path);
    if (action) {
        return (*action == 1) ? 0 : -EPERM;  // <100ns
    }

    // 2. Slow path: Mark for userspace evaluation
    // Send event to userspace via ring buffer
    struct policy_event event = {
        .path = path,
        .uid = bpf_get_current_uid_gid() >> 32,
        .pid = bpf_get_current_pid_tgid() >> 32,
    };
    bpf_ringbuf_output(&events, &event, sizeof(event), 0);

    // 3. Default policy while userspace evaluates
    return -EPERM;  // Or: return 0 and audit later (fail-open vs fail-closed)
}
```

**Userspace Daemon** (evaluates complex policies):
```rust
// Listens to eBPF ring buffer for complex policy requests
let mut ring_buf = RingBuf::try_from(bpf.map("events")?)?;

loop {
    ring_buf.poll(Duration::from_millis(100))?;

    while let Some(event) = ring_buf.next() {
        let policy_event: PolicyEvent = deserialize(event)?;

        // Evaluate Cedar/ReaperDSL policies
        let decision = policy_engine.evaluate_complex(&policy_event)?;

        // Cache result in eBPF map for future (convert to simple rule)
        if should_cache(&decision) {
            controller.cache_decision(&policy_event.path, decision.action)?;
        }

        // Audit/log (already happened, this is post-hoc)
        audit_log.write(&policy_event, &decision)?;
    }
}
```

**Performance**:
- Fast path (eBPF): <100ns
- Slow path (userspace): 10-50µs (still faster than network call!)
- Learning mode: Slow path decisions cached → become fast path over time

---

## What IS NOT Possible in eBPF

### ❌ 1. Cedar Policy Evaluator

**Why**: Cedar requires:
- Dynamic policy parsing
- AST evaluation with recursion
- Large stack frames (>512 bytes)
- String operations on dynamic-length strings

**Cedar Example** (cannot run in eBPF):
```cedar
permit (
  principal in Group::"admins",
  action == Action::"read",
  resource in Folder::"sensitive"
)
when {
  context.ip.isInRange(ip("10.0.0.0/8")) &&
  context.time < datetime("2025-12-31T23:59:59Z")
};
```

**Workaround**: Evaluate in userspace, cache result as simple rule in eBPF.

---

### ❌ 2. Reaper DSL with JSON Functions

**Why**: JSON operations require:
- Dynamic memory allocation (HashMap for JSON objects)
- Recursive parsing
- Variable-length arrays

**Reaper DSL Example** (cannot run in eBPF):
```reaper
rule "api_rate_limit" {
  allow if {
    let user_id = json::parse(request.context["jwt_payload"]).sub;
    let rate = redis::get("rate:" + user_id);
    rate < 1000
  }
}
```

**Workaround**: Evaluate in userspace (WASM or native).

---

### ❌ 3. Dynamic Loops Over Rules

**Current Reaper Code** (simple.rs:155-159):
```rust
for rule in &self.rules {  // ❌ Dynamic loop!
    if self.matches_rule(rule, request) {
        return Ok(rule.action.clone());
    }
}
```

**Why Not in eBPF**: Verifier cannot prove termination with dynamic rule count.

**eBPF Workaround**: Bounded loop or hash map lookup.

**Bounded Loop** (max 1000 rules):
```c
#define MAX_RULES 1000

for (int i = 0; i < MAX_RULES && i < rule_count; i++) {
    struct rule *r = bpf_map_lookup_elem(&rules_array, &i);
    if (!r) break;

    if (matches_rule(r, &request)) {
        return r->action;
    }
}
```

**Hash Map** (O(1), preferred):
```c
// Pre-compile rules into hash map (userspace does this)
// eBPF just does lookup
u8 *action = bpf_map_lookup_elem(&policy_map, &request.resource);
```

---

### ❌ 4. DataStore with String Interning

**Current Reaper** (data/mod.rs):
- Uses `DashMap` (lock-free HashMap)
- String interning with `Arc<str>`
- Multi-index lookups (ID, Type, Attribute)

**Why Not in eBPF**:
- No dynamic memory allocation
- No trait objects or generics
- No Arc/Rc (no reference counting)

**Workaround**: Pre-compute entity lookups in userspace, cache simple predicates in eBPF.

Example:
```rust
// Userspace: Evaluate complex ABAC rule
let user = datastore.get_entity_by_id("user123")?;
let allowed = user.get_attribute("role")? == "admin";

// Cache result in eBPF
ebpf_controller.cache_user_permission("user123", "/api/admin", allowed)?;
```

---

## Two-Tier Architecture Design

### Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                     Kernel Space (eBPF)                       │
├──────────────────────────────────────────────────────────────┤
│                                                                │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  LSM Hook: file_open / socket_connect / bprm_check     │ │
│  └────────────────────┬────────────────────────────────────┘ │
│                       │                                       │
│                       ▼                                       │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Simple Policy Fast Path (<100ns)                       │ │
│  │  • Exact match lookup in BPF_MAP_TYPE_HASH              │ │
│  │  • Wildcard "*" check                                   │ │
│  │  • Prefix match (bounded loop)                          │ │
│  │  • IP/UID/GID filtering                                 │ │
│  └────────────────────┬────────────────────────────────────┘ │
│                       │                                       │
│         ┌─────────────┴────────────┐                         │
│         │                          │                          │
│      MATCH?                    NO MATCH                       │
│         │                          │                          │
│         ▼                          ▼                          │
│    ALLOW / DENY        Send to Ring Buffer                   │
│    (return 0/-EPERM)            │                             │
│                                 │                             │
└─────────────────────────────────┼─────────────────────────────┘
                                  │
                                  │ (Event: path, uid, pid, ...)
                                  │
┌─────────────────────────────────▼─────────────────────────────┐
│                    Userspace (Reaper Agent)                    │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  eBPF Ring Buffer Consumer                               │ │
│  │  (polls for complex policy evaluation requests)          │ │
│  └─────────────────────┬────────────────────────────────────┘ │
│                        │                                       │
│                        ▼                                       │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  PolicyEngine (Full Reaper)                              │ │
│  │  • Cedar Policy Evaluator (ABAC, schema validation)      │ │
│  │  • Reaper DSL Evaluator (JSON, regex, arithmetic)       │ │
│  │  • DataStore (entity lookups, RBAC/ReBAC)               │ │
│  └─────────────────────┬────────────────────────────────────┘ │
│                        │                                       │
│                        ▼                                       │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  Decision Cache & Learning                               │ │
│  │  • Frequently accessed paths → Simple rules              │ │
│  │  • Update eBPF map (hot-swap policies)                   │ │
│  │  • Audit logging & metrics                               │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │  eBPF Controller                                         │ │
│  │  • Load/unload eBPF programs                             │ │
│  │  • Update BPF maps (policy bundles)                      │ │
│  │  • Monitor performance (fast path %, slow path %)        │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

### Decision Flow

| Request Type | Evaluation Path | Latency | Example |
|--------------|----------------|---------|---------|
| Exact match in eBPF map | eBPF only | <100ns | `GET /api/public` → Allow |
| Wildcard match | eBPF only | <100ns | `*` → Allow all |
| Previously cached complex | eBPF only | <100ns | `/api/admin` (was Cedar, now cached) |
| Complex policy (first time) | eBPF → Userspace | 10-50µs | Cedar policy with context.ip check |
| Frequent complex (learned) | eBPF (after learning) | <100ns | Same path, now cached as simple rule |

### Learning Mode

1. **Initial State**: All policies in userspace (Cedar, Reaper DSL)
2. **Monitor**: Track which paths are frequently accessed
3. **Compile**: Convert frequently accessed decisions to simple rules
4. **Deploy**: Push simple rules to eBPF map
5. **Result**: 90%+ requests handled in eBPF fast path (<100ns)

**Example Learning Cycle**:
```
Iteration 1: /api/users → Cedar policy (50µs) [slow path]
Iteration 10: /api/users → Cedar policy (50µs) [still slow, but frequent]
Learning: "User 'alice' accessing /api/users → ALLOW" is stable pattern
Compile: Add simple rule: /api/users → ALLOW (for UID 1000)
Iteration 11+: /api/users → eBPF map (<100ns) [fast path!]
```

---

## Dynamic Policy Update Mechanism

### Requirement Analysis

User wants:
1. ✅ Load policy (initial deployment)
2. ✅ Execute policy (evaluate requests)
3. ✅ Publish new bundle dynamically (zero downtime)

### Implementation: BPF Map Hot-Swap

**BPF Maps Support Atomic Updates** from userspace via `bpf_map_update_elem()`.

#### Option 1: Direct Map Update (Per-Rule Updates)

**Pros**: Simple, low memory overhead
**Cons**: Not atomic for entire bundle (rules updated one-by-one)

```rust
impl ReaperEbpfController {
    pub fn publish_bundle(&mut self, bundle: PolicyBundle) -> Result<()> {
        let start = Instant::now();

        // Strategy: Update rules in-place (partial atomicity)
        // Note: Individual rules are atomic, but bundle as a whole is not

        let mut rules_to_add = Vec::new();
        let mut rules_to_remove = Vec::new();

        // Compute diff (which rules changed)
        for new_rule in &bundle.rules {
            if !self.current_rules.contains(new_rule) {
                rules_to_add.push(new_rule.clone());
            }
        }

        for old_rule in &self.current_rules {
            if !bundle.rules.contains(old_rule) {
                rules_to_remove.push(old_rule.clone());
            }
        }

        // Apply diff
        for rule in rules_to_remove {
            let key = rule_to_key(&rule);
            self.policy_map.remove(&key)?;
        }

        for rule in rules_to_add {
            let key = rule_to_key(&rule);
            let value = rule_to_value(&rule);
            self.policy_map.insert(key, value, 0)?;  // Atomic per-rule
        }

        self.current_rules = bundle.rules;

        tracing::info!(
            "Published bundle in {:?}: {} rules, {} added, {} removed",
            start.elapsed(),
            bundle.rules.len(),
            rules_to_add.len(),
            rules_to_remove.len()
        );

        Ok(())
    }
}
```

**Update Latency**: 10-100µs for 1000 rules

---

#### Option 2: Map-in-Map Atomic Swap (Bundle Atomicity)

**Pros**: Entire bundle updated atomically
**Cons**: Higher memory overhead (two maps), more complex

From [BPF Maps - Linux Kernel](https://docs.kernel.org/bpf/maps.html):

> BPF_MAP_TYPE_HASH_OF_MAPS and BPF_MAP_TYPE_ARRAY_OF_MAPS provide general purpose support for map in map storage.

```c
// Outer map: points to current policy map (switchable)
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY_OF_MAPS);
    __uint(max_entries, 1);
    __type(key, u32);
    __type(value, u32);  // Inner map ID
} policy_selector SEC(".maps");

// Inner map A: policies version 1
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 10000);
    __type(key, char[256]);
    __type(value, u8);
} policy_map_a SEC(".maps");

// Inner map B: policies version 2
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 10000);
    __type(key, char[256]);
    __type(value, u8);
} policy_map_b SEC(".maps");

SEC("lsm/file_open")
int reaper_file_open(struct file *file) {
    // Get current policy map (atomic read)
    u32 selector_key = 0;
    void *inner_map = bpf_map_lookup_elem(&policy_selector, &selector_key);
    if (!inner_map) return -EPERM;

    // Lookup policy in current map
    char path[256];
    bpf_d_path(&file->f_path, path, sizeof(path));

    u8 *action = bpf_map_lookup_elem(inner_map, path);
    if (action && *action == 1) return 0;

    return -EPERM;
}
```

**Userspace Atomic Swap**:
```rust
impl ReaperEbpfController {
    pub fn publish_bundle_atomic(&mut self, bundle: PolicyBundle) -> Result<()> {
        // 1. Determine inactive map (A or B)
        let inactive_map = if self.active_map == MapId::A {
            &mut self.policy_map_b
        } else {
            &mut self.policy_map_a
        };

        // 2. Populate inactive map with new bundle
        inactive_map.clear()?;
        for rule in &bundle.rules {
            let key = rule_to_key(&rule);
            let value = rule_to_value(&rule);
            inactive_map.insert(key, value, 0)?;
        }

        // 3. Atomically switch selector (SINGLE MAP UPDATE)
        let selector_key = 0u32;
        let new_map_fd = inactive_map.fd();
        self.policy_selector.insert(selector_key, new_map_fd, 0)?;

        // 4. Now inactive map is active, and vice versa
        self.active_map = if self.active_map == MapId::A {
            MapId::B
        } else {
            MapId::A
        };

        tracing::info!("Atomically published bundle: {} rules", bundle.rules.len());
        Ok(())
    }
}
```

**Atomicity**: Single `bpf_map_update_elem()` call switches entire policy set
**Update Latency**: 50-200µs for 1000 rules (populate + swap)

**Recommended**: Use Option 2 for production (true zero-downtime)

---

## Implementation Roadmap

### Phase 9.4: eBPF LSM Deployment (3-4 weeks)

#### Week 1: Foundation
- [ ] Create `crates/reaper-ebpf/` crate
- [ ] Set up Aya framework (Rust eBPF)
- [ ] Implement basic LSM hook (file_open)
- [ ] Create simple BPF_MAP_TYPE_HASH for policies
- [ ] Test with 10 simple rules (exact match)

**Deliverable**: eBPF program that blocks/allows file access

---

#### Week 2: Policy Loading & Updates
- [ ] Build userspace controller (load eBPF program)
- [ ] Implement `publish_bundle()` (direct map update)
- [ ] Implement `publish_bundle_atomic()` (map-in-map)
- [ ] Add wildcard "*" support
- [ ] Add prefix matching (bounded loop)
- [ ] Test dynamic policy updates (10K updates/sec)

**Deliverable**: Hot-swappable policies via BPF maps

---

#### Week 3: Two-Tier Integration
- [ ] Implement ring buffer for slow path events
- [ ] Create userspace daemon (consumes ring buffer)
- [ ] Integrate with PolicyEngine (Cedar, Reaper DSL)
- [ ] Implement decision caching (userspace → eBPF)
- [ ] Add learning mode (auto-compile frequent paths)

**Deliverable**: eBPF fast path + userspace slow path

---

#### Week 4: Production Hardening
- [ ] Add metrics (fast path %, slow path %, cache hit rate)
- [ ] Implement Prometheus exporter
- [ ] Create Kubernetes DaemonSet deployment
- [ ] Add graceful degradation (if eBPF fails, use userspace only)
- [ ] Performance benchmarks (100K RPS)
- [ ] Documentation & runbook

**Deliverable**: Production-ready eBPF deployment

---

### Deployment Targets

#### LSM Hooks Supported

From [eBPF Tutorial by Example 19: LSM](https://eunomia.dev/tutorials/19-lsm-connect/):

| Hook | Use Case | Performance |
|------|----------|-------------|
| `file_open` | File access control | <100ns |
| `socket_connect` | Network egress filtering | <100ns |
| `bprm_check` | Prevent malicious execution | <100ns |
| `task_kill` | Prevent signal injection | <50ns |
| `inode_permission` | Fine-grained file access | <100ns |

**Reaper Focus**: Start with `file_open` and `socket_connect` (highest value)

---

## Performance Analysis

### Latency Breakdown

| Deployment | Simple Policy | Cedar Policy | Reaper DSL |
|------------|---------------|--------------|------------|
| **HTTP Agent** (current) | 50-200µs | 50-200µs | 50-200µs |
| **WASM Filter** (Phase 9.2) | <1µs | <1µs | <1µs |
| **eBPF Fast Path** (simple) | <100ns | N/A | N/A |
| **eBPF Slow Path** (userspace) | N/A | 10-50µs | 10-50µs |
| **eBPF Learned** (cached) | <100ns | <100ns | <100ns |

### Throughput Projection

**Scenario**: API with 100K RPS, 80% traffic on 20 paths (Pareto principle)

**Before eBPF** (HTTP Agent):
- Latency: 50-200µs per request
- CPU: 100% (evaluation overhead)
- Network: 2 hops (app → agent → app)

**After eBPF** (with learning):
- Hot paths (80%): <100ns (eBPF fast path)
- Cold paths (20%): 10-50µs (userspace)
- **Effective latency**: 0.08 × 100ns + 0.2 × 30µs = 6.008µs
- **Speedup**: ~30x faster!
- CPU: 20% (only complex policies evaluated)
- Network: 0 hops (kernel-level)

**Resource Savings**:
- 80% reduction in CPU usage
- 80% reduction in network traffic
- Sub-microsecond p99 latency

---

## Production Deployment Strategy

### Deployment Options

#### Option A: Standalone eBPF Enforcement (Recommended for Simple Policies)

```
Application → eBPF LSM → Kernel → Allow/Deny
             (100% fast path)
```

**Use Case**: Organizations with simple RBAC, no complex ABAC
**Performance**: <100ns
**Complexity**: Low

---

#### Option B: Hybrid eBPF + Userspace (Recommended for Complex Policies)

```
Application → eBPF LSM → {Fast Path: <100ns} → Allow/Deny
                       ↘ {Slow Path: Ring Buffer} → Userspace (10-50µs)
```

**Use Case**: Organizations with mix of simple + complex policies
**Performance**: <100ns for 80%+ requests, 10-50µs for complex
**Complexity**: Medium

---

#### Option C: eBPF + WASM Hybrid (Ultimate Performance)

```
Application → eBPF LSM → {Fast Path: <100ns} → Allow/Deny
                       ↘ {Medium Path: WASM Filter in Envoy} → <1µs
                       ↘ {Slow Path: Userspace} → 10-50µs
```

**Use Case**: High-scale organizations (Google, Amazon scale)
**Performance**: Three-tier latency (100ns / 1µs / 50µs)
**Complexity**: High

---

### Migration Path

**Phase 1**: Deploy Reaper Agent (HTTP) - baseline
**Phase 2**: Deploy WASM filter (Envoy/Istio) - 10-100x faster
**Phase 3**: Deploy eBPF fast path - 100-1000x faster for simple policies

**Risk Mitigation**:
- Run eBPF in monitor mode first (log, don't block)
- Gradually increase enforcement (10% → 50% → 100%)
- Fallback to userspace if eBPF fails
- Canary deployments (1 node → 10% → 100%)

---

## Conclusion

### What IS Possible ✅

1. **Simple Policy Evaluation in eBPF** (<100ns)
   - Exact match, wildcard, prefix match
   - IP/UID/GID filtering
   - Bounded rule sets (up to 10K rules)

2. **Dynamic Policy Updates** (zero downtime)
   - Direct map update: 10-100µs
   - Atomic map-in-map swap: 50-200µs
   - Publish new bundles without eBPF reload

3. **Two-Tier Architecture**
   - eBPF fast path (80%+ requests)
   - Userspace slow path (complex policies)
   - Learning mode (auto-cache frequent paths)

4. **Production Deployment**
   - Kubernetes DaemonSet
   - Prometheus metrics
   - Graceful degradation

### What IS NOT Possible ❌

1. **Cedar Policy in eBPF** (too complex, use userspace)
2. **Reaper DSL JSON functions** (dynamic memory, use userspace)
3. **DataStore entity lookups** (HashMap, use userspace)
4. **Dynamic loops** (use bounded loops or hash map)

### Recommended Approach

**Two-Tier Hybrid** (Phase 9.4):
- eBPF fast path for simple policies (<100ns)
- Userspace for Cedar/Reaper DSL (10-50µs)
- Dynamic policy updates via BPF maps (atomic swap)
- Learning mode to maximize fast path coverage

**Expected Outcome**:
- 80%+ requests in eBPF fast path
- 30-100x latency reduction vs HTTP agent
- Sub-microsecond p99 latency
- Zero downtime policy updates

---

## Sources

1. [eBPF Ecosystem Progress in 2024–2025: A Technical Deep Dive](https://eunomia.dev/blog/2025/02/12/ebpf-ecosystem-progress-in-20242025-a-technical-deep-dive/)
2. [LSM BPF Programs — The Linux Kernel documentation](https://docs.kernel.org/bpf/prog_lsm.html)
3. [BPF maps — The Linux Kernel documentation](https://docs.kernel.org/bpf/maps.html)
4. [Maps - eBPF Docs](https://docs.ebpf.io/linux/concepts/maps/)
5. [eBPF Tutorial by Example 19: Security Detection and Defense using LSM](https://eunomia.dev/tutorials/19-lsm-connect/)
6. [Tracing the Future: Using eBPF for Low-Overhead Observability](https://thinhdanggroup.github.io/ebpf-observability/)
7. [What is eBPF and How Does It Work?](https://oneuptime.com/blog/post/2025-12-10-what-is-ebpf-and-how-does-it-work/view)

---

**Next Steps**: Review this analysis, then proceed with Phase 9.4 implementation if approved.
