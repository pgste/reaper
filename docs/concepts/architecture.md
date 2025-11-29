# Reaper Codebase Architecture Overview

## 1. Directory Structure and Main Components

```
reaper/
├── crates/                      # Core libraries
│   ├── reaper-core/            # Core types, traits, and utilities
│   ├── policy-engine/          # Main policy evaluation engine
│   ├── message-queue/          # Async messaging infrastructure
│   └── metrics/                # Performance monitoring
├── services/                    # Standalone services
│   ├── reaper-agent/           # Policy enforcement service (Port 8080)
│   └── reaper-platform/        # Policy management platform (Port 8081)
└── tools/
    └── reaper-cli/             # Command-line interface for management

Total codebase: ~1,682 lines of main Rust code across services
```

## 2. Core Architecture: Client-Server Separation

### A. Service Architecture

**Two-Service Model:**

1. **Reaper Platform (Port 8081)** - Management Layer
   - Policy CRUD operations (Create, Read, Update, Delete)
   - Policy versioning and lifecycle management
   - Agent management (placeholder for full implementation)
   - Central deployment coordination
   - Deployment statistics tracking

2. **Reaper Agent (Port 8080)** - Enforcement Layer
   - Sub-microsecond policy evaluation
   - Policy hot-swapping with zero downtime
   - Request processing and caching
   - Performance metrics tracking
   - Receives policies from Platform for deployment

### B. API Endpoints

**Platform API (8081):**
```
GET    /health                      # Health check
GET    /metrics                     # Platform metrics
GET    /api/v1/policies            # List all policies
POST   /api/v1/policies            # Create new policy
GET    /api/v1/policies/:id        # Get policy details
PUT    /api/v1/policies/:id        # Update policy
DELETE /api/v1/policies/:id        # Delete policy
POST   /api/v1/policies/:id/deploy # Deploy to agents
GET    /api/v1/agents              # List agents (placeholder)
GET    /api/v1/agents/:id          # Get agent details (placeholder)
```

**Agent API (8080):**
```
GET    /health                      # Health check
GET    /metrics                     # Agent performance metrics
POST   /api/v1/messages            # Evaluate policy request
POST   /api/v1/policies/deploy     # Deploy policy (from platform)
GET    /api/v1/policies            # List active policies
```

## 3. Policy Engine Architecture

### Core Components (500 lines in engine.rs)

**PolicyEngine** - High-Performance Lock-Free Store
- Uses `DashMap<PolicyId, Arc<EnhancedPolicy>>` for lock-free concurrent access
- Uses `DashMap<String, PolicyId>` for name-based lookups
- Atomic hot-swapping: policies are replaced atomically with zero downtime
- Performance: Policy lookup in ~50-200ns (nanoseconds)

**Policy Lifecycle:**
1. Create/Update: `deploy_policy()` - atomically inserts/replaces policy
2. Evaluate: `evaluate()` - looks up policy and runs evaluator
3. Remove: `remove_policy()` - atomically removes policy

### Key Data Structures

```rust
pub struct EnhancedPolicy {
    pub id: PolicyId,                    // UUID
    pub version: u64,                    // Versioning support
    pub name: String,
    pub description: String,
    pub language: PolicyLanguage,        // Simple, Cedar, or Custom
    pub content: String,                 // Raw policy text
    pub rules: Vec<PolicyRule>,          // For Simple language
    pub created_at: DateTime,
    pub updated_at: DateTime,
    evaluator: Option<Arc<dyn PolicyEvaluator>>, // Lazy-loaded
}

pub struct PolicyRequest {
    pub resource: String,
    pub action: String,
    pub context: HashMap<String, String>, // Additional context
}

pub struct PolicyDecision {
    pub decision: PolicyAction,           // Allow, Deny, or Log
    pub policy_id: PolicyId,
    pub policy_version: u64,
    pub evaluation_time_ns: u64,          // Sub-microsecond tracking
    pub matched_rule: Option<usize>,
}
```

## 4. Policy Language Support

### Multi-Language Evaluator Architecture

**PolicyEvaluator Trait** - Pluggable interface for policy languages:
```rust
pub trait PolicyEvaluator: Send + Sync + Debug {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction>;
    fn validate(&self) -> Result<(), ReaperError>;
    fn evaluator_type(&self) -> &str;
    fn metadata(&self) -> Option<EvaluatorMetadata>;
}
```

### Implemented Evaluators

1. **SimplePolicyEvaluator** (226 lines)
   - Wildcard matching (`*` matches any resource)
   - First-match-wins semantics
   - Default deny if no rules match
   - Performance: Sub-microsecond (< 1µs)
   - Best for: High-throughput APIs with simple rules

2. **CedarPolicyEvaluator** (319 lines)
   - AWS Cedar policy language support
   - Rich ABAC (Attribute-Based Access Control) capabilities
   - Schema validation
   - Condition evaluation against attributes
   - Performance: 10-50 microseconds depending on complexity
   - Best for: Complex authorization logic

3. **ReaperDSLEvaluator** (386 lines)
   - Native Reaper DSL compiled to fast evaluator
   - Custom optimized for Reaper's specific use cases
   - Future: Compile-time optimization
   - Performance: Sub-microsecond with optimization

4. **ReaperPolicy** - Policy Format Support
   - `.reap` format: Rust-like DSL (most concise)
   - `.yaml` format: YAML for human readability
   - `.json` format: JSON for programmatic use
   - All compile to identical internal representation
   - Binary bundle format (`.rbb`) for fast loading

## 5. Policy Loading and Management

### ReaperPolicy Module (20 supported functions)

**File-Based Loading:**
```rust
ReaperPolicy::from_file(path)           // Auto-detect .reap
ReaperPolicy::from_yaml_file(path)      // Explicit YAML
ReaperPolicy::from_json_file(path)      // Explicit JSON
ReaperPolicy::from_file_auto(path)      // Detect by extension
```

**String-Based Parsing:**
```rust
ReaperPolicy::from_str(input)           // .reap DSL
ReaperPolicy::from_yaml_str(input)      // YAML string
ReaperPolicy::from_json_str(input)      // JSON string
```

**Bundle Support (Pre-compiled Binary):**
```rust
policy.compile_to_bundle()              // Generate .rbb
ReaperPolicy::from_bundle(bytes)        // Load .rbb
```

### Platform API Workflow

1. **Policy Creation:**
   Platform receives policy definition → Validates rules → Deploys to engine

2. **Policy Update:**
   Platform receives update → Version increment → Hot-swap deployment (atomic)

3. **Policy Deployment to Agents:**
   Platform calls `/api/v1/policies/:id/deploy` → Agent receives at `/api/v1/policies/deploy`

## 6. Data Store Architecture (ABAC/ReBAC Support)

### Multi-Index Store (store.rs - 100+ lines)

**Entity Storage:**
```rust
pub struct DataStore {
    interner: Arc<StringInterner>,      // String deduplication (~60% memory savings)
    entities: Arc<DashMap<EntityId, Arc<Entity>>>,  // Primary index
    type_index: Arc<DashMap<EntityType, HashSet<EntityId>>>,  // By type
    attribute_index: Arc<DashMap<(AttrKey, AttrValue), HashSet<EntityId>>>,  // By attribute
    composite_index: Arc<DashMap<(Type, AttrKey, AttrValue), HashSet<EntityId>>>,  // Composite
}
```

**Entity Definition:**
```rust
pub struct Entity {
    pub id: EntityId,                   // Interned
    pub entity_type: EntityType,        // Interned
    pub attributes: Attributes,         // HashMap<InternedString, AttributeValue>
    pub parent: Option<EntityId>,       // For hierarchical relationships
}

pub enum AttributeValue {
    String(InternedString),             // Memory-efficient strings
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<AttributeValue>),
    Null,
}
```

**Loading Data:**
- JSON format support via `DataLoader`
- Automatic string interning for ~60% memory savings
- Supports complex attribute types and relationships
- Fast lookups: ID (20-50ns), Type (100-200ns), Attribute (100-300ns)

## 7. Performance Characteristics

### Design Goals
- **Evaluation Latency:** < 1 microsecond p99 (Simple policies)
- **Memory Usage:** < 50MB per agent instance
- **Throughput:** > 100K decisions/second per agent
- **Startup Time:** < 100ms cold start

### Optimization Techniques Used
1. **Lock-free concurrent data structures** (DashMap)
2. **Zero-copy sharing** via Arc<T>
3. **String interning** for entity attributes (~60% memory reduction)
4. **Atomic operations** for hot-swapping policies
5. **Lazy evaluation** of policy content
6. **Multi-index strategy** for fast entity lookups

## 8. Testing and Quality Assurance

### Testing Framework Stack
- **Unit Tests:** tokio-test, criterion benchmarks
- **BDD Tests:** Cucumber/Gherkin integration (21 framework)
- **Performance Benchmarks:** criterion with HTML reports
- **Property Testing:** proptest

### Test Files
- `reaper_bdd_tests.rs` - Core BDD tests
- `policy_bdd_tests.rs` - Policy engine BDD
- `gherkin_tests.rs` - Full Gherkin/Cucumber integration
- `message_queue_bdd_tests.rs` - Async messaging tests

## 9. Communication Infrastructure

### Message Queue (Async Foundation)
- Minimal stub implementation currently
- Foundation for future async communication between components
- Future use: Policy change propagation, metrics aggregation

### HTTP Transport
- Built on Axum web framework (async/await)
- Tokio async runtime
- Request/Response JSON serialization with serde

## 10. CLI Tool (reaper-cli)

### Commands
```bash
# Policy evaluation
reaper eval --policy <file> --data <json> --principal <id> \
            --action <action> --resource <resource>

# Policy compilation
reaper compile <input-files> --output bundle.rbb [--optimize]

# Policy validation
reaper validate <policy-file> [--data <json>]

# Policy management (via Platform API)
reaper policy list
reaper policy create <name> [--action allow] [--resource *]
reaper policy update <id> [--name] [--action] [--resource]
reaper policy delete <id>
reaper policy deploy <id>

# Agent management
reaper agent list
reaper agent get <id>

# System status
reaper status
reaper benchmark [--requests 1000]
```

## 11. Key Architectural Patterns

### 1. Pluggable Language Support
- PolicyEvaluator trait enables adding new languages without core changes
- Currently: Simple, Cedar, ReaperDSL
- Future: Custom optimizations, other languages

### 2. Atomic Hot-Swapping
- Policies replaced atomically using DashMap
- No downtime during updates
- Old policy dropped when Arc reference count reaches zero

### 3. Lock-Free Concurrency
- DashMap for reads without blocking
- Sub-microsecond lookups
- High concurrency for multi-threaded agents

### 4. String Interning
- All entity strings interned via StringInterner
- ~60% memory savings for duplicate strings
- Shared across all entities in DataStore

### 5. Layered Validation
- Syntax validation on parsing
- Semantic validation on evaluator creation
- Type checking before deployment

## 12. Crate Dependencies

### Core Dependencies
- **tokio**: Async runtime (full features)
- **axum**: Web framework for HTTP APIs
- **serde/serde_json**: Serialization
- **dashmap**: Lock-free concurrent HashMap
- **parking_lot**: High-performance mutex
- **uuid**: Policy ID generation
- **tracing**: Structured logging
- **clap**: CLI argument parsing

## 13. Entry Points and Main APIs

### Service Entry Points
1. **reaper-agent** main(): HTTP server on :8080, policy evaluation
2. **reaper-platform** main(): HTTP server on :8081, policy management
3. **reaper-cli** main(): Command-line tool for testing and management

### Core Library API Entry Points
1. `PolicyEngine::new()` - Create policy engine
2. `PolicyEngine::deploy_policy(policy)` - Hot-swap a policy
3. `PolicyEngine::evaluate(&policy_id, &request)` - Evaluate a request
4. `ReaperPolicy::from_file()` - Load policy from file
5. `DataStore::new()` - Create entity data store
6. `DataLoader::load_json()` - Load entity data

## 14. Data Flow

### Policy Deployment Flow
```
reaper-platform API (create/update) 
  → PolicyEngine::deploy_policy() 
  → DashMap atomic insert 
  → Evaluator built and cached 
  → Zero-downtime hot-swap complete
```

### Policy Evaluation Flow
```
reaper-agent API (/api/v1/messages) 
  → PolicyRequest extraction 
  → PolicyEngine::get_policy() (lock-free lookup ~50ns) 
  → Evaluator evaluation (1-50µs depending on language) 
  → PolicyDecision returned 
  → Metrics recorded
```

### Agent-Platform Coordination
```
reaper-platform creates/updates policy 
  → reaper-platform calls POST /api/v1/policies/deploy on agent 
  → agent::deploy_policy() handler 
  → PolicyEngine::deploy_policy() 
  → Agent confirms deployment
```

## 15. Configuration and State Management

### Agent State
```rust
struct AgentState {
    policy_engine: PolicyEngine,    // Shared across all handlers
    stats: Arc<AgentStats>,         // Performance metrics
}
```

### Platform State
```rust
struct PlatformState {
    policy_engine: PolicyEngine,              // Policies
    deployment_stats: Arc<RwLock<DeploymentStats>>,  // Deployment tracking
}
```

### State Sharing
- State wrapped in `Arc<State>` for thread-safe sharing
- Extracted via Axum `State(state)` extractor
- Cloned for each async handler

## Summary

Reaper is a **high-performance, distributed policy enforcement system** with:
- **Clear separation**: Platform (management) vs Agent (enforcement)
- **Multiple policy languages**: Simple, Cedar, ReaperDSL
- **Lock-free architecture**: Sub-microsecond evaluation
- **Atomic deployments**: Zero-downtime policy updates
- **Flexible data**: Entity-based ABAC/ReBAC support
- **Production-ready**: Comprehensive testing, monitoring, and CLI

The architecture prioritizes **performance** (sub-microsecond latency), **concurrency** (lock-free data structures), and **deployability** (atomic hot-swapping).
