# Enforcement.md - AI Assistant Guide for Reaper

## Project Overview

**Reaper** is a high-performance policy enforcement platform built in Rust for enterprise sidecars. It delivers:
- Sub-microsecond policy evaluation latency (< 1μs p99)
- 60-80% memory reduction vs JVM-based policy engines
- Zero-downtime policy updates via atomic hot-swapping
- Distributed agent management with centralized policy control

## Repository Structure

```
/home/user/reaper/
├── Cargo.toml                 # Workspace configuration
├── Makefile                   # Development commands
├── README.md                  # Project documentation
├── CHANGELOG.md               # Release history
├── crates/                    # Core libraries
│   ├── reaper-core/           # Core types, traits, error handling
│   ├── policy-engine/         # Policy evaluation engine (hot-swap capable)
│   ├── message-queue/         # Message infrastructure (skeleton)
│   └── metrics/               # Performance monitoring (skeleton)
├── services/                  # Deployable services
│   ├── reaper-agent/          # Policy enforcement service (port 8080)
│   └── reaper-platform/       # Policy management service (port 8081)
├── tools/
│   └── reaper-cli/            # Command-line interface
└── scripts/                   # Automation scripts
    ├── dev-setup.sh           # Install dev dependencies
    ├── release.sh             # Release automation
    └── verify-setup.sh        # Verify setup
```

## Quick Commands

```bash
# Build and test
make build          # Build debug and release
make test           # Run all tests (unit + BDD + integration)
make bench          # Run benchmarks
make check          # Format, lint (clippy), and test

# Development
make dev            # Auto-reload development mode (cargo watch)
make agent          # Run reaper-agent on port 8080
make platform       # Run reaper-platform on port 8081
make dev-services   # Run both services in parallel

# Quality
make coverage       # Generate HTML coverage report
make bdd            # Run BDD tests only

# Setup
make setup          # Install required dev tools
```

## Architecture

### Core Libraries (crates/)

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `reaper-core` | Core types, traits, error definitions | `src/lib.rs`, `src/error.rs`, `src/policy.rs`, `src/agent.rs` |
| `policy-engine` | High-performance policy evaluation with hot-swap | `src/engine.rs` (370 lines) |
| `message-queue` | Async message processing (placeholder) | `src/lib.rs` |
| `metrics` | Performance/compliance monitoring (placeholder) | `src/lib.rs` |

### Services (services/)

| Service | Port | Purpose |
|---------|------|---------|
| `reaper-agent` | 8080 | Policy enforcement, evaluates requests against policies |
| `reaper-platform` | 8081 | Centralized policy management, CRUD operations, deployment |

### Key API Endpoints

**Reaper Agent (8080)**:
- `POST /api/v1/messages` - Evaluate message against policies
- `POST /api/v1/policies/deploy` - Receive policy deployment
- `GET /api/v1/policies` - List deployed policies
- `GET /health` - Health check
- `GET /metrics` - Performance metrics

**Reaper Platform (8081)**:
- `GET/POST /api/v1/policies` - List/create policies
- `GET/PUT/DELETE /api/v1/policies/:id` - Individual policy CRUD
- `POST /api/v1/policies/:id/deploy` - Deploy policy to agents
- `GET /health`, `GET /metrics` - Health and metrics

## Code Conventions

### Error Handling
- Use `thiserror` for error type derivation
- Return `Result<T>` types (aliased in each crate)
- Error types defined in `reaper-core/src/error.rs`

### Async/Concurrency
- Runtime: Tokio with full features
- Lock-free maps: `DashMap` for concurrent policy storage
- Efficient locks: `parking_lot::RwLock` for default policy
- Zero-copy sharing: `Arc<Policy>` across threads

### Logging/Tracing
- Framework: `tracing` crate
- Use `#[instrument]` attribute on handler functions
- Log macros: `info!`, `warn!`, `error!`, `debug!`

### Code Style
- Edition: Rust 2021
- Run `cargo fmt` before commits
- Run `cargo clippy` for lints
- License: MIT OR Apache-2.0

## Testing Strategy

### Test Types

1. **Unit Tests**: Embedded in source files with `#[cfg(test)]`
   - Example: `crates/policy-engine/src/engine.rs`

2. **BDD Tests**: Cucumber-based in `tests/*_bdd_tests.rs`
   - Feature files in `tests/features/*.feature`
   - Uses Given/When/Then Gherkin syntax

3. **Benchmarks**: Criterion-based in `benches/*.rs`
   - Performance targets: < 1μs p99 evaluation latency

### Running Tests
```bash
cargo test                    # All tests
cargo test -p policy-engine   # Single crate
make bdd                      # BDD tests only
make bench                    # Benchmarks
```

### Test File Locations
- `crates/*/tests/*_bdd_tests.rs` - BDD test implementations
- `crates/*/tests/features/*.feature` - Gherkin feature files
- `crates/*/benches/*.rs` - Benchmark files

## Key Dependencies

| Category | Crate | Purpose |
|----------|-------|---------|
| Async | `tokio` | Async runtime |
| HTTP | `axum` | Web framework |
| Serialization | `serde`, `serde_json` | JSON handling |
| Concurrency | `dashmap`, `parking_lot` | Lock-free/efficient locks |
| Errors | `thiserror`, `anyhow` | Error handling |
| CLI | `clap` | Argument parsing |
| Testing | `cucumber`, `criterion` | BDD, benchmarks |
| Tracing | `tracing`, `tracing-subscriber` | Observability |

## Development Workflow

### Making Changes

1. **Start dev mode**: `make dev` (auto-reload on changes)
2. **Run services**: `make dev-services` (agent + platform)
3. **Test changes**: `make check` (format + lint + test)

### Adding Features

1. Create/modify types in `reaper-core` if needed
2. Implement logic in appropriate crate (`policy-engine`, etc.)
3. Expose via service endpoints in `reaper-agent` or `reaper-platform`
4. Add CLI commands in `reaper-cli` if user-facing
5. Write BDD tests in feature files

### Performance Considerations

- Policy evaluation must stay under 1μs p99
- Use `DashMap` for hot-path policy lookups
- Avoid allocations in evaluation path
- Use `Arc` for zero-copy policy sharing
- Run `make bench` to verify performance

## Important Patterns

### Policy Engine Hot-Swap
```rust
// Atomic policy update without downtime
engine.hot_swap_policy(policy_id, new_policy)?;
```

### Policy Evaluation
```rust
// Fast-path evaluation
let action = engine.evaluate(&policy_id, &context)?;
match action {
    PolicyAction::Allow => { /* proceed */ },
    PolicyAction::Deny => { /* reject */ },
    PolicyAction::Log => { /* log and proceed */ },
}
```

### Service State Management
- `AgentState` in reaper-agent wraps `PolicyEngine` + `AgentStats`
- `PlatformState` in reaper-platform wraps `PolicyEngine` + deployment stats
- Both use `Arc<State>` for sharing across handlers

## File Navigation Tips

| To find... | Look in... |
|------------|------------|
| Error types | `crates/reaper-core/src/error.rs` |
| Policy structs | `crates/reaper-core/src/policy.rs` |
| Policy evaluation logic | `crates/policy-engine/src/engine.rs` |
| Agent HTTP handlers | `services/reaper-agent/src/main.rs` |
| Platform HTTP handlers | `services/reaper-platform/src/main.rs` |
| CLI commands | `tools/reaper-cli/src/main.rs` |
| API endpoint constants | `crates/reaper-core/src/lib.rs` |

## Common Tasks

### Add a new policy type
1. Define in `crates/reaper-core/src/policy.rs`
2. Implement evaluation in `crates/policy-engine/src/engine.rs`
3. Add tests in `crates/policy-engine/tests/`

### Add a new API endpoint
1. Define route constant in `crates/reaper-core/src/lib.rs`
2. Implement handler in service (`reaper-agent` or `reaper-platform`)
3. Add CLI command if needed in `tools/reaper-cli/src/main.rs`

### Add a new CLI command
1. Add command variant to CLI enum in `tools/reaper-cli/src/main.rs`
2. Implement handler function
3. Wire up in main match statement

## DevContainer Support

The project includes VS Code devcontainer configuration (`.devcontainer/devcontainer.json`):
- Image: `mcr.microsoft.com/devcontainers/rust:latest`
- Extensions: rust-analyzer
- Min CPUs: 2

## Release Process

```bash
make release              # Patch release (default)
make release VERSION=minor  # Minor release
make release VERSION=major  # Major release
```

Release script runs: tests, format check, clippy, then cargo-release.

---

## Related Documentation

- **[bundles.md](bundles.md)** - Bundle architecture, compilation, atomic deployment
