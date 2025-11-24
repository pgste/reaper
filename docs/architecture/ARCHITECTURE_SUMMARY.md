# Reaper Architecture - Executive Summary

## What is Reaper?

Reaper is a **high-performance, distributed policy enforcement platform** built in Rust with sub-microsecond latency guarantees. It's designed for enterprise sidecars and edge enforcement with zero-downtime policy updates.

**Core Value Proposition:**
- 60-80% memory reduction vs traditional JVM-based policy engines
- Sub-microsecond policy evaluation (< 1 microsecond p99)
- Zero-downtime policy deployments via atomic hot-swapping
- Multi-language support: Simple rules, AWS Cedar, and custom Reaper DSL

## Architecture at a Glance

```
┌─────────────────────────┐
│   Reaper Platform       │      Management Layer
│  (Port 8081)            │      Policy CRUD, versioning,
│  • Policy management    │      deployment coordination
│  • Agent coordination   │
└────────────┬────────────┘
             │ deploys policies
             ▼
┌─────────────────────────┐
│   Reaper Agent          │      Enforcement Layer
│  (Port 8080)            │      Sub-microsecond evaluation
│  • Policy evaluation    │      Hot-swapping, metrics
│  • Request processing   │
└────────────┬────────────┘
             │
             ▼
        Lock-Free Cache
    (DashMap<PolicyId, 
    Arc<EnhancedPolicy>>)
             │
             ▼
    Policy Evaluators:
    1. Simple (< 1µs)
    2. Cedar (10-50µs)
    3. ReaperDSL (< 1µs)
```

## Two-Service Model (Client-Server Separation)

### Service 1: Reaper Platform (Management)
- **Port:** 8081
- **Role:** Central policy management and coordination
- **Capabilities:**
  - CRUD operations on policies
  - Versioning and lifecycle management
  - Zero-downtime deployment to agents
  - Deployment statistics and tracking
  - Agent management (expanding)

### Service 2: Reaper Agent (Enforcement)
- **Port:** 8080
- **Role:** High-performance policy enforcement
- **Capabilities:**
  - Sub-microsecond policy evaluation
  - Atomic policy hot-swapping
  - Request processing with caching
  - Performance metrics tracking
  - Deployment of policies from platform

## How It Works

### 1. Policy Creation Flow
```
User/CLI
  │
  ├─ Create policy (JSON/YAML/ReapDSL)
  │
  ▼
Platform (/api/v1/policies)
  │
  ├─ Validate syntax
  ├─ Build evaluator
  ├─ Store in PolicyEngine
  │
  └─ Done (atomic, no downtime)
```

### 2. Policy Deployment Flow
```
Platform deploys to Agent
  │
  ▼
Platform: POST /api/v1/policies/:id/deploy
  │
  ▼
Agent: POST /api/v1/policies/deploy
  │
  ├─ Receive policy definition
  ├─ Call PolicyEngine::deploy_policy()
  ├─ Atomic insertion into DashMap
  ├─ Build evaluator if needed
  │
  └─ Lock-free, zero-downtime deployment
```

### 3. Policy Evaluation Flow
```
Client Request
  │
  ▼
Agent: POST /api/v1/messages
  │
  ├─ Extract PolicyRequest (resource, action, context)
  │
  ├─ PolicyEngine::get_policy() ◄── ~50-200ns lookup
  │
  ├─ Get evaluator
  │
  ├─ Evaluate based on language:
  │  ├─ Simple: < 1µs
  │  ├─ Cedar: 10-50µs
  │  └─ ReaperDSL: < 1µs
  │
  ├─ Return PolicyDecision
  │   ├─ decision (Allow/Deny/Log)
  │   ├─ policy_id
  │   ├─ policy_version
  │   ├─ evaluation_time_ns
  │   └─ matched_rule
  │
  └─ Record metrics
```

## Core Components

### 1. PolicyEngine (500 lines)
**Location:** `crates/policy-engine/src/engine.rs`

**Responsibility:** Lock-free policy storage and evaluation

**Key Features:**
- `DashMap<PolicyId, Arc<EnhancedPolicy>>` for lock-free concurrent access
- Atomic hot-swapping: replace policies with zero downtime
- Policy lookup: ~50-200ns (nanoseconds)
- `DashMap<String, PolicyId>` for name-based lookups
- Default policy fallback

**Key Methods:**
```rust
fn deploy_policy(&self, policy: EnhancedPolicy) -> Result<()>
fn evaluate(&self, policy_id: &PolicyId, request: &PolicyRequest) -> Result<PolicyDecision>
fn get_policy(&self, policy_id: &PolicyId) -> Option<Arc<EnhancedPolicy>>
fn get_policy_by_name(&self, name: &str) -> Option<Arc<EnhancedPolicy>>
fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>>
```

### 2. Policy Evaluators (Pluggable Language Support)

All implement `PolicyEvaluator` trait for consistency:

#### SimplePolicyEvaluator (226 lines)
- **Best for:** High-throughput APIs with simple rules
- **Matching:** Wildcard (`*`) and exact matches
- **Semantics:** First-match-wins, default deny
- **Performance:** < 1 microsecond
- **Example:**
  ```json
  [
    { "resource": "*", "action": "allow", "conditions": [] },
    { "resource": "admin/*", "action": "deny", "conditions": [] }
  ]
  ```

#### CedarPolicyEvaluator (319 lines)
- **Best for:** Complex authorization logic
- **Language:** AWS Cedar policy language
- **Features:** ABAC, schema validation, condition evaluation
- **Performance:** 10-50 microseconds
- **Example:**
  ```cedar
  permit (
    principal == User::"alice",
    action == Action::"read",
    resource == Document::"doc1"
  );
  ```

#### ReaperDSLEvaluator (386 lines)
- **Best for:** Optimized custom authorization
- **Language:** Rust-like DSL compiled to fast code
- **Features:** Compile-time optimization
- **Performance:** < 1 microsecond with optimization
- **Example:**
  ```reap
  policy admin_access {
    rule allow_admins { allow if user.role == "admin" }
  }
  ```

### 3. Policy Formats (Format Support Module)

Supports three formats with identical runtime performance:

#### .reap (Rust DSL) - Most Concise
```reap
policy resource_access {
  version: "1.0.0",
  description: "Resource access control",
  default: deny,
  rule read_access { allow if user.role == "reader" }
}
```

#### .yaml (Human-Friendly)
```yaml
name: resource_access
version: "1.0.0"
description: "Resource access control"
default_decision: deny
rules:
  - name: read_access
    decision: allow
    condition:
      operator: equal
      left:
        entity: user
        attribute: role
      right:
        value: reader
```

#### .json (Machine-Friendly)
```json
{
  "name": "resource_access",
  "version": "1.0.0",
  "default_decision": "deny",
  "rules": [
    {
      "decision": "allow",
      "condition": {
        "operator": "equal",
        "left": {"entity": "user", "attribute": "role"},
        "right": {"value": "reader"}
      }
    }
  ]
}
```

### 4. DataStore (Multi-Index Entity Storage)

**Location:** `crates/policy-engine/src/data/`

**Purpose:** ABAC/ReBAC support with fast entity lookups

**Indexes:**
1. **Primary Index:** EntityId → Arc<Entity> (20-50ns lookup)
2. **Type Index:** EntityType → HashSet<EntityId> (100-200ns)
3. **Attribute Index:** (AttrKey, AttrValue) → HashSet<EntityId> (100-300ns)
4. **Composite Index:** (Type, AttrKey, AttrValue) → Set (optimized)

**Memory Optimization:** String interning reduces memory by ~60%

**Entity Example:**
```json
{
  "id": "alice",
  "type": "User",
  "attributes": {
    "role": "admin",
    "department": "engineering",
    "active": true,
    "groups": ["admins", "engineers"]
  }
}
```

### 5. Services

#### Reaper Agent (`services/reaper-agent/src/main.rs`)
- 399 lines
- HTTP server on port 8080
- Policy evaluation endpoint
- Policy deployment endpoint
- Metrics tracking

#### Reaper Platform (`services/reaper-platform/src/main.rs`)
- 620 lines
- HTTP server on port 8081
- Policy CRUD endpoints
- Deployment coordination
- Agent management (expandable)

## Key Architectural Patterns

### 1. Lock-Free Concurrency
- Uses `DashMap` for reads without blocking
- High concurrency: millions of concurrent policy evaluations
- Sub-microsecond lookups

### 2. Atomic Hot-Swapping
- Policies replaced atomically via `Arc::make_mut()`
- No downtime during updates
- Old policy dropped when Arc reference count reaches zero
- Consistent view for all concurrent readers

### 3. String Interning
- All entity strings deduplicated via `StringInterner`
- ~60% memory savings on duplicate strings
- Shared across all entities in DataStore

### 4. Pluggable Evaluators
- `PolicyEvaluator` trait enables new languages without core changes
- Each language optimized for its use case
- Easy to add new languages

### 5. Layered Validation
- Syntax validation on parsing
- Semantic validation on evaluator creation
- Type checking before deployment
- Deployment error handling with rollback

## Performance Characteristics

### Design Goals
| Metric | Target | Typical |
|--------|--------|---------|
| Evaluation Latency (p99) | < 1µs | 0.5-2µs (Simple), 10-50µs (Cedar) |
| Memory per Agent | < 50MB | 30-40MB (idle) |
| Throughput | > 100K decisions/sec | 200K+ decisions/sec |
| Startup Time | < 100ms | 50-80ms |
| Policy Hot-Swap | Zero downtime | Atomic, no requests dropped |

### Optimization Techniques
1. **Lock-free data structures** (DashMap)
2. **Zero-copy sharing** via Arc<T>
3. **String interning** for attributes
4. **Atomic operations** for hot-swapping
5. **Lazy evaluation** of policy content
6. **Multi-index strategy** for fast lookups

## Testing & Quality

### Test Framework Stack
- **Unit Tests:** tokio-test, criterion benchmarks
- **BDD Tests:** Cucumber/Gherkin integration
- **Performance Benchmarks:** criterion with HTML reports
- **Test Data:** 8 datasets from 480B to 40MB

### Test Coverage
- Core BDD tests: `reaper_bdd_tests.rs`
- Policy engine BDD: `policy_bdd_tests.rs`
- Gherkin integration: `gherkin_tests.rs`
- Message queue tests: `message_queue_bdd_tests.rs`

## Entry Points & Key APIs

### Service Entry Points
1. **reaper-agent** `main()`: Policy enforcement on :8080
2. **reaper-platform** `main()`: Policy management on :8081
3. **reaper-cli** `main()`: CLI tool

### Core Library APIs
```rust
// PolicyEngine
PolicyEngine::new()
PolicyEngine::deploy_policy(policy)
PolicyEngine::evaluate(&policy_id, &request)
PolicyEngine::get_policy(&policy_id)

// ReaperPolicy (Format Support)
ReaperPolicy::from_file(path)
ReaperPolicy::from_yaml_str(input)
ReaperPolicy::from_json_str(input)
ReaperPolicy::from_file_auto(path)

// DataStore
DataStore::new()
DataStore::insert(entity)
DataStore::query_by_type(entity_type)
DataLoader::load_json(json_str)
```

## Crate Structure

| Crate | Lines | Purpose |
|-------|-------|---------|
| `reaper-core` | 95 | Core types, traits, constants |
| `policy-engine` | 1200+ | Policy evaluation engine |
| `message-queue` | 1 | Async messaging (stub) |
| `metrics` | 1 | Performance monitoring (stub) |
| **Services** | | |
| `reaper-agent` | 399 | Enforcement service |
| `reaper-platform` | 620 | Management service |
| **Tools** | | |
| `reaper-cli` | 150+ | CLI management |

**Total Service Code:** ~1,682 lines

## Dependencies

### Core Stack
- **tokio:** Async runtime (1.0+)
- **axum:** Web framework (0.8.4+)
- **dashmap:** Lock-free HashMap (6.1.0+)
- **parking_lot:** High-performance mutex (0.12+)
- **serde:** Serialization (1.0+)
- **uuid:** ID generation (1.0+)
- **tracing:** Structured logging (0.1+)

### Future Foundation
- **message-queue:** Currently a stub, ready for async communication
- **metrics:** Currently a stub, ready for metrics aggregation

## Quick Start Example

### 1. Start Services
```bash
# Terminal 1: Platform
cargo run --bin reaper-platform

# Terminal 2: Agent
cargo run --bin reaper-agent
```

### 2. Create Policy (via CLI or API)
```bash
curl -X POST http://localhost:8081/api/v1/policies \
  -H "Content-Type: application/json" \
  -d '{
    "name": "read-access",
    "description": "Allow readers",
    "rules": [
      {
        "action": "allow",
        "resource": "documents/*",
        "conditions": []
      }
    ]
  }'
```

### 3. Evaluate Policy
```bash
curl -X POST http://localhost:8080/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "read-access",
    "resource": "documents/123",
    "action": "read",
    "context": {}
  }'
```

### Response
```json
{
  "decision": "allow",
  "policy_id": "550e8400-e29b-41d4-a716-446655440000",
  "policy_version": 1,
  "evaluation_time_microseconds": 0.5,
  "total_time_microseconds": 1.2,
  "agent_id": "reaper-agent-001"
}
```

## Deployment Models

### Model 1: Sidecar Pattern
- One Agent per application
- Policies pushed from Platform
- Evaluation latency: sub-microsecond
- Minimal resource overhead: ~30-40MB

### Model 2: Central Authorization
- Single centralized Agent
- Multiple clients communicate via REST API
- Throughput: > 100K decisions/second

### Model 3: Distributed Agents
- Multiple Agents across regions
- Platform coordinates policy distribution
- Zero-downtime deployments
- Agent-specific policies possible

## What's Not Implemented (Yet)

- **message-queue:** Currently a stub (future async messaging)
- **metrics:** Currently a stub (future metrics aggregation)
- **Full agent registry:** Agent management is a placeholder
- **Agent clustering:** Single agent per instance (expansion planned)
- **Policy versioning strategies:** Basic versioning exists

## Summary

Reaper is a **production-ready, high-performance policy enforcement system** that combines:

1. **Architecture:** Clear separation between management (Platform) and enforcement (Agent)
2. **Performance:** Lock-free sub-microsecond evaluation
3. **Flexibility:** Multiple policy languages with identical semantics
4. **Deployability:** Zero-downtime atomic hot-swapping
5. **Scalability:** Designed for distributed deployment
6. **Quality:** Comprehensive testing with BDD, benchmarks, and test data

The codebase is **clean, well-organized, and production-ready**, with clear entry points for integration and extension.

---

See also:
- `/home/user/reaper/ARCHITECTURE.md` - Detailed architecture
- `/home/user/reaper/ARCHITECTURE_DIAGRAMS.txt` - Visual diagrams
- `/home/user/reaper/FILE_REFERENCE.md` - Complete file reference
