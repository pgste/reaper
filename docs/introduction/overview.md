# Overview

Reaper is a high-performance authorization engine designed for modern distributed systems. Built in Rust, it provides sub-microsecond policy evaluation with zero-downtime updates.

## What is Reaper?

Reaper is a **policy enforcement platform** that helps you implement authorization in your applications. It evaluates access control policies to make authorization decisions at runtime.

Unlike traditional authorization systems, Reaper is optimized for:

- **Extreme low latency** - Sub-microsecond decision times
- **High throughput** - Millions of decisions per second
- **Zero downtime** - Atomic policy updates without service interruption
- **Memory efficiency** - 60-80% less memory than JVM-based solutions

## Use Cases

Reaper is ideal for:

### Microservices Authorization
Centralize authorization logic across your microservices with consistent policy enforcement.

```
User Request → API Gateway → Reaper Agent → Service
                                  ↓
                           Allow/Deny Decision
```

### Multi-Tenant SaaS Applications
Implement complex tenant isolation and access control policies.

```reap
policy tenant_isolation {
  permit principal.tenant_id == resource.tenant_id
}
```

### High-Performance APIs
Make millions of authorization decisions per second without adding latency.

**Performance**: < 500ns mean latency for RBAC policies

### Zero-Trust Networks
Enforce fine-grained access control at every network boundary.

## Architecture

Reaper uses a two-service architecture:

### Reaper Platform (Management Layer)
- Centralized policy repository
- Policy versioning and lifecycle management
- Deployment coordination to agents
- Runs on port 8081

### Reaper Agent (Enforcement Layer)
- High-performance policy evaluation
- Atomic policy hot-swapping
- Sub-microsecond latency
- Runs on port 8080

```
┌──────────────────────────────────────────┐
│       Reaper Platform (Port 8081)        │
│   • Policy CRUD                          │
│   • Versioning                           │
│   • Deployment                           │
└────────────┬─────────────────────────────┘
             │
             │ Deploy Policy
             │
    ┌────────▼────────┐
    │                 │
    │  Reaper Agent   │
    │  (Port 8080)    │
    │                 │
    │  • Evaluate     │
    │  • Hot-swap     │
    │  • Metrics      │
    └─────────────────┘
```

## Key Concepts

### Policies
Policies define authorization rules. Reaper supports multiple formats:

- **REAP DSL** - Native Reaper policy language
- **YAML** - Structured YAML format
- **JSON** - Machine-readable JSON format
- **AWS Cedar** - Compatible with AWS Cedar policies

### Evaluation
Policy evaluation is the core operation:

```rust
let decision = agent.evaluate(PolicyRequest {
    principal: "user:alice",
    action: "read",
    resource: "document:123"
});
// Returns: Allow or Deny
```

### Hot-Swapping
Policies can be updated atomically without restarting the agent:

1. Platform receives new policy
2. Policy is validated and compiled
3. Agent atomically swaps policy
4. Zero downtime, no dropped requests

## Performance Characteristics

| Metric | Value | Notes |
|--------|-------|-------|
| **Latency (RBAC)** | < 500ns mean | Simple role-based policies |
| **Latency (ABAC)** | < 1µs mean | Attribute-based policies |
| **Latency (Cedar)** | 10-50µs | AWS Cedar policies |
| **Throughput** | > 2M ops/sec | Single-core performance |
| **Memory** | < 50MB | Per agent instance |
| **Hot-swap** | ~1µs | Policy replacement time |

See [Performance Benchmarks](../performance/benchmarks.md) for detailed results.

## Comparison with Other Engines

### vs Open Policy Agent (OPA)

| Feature | Reaper | OPA |
|---------|--------|-----|
| **Language** | Rust | Go |
| **Latency** | < 1µs | ~100µs |
| **Memory** | < 50MB | ~200MB |
| **Hot-swap** | Atomic, zero-downtime | Requires reload |
| **Policy Language** | REAP/Cedar/YAML | Rego |

### vs AWS Cedar

| Feature | Reaper | Cedar |
|---------|--------|-------|
| **Deployment** | Self-hosted | AWS only |
| **Languages** | REAP, Cedar, YAML, JSON | Cedar only |
| **Hot-swap** | Yes | N/A |
| **Performance** | < 1µs (native) | 10-50µs |

See [Rego Comparison](../advanced/rego-comparison.md) for detailed analysis.

## Design Philosophy

Reaper is built on these core principles:

### 1. Performance First
Every design decision prioritizes latency and throughput. Lock-free data structures, zero-copy operations, and careful memory management ensure sub-microsecond performance.

### 2. Zero Downtime
Production systems can't afford downtime. Atomic policy hot-swapping ensures policies can be updated without dropping requests or restarting services.

### 3. Developer Experience
Authorization should be easy to implement and test. Multiple policy formats, rich CLI tooling, and BDD testing support make Reaper developer-friendly.

### 4. Production Ready
Comprehensive testing (unit, integration, BDD, scale tests), detailed metrics, and battle-tested deployment patterns ensure Reaper is ready for production workloads.

## Next Steps

- **[Why Reaper?](./why-reaper.md)** - Learn why you should choose Reaper
- **[Key Features](./key-features.md)** - Explore Reaper's features in detail
- **[Quick Start](../getting-started/quick-start.md)** - Get started with Reaper in minutes
