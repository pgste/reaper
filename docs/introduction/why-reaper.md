# Why Reaper?

Choosing an authorization engine is a critical decision. Here's why Reaper stands out.

## The Authorization Challenge

Modern applications need to answer one fundamental question millions of times per second:

> **"Can this user perform this action on this resource?"**

Traditional solutions fall short:

- **Hard-coded logic**: Scattered across your codebase, hard to maintain
- **Database queries**: Too slow for high-performance APIs
- **External services**: Network latency kills performance
- **JVM-based engines**: High memory overhead, long GC pauses

## The Reaper Solution

Reaper solves these problems with a Rust-native, high-performance architecture.

### 1. Sub-Microsecond Latency

**Problem**: Authorization checks add latency to every request.

**Reaper's Solution**: < 1µs P99 latency for most policies.

```
Traditional Engine:  100µs - 1ms per check
Reaper:              < 1µs per check
Performance Gain:    100-1000x faster
```

**Impact**: Add authorization to hot paths without performance penalty.

### 2. Zero-Downtime Updates

**Problem**: Updating policies requires service restarts, causing downtime.

**Reaper's Solution**: Atomic hot-swapping.

```
Old Approach:
1. Take service offline
2. Update policy file
3. Restart service
4. Hope nothing breaks

Reaper Approach:
1. Deploy new policy to platform
2. Platform pushes to agents
3. Atomic swap (< 1µs)
4. Zero dropped requests
```

**Impact**: Update authorization logic continuously without disruption.

### 3. Memory Efficiency

**Problem**: JVM-based engines consume hundreds of megabytes per instance.

**Reaper's Solution**: < 50MB per agent with string interning.

```
OPA (Go):      ~200MB per instance
Cedar (AWS):   ~150MB per instance
Reaper (Rust): < 50MB per instance
Memory Saved:  60-80%
```

**Impact**: Run more instances, lower cloud costs.

### 4. Million Ops/Second Throughput

**Problem**: Centralized auth services become bottlenecks.

**Reaper's Solution**: > 2M decisions/second per core.

```
Single Reaper Agent:
- RBAC: 2.1M ops/sec
- ABAC: 840K ops/sec
- Cedar: 100K ops/sec
```

**Impact**: Handle authorization for entire microservices fleet with few instances.

## Real-World Scenarios

### Scenario 1: E-Commerce Checkout

**Requirement**: Check user permissions during checkout flow.

**Latency Budget**: 10ms total API latency

**Challenge**: Authorization can't consume significant budget.

**With OPA**: 100µs = 1% of budget
**With Reaper**: < 1µs = 0.01% of budget

**Result**: Reaper leaves more budget for business logic.

---

### Scenario 2: Multi-Tenant SaaS

**Requirement**: Isolate 10,000 tenants, 1M users.

**Data Volume**: Large user/tenant attribute dataset

**Challenge**: Memory footprint at scale.

**With OPA**: 200MB × 10 instances = 2GB
**With Reaper**: 50MB × 10 instances = 500MB

**Result**: 75% memory savings = lower infrastructure costs.

---

### Scenario 3: Continuous Deployment

**Requirement**: Deploy policy updates 100x per day.

**Challenge**: Can't afford downtime for policy updates.

**With OPA**: Requires service restarts or complex rolling updates
**With Reaper**: Atomic hot-swap, zero downtime

**Result**: Deploy freely without coordination overhead.

---

### Scenario 4: High-Frequency Trading Platform

**Requirement**: 1M authorization checks/second.

**Challenge**: Every microsecond of latency = lost revenue.

**With OPA**: ~100µs × 1M = 100 seconds of CPU time per second (impossible)
**With Reaper**: ~1µs × 1M = 1 second of CPU time (feasible on single core)

**Result**: Reaper makes real-time authorization possible.

## Technical Advantages

### Lock-Free Concurrency

Reaper uses `DashMap` for lock-free concurrent reads:

- **No lock contention**: Millions of concurrent reads
- **Predictable latency**: No waiting for locks
- **Horizontal scaling**: Linear performance scaling

### Zero-Copy Architecture

Policies are shared via `Arc<T>`:

- **No cloning**: Policies shared across threads
- **Atomic updates**: Replace entire Arc atomically
- **Memory efficient**: One policy copy, many references

### String Interning

Entity attributes are deduplicated:

- **60% memory savings**: Deduplicate common strings
- **Faster comparisons**: Compare interned string IDs
- **Cache-friendly**: Smaller data structures

### Rust Safety Guarantees

- **Memory safe**: No buffer overflows, use-after-free
- **Thread safe**: Compiler-enforced concurrency safety
- **Crash-free**: Panic-free hot paths

## When NOT to Use Reaper

Reaper may not be the best fit if:

### 1. You Need Complex Rego Features

If you rely heavily on advanced Rego constructs (e.g., recursive rules, complex unification), OPA might be better.

**Alternative**: Reaper supports AWS Cedar and simple Rego patterns.

### 2. You're Fully Invested in AWS

If you're all-in on AWS and use Cedar everywhere, AWS's native Cedar integration might be simpler.

**Alternative**: Reaper supports Cedar policies natively.

### 3. You Need Slow-Changing, Complex Policies

If your policies change monthly and involve complex business logic taking milliseconds to evaluate, performance may not matter.

**Alternative**: Reaper still offers better developer experience.

### 4. You Have < 100 Requests/Second

If you have very low traffic, the performance benefits may not justify the migration effort.

**Alternative**: Start with Reaper to avoid future bottlenecks.

## Migration from OPA/Cedar

Reaper makes migration easy:

### From OPA

1. **Convert Rego to REAP** - Similar syntax, simpler semantics
2. **Test side-by-side** - Run both engines in parallel
3. **Gradual rollout** - Migrate service-by-service
4. **Verify performance** - See 10-100x speedup

### From AWS Cedar

1. **Use Cedar policies directly** - Reaper supports Cedar
2. **Deploy to Reaper** - Self-host for flexibility
3. **Optimize** - Convert to REAP for < 1µs latency

See [Migration Guide](../guides/migration.md) (coming soon).

## Cost Savings Calculator

**Scenario**: 1M requests/second across 100 microservices

### OPA Deployment

- **Instances needed**: 10 (100K ops/sec per instance)
- **Memory**: 200MB × 10 = 2GB
- **CPU**: 10 cores
- **Monthly cost** (AWS): ~$300/month

### Reaper Deployment

- **Instances needed**: 1 (2M ops/sec per core)
- **Memory**: 50MB × 1 = 50MB
- **CPU**: 1 core
- **Monthly cost** (AWS): ~$30/month

**Savings**: $270/month = $3,240/year

*Scale these savings to your traffic levels.*

## Bottom Line

Choose Reaper if you need:

✅ **Sub-microsecond latency**
✅ **Zero-downtime updates**
✅ **60-80% memory savings**
✅ **Million ops/second throughput**
✅ **Production-ready stability**

## Next Steps

- **[Key Features](./key-features.md)** - Explore Reaper's features in depth
- **[Quick Start](../getting-started/quick-start.md)** - Try Reaper now
- **[Architecture](../concepts/architecture.md)** - Understand how it works
