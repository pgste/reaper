# Reaper Policy Engine Documentation

Welcome to the Reaper Policy Engine documentation. Reaper is a high-performance, production-ready authorization engine built in Rust with sub-microsecond latency guarantees.

## Quick Links

- **[Introduction](./introduction/overview.md)** - Learn what Reaper is and why you should use it
- **[Getting Started](./getting-started/installation.md)** - Install and run your first policy
- **[Guides](./guides/policy-languages.md)** - Step-by-step guides for common tasks
- **[Concepts](./concepts/architecture.md)** - Deep dive into Reaper's architecture
- **[API Reference](./reference/api/platform-api.md)** - Complete API documentation
- **[Performance](./performance/benchmarks.md)** - Benchmark results and optimization guides

---

## What is Reaper?

Reaper is a distributed policy enforcement platform that provides:

- **Sub-microsecond latency** - < 1µs P99 for simple policies, 10-50µs for ABAC
- **High throughput** - > 2M decisions/second per core
- **Zero-downtime updates** - Atomic policy hot-swapping
- **60-80% memory reduction** - Compared to JVM-based engines
- **Multiple policy languages** - Simple rules, AWS Cedar, native Reaper DSL
- **Production-ready** - Battle-tested with comprehensive scale tests

---

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│           Reaper Platform (Port 8081)           │
│  Policy Management • Versioning • Deployment    │
└────────────────┬────────────────────────────────┘
                 │
                 │ Deploy Policies
                 ▼
┌─────────────────────────────────────────────────┐
│            Reaper Agent (Port 8080)             │
│  Sub-µs Evaluation • Hot-Swapping • Metrics     │
└─────────────────────────────────────────────────┘
```

**Platform** handles policy lifecycle and deployment coordination.
**Agent** handles high-performance policy evaluation.

---

## Key Features

### Performance

- **Sub-microsecond latency**: < 1µs P99 for RBAC/Simple policies
- **Millions of ops/sec**: > 2M decisions/second on single core
- **Linear scaling**: O(n) complexity for comprehensions
- **Memory efficient**: 60-80% less memory than JVM engines

### Reliability

- **Zero-downtime updates**: Atomic policy hot-swapping
- **Lock-free concurrency**: DashMap for concurrent reads
- **Crash-safe**: Policies validated before deployment
- **Comprehensive testing**: Unit, integration, BDD, and scale tests

### Developer Experience

- **Multiple formats**: REAP DSL, YAML, JSON
- **Familiar syntax**: Cedar-like policy language
- **CLI tooling**: Full CLI for policy management
- **Rich testing**: BDD framework with Cucumber/Gherkin

---

## Quick Start

```bash
# Install Reaper
cargo install reaper-cli

# Run the platform
cargo run --bin reaper-platform

# Run the agent
cargo run --bin reaper-agent

# Write your first policy
cat > my-policy.reap << 'EOF'
policy rbac {
  permit principal.role == "admin"
}
EOF

# Evaluate a request
reaper-cli eval --policy my-policy.reap \
  --principal "user:alice" \
  --action "read" \
  --resource "document:123"
```

See [Getting Started](./getting-started/installation.md) for detailed instructions.

---

## Documentation Sections

### 📚 Introduction
Learn about Reaper's design philosophy, use cases, and how it compares to other authorization engines.

- [Overview](./introduction/overview.md)
- [Why Reaper?](./introduction/why-reaper.md)
- [Key Features](./introduction/key-features.md)

### 🚀 Getting Started
Get up and running with Reaper in minutes.

- [Installation](./getting-started/installation.md)
- [Quick Start](./getting-started/quick-start.md)
- [First Policy](./getting-started/first-policy.md)
- [Examples](./getting-started/examples.md)

### 📖 Guides
Step-by-step guides for common tasks.

- [Policy Languages](./guides/policy-languages.md)
- [Deployment](./guides/deployment.md)
- [Testing Policies](./guides/testing.md)
- [Performance Tuning](./guides/performance-tuning.md)

### 🧠 Concepts
Deep dive into Reaper's architecture and design.

- [Architecture](./concepts/architecture.md)
- [Policy Engine](./concepts/policy-engine.md)
- [Data Store](./concepts/data-store.md)
- [Evaluators](./concepts/evaluators.md)
- [Policy Formats](./concepts/policy-formats.md)

### 📘 Reference
Complete API and syntax reference.

- [Platform API](./reference/api/platform-api.md)
- [Agent API](./reference/api/agent-api.md)
- [CLI Reference](./reference/cli.md)
- [Policy Syntax](./reference/policy-syntax.md)

### ⚡ Performance
Benchmarks, optimization, and scale testing.

- [Benchmarks](./performance/benchmarks.md)
- [Scale Tests](./performance/scale-tests.md)
- [Optimization Guide](./performance/optimization.md)

### 🏗️ Deployment
Deploy Reaper in different patterns.

- [Deployment Patterns](./deployment/deployment-patterns.md)
- [Sidecar Mode](./deployment/sidecar.md)
- [Standalone Mode](./deployment/standalone.md)

### 🔧 Advanced
Advanced topics for power users.

- [Custom Evaluators](./advanced/custom-evaluators.md)
- [Policy Hot-Swapping](./advanced/hot-swapping.md)
- [Rego Comparison](./advanced/rego-comparison.md)

---

## Community & Support

- **GitHub**: [github.com/your-org/reaper](https://github.com/your-org/reaper)
- **Issues**: Report bugs and request features
- **Discussions**: Ask questions and share ideas

---

## License

Reaper is licensed under the MIT License. See [LICENSE](../LICENSE) for details.
