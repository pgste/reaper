# Reaper Codebase - Quick File Reference

## Core Libraries (Crates)

### reaper-core (32 lines)
Main entry point for core types and traits.

| File | Purpose | Lines |
|------|---------|-------|
| `src/lib.rs` | Module exports, version info, API endpoints | 30 |
| `src/policy.rs` | Basic Policy struct, PolicyEngine trait | 19 |
| `src/agent.rs` | Agent, AgentStatus, AgentId types | 26 |
| `src/platform.rs` | Platform, PlatformConfig, AgentRegistry | 18 |
| `src/error.rs` | ReaperError enum, error handling | 32 |

### policy-engine (Core Logic)

| File | Purpose | Lines | Key Components |
|------|---------|-------|-----------------|
| `src/lib.rs` | Main exports for policy engine | 36 | Re-exports all modules |
| `src/engine.rs` | **PolicyEngine, EnhancedPolicy, PolicyDecision** | 500 | Lock-free policy store, hot-swapping |
| **Evaluators Module** | | | |
| `src/evaluators/mod.rs` | PolicyEvaluator trait definition | 120 | Pluggable language interface |
| `src/evaluators/simple.rs` | Simple rule-based evaluator | 226 | Wildcard matching, <1µs perf |
| `src/evaluators/cedar.rs` | AWS Cedar policy support | 319 | ABAC, complex conditions |
| `src/evaluators/cedar_integration.rs` | Cedar entity integration | 245 | Entity to Cedar mapping |
| `src/evaluators/reaper_dsl.rs` | Native Reaper DSL evaluator | 386 | Compiled policies, optimized |
| **Policy Format Module** | | | |
| `src/reap/mod.rs` | **ReaperPolicy** main entry point | - | Load .reap/.yaml/.json files |
| `src/reap/parser.rs` | .reap format parser | - | PEG grammar parsing |
| `src/reap/yaml_parser.rs` | YAML/JSON format support | - | Format equivalence |
| `src/reap/ast.rs` | Abstract syntax tree | - | Policy representation |
| `src/reap/compiler.rs` | Compile AST to evaluator | - | Validation, optimization |
| `src/reap/bundle.rs` | Binary bundle format (.rbb) | - | Pre-compiled policies |
| **Data Store Module** | | | |
| `src/data/mod.rs` | Data module root | - | Module exports |
| `src/data/store.rs` | **DataStore** with multi-index | 100+ | Lock-free entity storage |
| `src/data/entity.rs` | **Entity, AttributeValue** | 120+ | Entity definitions |
| `src/data/interning.rs` | **StringInterner** | - | String deduplication |
| `src/data/loader.rs` | **DataLoader** | 120+ | Load JSON entity data |
| **Testing Module** | | | |
| `src/gherkin/mod.rs` | Gherkin/Cucumber support | - | BDD test framework |
| `src/gherkin/world.rs` | **TestContext** for BDD | - | Test fixtures |
| `tests/policy_bdd_tests.rs` | Policy engine BDD tests | - | Feature-based tests |
| `tests/gherkin_tests.rs` | Full Gherkin integration | - | Cucumber framework |

### message-queue (1 line)
Placeholder for future async messaging infrastructure.

| File | Purpose |
|------|---------|
| `src/lib.rs` | Currently a stub, foundation for async communication |

### metrics (1 line)
Placeholder for performance monitoring infrastructure.

| File | Purpose |
|------|---------|
| `src/lib.rs` | Currently a stub, foundation for metrics collection |

## Services

### reaper-agent (Main Enforcement Service)

| File | Lines | Purpose | Key Components |
|------|-------|---------|-----------------|
| `src/main.rs` | 399 | Policy enforcement service | HTTP server on port 8080 |

**API Endpoints:**
- `GET /health` - Health check
- `GET /metrics` - Performance metrics
- `POST /api/v1/messages` - Evaluate policy request
- `POST /api/v1/policies/deploy` - Deploy policy (from platform)
- `GET /api/v1/policies` - List active policies

**Internal Components:**
- `AgentState` - Shared state (PolicyEngine + stats)
- `AgentStats` - Atomic counters for metrics
- Request handlers for each API endpoint

### reaper-platform (Policy Management Service)

| File | Lines | Purpose | Key Components |
|------|-------|---------|-----------------|
| `src/main.rs` | 620 | Policy management platform | HTTP server on port 8081 |

**API Endpoints:**
- `GET /api/v1/policies` - List policies
- `POST /api/v1/policies` - Create policy
- `GET /api/v1/policies/:id` - Get policy
- `PUT /api/v1/policies/:id` - Update policy
- `DELETE /api/v1/policies/:id` - Delete policy
- `POST /api/v1/policies/:id/deploy` - Deploy to agents
- `GET /api/v1/agents` - List agents (placeholder)

**Internal Components:**
- `PlatformState` - Shared state (PolicyEngine + deployment stats)
- `DeploymentStats` - Track deployment success/failure
- Request handlers for each API endpoint

## Tools

### reaper-cli (Command-Line Interface)

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | 150+ | CLI tool for Reaper management |

**Commands:**
- `eval` - Evaluate policy file locally
- `validate` - Validate policy syntax
- `compile` - Compile policy to bundle
- `policy` - Policy management commands
  - `list` - List all policies
  - `create` - Create new policy
  - `update` - Update policy
  - `delete` - Delete policy
  - `deploy` - Deploy to agents
- `agent` - Agent management
- `status` - Platform status
- `demo` - Demo workflow
- `benchmark` - Performance testing

## Documentation Files

| File | Purpose |
|------|---------|
| `README.md` | Project overview, quick start |
| `YAML_FORMAT.md` | YAML/JSON policy format specification |
| `GHERKIN_INTEGRATION.md` | BDD testing documentation |
| `POLICY_TESTS.md` | Policy testing guide |
| `HTTP_OPTIMIZATION_GUIDE.md` | HTTP/sidecar optimization |
| `PERFORMANCE_ANALYSIS.md` | Performance benchmarks |
| `OPTIMIZATION_ANALYSIS.md` | Multilayer policy optimization |
| `SIDECAR_DEPLOYMENT.md` | Sidecar deployment guide |

## Test Data Files

| File | Size | Purpose |
|------|------|---------|
| `test-data.json` | 480B | Simple test data |
| `rbac-test-data.json` | 789KB | RBAC policy test data |
| `abac-test-data.json` | 1.1MB | ABAC policy test data |
| `rebac-test-data.json` | 1.6MB | Relationship-based ACL test data |
| `multilayer-test-data.json` | 2.1MB | Combined RBAC+ABAC+ReBAC |
| `large-test-data.json` | 345KB | Large dataset |
| `huge-test-data.json` | 40MB | Performance stress test |

## Code Organization Summary

```
crates/reaper-core/
├── Minimal core types (Policy, Agent, Platform)
├── Error types and constants
└── API endpoint constants

crates/policy-engine/
├── PolicyEngine (core evaluation loop)
├── EnhancedPolicy (policy with metadata)
├── PolicyEvaluator trait (pluggable languages)
├── Evaluators (Simple, Cedar, ReaperDSL)
├── ReaperPolicy (format support: .reap, .yaml, .json)
├── DataStore (entity storage with indexing)
└── Testing (BDD, unit tests)

services/reaper-agent/
├── HTTP server on port 8080
├── Policy evaluation endpoint
├── Policy deployment endpoint
└── Metrics endpoint

services/reaper-platform/
├── HTTP server on port 8081
├── Policy CRUD endpoints
├── Deployment coordination
└── Agent management (placeholder)

tools/reaper-cli/
├── Policy validation and compilation
├── Policy evaluation (offline)
├── API-based policy management
└── Performance benchmarking
```

## Key Architecture Files

| File | Key Responsibility |
|------|-------------------|
| `crates/policy-engine/src/engine.rs` | Lock-free PolicyEngine with hot-swapping |
| `crates/policy-engine/src/evaluators/mod.rs` | Pluggable evaluator trait |
| `crates/policy-engine/src/data/store.rs` | Multi-index DataStore |
| `services/reaper-agent/src/main.rs` | Policy enforcement service |
| `services/reaper-platform/src/main.rs` | Policy management service |
| `tools/reaper-cli/src/main.rs` | CLI management tool |

## Dependency Graph

```
reaper-core (core types)
    ↑
    ├── policy-engine (depends on reaper-core)
    │   ├── Uses: dashmap, parking_lot, serde, uuid, tracing
    │   └── Provides: PolicyEngine, Evaluators, DataStore
    │
    ├── message-queue (stub)
    │   └── Depends on: reaper-core
    │
    └── metrics (stub)
        └── Depends on: reaper-core

reaper-agent (service)
    ├── Depends on: policy-engine, reaper-core
    └── Uses: axum, tokio, serde_json, tracing

reaper-platform (service)
    ├── Depends on: policy-engine, reaper-core
    └── Uses: axum, tokio, serde_json, tracing

reaper-cli (tool)
    ├── Depends on: policy-engine, reaper-core
    └── Uses: clap, reqwest (for HTTP calls to platform/agent)
```

## File Size Summary

Most important/largest files:
1. `services/reaper-platform/src/main.rs` - 620 lines (full platform service)
2. `services/reaper-agent/src/main.rs` - 399 lines (full agent service)
3. `crates/policy-engine/src/engine.rs` - 500 lines (core policy engine)
4. `crates/policy-engine/src/evaluators/reaper_dsl.rs` - 386 lines (ReaperDSL)
5. `crates/policy-engine/src/evaluators/cedar.rs` - 319 lines (Cedar)
6. `crates/policy-engine/src/evaluators/simple.rs` - 226 lines (Simple)
7. `tools/reaper-cli/src/main.rs` - 150+ lines (CLI tool)

Total service code: ~1,682 lines (excluding tests, docs, and data files)
