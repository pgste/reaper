# Reaper - High-Performance Policy Enforcement Platform

![CI Status](https://github.com/your-org/reaper/workflows/Reaper%20CI/badge.svg)

**Reaper Agent** provides sub-microsecond policy enforcement for enterprise sidecars, while **Reaper Platform** manages distributed agents with zero-downtime deployments.

## 🎯 Core Value Proposition

- **60-80% Memory Reduction** vs traditional JVM-based policy engines
- **Sub-microsecond Decision Latency** for cost-effective sidecar deployment
- **Zero-downtime Policy Updates** using atomic swapping
- **Enterprise-grade Reliability** with comprehensive BDD testing

## 🚀 Quick Start

### Prerequisites
- Rust 1.70+ (install via [rustup](https://rustup.rs/))
- Make (for convenience commands)

### Initial Setup

```bash
# Clone the repository
git clone <repository-url>
cd reaper

# Run one-time setup (installs cargo-watch, etc.)
make setup

# Build all workspace members
make build
# or directly: cargo build --workspace
```

### Running Services Locally

**Option 1: Run both services together**
```bash
make dev-services
# Agent available at: http://localhost:8080
# Platform available at: http://localhost:8081
```

**Option 2: Run services separately (in different terminals)**
```bash
# Terminal 1 - Run Reaper Platform (management layer)
make platform
# or: cargo run --bin reaper-platform

# Terminal 2 - Run Reaper Agent (enforcement layer)
make agent
# or: cargo run --bin reaper-agent
```

**Option 3: Development mode with auto-reload**
```bash
make dev
# Watches for file changes and runs checks + tests automatically
```

### Verify Services are Running

```bash
# Check Platform health
curl http://localhost:8081/health

# Check Agent health
curl http://localhost:8080/health

# View Platform metrics
curl http://localhost:8081/metrics

# View Agent metrics
curl http://localhost:8080/metrics
```

### Build CLI Tool

```bash
make cli
# or: cargo build --bin reaper-cli

# Try it out
./target/debug/reaper-cli status
```

## 🏗️ Architecture

### Core Components

- **Policy Engine** (Library) - Standalone Rust crate, embeddable anywhere
- **Reaper Agent** - Independent policy enforcement service
- **Reaper Sync Client** (Future) - Optional integration with management server
- **Management Server** (Future) - Centralized policy management and orchestration

### Supporting Components

- **Reaper CLI** - Command-line interface for policy management
- **Reaper SDK** - Client library for application integration
- **Reaper eBPF** - Kernel-level policy enforcement (experimental)

### Architecture Documentation

- **[Architecture Overview](docs/architecture/ARCHITECTURE_SUMMARY.md)** - Executive summary and quick start
- **[Detailed Architecture](docs/architecture/ARCHITECTURE.md)** - In-depth technical reference
- **[Client Separation Design](docs/architecture/REAPER_CLIENT_SEPARATION.md)** - Separation between engine and sync client
- **[Deployment Patterns](docs/deployment/DEPLOYMENT_PATTERNS.md)** - Standalone, Integrated, and Embedded patterns
- **[Implementation Plan](docs/development/IMPLEMENTATION_PLAN.md)** - Roadmap for sync client and server

📚 **[View All Documentation](docs/)** - Comprehensive docs organized by topic

### Key Architectural Principles

1. **Reaper is fully independent** - Works standalone without external services
2. **Web Client is optional** - Enables centralized management when needed
3. **Multiple deployment patterns** - Choose the pattern that fits your needs
4. **Zero-downtime updates** - Atomic hot-swapping for all deployment modes

## 📊 API Endpoints & Examples

### Reaper Agent (Port 8080)

**Health & Metrics**
```bash
# Health check
curl http://localhost:8080/health

# Performance metrics
curl http://localhost:8080/metrics

# List active policies
curl http://localhost:8080/api/v1/policies
```

**Policy Evaluation**
```bash
# Evaluate a policy request
curl -X POST http://localhost:8080/api/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "my-policy",
    "resource": "documents/123",
    "action": "read",
    "context": {
      "user_role": "admin",
      "department": "engineering"
    }
  }'

# Response format:
# {
#   "decision": "allow",
#   "policy_id": "550e8400-e29b-41d4-a716-446655440000",
#   "policy_version": 1,
#   "evaluation_time_microseconds": 0.5,
#   "total_time_microseconds": 1.2,
#   "agent_id": "reaper-agent-001"
# }
```

### Reaper Platform (Port 8081)

**Health & Metrics**
```bash
# Health check
curl http://localhost:8081/health

# Platform metrics
curl http://localhost:8081/metrics
```

**Policy Management**
```bash
# List all policies
curl http://localhost:8081/api/v1/policies

# Create a new policy
curl -X POST http://localhost:8081/api/v1/policies \
  -H "Content-Type: application/json" \
  -d '{
    "name": "read-access",
    "description": "Allow read access to documents",
    "language": "Simple",
    "rules": [
      {
        "action": "allow",
        "resource": "documents/*",
        "conditions": []
      }
    ]
  }'

# Get policy by ID (replace {policy-id} with actual UUID)
curl http://localhost:8081/api/v1/policies/{policy-id}

# Update a policy (replace {policy-id} with actual UUID)
curl -X PUT http://localhost:8081/api/v1/policies/{policy-id} \
  -H "Content-Type: application/json" \
  -d '{
    "name": "updated-policy",
    "description": "Updated description"
  }'

# Delete a policy (replace {policy-id} with actual UUID)
curl -X DELETE http://localhost:8081/api/v1/policies/{policy-id}

# Deploy policy to agent (replace {policy-id} with actual UUID)
curl -X POST http://localhost:8081/api/v1/policies/{policy-id}/deploy
```

**Agent Management**
```bash
# List all agents
curl http://localhost:8081/api/v1/agents

# Get agent details (replace {agent-id} with actual UUID)
curl http://localhost:8081/api/v1/agents/{agent-id}
```

## 🧪 Testing

### Running All Tests
```bash
# Run all tests (unit + integration + BDD)
make test

# Or manually:
cargo test --workspace
```

### Unit Tests
```bash
# Run all unit tests
cargo test --workspace --lib

# Run tests for a specific crate
cargo test -p policy-engine --lib
cargo test -p reaper-core --lib
cargo test -p reaper-agent --lib

# Run a specific test by name
cargo test --lib test_policy_evaluation

# Run tests with output
cargo test --workspace --lib -- --nocapture
```

### BDD Scenarios (Cucumber/Gherkin)
```bash
# Run all BDD tests
make bdd

# Or manually:
cargo test --workspace --test '*bdd*'

# Run specific BDD test files
cargo test --test policy_bdd_tests
cargo test --test gherkin_tests
cargo test --test agent_bdd_tests
cargo test --test platform_bdd_tests

# Run with scenario filtering
cargo test --test policy_bdd_tests -- --name "simple policy"
```

### Integration Tests
```bash
# Run integration tests (if available)
cargo test --workspace --test '*integration*'
```

### Performance Benchmarks
```bash
# Run all benchmarks
make bench

# Run benchmarks with summary report (recommended!)
make bench-summary

# Or manually:
cargo bench --workspace

# Run specific benchmark
cargo bench -p policy-engine

# Generate HTML reports (saved to target/criterion/)
cargo bench --workspace -- --save-baseline main
```

**Note**: When running `cargo bench`, unit tests appear as "ignored" - this is normal! Benchmark mode only runs benchmark functions, not `#[test]` functions.

### Test Coverage
```bash
# Generate test coverage report (requires cargo-tarpaulin)
make coverage

# Install tarpaulin if needed
cargo install cargo-tarpaulin

# View HTML report
open coverage/index.html
```

### Code Quality Checks
```bash
# Run all quality checks (format, clippy, tests)
make check

# Individual checks:
cargo fmt --check           # Check formatting
cargo fmt                   # Auto-format code
cargo clippy --workspace -- -D warnings  # Lint checks
```

### Performance Test Examples
```bash
# Run volume/stress tests
cargo run --release --example test_rbac_10k
cargo run --release --example test_abac_10k
cargo run --release --example test_multilayer_10k
cargo run --release --example memory_volume_test
```

## 🚢 Release Process

```bash
# Patch release
make release

# Minor release
make release VERSION=minor

# Major release
make release VERSION=major
```

## 🔄 CI/CD Pipeline

The project includes a comprehensive GitHub Actions pipeline that runs on every push and pull request:

### Pipeline Stages

**Stage 1: Lint & Analyze** (Sequential)
- Code formatting check (`cargo fmt`)
- Clippy linting with warnings as errors
- Generates Clippy report artifact

**Stage 2: Unit Tests** (Sequential - Blocks pipeline if fails)
- Runs all workspace unit tests
- Generates test summary and detailed results
- **Build fails if unit tests fail**

**Stage 3: Concurrent Performance & BDD Tests** (Parallel - Don't fail build)

The following run in parallel after unit tests pass:

**Volume Tests** (Matrix Strategy)
- `multilayer` - Combined RBAC + ABAC + ReBAC (10k iterations)
- `rbac` - Role-Based Access Control (10k iterations)
- `abac` - Attribute-Based Access Control (10k iterations)
- `rebac` - Relationship-Based Access Control (10k iterations)

Each generates:
- Full test output with latency statistics
- Performance metrics (mean, P95, P99)
- Decision distribution
- Throughput analysis

**Memory & Scale Test**
- 100k entity dataset
- Comparison of 1k vs 100k performance
- Memory efficiency analysis
- Memory leak detection

**BDD Tests (Cucumber)**
- Runs all Gherkin/Cucumber feature tests
- Generates Cucumber JSON report
- Scenario execution summary

### Artifacts Generated

All test runs produce artifacts available for download:

| Artifact | Contents |
|----------|----------|
| `clippy-report` | Static analysis results |
| `unit-test-results` | Unit test output & summary |
| `volume-test-{policy}` | Volume test results for each policy type |
| `memory-volume-test` | 100k entity memory & performance analysis |
| `bdd-test-results` | BDD test output & summaries |
| `cucumber-report` | Cucumber JSON report |
| `combined-test-report` | Comprehensive markdown report of all tests |

### Viewing Results

**On Pull Requests:**
- CI bot automatically comments with combined test report
- All artifacts available in the Actions tab

**On Main/Develop:**
- All artifacts retained for 90 days
- Combined report shows performance trends

### Local CI Simulation

Run the same checks locally:

```bash
# Lint & analyze
cargo fmt --check
cargo clippy --workspace -- -D warnings

# Unit tests
cargo test --workspace --lib

# Volume tests (run all in parallel)
cargo run -p policy-engine --example generate_multilayer_data --release
cargo run -p policy-engine --example test_multilayer_10k --release

# BDD tests
cargo test --workspace --test '*bdd*'
```

## 🔧 Development

### Project Structure

```
reaper/
├── crates/                          # Core libraries
│   ├── reaper-core/                # Core types and traits
│   ├── policy-engine/              # Policy evaluation engine
│   │   ├── src/
│   │   │   ├── engine.rs          # PolicyEngine (lock-free store)
│   │   │   ├── evaluators/        # Policy language evaluators
│   │   │   ├── data/              # DataStore for ABAC/ReBAC
│   │   │   ├── reap/              # Policy format support
│   │   │   └── gherkin/           # Cucumber integration
│   │   ├── tests/                 # BDD tests
│   │   ├── examples/              # Performance tests
│   │   └── benches/               # Benchmarks
│   ├── message-queue/             # Async messaging (stub)
│   └── metrics/                   # Monitoring (stub)
├── services/                       # Standalone services
│   ├── reaper-agent/              # Policy enforcement (port 8080)
│   │   ├── src/main.rs           # Agent service
│   │   └── tests/                # Agent BDD tests
│   └── reaper-platform/           # Management layer (port 8081)
│       ├── src/main.rs           # Platform service
│       └── tests/                # Platform BDD tests
├── tools/
│   └── reaper-cli/                # Command-line interface
├── docs/                          # All documentation
│   ├── architecture/             # Architecture docs
│   ├── deployment/               # Deployment guides
│   ├── performance/              # Performance docs
│   ├── testing/                  # Testing guides
│   └── development/              # Dev guides
├── Makefile                       # Development commands
└── Cargo.toml                     # Workspace configuration
```

### Development Workflow

**Day-to-day Development**
```bash
# Start auto-reload development mode
make dev
# This watches for changes and runs checks + tests

# Or work on specific components:
cd crates/policy-engine
cargo watch -x check -x test
```

**Building**
```bash
# Build all workspace members (debug)
cargo build --workspace

# Build specific crate
cargo build -p policy-engine

# Build release version (optimized)
cargo build --workspace --release

# Quick check without building artifacts
cargo check --workspace
```

**Working with Services**
```bash
# Run individual services
make agent                    # Run Agent on 8080
make platform                 # Run Platform on 8081
make dev-services            # Run both services

# Run with logging
RUST_LOG=debug cargo run --bin reaper-agent
RUST_LOG=info cargo run --bin reaper-platform
```

**CLI Development**
```bash
# Build and test CLI
make cli
./target/debug/reaper-cli --help

# Run CLI commands directly during development
cargo run --bin reaper-cli -- status
cargo run --bin reaper-cli -- policy list
```

**Cleaning Build Artifacts**
```bash
make clean
# or: cargo clean
```

### Common Development Tasks

**Adding a New Policy Language Evaluator**
1. Create evaluator in `crates/policy-engine/src/evaluators/`
2. Implement `PolicyEvaluator` trait
3. Add to `PolicyLanguage` enum
4. Update `EnhancedPolicy::build_evaluator()`
5. Write BDD tests in `tests/features/`
6. Add benchmarks

**Debugging Policy Evaluation**
```bash
# Enable detailed logging
RUST_LOG=policy_engine=trace cargo run --bin reaper-agent

# Check metrics
curl http://localhost:8080/metrics

# Run benchmarks to identify bottlenecks
cargo bench -p policy-engine
```

**Testing Policy Deployment**
```bash
# Terminal 1: Start services
make dev-services

# Terminal 2: Create and test policy
curl -X POST http://localhost:8081/api/v1/policies \
  -H "Content-Type: application/json" \
  -d '{"name":"test","description":"Test policy","rules":[]}'

# List policies
curl http://localhost:8081/api/v1/policies
```

### Quick Reference Commands

| Command | Description |
|---------|-------------|
| `make setup` | One-time dev environment setup |
| `make dev` | Auto-reload development mode |
| `make build` | Build all workspace members |
| `make test` | Run all tests |
| `make bdd` | Run BDD scenarios |
| `make bench` | Run performance benchmarks |
| `make check` | Format, lint, and test |
| `make coverage` | Generate test coverage |
| `make agent` | Run Reaper Agent |
| `make platform` | Run Reaper Platform |
| `make cli` | Build CLI tool |
| `make clean` | Remove build artifacts |
| `make release` | Create a release |

## 📈 Performance Goals

- **Policy Evaluation**: < 1 microsecond p99 latency
- **Memory Usage**: < 50MB per agent instance
- **Throughput**: > 100K decisions/second per agent
- **Startup Time**: < 100ms cold start

## 🎖️ Enterprise Features

- Zero-downtime policy deployments
- Centralized agent management
- Real-time compliance monitoring
- Automated rollback capabilities
- Audit logging and reporting

---

Built with ❤️ using Rust for maximum performance and reliability.