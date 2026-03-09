# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Reaper is a high-performance policy enforcement platform written in Rust with sub-microsecond latency guarantees. It provides:
- 60-80% memory reduction vs traditional JVM-based policy engines
- Sub-microsecond policy evaluation (< 1µs p99)
- Zero-downtime policy deployments via atomic hot-swapping
- Support for multiple policy languages (Simple rules, AWS Cedar, Reaper DSL)

## Development Commands

### Setup and Build
```bash
make setup              # One-time development environment setup
make build              # Build all workspace members
cargo build --workspace # Alternative: direct cargo build
```

### Running Services
```bash
make dev-services       # Run both Agent (8080) and Platform (8081)
make agent              # Run only Reaper Agent on port 8080
make platform           # Run only Reaper Platform on port 8081
make cli                # Build the CLI tool
```

### Docker Deployment
```bash
# Just the agent (standalone enforcement)
docker compose --profile engine up -d

# Agent + Platform (simple management)
docker compose --profile platform up -d

# Enterprise stack (Agent + Management + PostgreSQL)
docker compose --profile management up -d

# Full stack with observability (Prometheus, Grafana, Tempo, Loki)
docker compose --profile full --profile observability up -d
```

| Profile | Services | Use Case |
|---------|----------|----------|
| `engine` | Agent | Simple policy enforcement |
| `platform` | Agent, Platform | Basic management |
| `management` | Agent, Management, PostgreSQL | Enterprise with centralized control |
| `observability` | Prometheus, Grafana, Tempo, Loki | Monitoring stack |
| `full` | All core services | Complete deployment |

### Development Workflow
```bash
make dev                # Auto-reload on changes (cargo watch)
make check              # Run fmt, clippy, and tests
cargo fmt --check       # Format check
cargo clippy --workspace -- -D warnings  # Lint
```

### Testing
```bash
# Run all tests (unit + integration + BDD)
make test

# Run only unit tests
cargo test --workspace --lib

# Run only BDD scenarios (Cucumber/Gherkin)
make bdd
cargo test --workspace --test '*bdd*'

# Run specific BDD test file
cargo test --test gherkin_tests
cargo test --test platform_bdd_tests

# Run benchmarks
make bench
cargo bench --workspace
```

### Single Test Execution
```bash
# Run a specific test by name
cargo test --lib <test_name>

# Run a specific test in a crate
cargo test -p policy-engine --lib <test_name>

# Run a specific BDD scenario
cargo test --test gherkin_tests -- --name "scenario_name"
```

### Code Quality
```bash
make coverage           # Generate test coverage report (HTML in coverage/)
cargo tarpaulin --workspace --out Html --output-dir coverage/
```

### Release
```bash
make release            # Patch release (default)
make release VERSION=minor
make release VERSION=major
```

## Architecture Overview

### Two-Service Model

**Reaper Platform (Port 8081)** - Management Layer
- Policy CRUD operations and versioning
- Centralized deployment coordination to agents
- Agent management (expanding)
- Located in: `services/reaper-platform/`

**Reaper Agent (Port 8080)** - Enforcement Layer
- Sub-microsecond policy evaluation
- Atomic policy hot-swapping (zero downtime)
- Request processing and metrics
- Located in: `services/reaper-agent/`

### Workspace Structure

```
reaper/
├── crates/
│   ├── reaper-core/        # Core types, traits
│   ├── policy-engine/      # Policy evaluation engine
│   │   ├── src/
│   │   │   ├── engine.rs   # PolicyEngine - lock-free store
│   │   │   ├── evaluators/ # SimplePolicyEvaluator, CedarPolicyEvaluator, ReaperDSLEvaluator
│   │   │   ├── data/       # DataStore - multi-index entity storage
│   │   │   ├── reap/       # ReaperPolicy - format support (.reap/.yaml/.json)
│   │   │   └── gherkin/    # Cucumber/Gherkin integration
│   ├── reaper-sdk/         # Client SDK (HTTP + future UDP support)
│   └── reaper-ebpf/        # eBPF kernel integration (experimental)
├── services/
│   ├── reaper-agent/       # Agent service - enforcement layer
│   ├── reaper-platform/    # Platform service - management layer
│   ├── reaper-management/  # Multi-tenant management server
│   └── reaper-sync/        # Policy synchronization client
├── tools/
│   └── reaper-cli/         # CLI management tool
├── deploy/
│   ├── kubernetes/         # Raw K8s manifests
│   └── helm/reaper/        # Helm chart
├── docs/                   # Organized documentation
│   ├── getting-started/
│   ├── concepts/           # Bundle format, event-driven loading
│   ├── architecture/
│   ├── deployment/         # Operations guide, deployment patterns
│   ├── performance/
│   └── HISTORY.md          # Historical milestones
├── benchmarks/
│   └── reaper-vs-opa/      # Reaper vs OPA comparison benchmark
└── test-data/              # Test policies and data files
```

### Core Components

**PolicyEngine** (`crates/policy-engine/src/engine.rs` - 500 lines)
- Lock-free concurrent policy store using `DashMap<PolicyId, Arc<EnhancedPolicy>>`
- Atomic hot-swapping: policies replaced atomically with zero downtime
- Policy lookup: ~50-200ns (nanoseconds)
- Key methods:
  - `deploy_policy(&self, policy: EnhancedPolicy)` - Atomic insert/replace
  - `evaluate(&self, policy_id, request)` - Evaluate request against policy
  - `get_policy(&self, policy_id)` - Lock-free policy lookup
  - `get_policy_by_name(&self, name)` - Name-based lookup

**Policy Evaluators** (Pluggable via `PolicyEvaluator` trait)
1. **SimplePolicyEvaluator** (226 lines) - Wildcard matching, < 1µs, first-match-wins
2. **CedarPolicyEvaluator** (319 lines) - AWS Cedar, ABAC, 10-50µs
3. **ReaperDSLEvaluator** (386 lines) - Native Reaper DSL, < 1µs with optimization

**DataStore** (`crates/policy-engine/src/data/`)
- Multi-index entity storage for ABAC/ReBAC support
- String interning reduces memory by ~60%
- Fast lookups: ID (20-50ns), Type (100-200ns), Attribute (100-300ns)
- Supports JSON data loading via `DataLoader`

**ReaperPolicy** (`crates/policy-engine/src/reap/`)
- Support for three policy formats: `.reap` (DSL), `.yaml`, `.json`
- Binary bundle format (`.rbb`) for fast loading
- All formats compile to identical internal representation
- Functions:
  - `from_file(path)` - Auto-detect .reap
  - `from_yaml_file(path)` / `from_json_file(path)` - Explicit format
  - `from_file_auto(path)` - Detect by extension
  - `compile_to_bundle()` - Generate .rbb
  - `from_bundle(bytes)` - Load .rbb

## Key Architectural Patterns

### Lock-Free Concurrency
- Uses `DashMap` for lock-free reads without blocking
- High concurrency: millions of concurrent policy evaluations
- Sub-microsecond lookups

### Atomic Hot-Swapping
- Policies replaced atomically using `Arc::make_mut()`
- Zero downtime during updates
- Old policy dropped when Arc reference count reaches zero
- Consistent view for all concurrent readers

### String Interning
- All entity strings deduplicated via `StringInterner`
- ~60% memory savings on duplicate strings
- Shared across all entities in DataStore

### Pluggable Evaluators
- `PolicyEvaluator` trait enables adding new languages without core changes
- Each language optimized for its use case (Simple < 1µs, Cedar 10-50µs)

## API Endpoints

### Platform API (Port 8081)
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

### Agent API (Port 8080)
```
GET    /health                      # Health check
GET    /ready                       # Readiness check
GET    /live                        # Liveness check
GET    /metrics                     # Agent performance metrics
POST   /api/v1/messages            # Evaluate policy request
POST   /api/v1/policies/deploy     # Deploy policy (from platform)
GET    /api/v1/policies            # List active policies

# Decision Logging (OPA-style audit logs)
GET    /api/v1/decisions           # Query recent decisions (paginated)
GET    /api/v1/decisions/stats     # Decision statistics
GET    /api/v1/decisions/stream    # SSE stream of decisions (real-time)
POST   /api/v1/decisions/export    # Export to file (NDJSON format)
```

## Decision Logging (OPA-style Audit)

Reaper provides structured decision logging for audit, compliance, and observability.

### Configuration
```bash
REAPER_DECISION_LOG_ENABLED=true         # Enable decision logging
REAPER_DECISION_LOG_CAPACITY=10000       # Ring buffer capacity
REAPER_DECISION_LOG_FILE=/var/log/reaper/decisions.ndjson
```

### Decision Log Entry Format
```json
{
  "timestamp": "2024-01-15T10:30:00Z",
  "decision_id": "uuid",
  "principal": "alice",
  "action": "read",
  "resource": "/api/data",
  "decision": "allow",
  "policy_id": "uuid",
  "policy_name": "data-access",
  "evaluation_time_ns": 450,
  "agent_id": "agent-1"
}
```

### Core Components
- **DecisionLogEntry** (`crates/policy-engine/src/decision_log.rs`) - Structured log entry
- **DecisionBuffer** (`crates/policy-engine/src/decision_buffer.rs`) - Lock-free ring buffer
- **Agent Endpoints** - Query, stream, and export decisions

### SIEM Integration
Decision logs use NDJSON format, compatible with Splunk, Elasticsearch, Datadog, and Sumo Logic.

## Data Flow Patterns

### Policy Deployment Flow
```
Platform API (create/update)
  → PolicyEngine::deploy_policy()
  → DashMap atomic insert
  → Evaluator built and cached
  → Zero-downtime hot-swap complete
```

### Policy Evaluation Flow
```
Agent API (/api/v1/messages)
  → PolicyRequest extraction
  → PolicyEngine::get_policy() (~50ns lookup)
  → Evaluator evaluation (1-50µs depending on language)
  → PolicyDecision returned
  → Metrics recorded
```

### Agent-Platform Coordination
```
Platform creates/updates policy
  → Platform calls POST /api/v1/policies/deploy on agent
  → agent::deploy_policy() handler
  → PolicyEngine::deploy_policy()
  → Agent confirms deployment
```

## Testing Strategy

### Test Framework Stack
- **Unit Tests**: tokio-test, proptest for property testing
- **BDD Tests**: Cucumber/Gherkin integration with `.feature` files
- **Benchmarks**: criterion with HTML reports

### Test Files by Component
- `crates/reaper-core/tests/reaper_bdd_tests.rs` - Core BDD tests
- `crates/policy-engine/tests/gherkin_tests.rs` - Full Gherkin/Cucumber integration
- `services/reaper-platform/tests/platform_bdd_tests.rs` - Platform BDD tests

### Gherkin Feature Files
- Located in `*/tests/features/*.feature`
- Examples: `rbac.feature`, `abac.feature`, `multilayer.feature`
- Run with: `cargo test --test gherkin_tests`

### Performance Test Examples
- `crates/policy-engine/examples/test_rbac_10k.rs` - RBAC at scale
- `crates/policy-engine/examples/test_abac_10k.rs` - ABAC at scale
- `crates/policy-engine/examples/test_multilayer_10k.rs` - Multilayer policies
- `crates/policy-engine/examples/memory_volume_test.rs` - Memory profiling

## Benchmarking

### Reaper vs OPA Comparison Benchmark

A comprehensive benchmark suite for comparing Reaper and OPA performance:

**Location**: `benchmarks/reaper-vs-opa/`

**Quick Start**:
```bash
cd benchmarks/reaper-vs-opa

# Docker mode (fully automated)
DOCKER=1 ./run-benchmark.sh

# Local mode (requires services running)
./run-benchmark.sh --requests 10000 --concurrency 50
```

**Features**:
- Equivalent policies in .reap (Reaper) and .rego (OPA) formats
- RBAC and ABAC scenarios
- HDR histogram for accurate latency percentiles
- Multiple output formats (table, JSON, CSV)
- Docker orchestration for reproducible tests
- CI integration ready

**Documentation**:
- `QUICKSTART.md` - Quick start guide
- `README.md` - Comprehensive documentation
- `VALIDATION.md` - Build verification report

**Metrics Collected**:
- Throughput (requests/second)
- Latency distribution (p50, p95, p99, max)
- Success rate
- Performance comparison (Reaper vs OPA)

## Performance Characteristics

### Design Goals
- **Evaluation Latency**: < 1µs p99 (Simple policies), 10-50µs (Cedar)
- **Memory Usage**: < 50MB per agent instance
- **Throughput**: > 100K decisions/second per agent
- **Startup Time**: < 100ms cold start
- **Policy Hot-Swap**: Zero downtime, atomic

### Optimization Techniques
1. Lock-free data structures (DashMap)
2. Zero-copy sharing via Arc<T>
3. String interning for attributes (~60% memory reduction)
4. Atomic operations for hot-swapping
5. Lazy evaluation of policy content
6. Multi-index strategy for fast entity lookups

## Important Code Patterns

### Policy Engine State Sharing
```rust
struct AgentState {
    policy_engine: PolicyEngine,    // Shared across all handlers
    stats: Arc<AgentStats>,         // Performance metrics
}

// Wrapped in Arc<State> for thread-safe sharing
// Extracted via Axum State(state) extractor
```

### Atomic Policy Deployment
```rust
// Policies are stored in DashMap with Arc for zero-copy sharing
DashMap<PolicyId, Arc<EnhancedPolicy>>

// Hot-swap is atomic - no locks, no downtime
policies.insert(policy_id, Arc::new(policy));
```

### Evaluator Trait Implementation
When adding a new policy language:
1. Implement `PolicyEvaluator` trait
2. Add to `PolicyLanguage` enum
3. Update `EnhancedPolicy::build_evaluator()`
4. Test with BDD scenarios

## CLI Tool Usage

```bash
# Policy evaluation
./target/debug/reaper-cli eval --policy <file> --data <json> \
    --principal <id> --action <action> --resource <resource>

# Policy testing (CI/CD integration)
./target/debug/reaper-cli test --policy <file> --data <json> \
    --principal <id> --action <action> --resource <resource> \
    --expect <allow|deny> [--verbose]

# Batch test suite (YAML format)
./target/debug/reaper-cli test-suite --file tests.yaml [--fail-fast]

# Policy compilation to binary bundle
./target/debug/reaper-cli compile <input-files> --output bundle.rbb

# Bundle operations
./target/debug/reaper-cli bundle info <bundle.rbb>
./target/debug/reaper-cli bundle deploy <bundle.rbb> [--data <json>] [--force]
./target/debug/reaper-cli bundle validate <bundle.rbb>

# Policy validation
./target/debug/reaper-cli validate <policy-file> [--data <json>]

# Policy management via Platform API
./target/debug/reaper-cli policy list
./target/debug/reaper-cli policy create <name> [--action allow] [--resource *]
./target/debug/reaper-cli policy update <id> [--name] [--action] [--resource]
./target/debug/reaper-cli policy deploy <id>

# System status
./target/debug/reaper-cli status
./target/debug/reaper-cli benchmark [--requests 1000]
```

### Test Suite Format (YAML)
```yaml
tests:
  - name: "Admin can access dashboard"
    policy: "policies/admin.reap"
    data: "data/entities.json"
    principal: "admin_alice"
    action: "access"
    resource: "/admin/dashboard"
    expect: allow

  - name: "Viewer cannot delete"
    policy: "policies/rbac.reap"
    data: "data/entities.json"
    principal: "viewer_bob"
    action: "delete"
    resource: "/api/data"
    expect: deny
```

## Core Dependencies

- **tokio**: Async runtime (full features)
- **axum**: Web framework (v0.8.4+)
- **dashmap**: Lock-free HashMap (v6.1.0+)
- **parking_lot**: High-performance mutex (v0.12+)
- **serde/serde_json**: Serialization
- **uuid**: Policy ID generation
- **tracing**: Structured logging
- **cedar-policy**: AWS Cedar support (v4.2)
- **cucumber**: BDD testing (v0.21.1)

## Documentation Files

- **README.md** - Quick start and overview (root)
- **CHANGELOG.md** - Version history (root)
- **docs/** - All documentation organized by topic:
  - **architecture/** - Core architectural documentation
    - ARCHITECTURE.md - Detailed architecture (primary technical reference)
    - ARCHITECTURE_SUMMARY.md - Executive summary (start here!)
    - REAPER_CLIENT_SEPARATION.md - Client/server separation design
    - FILE_REFERENCE.md - Complete file reference
  - **concepts/** - Core concepts and formats
    - BUNDLE_FORMAT.md - .rbb binary bundle specification
    - EVENT_DRIVEN_LOADING.md - SSE-based policy sync
  - **deployment/** - Deployment patterns and guides
    - OPERATIONS_GUIDE.md - Production operations (health, metrics, troubleshooting)
    - DEPLOYMENT_PATTERNS.md - Standalone, Integrated, Embedded patterns
    - SIDECAR_DEPLOYMENT.md - Sidecar deployment guide
  - **performance/** - Performance optimization documentation
    - PERFORMANCE_ANALYSIS.md - Benchmarks and analysis
    - OPTIMIZATION_ANALYSIS.md - Optimization strategies
    - HTTP_OPTIMIZATION_GUIDE.md - HTTP layer optimization
  - **testing/** - Testing frameworks and strategies
    - GHERKIN_INTEGRATION.md - Cucumber/Gherkin BDD integration
    - POLICY_TESTS.md - Policy testing guide
  - **development/** - Implementation plans and guides
    - IMPLEMENTATION_PLAN.md - Roadmap for sync client and server
    - YAML_FORMAT.md - YAML policy format specification
  - **HISTORY.md** - Historical milestones and lessons learned

## Development Notes

- The workspace uses Rust 2021 edition
- All crates share dependencies via workspace.dependencies in root Cargo.toml
- BDD tests use `harness = false` in Cargo.toml [[test]] sections
- Agent observability uses Prometheus + OpenTelemetry directly
- eBPF integration is experimental (Linux only)
- Decision logging provides OPA-style audit trails
- Docker profiles enable flexible deployment patterns

## Architecture Evolution Plan

### 4-Layer Architecture

**Layer 1: Engine** (`crates/policy-engine/`) - Sub-microsecond evaluation core
- Target: < 500ns p99 latency for compiled policies
- Optimizations:
  - Arena allocators (bumpalo) for zero-allocation evaluation loops
  - Zero-copy request parsing with simd-json
  - Thread-local caches for regex and string interning
  - Pre-compiled policy bundles (.rpp) with hints
  - SIMD string matching for pattern operations
- Key files: `engine.rs`, `evaluators/reaper_dsl.rs`, `reap/compiler.rs`

**Layer 2: Agent** (`services/reaper-agent/`) - High-throughput enforcement
- Target: > 200K req/s per agent instance
- Optimizations:
  - Thread-per-core runtime (consider glommio/monoio for io_uring)
  - Connection pooling with keep-alive
  - Request batching for bulk evaluations
  - Zero-copy response serialization
- Key files: `main.rs`, bundle deployment endpoints

**Layer 3: Platform** (`services/reaper-platform/`) - Multi-agent orchestration
- Responsibilities:
  - Policy CRUD and versioning
  - Agent registration and health monitoring
  - Coordinated policy deployments
  - Metrics aggregation across fleet
- Future: Ambient mesh pattern (sidecar-less)
- Key files: `main.rs`, deployment coordination

**Layer 4: CLI** (`tools/reaper-cli/`) - Developer experience
- Commands:
  - `eval` - Local policy evaluation
  - `test` - Single policy assertion (CI/CD integration)
  - `test-suite` - Batch test execution from YAML
  - `bundle` - Create/deploy policy packages
  - `validate` - Policy validation
  - `policy` - CRUD via Platform API
  - `status` / `benchmark` - System diagnostics
- Key files: `main.rs`

### Bundle System (.rpp)

Multi-policy packages with pre-compilation hints:
```rust
PolicyPackage {
    policies: Vec<Policy>,          // Multiple policy ASTs
    hints: PrecompilationHints {
        strings_to_intern: HashSet,  // Pre-intern at load time
        regex_patterns: HashSet,      // Pre-compile regex cache
    },
    metadata: PackageMetadata,
}
```

### Performance Targets

| Component | Metric | Target |
|-----------|--------|--------|
| Engine (compiled) | p99 latency | < 500ns |
| Engine (AST) | p99 latency | < 5µs |
| Agent | Throughput | > 200K req/s |
| Agent | Memory | < 50MB |
| Bundle load | Cold start | < 10ms |
| Hot-swap | Downtime | 0ms |

### Optimization Techniques Reference

1. **Arena Allocators**: Use bumpalo for request-scoped allocations
2. **Zero-Copy Parsing**: simd-json for request deserialization
3. **String Interning**: Pre-intern strings at bundle creation, not evaluation
4. **Regex Caching**: Thread-local RegexSet with pre-compiled patterns
5. **SIMD Matching**: memchr/stringzilla for pattern matching
6. **Lock-Free Structures**: DashMap for concurrent policy store
7. **Atomic Hot-Swap**: Arc-based policy replacement

## Common Workflows

### Adding a New Policy Language
1. Create new evaluator in `crates/policy-engine/src/evaluators/`
2. Implement `PolicyEvaluator` trait
3. Add variant to `PolicyLanguage` enum
4. Update `EnhancedPolicy::build_evaluator()`
5. Write BDD tests in `tests/features/`
6. Add benchmarks

### Debugging Policy Evaluation
1. Check Agent logs for evaluation metrics
2. Use `GET /metrics` endpoint for performance data
3. Run benchmarks: `cargo bench -p policy-engine`
4. Profile with examples: `cargo run --example test_rbac_10k --release`

### Testing Policy Deployment
1. Start Platform: `make platform`
2. Start Agent: `make agent`
3. Create policy via Platform API (port 8081)
4. Deploy to agent: `POST /api/v1/policies/:id/deploy`
5. Verify with agent: `GET /api/v1/policies` (port 8080)
6. Evaluate: `POST /api/v1/messages` (port 8080)
