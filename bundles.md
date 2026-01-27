# Reaper Bundle Architecture

## Overview

Reaper supports three bundle formats for flexible policy and data deployment with compilation for optimal runtime performance.

## Bundle Types

| Format | Extension | Contents | Use Case |
|--------|-----------|----------|----------|
| Policy Bundle | `.rpb` | Policies only | Logic updates, may depend on external data |
| Data Bundle | `.rdb` | Reference data only | Data updates without policy changes |
| Reaper Bundle | `.rbb` | Policies + Data | Self-contained, atomic deployment |

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           BUNDLE TYPES                                   │
├─────────────────────┬─────────────────────┬─────────────────────────────┤
│   Policy Bundle     │    Data Bundle      │   Reaper Bundle             │
│   (.rpb)            │    (.rdb)           │   (.rbb)                    │
├─────────────────────┼─────────────────────┼─────────────────────────────┤
│ • Policies only     │ • Reference data    │ • Policies + Data           │
│ • Rules & logic     │ • Lookup tables     │ • Self-contained            │
│ • Data dependencies │ • IP allowlists     │ • Atomic deployment         │
│ • Version control   │ • Config values     │ • Pre-compiled              │
└─────────────────────┴─────────────────────┴─────────────────────────────┘
```

## Deployment Source Tracking

Every policy and data entry tracks its origin:

```rust
enum DeploymentSource {
    Independent,                    // Deployed standalone
    PolicyBundle { id, name },      // From .rpb
    ReaperBundle { id, name },      // From .rbb
}

enum DataSource {
    DataBundle { id, name },        // From .rdb
    ReaperBundle { id, name },      // From .rbb
    ApiUpload { timestamp },        // Direct API upload
    Stream { source_id, type },     // Future: Kafka/Redis/Webhook
}
```

---

## Bundle Compilation Pipeline

Bundles are compiled before deployment for optimal runtime performance:

```
Source Bundle ──▶ Compile ──▶ Compiled Bundle (.rbb)
                    │
    ┌───────────────┼───────────────┐
    │               │               │
    ▼               ▼               ▼
 Validate       Intern          Index
 • Schema       • Strings       • Rule lookup
 • Rules        • Keys          • Policy by ID/name
 • Data deps    • Deduplicate   • Data by key
                    │
                    ▼
               Optimize
               • Pre-hash sets
               • Compile regex
               • Build decision trees
```

### Compilation Features

| Feature | Benefit |
|---------|---------|
| **String Interning** | 4-byte handle vs 24+ byte String |
| **Pre-hashed Sets** | O(1) membership checks for allowlists |
| **Compiled Regex** | No runtime regex compilation |
| **Rule Indexing** | O(1) exact match, O(log n) prefix match |
| **Deduplication** | Shared strings reduce memory 40-60% |

### String Interning

```rust
/// Interned string handle - 4 bytes instead of 24+ bytes
pub struct InternedStr(u32);

/// String table serialized with bundle
pub struct StringTable {
    strings: Vec<String>,
    lookup: HashMap<String, u32>,
}
```

All policy names, resource patterns, condition keys, and data keys are interned into a shared string table. This eliminates duplicate string allocations and reduces memory footprint significantly.

### Compiled Policy Structure

```rust
pub struct CompiledPolicy {
    pub id: Uuid,
    pub version: u64,
    pub name: u32,              // Interned string reference
    pub description: u32,       // Interned string reference
    pub rules: Vec<CompiledRule>,
    pub rule_index: RuleIndex,  // Pre-built for fast lookup
    pub deployment_source: DeploymentSource,
    pub created_at: i64,        // Unix timestamp (compact)
}

pub struct CompiledRule {
    pub action: PolicyAction,
    pub resource: u32,          // Interned
    pub compiled_regex: Option<regex::Regex>,  // Pre-compiled
    pub resource_hash: u64,     // For O(1) exact match
    pub conditions: Vec<CompiledCondition>,
}

pub struct RuleIndex {
    pub exact_match: HashMap<u64, Vec<usize>>,   // Hash -> rule indices
    pub prefix_patterns: Vec<(String, Vec<usize>)>,
    pub catch_all: Vec<usize>,
}
```

### Compiled Data Structure

```rust
pub struct CompiledDataEntry {
    pub key: u32,               // Interned
    pub version: u64,
    pub data_type: DataType,
    pub compiled_data: CompiledData,
}

pub enum CompiledData {
    HashSet(HashSet<u64>),      // Pre-hashed for O(1) lookup
    SortedSet(Vec<u64>),        // Binary search for small sets
    Map(HashMap<u32, Value>),   // Interned keys
    Json(serde_json::Value),    // Complex structures
    MMap { offset, len },       // Memory-mapped large data
}
```

---

## Atomic Deployment (No Memory Doubling)

Bundle swaps use epoch-based reclamation to avoid memory spikes:

```
Memory Timeline:
─────────────────────────────────────────────────────────
t0: [Old: 100MB]                     ← Serving traffic
t1: [Old: 100MB] + [Staging: 100MB]  ← Brief spike during load
t2: [New: 100MB] + [Old: draining]   ← Atomic pointer swap (CAS)
t3: [New: 100MB]                     ← Old reclaimed when safe
─────────────────────────────────────────────────────────
Peak: ~1.1x bundle size (not 2x)
```

### Atomic Bundle Slot

```rust
pub struct AtomicBundleSlot {
    /// Current bundle (epoch-protected pointer)
    current: Atomic<CompiledBundle>,
    /// Version counter for ABA prevention
    version: AtomicU64,
    /// Staging slot for new bundle
    staging: Mutex<Option<CompiledBundle>>,
}

impl AtomicBundleSlot {
    /// Lock-free read (wait-free for readers)
    pub fn load(&self) -> Option<BundleGuard> { }

    /// Stage new bundle (during compilation)
    pub fn stage(&self, bundle: CompiledBundle) { }

    /// Atomic swap: staging -> current
    pub fn commit(&self) -> Result<u64> { }
}
```

### Deployment Strategies

| Strategy | Memory Impact | Use Case |
|----------|---------------|----------|
| **Full Swap** | ~1.1x size | Major version updates |
| **Delta Update** | ~1.05x size | Incremental changes |
| **Hot Data Swap** | O(entry) | Single data entry update |
| **Memory-Mapped** | O(1) heap | Large datasets (>10MB) |

### Delta Updates

```rust
pub struct BundleDelta {
    pub base_version: u64,
    pub target_version: u64,
    pub base_checksum: u64,

    pub policies_upsert: Vec<CompiledPolicy>,
    pub policies_remove: Vec<Uuid>,

    pub data_upsert: Vec<CompiledDataEntry>,
    pub data_remove: Vec<u32>,

    pub string_table_additions: Vec<String>,
}

impl CompiledBundle {
    /// Apply delta with copy-on-write semantics
    pub fn apply_delta(&self, delta: BundleDelta) -> Result<CompiledBundle> {
        // Only changed policies are copied
        // Unchanged policies use Arc (zero-copy)
    }
}
```

---

## Data Update Sources

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      DATA UPDATE SOURCES                                 │
├─────────────────────┬─────────────────────┬─────────────────────────────┤
│   REST API ✓        │    Kafka (Future)   │   File Watch (Future)       │
├─────────────────────┼─────────────────────┼─────────────────────────────┤
│ • PUT /data/:key    │ • Topic subscribe   │ • Hot-reload on change      │
│ • Immediate update  │ • Real-time sync    │ • ConfigMap support         │
│ • Versioned         │ • Event-driven      │ • S3/GCS polling            │
└─────────────────────┴─────────────────────┴─────────────────────────────┘
        ✓ Current             ⏳ Planned              ⏳ Planned
```

### Future: Stream Subscriptions

```rust
pub struct DataSubscription {
    pub id: Uuid,
    pub data_key: DataKey,
    pub source: StreamSourceType,
    pub transform: Option<DataTransform>,
    pub update_policy: UpdatePolicy,
}

pub enum StreamSourceType {
    Kafka { topic: String, consumer_group: String },
    Redis { channel: String },
    Webhook { endpoint: String },
    FileWatch { path: String },
}

pub enum UpdateStrategy {
    Replace,    // Replace entire entry
    Merge,      // Merge with existing (maps/sets)
    Append,     // Append (lists/time-series)
}
```

---

## API Reference

### Platform API (port 8081)

```
# Policy Bundles (.rpb)
GET    /api/v1/policy-bundles              List policy bundles
POST   /api/v1/policy-bundles              Create policy bundle
GET    /api/v1/policy-bundles/:id          Get bundle details
PUT    /api/v1/policy-bundles/:id          Update bundle
DELETE /api/v1/policy-bundles/:id          Delete bundle
POST   /api/v1/policy-bundles/:id/deploy   Deploy to agents

# Data Bundles (.rdb)
GET    /api/v1/data-bundles                List data bundles
POST   /api/v1/data-bundles                Create data bundle
GET    /api/v1/data-bundles/:id            Get bundle details
PUT    /api/v1/data-bundles/:id            Update bundle
DELETE /api/v1/data-bundles/:id            Delete bundle
POST   /api/v1/data-bundles/:id/deploy     Deploy to agents

# Combined Bundles (.rbb)
GET    /api/v1/bundles                     List all bundles
POST   /api/v1/bundles                     Create combined bundle
GET    /api/v1/bundles/:id                 Get bundle details
DELETE /api/v1/bundles/:id                 Delete bundle
POST   /api/v1/bundles/:id/deploy          Deploy to agents
POST   /api/v1/bundles/import              Import from .rbb file
GET    /api/v1/bundles/:id/export          Export to .rbb file

# Direct Data API
GET    /api/v1/data                        List all data keys
GET    /api/v1/data/:key                   Get data entry
PUT    /api/v1/data/:key                   Hot-swap data entry
DELETE /api/v1/data/:key                   Remove data entry
```

### Agent API (port 8080)

```
POST   /api/v1/bundles/deploy              Receive bundle deployment
GET    /api/v1/bundles                     List deployed bundles
GET    /api/v1/bundles/:id                 Get deployed bundle details
```

---

## CLI Commands

```bash
# Policy bundles
reaper policy-bundle list
reaper policy-bundle create <name> --policies <id1,id2> [--requires-data <keys>]
reaper policy-bundle deploy <id>
reaper policy-bundle export <id> -o bundle.rpb

# Data bundles
reaper data-bundle list
reaper data-bundle create <name> --data <key1:file1.json,key2:file2.json>
reaper data-bundle deploy <id>
reaper data-bundle export <id> -o data.rdb

# Combined bundles
reaper bundle list
reaper bundle create <name> --policies <ids> --data <files>
reaper bundle compile <id>               # Pre-compile for deployment
reaper bundle deploy <id>                # Deploy to agents
reaper bundle import file.rbb            # Import and deploy
reaper bundle export <id> -o file.rbb    # Export compiled bundle

# Direct data management
reaper data list
reaper data get <key>
reaper data set <key> --file data.json --type set
reaper data delete <key>

# Future: Streaming
# reaper data subscribe <key> --kafka <topic> --transform <jmespath>
# reaper data unsubscribe <key>
```

---

## File Structure (Planned)

```
crates/
├── reaper-core/
│   └── src/
│       └── bundle.rs           # Bundle types, manifests, sources
├── policy-engine/
│   └── src/
│       ├── engine.rs           # Updated with bundle support
│       ├── intern.rs           # String interning, StringTable
│       ├── compiled.rs         # CompiledPolicy, CompiledDataEntry
│       ├── compiler.rs         # BundleCompiler, pipeline
│       └── atomic_slot.rs      # AtomicBundleSlot, epoch-based swap
```

---

## Implementation Phases

| Phase | Scope | Dependencies | Status |
|-------|-------|--------------|--------|
| 1 | Core bundle types | None | ⏳ Planned |
| 2 | String interning | Phase 1 | ⏳ Planned |
| 3 | Compiled structures | Phase 2 | ⏳ Planned |
| 4 | Bundle compiler | Phase 3 | ⏳ Planned |
| 5 | Atomic deployment | Phase 4 | ⏳ Planned |
| 6 | Delta updates | Phase 5 | ⏳ Planned |
| 7 | Platform/Agent APIs | Phase 5 | ⏳ Planned |
| 8 | CLI commands | Phase 7 | ⏳ Planned |
| 9 | Memory-mapped data | Phase 5 | ⏳ Future |
| 10 | Kafka/streaming | Phase 7 | ⏳ Future |

---

## Memory Guarantees

| Scenario | Peak Memory | Guarantee |
|----------|-------------|-----------|
| Full bundle swap | ~1.1x bundle | No doubling |
| Delta update | ~1.05x bundle | Copy-on-write |
| Data hot-swap | O(entry size) | Single entry |
| Large data (>10MB) | O(1) heap | Memory-mapped |

---

## BDD Test Scenarios (Planned)

```gherkin
Feature: Policy Bundling
  Scenario: Create and deploy a policy bundle
    Given policies "auth-policy" and "logging-policy" exist
    When I create a bundle with policies "auth-policy,logging-policy"
    And I deploy the bundle to the agent
    Then all policies should show deployment source "bundle:<name>"

  Scenario: Bundle compilation reduces memory
    Given a bundle with 100 policies and duplicate strings
    When I compile the bundle
    Then the compiled size should be < 60% of original
    And string table should have deduplicated entries

  Scenario: Atomic deployment without memory spike
    Given a deployed bundle of 100MB
    When I deploy a new 100MB bundle
    Then peak memory should not exceed 120MB
    And there should be zero evaluation failures during swap

  Scenario: Delta update for incremental changes
    Given a deployed bundle version 1
    When I update 2 policies and deploy as delta
    Then only changed policies should be copied
    And unchanged policies should share memory with v1

  Scenario: Data bundle supports policy evaluation
    Given a data bundle with "ip-allowlist" set
    And a policy that checks "ip-allowlist" membership
    When I evaluate a request with IP in the allowlist
    Then the decision should be Allow
```
