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
cargo test --test policy_bdd_tests
cargo test --test gherkin_tests

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
cargo test --test policy_bdd_tests -- --name "scenario_name"
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
│   ├── reaper-core/        # Core types, traits (95 lines)
│   ├── policy-engine/      # Policy evaluation engine (1200+ lines)
│   │   ├── src/
│   │   │   ├── engine.rs   # PolicyEngine - lock-free store (500 lines)
│   │   │   ├── evaluators/ # SimplePolicyEvaluator, CedarPolicyEvaluator, ReaperDSLEvaluator
│   │   │   ├── data/       # DataStore - multi-index entity storage
│   │   │   ├── reap/       # ReaperPolicy - format support (.reap/.yaml/.json)
│   │   │   └── gherkin/    # Cucumber/Gherkin integration
│   ├── message-queue/      # Async messaging (stub)
│   └── metrics/            # Performance monitoring (stub)
├── services/
│   ├── reaper-agent/       # Agent service (399 lines)
│   └── reaper-platform/    # Platform service (620 lines)
└── tools/
    └── reaper-cli/         # CLI management tool (150+ lines)
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
GET    /metrics                     # Agent performance metrics
POST   /api/v1/messages            # Evaluate policy request
POST   /api/v1/policies/deploy     # Deploy policy (from platform)
GET    /api/v1/policies            # List active policies
```

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
- `crates/policy-engine/tests/policy_bdd_tests.rs` - Policy engine BDD
- `crates/policy-engine/tests/gherkin_tests.rs` - Full Gherkin/Cucumber integration
- `crates/message-queue/tests/message_queue_bdd_tests.rs` - Async messaging tests
- `services/reaper-agent/tests/agent_bdd_tests.rs` - Agent BDD tests
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

# Policy compilation to binary bundle
./target/debug/reaper-cli compile <input-files> --output bundle.rbb

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
  - **deployment/** - Deployment patterns and guides
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
    - EXPLORATION_REPORT.md - Codebase exploration

## Development Notes

- The workspace uses Rust 2021 edition
- All crates share dependencies via workspace.dependencies in root Cargo.toml
- BDD tests use `harness = false` in Cargo.toml [[test]] sections
- message-queue and metrics crates are currently stubs for future functionality
- Agent management in Platform is a placeholder for full implementation
- Policy versioning is basic - expansion planned

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
