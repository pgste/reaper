# eBPF Data & Policy Promotion Implementation Plan

This document outlines the complete implementation plan for the adaptive data layer and intelligent policy promotion system.

## Phase 1: Core Data Structures (Week 1)

### 1.1 eBPF Kernel Structures

**File**: `crates/reaper-ebpf/reaper-ebpf-kern/src/entity.rs` (NEW)

```rust
// Entity type definitions
#[repr(u8)]
pub enum EntityType {
    User = 0,
    Role = 1,
    Group = 2,
    Resource = 3,
    Permission = 4,
    Custom = 255,
}

// String attribute
#[repr(C)]
pub struct StringAttr {
    pub key: [u8; 32],
    pub value: [u8; 64],
}

// Numeric attribute
#[repr(C)]
pub struct NumericAttr {
    pub key: [u8; 32],
    pub value: i64,
}

// Relationship
#[repr(C)]
pub struct Relationship {
    pub rel_type: [u8; 32],
    pub target_id: [u8; 64],
}

// Core entity
#[repr(C)]
pub struct Entity {
    pub id: [u8; 64],
    pub entity_type: EntityType,
    pub string_attrs: [StringAttr; 8],
    pub string_count: u8,
    pub numeric_attrs: [NumericAttr; 8],
    pub numeric_count: u8,
    pub relationships: [Relationship; 16],
    pub relationship_count: u8,
    pub flags: u64,
    pub version: u32,
    pub created_at: u64,
    pub updated_at: u64,
}
```

**eBPF Maps** - `crates/reaper-ebpf/reaper-ebpf-kern/src/lib.rs`

```rust
// Tier 1: Direct entity maps
#[map]
static USERS: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(10000, 0);

#[map]
static ROLES: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(1000, 0);

#[map]
static RESOURCES: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(10000, 0);

#[map]
static GROUPS: HashMap<[u8; 64], Entity> =
    HashMap::with_max_entries(5000, 0);

// Tier 2: Shard management (for 10K-100K entities)
#[map]
static ENTITY_SHARDS: HashMap<u32, u32> =
    HashMap::with_max_entries(16, 0);

// Tier 3: Bloom filter (for 100K-1M entities)
#[map]
static ENTITY_BLOOM: Array<u64> =
    Array::with_max_entries(128, 0);  // 1024 bytes = 8192 bits
```

### 1.2 Userspace Data Structures

**File**: `crates/reaper-ebpf/src/entity.rs` (NEW)

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON input format for entity data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDataset {
    pub dataset: String,
    pub version: String,
    pub entities: HashMap<String, EntityData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    #[serde(rename = "type")]
    pub entity_type: String,

    #[serde(default)]
    pub string_attrs: HashMap<String, String>,

    #[serde(default)]
    pub numeric_attrs: HashMap<String, i64>,

    #[serde(default)]
    pub relationships: Vec<RelationshipData>,

    #[serde(default)]
    pub flags: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipData {
    #[serde(rename = "type")]
    pub rel_type: String,
    pub target: String,
}

/// Validation result
#[derive(Debug)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub tier: DataTier,
    pub estimated_memory: usize,
    pub entity_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataTier {
    Tier1Direct,      // < 10K
    Tier2Sharded,     // 10K-100K
    Tier3Partitioned, // 100K-1M
}
```

## Phase 2: Data Ingestion Pipeline (Week 1-2)

### 2.1 Validation Engine

**File**: `crates/reaper-ebpf/src/validation.rs` (NEW)

```rust
pub struct EntityValidator {
    max_string_attrs: usize,
    max_numeric_attrs: usize,
    max_relationships: usize,
    max_string_len: usize,
}

impl EntityValidator {
    pub fn validate_dataset(&self, dataset: &EntityDataset) -> ValidationResult {
        let mut result = ValidationResult::new();

        for (id, entity) in &dataset.entities {
            // Check ID length
            if id.len() > 64 {
                result.errors.push(format!(
                    "Entity '{}' ID exceeds 64 bytes",
                    id
                ));
            }

            // Check string attribute count
            if entity.string_attrs.len() > self.max_string_attrs {
                result.errors.push(format!(
                    "Entity '{}' has {} string attrs (max {})",
                    id,
                    entity.string_attrs.len(),
                    self.max_string_attrs
                ));
            }

            // Check string attribute lengths
            for (key, value) in &entity.string_attrs {
                if key.len() > 32 {
                    result.errors.push(format!(
                        "Entity '{}' attr key '{}' exceeds 32 bytes",
                        id, key
                    ));
                }
                if value.len() > self.max_string_len {
                    result.errors.push(format!(
                        "Entity '{}' attr '{}' value exceeds {} bytes",
                        id, key, self.max_string_len
                    ));
                }
            }

            // Check numeric attrs
            if entity.numeric_attrs.len() > self.max_numeric_attrs {
                result.errors.push(format!(
                    "Entity '{}' has {} numeric attrs (max {})",
                    id,
                    entity.numeric_attrs.len(),
                    self.max_numeric_attrs
                ));
            }

            // Check relationships
            if entity.relationships.len() > self.max_relationships {
                result.errors.push(format!(
                    "Entity '{}' has {} relationships (max {})",
                    id,
                    entity.relationships.len(),
                    self.max_relationships
                ));
            }

            // Validate relationship targets exist
            for rel in &entity.relationships {
                if !dataset.entities.contains_key(&rel.target) {
                    result.warnings.push(format!(
                        "Entity '{}' references non-existent target '{}'",
                        id, rel.target
                    ));
                }
            }

            // Check flags
            if entity.flags.len() > 64 {
                result.errors.push(format!(
                    "Entity '{}' has {} flags (max 64)",
                    id,
                    entity.flags.len()
                ));
            }
        }

        // Determine tier
        result.tier = self.determine_tier(dataset.entities.len());
        result.entity_count = dataset.entities.len();
        result.estimated_memory = self.estimate_memory(dataset);
        result.valid = result.errors.is_empty();

        result
    }

    fn determine_tier(&self, count: usize) -> DataTier {
        match count {
            0..=10000 => DataTier::Tier1Direct,
            10001..=100000 => DataTier::Tier2Sharded,
            _ => DataTier::Tier3Partitioned,
        }
    }

    fn estimate_memory(&self, dataset: &EntityDataset) -> usize {
        // Entity size in eBPF: ~2KB per entity
        const ENTITY_SIZE: usize = 2048;

        dataset.entities.len() * ENTITY_SIZE
    }
}
```

### 2.2 Data Loader

**File**: `crates/reaper-ebpf/src/data_loader.rs` (NEW)

```rust
pub struct DataLoader {
    controller: Arc<RwLock<EbpfController>>,
    validator: EntityValidator,
}

impl DataLoader {
    pub async fn load_dataset(&mut self, dataset: EntityDataset) -> Result<LoadStats> {
        // 1. Validate
        let validation = self.validator.validate_dataset(&dataset);
        if !validation.valid {
            anyhow::bail!("Validation failed: {:?}", validation.errors);
        }

        info!(
            "Loading {} entities (Tier: {:?}, Memory: {} MB)",
            validation.entity_count,
            validation.tier,
            validation.estimated_memory / 1024 / 1024
        );

        // 2. Convert to eBPF format
        let mut stats = LoadStats::new();
        for (id, entity_data) in dataset.entities {
            let entity = self.convert_to_ebpf_entity(&id, &entity_data)?;

            // 3. Insert into appropriate map
            match entity.entity_type {
                EntityType::User => {
                    self.insert_user(&id, entity).await?;
                    stats.users += 1;
                }
                EntityType::Role => {
                    self.insert_role(&id, entity).await?;
                    stats.roles += 1;
                }
                EntityType::Resource => {
                    self.insert_resource(&id, entity).await?;
                    stats.resources += 1;
                }
                EntityType::Group => {
                    self.insert_group(&id, entity).await?;
                    stats.groups += 1;
                }
                _ => {
                    stats.custom += 1;
                }
            }
        }

        info!("Loaded: {} users, {} roles, {} resources, {} groups",
              stats.users, stats.roles, stats.resources, stats.groups);

        Ok(stats)
    }

    fn convert_to_ebpf_entity(
        &self,
        id: &str,
        data: &EntityData,
    ) -> Result<Entity> {
        let mut entity = Entity::default();

        // Set ID
        let id_bytes = id.as_bytes();
        let len = id_bytes.len().min(64);
        entity.id[..len].copy_from_slice(&id_bytes[..len]);

        // Set type
        entity.entity_type = self.parse_entity_type(&data.entity_type)?;

        // Convert string attributes
        entity.string_count = 0;
        for (key, value) in data.string_attrs.iter().take(8) {
            let mut attr = StringAttr::default();

            let key_bytes = key.as_bytes();
            let key_len = key_bytes.len().min(32);
            attr.key[..key_len].copy_from_slice(&key_bytes[..key_len]);

            let val_bytes = value.as_bytes();
            let val_len = val_bytes.len().min(64);
            attr.value[..val_len].copy_from_slice(&val_bytes[..val_len]);

            entity.string_attrs[entity.string_count as usize] = attr;
            entity.string_count += 1;
        }

        // Convert numeric attributes
        entity.numeric_count = 0;
        for (key, value) in data.numeric_attrs.iter().take(8) {
            let mut attr = NumericAttr::default();

            let key_bytes = key.as_bytes();
            let key_len = key_bytes.len().min(32);
            attr.key[..key_len].copy_from_slice(&key_bytes[..key_len]);
            attr.value = *value;

            entity.numeric_attrs[entity.numeric_count as usize] = attr;
            entity.numeric_count += 1;
        }

        // Convert relationships
        entity.relationship_count = 0;
        for rel in data.relationships.iter().take(16) {
            let mut relationship = Relationship::default();

            let type_bytes = rel.rel_type.as_bytes();
            let type_len = type_bytes.len().min(32);
            relationship.rel_type[..type_len].copy_from_slice(&type_bytes[..type_len]);

            let target_bytes = rel.target.as_bytes();
            let target_len = target_bytes.len().min(64);
            relationship.target_id[..target_len].copy_from_slice(&target_bytes[..target_len]);

            entity.relationships[entity.relationship_count as usize] = relationship;
            entity.relationship_count += 1;
        }

        // Convert flags to bitmap
        entity.flags = 0;
        for (idx, (_key, value)) in data.flags.iter().enumerate().take(64) {
            if *value {
                entity.flags |= 1u64 << idx;
            }
        }

        // Set metadata
        entity.version = 1;
        entity.created_at = chrono::Utc::now().timestamp() as u64;
        entity.updated_at = entity.created_at;

        Ok(entity)
    }

    async fn insert_user(&mut self, id: &str, entity: Entity) -> Result<()> {
        let mut controller = self.controller.write().await;
        let key = self.string_to_key(id)?;
        controller.insert_entity_user(key, entity)?;
        Ok(())
    }

    // Similar for insert_role, insert_resource, insert_group
}

#[derive(Debug, Default)]
pub struct LoadStats {
    pub users: usize,
    pub roles: usize,
    pub resources: usize,
    pub groups: usize,
    pub custom: usize,
}
```

## Phase 3: Policy Analysis Engine (Week 2)

### 3.1 Promotability Analyzer

**File**: `crates/reaper-ebpf/src/policy_analyzer.rs` (NEW)

```rust
use policy_engine::reap::{Policy, ReapRule as Rule, ReapCondition as Condition};

#[derive(Debug, Clone)]
pub enum PromotionDecision {
    /// Can be promoted to eBPF
    Promote {
        complexity: u32,         // 0-10 (0 = trivial, 10 = complex)
        map_lookups: u32,        // Number of map lookups required
        required_maps: Vec<String>, // Which maps are needed
        estimated_latency_ns: u32,
    },

    /// Must stay in userspace
    KeepUserspace {
        reason: String,
    },

    /// Partially promotable
    Partial {
        promotable_conditions: Vec<Condition>,
        userspace_conditions: Vec<Condition>,
    },
}

pub struct PolicyAnalyzer {
    // Known entity types and their maps
    entity_maps: HashMap<String, String>,
}

impl PolicyAnalyzer {
    pub fn analyze_policy(&self, policy: &Policy) -> PolicyAnalysis {
        let mut analysis = PolicyAnalysis::new(&policy.name);

        for rule in &policy.rules {
            let decision = self.analyze_rule(rule);
            analysis.add_rule_decision(&rule.name, decision);
        }

        analysis
    }

    pub fn analyze_rule(&self, rule: &Rule) -> PromotionDecision {
        self.analyze_condition(&rule.condition, 0)
    }

    fn analyze_condition(&self, condition: &Condition, depth: u32) -> PromotionDecision {
        // Depth limit for safety (eBPF has limited stack)
        if depth > 3 {
            return PromotionDecision::KeepUserspace {
                reason: "Condition depth exceeds eBPF stack limits (max 3)".to_string(),
            };
        }

        match condition {
            Condition::True | Condition::False => {
                PromotionDecision::Promote {
                    complexity: 0,
                    map_lookups: 0,
                    required_maps: vec![],
                    estimated_latency_ns: 10, // Trivial
                }
            }

            Condition::Comparison { left, op, right } => {
                self.analyze_comparison(left, op, right)
            }

            Condition::And(conditions) => {
                self.analyze_and_conditions(conditions, depth)
            }

            Condition::Or(conditions) => {
                self.analyze_or_conditions(conditions, depth)
            }

            Condition::Not(inner) => {
                self.analyze_condition(inner, depth + 1)
            }

            Condition::Expr(_) => {
                PromotionDecision::KeepUserspace {
                    reason: "Complex expressions require userspace evaluation".to_string(),
                }
            }

            Condition::Assignment { .. } => {
                PromotionDecision::KeepUserspace {
                    reason: "Variable assignments require userspace".to_string(),
                }
            }
        }
    }

    fn analyze_comparison(
        &self,
        left: &ComparisonLeft,
        op: &Operator,
        right: &ComparisonRight,
    ) -> PromotionDecision {
        // Check if left side is entity attribute
        match left {
            ComparisonLeft::EntityAttr(attr) => {
                // Determine which map to use
                let map_name = self.get_map_for_entity(&attr.entity);

                // Check if we support this lookup
                if attr.index.is_some() {
                    // Array/object indexing
                    return PromotionDecision::KeepUserspace {
                        reason: "Array/object indexing not supported in eBPF".to_string(),
                    };
                }

                // Check operator support
                match op {
                    Operator::Equal | Operator::NotEqual => {
                        // Supported
                    }
                    Operator::GreaterThan
                    | Operator::LessThan
                    | Operator::GreaterEqual
                    | Operator::LessEqual => {
                        // Numeric comparison - check if attribute is numeric
                        // (would need type inference here)
                    }
                    Operator::In => {
                        // IN operator - check right side
                        if let ComparisonRight::Value(Value::Array(arr)) = right {
                            if arr.len() > 64 {
                                return PromotionDecision::KeepUserspace {
                                    reason: "IN with >64 values not supported in eBPF".to_string(),
                                };
                            }
                        }
                    }
                }

                PromotionDecision::Promote {
                    complexity: 1,
                    map_lookups: 1,
                    required_maps: vec![map_name],
                    estimated_latency_ns: 50, // Single map lookup
                }
            }

            ComparisonLeft::VarAttr(_) => {
                PromotionDecision::KeepUserspace {
                    reason: "Variable attributes require userspace".to_string(),
                }
            }

            ComparisonLeft::Expr(_) => {
                PromotionDecision::KeepUserspace {
                    reason: "Expressions require userspace evaluation".to_string(),
                }
            }
        }
    }

    fn get_map_for_entity(&self, entity: &Entity) -> String {
        match entity {
            Entity::User => "USERS".to_string(),
            Entity::Resource => "RESOURCES".to_string(),
            Entity::Context => "CONTEXT".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct PolicyAnalysis {
    pub policy_name: String,
    pub total_rules: usize,
    pub promotable_rules: usize,
    pub userspace_rules: usize,
    pub rule_decisions: HashMap<String, PromotionDecision>,
    pub estimated_fast_path_percent: f64,
}
```

## Phase 4: CLI Tools (Week 2-3)

### 4.1 Data Validation Command

**File**: `tools/reaper-cli/src/commands/validate_data.rs` (NEW)

```bash
# Usage
reaper-cli validate-data --file users.json
reaper-cli validate-data --file users.json --check-ebpf
reaper-cli validate-data --file users.json --format table
```

**Implementation**:

```rust
pub async fn validate_data(args: &ValidateDataArgs) -> Result<()> {
    // Load JSON
    let content = fs::read_to_string(&args.file)?;
    let dataset: EntityDataset = serde_json::from_str(&content)?;

    // Validate
    let validator = EntityValidator::default();
    let result = validator.validate_dataset(&dataset);

    // Display results
    match args.format {
        OutputFormat::Table => print_validation_table(&result),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
    }

    if !result.valid {
        std::process::exit(1);
    }

    Ok(())
}

fn print_validation_table(result: &ValidationResult) {
    println!("\n=== Entity Dataset Validation ===\n");
    println!("Status: {}", if result.valid { "✓ VALID" } else { "✗ INVALID" });
    println!("Entities: {}", result.entity_count);
    println!("Tier: {:?}", result.tier);
    println!("Estimated Memory: {} MB", result.estimated_memory / 1024 / 1024);
    println!();

    if !result.errors.is_empty() {
        println!("Errors:");
        for error in &result.errors {
            println!("  ✗ {}", error);
        }
        println!();
    }

    if !result.warnings.is_empty() {
        println!("Warnings:");
        for warning in &result.warnings {
            println!("  ⚠ {}", warning);
        }
        println!();
    }
}
```

### 4.2 Policy Analysis Command

**File**: `tools/reaper-cli/src/commands/analyze_policy.rs` (NEW)

```bash
# Usage
reaper-cli analyze-policy --file policy.reap
reaper-cli analyze-policy --file policy.reap --check-ebpf
reaper-cli analyze-policy --file policy.reap --show-recommendations
```

**Output**:

```
=== Policy Analysis: example_policy ===

Rules: 5 total
  ✓ 3 promotable to eBPF (60%)
  ⚠ 2 must stay in userspace (40%)

eBPF-Ready Rules:
  ✓ rule_1: Simple comparison (complexity: 1, latency: ~50ns)
  ✓ rule_2: Two attribute checks (complexity: 2, latency: ~100ns)
  ✓ rule_3: Wildcard match (complexity: 0, latency: ~10ns)

Userspace Rules:
  ✗ rule_4: Comprehension (reason: requires dynamic iteration)
  ✗ rule_5: Regex (reason: no regex engine in eBPF)

Estimated Performance:
  Fast Path: 60% of requests
  Avg Latency: ~80ns (eBPF) + ~25µs (userspace weighted)

Recommendations:
  • Consider simplifying rule_4 to avoid comprehension
  • Replace regex in rule_5 with prefix/suffix matching
```

## Phase 5: Auto-Promotion Logic (Week 3)

### 5.1 Enhanced Learning Engine

**File**: `crates/reaper-ebpf/src/learning.rs` (UPDATE)

```rust
impl LearningEngine {
    pub fn analyze_promotion_candidates(&self) -> Vec<PromotionCandidate> {
        let mut candidates = Vec::new();

        for entry in self.patterns.iter() {
            let (resource, pattern) = entry.pair();

            if !self.should_promote(resource) {
                continue;
            }

            // Analyze if this pattern can be represented in eBPF
            let analysis = self.analyze_pattern_for_ebpf(pattern);

            if analysis.is_promotable {
                candidates.push(PromotionCandidate {
                    resource: resource.clone(),
                    decision: pattern.decision.clone(),
                    access_count: pattern.count,
                    complexity: analysis.complexity,
                    estimated_latency_ns: analysis.estimated_latency_ns,
                });
            }
        }

        // Sort by access count (hottest first)
        candidates.sort_by(|a, b| b.access_count.cmp(&a.access_count));

        candidates
    }

    fn analyze_pattern_for_ebpf(&self, pattern: &AccessPattern) -> EbpfAnalysis {
        // Check if decision is stable
        if pattern.decision_changes > 0 {
            return EbpfAnalysis {
                is_promotable: false,
                reason: Some("Unstable decision pattern".to_string()),
                ..Default::default()
            };
        }

        // Check if we have UID/GID requirements
        let has_identity = pattern.uid.is_some() || pattern.gid.is_some();

        // Simple patterns are always promotable
        EbpfAnalysis {
            is_promotable: true,
            complexity: if has_identity { 1 } else { 0 },
            estimated_latency_ns: if has_identity { 50 } else { 10 },
            reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromotionCandidate {
    pub resource: String,
    pub decision: PolicyAction,
    pub access_count: u64,
    pub complexity: u32,
    pub estimated_latency_ns: u32,
}

#[derive(Debug, Default)]
struct EbpfAnalysis {
    is_promotable: bool,
    complexity: u32,
    estimated_latency_ns: u32,
    reason: Option<String>,
}
```

## Implementation Phases Summary

| Phase | Duration | Deliverables |
|-------|----------|--------------|
| **Phase 1** | Week 1 | Core data structures (kernel + userspace) |
| **Phase 2** | Week 1-2 | Validation engine + data loader |
| **Phase 3** | Week 2 | Policy analysis engine |
| **Phase 4** | Week 2-3 | CLI validation tools |
| **Phase 5** | Week 3 | Auto-promotion with feedback |

## Success Criteria

- [ ] Can load 100K entities in < 1 second
- [ ] Validation catches all schema violations
- [ ] CLI provides clear feedback on eBPF compatibility
- [ ] Auto-promotion achieves >80% fast path rate
- [ ] Policy analysis correctly identifies promotable rules
- [ ] Data lookup latency < 100ns p99

## Next Steps

Do you want me to proceed with implementing these components? I suggest we start with:

1. **Phase 1 + 2 first**: Get the data layer working (structures + validation + loading)
2. **Test with real data**: Validate with your use cases
3. **Phase 3-5**: Build the intelligence layer (analysis + CLI + auto-promotion)

This way we can validate the foundation before building the advanced features.

Shall I proceed?
