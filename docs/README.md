# Reaper Documentation

Welcome to the Reaper Policy Engine documentation!

## 📚 Documentation Structure

This documentation is organized for progressive learning, from introduction to advanced topics:

### 🎯 [Introduction](./introduction/overview.md)
Learn what Reaper is, why you should use it, and its key features.

- **[Overview](./introduction/overview.md)** - What is Reaper?
- **[Why Reaper?](./introduction/why-reaper.md)** - Why choose Reaper over OPA/Cedar
- **[Key Features](./introduction/key-features.md)** - Core features and benefits

### 🚀 [Getting Started](./getting-started/installation.md)
Get up and running in minutes.

- **[Installation](./getting-started/installation.md)** - Install Reaper
- **[Quick Start](./getting-started/quick-start.md)** - 5-minute tutorial
- **[First Policy](./getting-started/first-policy.md)** - Write your first policy
- **[Examples](./getting-started/examples.md)** - Example policies

### 📖 [Guides](./guides/policy-languages.md)
Step-by-step guides for common tasks.

- **[Policy Languages](./guides/policy-languages.md)** - REAP, Cedar, YAML, JSON
- **[Deployment](./guides/deployment.md)** - Deploy Reaper to production
- **[Testing](./guides/testing.md)** - Test your policies
- **[Performance Tuning](./guides/performance-tuning.md)** - Optimize performance

### 🧠 [Concepts](./concepts/architecture.md)
Deep dive into Reaper's architecture and design.

- **[Architecture](./concepts/architecture.md)** - System architecture
- **[Policy Engine](./concepts/policy-engine.md)** - Policy engine internals
- **[Data Store](./concepts/data-store.md)** - Entity data management
- **[Evaluators](./concepts/evaluators.md)** - Policy evaluator architecture
- **[Policy Formats](./concepts/policy-formats.md)** - Supported policy formats

### 📘 [Reference](./reference/api/platform-api.md)
Complete API and syntax reference.

- **[Platform API](./reference/api/platform-api.md)** - Platform HTTP API
- **[Agent API](./reference/api/agent-api.md)** - Agent HTTP API
- **[CLI Reference](./reference/cli.md)** - CLI commands
- **[Policy Syntax](./reference/policy-syntax.md)** - Policy syntax reference

### ⚡ [Performance](./performance/benchmarks.md)
Benchmark results and optimization guides.

- **[Benchmarks](./performance/benchmarks.md)** - Performance results
- **[Scale Tests](./performance/scale-tests.md)** - Scale testing methodology
- **[Optimization](./performance/optimization.md)** - Performance tuning

### 🏗️ [Deployment](./deployment/deployment-patterns.md)
Deploy Reaper in different patterns.

- **[Deployment Patterns](./deployment/deployment-patterns.md)** - Patterns overview
- **[Sidecar Mode](./deployment/sidecar.md)** - Sidecar deployment
- **[Standalone Mode](./deployment/standalone.md)** - Standalone deployment

### 🔧 [Advanced](./advanced/custom-evaluators.md)
Advanced topics for power users.

- **[Custom Evaluators](./advanced/custom-evaluators.md)** - Write custom evaluators
- **[Policy Hot-Swapping](./advanced/hot-swapping.md)** - Zero-downtime updates
- **[Rego Comparison](./advanced/rego-comparison.md)** - Comparison with OPA/Rego

---

## 🎯 Quick Links

| I want to... | Go to... |
|--------------|----------|
| **Understand what Reaper is** | [Introduction/Overview](./introduction/overview.md) |
| **Install Reaper** | [Getting Started/Installation](./getting-started/installation.md) |
| **Run my first policy** | [Getting Started/Quick Start](./getting-started/quick-start.md) |
| **See performance numbers** | [Performance/Benchmarks](./performance/benchmarks.md) |
| **Understand the architecture** | [Concepts/Architecture](./concepts/architecture.md) |
| **Deploy to production** | [Deployment/Patterns](./deployment/deployment-patterns.md) |
| **Optimize performance** | [Performance/Optimization](./performance/optimization.md) |
| **Write custom evaluators** | [Advanced/Custom Evaluators](./advanced/custom-evaluators.md) |

---

## 📊 Performance at a Glance

| Policy Type | Mean Latency | Throughput |
|-------------|--------------|------------|
| **RBAC** | 371ns | 1.9M ops/s |
| **ABAC** | 941ns | 846K ops/s |
| **ReBAC** | 519ns | 1.4M ops/s |
| **Multilayer** | 1.2µs | - |

**All tests exceed targets by 8-126x!** 🎯

See [Performance Benchmarks](./performance/benchmarks.md) for detailed results.

---

## 🚀 Quick Start

```bash
# 1. Install Reaper
cargo install reaper-cli

# 2. Write a policy
cat > policy.reap << 'EOF'
policy rbac {
  permit principal.role == "admin"
}
EOF

# 3. Evaluate
reaper-cli eval \
  --policy policy.reap \
  --principal "user:alice" \
  --action "read" \
  --resource "doc:123"

# Output: ✅ ALLOW
```

See [Quick Start Guide](./getting-started/quick-start.md) for the full tutorial.

---

## 🌟 Start Exploring!

- **New to Reaper?** → Start with [Introduction/Overview](./introduction/overview.md)
- **Ready to code?** → Jump to [Getting Started/Quick Start](./getting-started/quick-start.md)
- **Curious about performance?** → Check [Performance/Benchmarks](./performance/benchmarks.md)
- **Deploying to production?** → Read [Deployment/Patterns](./deployment/deployment-patterns.md)
