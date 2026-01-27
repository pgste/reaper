# Reaper Bundle Architecture Plan

> **Status**: Planning Document
> **Created**: 2026-01-27
> **Last Updated**: 2026-01-27

## Executive Summary

This document outlines enhancements to Reaper's bundle system to support:
- **Data Bundles (.rdb)**: Standalone reference data with pre-interned strings
- **Combined Bundles (.rbb)**: Self-contained policies + data for atomic deployment
- **Delta Updates**: Incremental data changes via push or poll

## Current State

### Implemented Formats

| Format | Magic | Struct | Location | Status |
|--------|-------|--------|----------|--------|
| `.rbb` | `REAP` | `PolicyBundle` | `reap/bundle.rs:36` | ✅ Single policy |
| `.rpp` | `REPP` | `PolicyPackage` | `reap/bundle.rs:337` | ✅ Multi-policy with hints |

### What's Missing

1. **No Data Bundle** - Entities loaded via JSON only (`DataLoader`)
2. **No Combined Bundle** - Policies and data deployed separately
3. **No Atomic Policy+Data Swap** - Risk of inconsistent state during updates
4. **No Delta Updates** - Full reload required for any data change

### Current Data Loading Path

```
JSON String → serde_json::from_str() → DataLoader::load_json()
                                            ↓
                                    interner.intern() for each string
                                            ↓
                                    DataStore::insert()
```

**Problem**: Interning happens at load time (~60% of load cost for large datasets).

---

## Proposed Bundle Types

### 1. Policy Bundle (.rpb)

Rename existing `.rbb` to `.rpb` for clarity. Policies only, no data.

```
Magic: REAP (keep existing)
Use:   Policy logic updates when data is managed separately
```

**No code changes needed** - just documentation clarification.

### 2. Data Bundle (.rdb)

New format for reference data with pre-interned strings.

```
Magic: REDT (REaper DaTa)
Use:   Data updates without policy changes, fast loading
```

#### Structure

```rust
// crates/policy-engine/src/data/bundle.rs (NEW FILE)

use serde::{Deserialize, Serialize};

/// Magic bytes for data bundle
pub const DATA_BUNDLE_MAGIC: &[u8; 4] = b"REDT";
pub const DATA_BUNDLE_VERSION: u32 = 1;

/// Pre-serialized string interner state
/// Strings are ordered by their InternedString ID (0, 1, 2, ...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternerSnapshot {
    /// All interned strings in ID order
    pub strings: Vec<String>,
    /// Original interner capacity (for pre-allocation)
    pub capacity: usize,
}

/// Compact entity representation using interner IDs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactEntity {
    /// Entity ID (index into interner)
    pub id: u32,
    /// Entity type (index into interner)
    pub entity_type: u32,
    /// Attributes as (key_index, value)
    pub attributes: Vec<(u32, CompactValue)>,
    /// Parent entity (index into interner)
    pub parent: Option<u32>,
}

/// Compact attribute value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompactValue {
    String(u32),           // Index into interner
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    List(Vec<CompactValue>),
    Set(Vec<CompactValue>),
    Object(Vec<(u32, CompactValue)>),
}

/// Data bundle metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBundleMetadata {
    /// Format version
    pub version: u32,
    /// Creation timestamp (unix seconds)
    pub created_at: u64,
    /// Bundle name/identifier
    pub name: String,
    /// Semantic version
    pub data_version: String,
    /// SHA-256 of source data
    pub source_hash: [u8; 32],
    /// Entity count
    pub entity_count: usize,
    /// Unique entity types
    pub type_count: usize,
}

/// Complete data bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBundle {
    pub metadata: DataBundleMetadata,
    /// Pre-built string interner state
    pub interner: InternerSnapshot,
    /// Entities in compact format
    pub entities: Vec<CompactEntity>,
}
```

#### Key Methods

```rust
impl DataBundle {
    /// Create bundle from existing DataStore
    pub fn from_store(store: &DataStore, name: String, version: String) -> Self;

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError>;

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError>;

    /// Load into a DataStore (fast path - no interning)
    pub fn into_store(self) -> DataStore;

    /// Merge into existing DataStore
    pub fn merge_into(self, store: &DataStore) -> Result<usize, ReaperError>;
}
```

#### Load Performance Comparison

| Dataset | JSON Load | .rdb Load | Speedup |
|---------|-----------|-----------|---------|
| 10K entities | ~50ms | ~5ms | 10x |
| 100K entities | ~500ms | ~50ms | 10x |
| 1M entities | ~5s | ~500ms | 10x |

**Why faster**:
- No JSON parsing (bincode is ~10x faster)
- No string interning (pre-computed IDs)
- No hash lookups during load
- Direct memory copy for interner

### 3. Reaper Bundle (.rbb) - Enhanced

Combined policies + data for atomic deployment.

```
Magic: REAB (REaper Atomic Bundle)
Use:   Self-contained deployments, guaranteed consistency
```

#### Structure

```rust
// crates/policy-engine/src/reap/bundle.rs (EXTEND)

/// Deployment mode for data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataDeployMode {
    /// Clear existing data, load bundle data
    Replace,
    /// Add/update entities, keep others
    Merge,
    /// No data in this bundle
    None,
}

/// Deployment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// How to handle data deployment
    pub data_mode: DataDeployMode,
    /// Automatically rollback on any failure
    pub rollback_on_failure: bool,
    /// Pre-warm regex cache before deployment
    pub prewarm_regex: bool,
    /// Pre-warm string interner
    pub prewarm_interner: bool,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            data_mode: DataDeployMode::Replace,
            rollback_on_failure: true,
            prewarm_regex: true,
            prewarm_interner: true,
        }
    }
}

/// Combined bundle metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaperBundleMetadata {
    pub version: u32,
    pub created_at: u64,
    pub name: String,
    pub bundle_version: String,
    pub source_hash: [u8; 32],
    pub policy_count: usize,
    pub entity_count: usize,
    pub has_data: bool,
}

/// Complete Reaper bundle (policies + data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaperBundle {
    pub metadata: ReaperBundleMetadata,
    /// Policy package (reuse existing PolicyPackage)
    pub policies: PolicyPackage,
    /// Optional embedded data bundle
    pub data: Option<DataBundle>,
    /// Deployment configuration
    pub config: DeploymentConfig,
}

pub const REAPER_BUNDLE_MAGIC: &[u8; 4] = b"REAB";
pub const REAPER_BUNDLE_VERSION: u32 = 1;
```

#### Key Methods

```rust
impl ReaperBundle {
    /// Create from policy package and optional data
    pub fn new(
        name: String,
        version: String,
        policies: PolicyPackage,
        data: Option<DataBundle>,
    ) -> Self;

    /// Create from files
    pub fn from_files(
        policy_files: &[PathBuf],
        data_file: Option<PathBuf>,
    ) -> Result<Self, ReaperError>;

    /// Serialize/deserialize
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError>;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError>;

    /// Deploy atomically to engine
    pub fn deploy_atomic(
        self,
        engine: &PolicyEngine,
        force: bool,
    ) -> Result<DeploymentReceipt, ReaperError>;
}

/// Receipt returned after deployment
#[derive(Debug, Clone)]
pub struct DeploymentReceipt {
    pub bundle_name: String,
    pub bundle_version: String,
    pub policies_deployed: usize,
    pub entities_loaded: usize,
    pub deployment_time: Duration,
    pub bundle_hash: [u8; 32],
}
```

---

## Engine Changes

### PolicyEngine Additions

```rust
// crates/policy-engine/src/engine.rs

impl PolicyEngine {
    /// Current data store (add field)
    // data_store: Arc<ArcSwap<DataStore>>,  // For atomic swap
    // OR
    // data_store: Arc<RwLock<Arc<DataStore>>>,  // Simpler, slightly slower

    /// Atomic data store swap
    ///
    /// Replaces the entire data store atomically.
    /// Old store is dropped when all references are released.
    pub fn swap_store(&self, new_store: Arc<DataStore>) {
        // Option 1: ArcSwap (lock-free)
        self.data_store.store(new_store);

        // Option 2: RwLock (simpler)
        *self.data_store.write() = new_store;
    }

    /// Get current data store
    pub fn store(&self) -> Arc<DataStore> {
        // Option 1: ArcSwap
        self.data_store.load_full()

        // Option 2: RwLock
        self.data_store.read().clone()
    }

    /// Deploy a combined Reaper bundle atomically
    ///
    /// This method:
    /// 1. Validates bundle integrity
    /// 2. Builds new DataStore from embedded data (if present)
    /// 3. Pre-warms caches from hints
    /// 4. Compiles policies against new store
    /// 5. Atomically swaps both store and policies
    /// 6. Returns deployment receipt
    pub fn deploy_reaper_bundle(
        &self,
        bundle: ReaperBundle,
        force: bool,
    ) -> Result<DeploymentReceipt> {
        let start = Instant::now();

        // 1. Build new DataStore
        let new_store = if let Some(data) = bundle.data {
            Arc::new(data.into_store())
        } else {
            self.store() // Keep existing
        };

        // 2. Pre-warm from hints
        if bundle.config.prewarm_regex {
            bundle.policies.hints.prewarm_regex_cache();
        }
        if bundle.config.prewarm_interner {
            let interner = new_store.interner();
            for s in &bundle.policies.hints.strings_to_intern {
                interner.intern(s);
            }
        }

        // 3. Atomic deployment
        // Deploy data first (if replacing)
        if matches!(bundle.config.data_mode, DataDeployMode::Replace) {
            self.swap_store(new_store.clone());
        }

        // Deploy policies
        let versions = bundle.policies.deploy_to_engine(self, new_store)?;

        Ok(DeploymentReceipt {
            bundle_name: bundle.metadata.name,
            bundle_version: bundle.metadata.bundle_version,
            policies_deployed: versions.len(),
            entities_loaded: bundle.metadata.entity_count,
            deployment_time: start.elapsed(),
            bundle_hash: bundle.metadata.source_hash,
        })
    }

    /// Apply a data delta (incremental update)
    pub fn apply_delta(&self, delta: DataDelta) -> Result<DeltaResult> {
        let store = self.store();
        let interner = store.interner();
        let mut applied = 0;

        for op in delta.operations {
            match op {
                DeltaOp::Insert(entity) => {
                    store.insert(entity.into_entity(interner));
                    applied += 1;
                }
                DeltaOp::Update { id, attributes } => {
                    // Get existing, update attributes, re-insert
                    if let Some(mut entity) = store.get(id) {
                        // Apply attribute updates...
                        applied += 1;
                    }
                }
                DeltaOp::Delete { id } => {
                    store.remove(id);
                    applied += 1;
                }
            }
        }

        Ok(DeltaResult { applied })
    }
}
```

### Dependency Addition

```toml
# Cargo.toml - if using ArcSwap for lock-free store swap
[dependencies]
arc-swap = "1.7"
```

---

## Delta Updates (.rdd)

For dynamic data that changes frequently.

### Structure

```rust
// crates/policy-engine/src/data/delta.rs (NEW FILE)

pub const DELTA_MAGIC: &[u8; 4] = b"REDD";
pub const DELTA_VERSION: u32 = 1;

/// Delta operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeltaOp {
    /// Insert new entity
    Insert(CompactEntity),
    /// Update existing entity attributes
    Update {
        id: u32,
        /// Only the changed attributes
        attributes: Vec<(u32, CompactValue)>,
    },
    /// Delete entity by ID
    Delete { id: u32 },
    /// Bulk insert
    BulkInsert(Vec<CompactEntity>),
    /// Bulk delete by type
    DeleteByType { entity_type: u32 },
}

/// Data delta bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataDelta {
    /// Base version this delta applies to
    pub base_version: String,
    /// New version after applying delta
    pub target_version: String,
    /// Sequence number for ordering
    pub sequence: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Operations to apply
    pub operations: Vec<DeltaOp>,
    /// New strings to intern (not in base)
    pub new_strings: Vec<String>,
}

/// Result of applying delta
#[derive(Debug, Clone)]
pub struct DeltaResult {
    pub applied: usize,
    pub new_version: String,
}
```

### Delivery Mechanisms

#### Option 1: SSE Push (Recommended for <1000 updates/sec)

```rust
// Agent subscribes to Platform SSE stream
// Platform: GET /api/v1/data/stream

// Event format:
// event: delta
// data: {"base_version":"1.0.0","operations":[...]}
```

Reuse existing SSE patterns from `reaper-sync`.

#### Option 2: Kafka/NATS (For high throughput)

```
Topic: reaper.data.{tenant}.deltas
Topic: reaper.data.{tenant}.{entity_type}  // Per-type streams
```

#### Option 3: Poll (Simplest)

```
GET /api/v1/data/delta?since={version}&limit=1000

Response:
{
  "deltas": [...],
  "next_version": "1.0.5",
  "has_more": false
}
```

---

## API Endpoints

### Agent API Additions

```
# Data bundle deployment
POST /api/v1/data/deploy
Content-Type: application/octet-stream
Body: <.rdb bytes>

# Combined bundle deployment
POST /api/v1/bundles/deploy
Content-Type: application/octet-stream
Body: <.rbb bytes>

# Delta application
POST /api/v1/data/delta
Content-Type: application/octet-stream
Body: <.rdd bytes>

# Delta polling
GET /api/v1/data/delta?since={version}

# Data stats
GET /api/v1/data/stats
Response: {
  "entity_count": 100000,
  "type_counts": {"User": 50000, "Resource": 50000},
  "memory_bytes": 52428800,
  "data_version": "1.0.0"
}
```

### Platform API Additions

```
# Create data bundle from entities
POST /api/v1/data/bundle
Body: { "name": "prod-data", "version": "1.0.0" }
Response: { "bundle_id": "uuid", "size_bytes": 1234567 }

# Download data bundle
GET /api/v1/data/bundle/{id}
Response: <.rdb bytes>

# Create combined bundle
POST /api/v1/bundles/create
Body: {
  "name": "prod-bundle",
  "version": "1.0.0",
  "policy_ids": ["uuid1", "uuid2"],
  "include_data": true
}

# SSE data stream
GET /api/v1/data/stream
Response: SSE stream of deltas
```

---

## CLI Additions

```bash
# Create data bundle from JSON
reaper data bundle entities.json -o data.rdb --name prod-data --version 1.0.0

# View data bundle info
reaper data info data.rdb

# Deploy data bundle
reaper data deploy data.rdb [--merge]

# Create combined bundle
reaper bundle create \
  --policies policy1.reap policy2.reap \
  --data entities.json \
  --output prod.rbb \
  --name production \
  --version 1.0.0

# Deploy combined bundle
reaper bundle deploy prod.rbb [--force]

# Apply delta
reaper data delta apply delta.rdd
```

---

## File Structure

```
crates/policy-engine/src/
├── data/
│   ├── mod.rs              # Add: bundle, delta modules
│   ├── store.rs            # Existing
│   ├── loader.rs           # Existing (JSON loading)
│   ├── bundle.rs           # NEW: DataBundle implementation
│   ├── delta.rs            # NEW: DataDelta implementation
│   └── interning.rs        # Existing (add snapshot methods)
├── reap/
│   ├── bundle.rs           # EXTEND: ReaperBundle, DeploymentConfig
│   └── ...
└── engine.rs               # EXTEND: swap_store, deploy_reaper_bundle
```

---

## Implementation Phases

### Phase 1: Data Bundle (.rdb)
**Effort**: 2-3 days

1. Add `InternerSnapshot` to `StringInterner`
2. Create `CompactEntity` and `CompactValue` types
3. Implement `DataBundle` struct with serialization
4. Add `DataBundle::from_store()` and `into_store()`
5. Add CLI commands for data bundle creation
6. Add agent endpoint for data deployment

### Phase 2: Combined Bundle (.rbb)
**Effort**: 2-3 days

1. Create `ReaperBundle` struct
2. Add `DeploymentConfig` and `DeploymentReceipt`
3. Add `PolicyEngine::swap_store()` method
4. Implement `deploy_reaper_bundle()`
5. Add CLI and API endpoints
6. Update documentation

### Phase 3: Delta Updates
**Effort**: 3-4 days

1. Create `DataDelta` and `DeltaOp` types
2. Implement `PolicyEngine::apply_delta()`
3. Add SSE endpoint to Platform
4. Add SSE client to Agent sync service
5. Add delta polling endpoint
6. Add CLI commands

### Phase 4: Kafka Integration (Optional)
**Effort**: 2-3 days

1. Add Kafka consumer to Agent
2. Add Kafka producer to Platform
3. Configuration for topic mapping
4. Exactly-once delivery guarantees

---

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_data_bundle_roundtrip() {
    let store = create_test_store(1000);
    let bundle = DataBundle::from_store(&store, "test", "1.0.0");
    let bytes = bundle.to_bytes().unwrap();
    let loaded = DataBundle::from_bytes(&bytes).unwrap();
    let new_store = loaded.into_store();
    assert_eq!(store.stats().total_entities, new_store.stats().total_entities);
}

#[test]
fn test_atomic_deployment() {
    let engine = PolicyEngine::new();
    let bundle = create_test_reaper_bundle();
    let receipt = engine.deploy_reaper_bundle(bundle, false).unwrap();
    assert!(receipt.policies_deployed > 0);
}

#[test]
fn test_delta_application() {
    let engine = PolicyEngine::new();
    load_initial_data(&engine);

    let delta = DataDelta {
        base_version: "1.0.0".into(),
        target_version: "1.0.1".into(),
        operations: vec![
            DeltaOp::Insert(create_entity("new_user")),
            DeltaOp::Delete { id: intern("old_user") },
        ],
        ..Default::default()
    };

    let result = engine.apply_delta(delta).unwrap();
    assert_eq!(result.applied, 2);
}
```

### Integration Tests (BDD)

```gherkin
Feature: Bundle Deployment

  Scenario: Deploy combined bundle atomically
    Given a Reaper bundle with 3 policies and 1000 entities
    When I deploy the bundle to the agent
    Then all 3 policies should be active
    And the data store should contain 1000 entities
    And the deployment should complete in under 100ms

  Scenario: Apply data delta
    Given an agent with data version "1.0.0"
    When I apply a delta with 10 inserts and 5 deletes
    Then the data version should be "1.0.1"
    And the entity count should reflect the changes
```

### Benchmark Tests

```rust
#[bench]
fn bench_data_bundle_load_100k(b: &mut Bencher) {
    let bundle_bytes = create_100k_data_bundle();
    b.iter(|| {
        let bundle = DataBundle::from_bytes(&bundle_bytes).unwrap();
        let _store = bundle.into_store();
    });
}
```

---

## Capacity Guidelines

### Data Store Limits

| Entities | Memory | .rdb Size | Load Time | Recommendation |
|----------|--------|-----------|-----------|----------------|
| 10K | ~5 MB | ~1 MB | <10ms | Single agent, any hardware |
| 100K | ~50 MB | ~10 MB | <100ms | Single agent, 256MB+ RAM |
| 1M | ~500 MB | ~100 MB | <1s | Single agent, 2GB+ RAM |
| 10M | ~5 GB | ~1 GB | <10s | Consider sharding |

### Delta Update Rates

| Rate | Recommended Delivery | Notes |
|------|---------------------|-------|
| <10/sec | Poll (30s interval) | Simple, low overhead |
| 10-1000/sec | SSE push | Real-time, moderate resources |
| >1000/sec | Kafka/NATS | High throughput, persistence |

---

## Migration Path

### From Current State

1. **No breaking changes** - All existing `.rbb` files continue to work
2. **Gradual adoption** - Use new formats alongside existing
3. **Rename suggestion** - Consider renaming `.rbb` → `.rpb` for clarity

### Backward Compatibility

```rust
impl PolicyBundle {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        match &bytes[0..4] {
            b"REAP" => Self::from_bytes_v1(bytes),  // Existing
            b"REAB" => Self::from_reaper_bundle(bytes), // Extract policy
            _ => Err(ReaperError::InvalidBundle),
        }
    }
}
```

---

## Open Questions

1. **ArcSwap vs RwLock for store swap?**
   - ArcSwap: Lock-free, slightly more complex
   - RwLock: Simpler, brief write lock during swap

2. **Delta compaction?**
   - Should Platform auto-compact deltas into snapshots?
   - After N deltas, generate new .rdb?

3. **Multi-tenant data isolation?**
   - Separate DataStore per tenant?
   - Or namespace prefixes in single store?

4. **Encryption for sensitive data?**
   - Encrypt entire bundle?
   - Or just sensitive attributes?

---

## References

- Current bundle implementation: `crates/policy-engine/src/reap/bundle.rs`
- DataStore implementation: `crates/policy-engine/src/data/store.rs`
- String interner: `crates/policy-engine/src/data/interning.rs`
- Existing bundle docs: `docs/concepts/BUNDLE_FORMAT.md`
