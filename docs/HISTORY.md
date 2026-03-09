# Reaper Development History

This document captures key milestones and learnings from Reaper's development.

## Timeline Overview

### Phase 1: Core Entity System (November 2024)

**Objective:** Enable 100k+ entity loading with generic, entity-type agnostic design.

**Key Achievements:**
- `DataStore`: Entity type indexing with lock-free DashMap
- `DataLoader`: Direct JSON value loading (40% memory savings vs re-serialization)
- Successfully scaled from 100 users to 100,000 users

**Performance Results (100k users):**
- Load time: 7.42s
- Memory: 64.43 MB (83% reduction from initial OOM)
- Throughput: 513k ops/sec
- Mean latency: 1.17µs

### Phase 2: Policy Evaluation Optimization

**Objective:** Optimize policy evaluation for sub-microsecond latency.

**Key Learnings:**
- **Baseline is excellent**: Simple evaluator achieved 341ns mean, 2.9M req/s
- **Complexity doesn't equal performance**: Indexed engine was 6-8x slower
- **Compiled evaluator**: 3x slower due to abstraction overhead
- **Decision Matrix**: Useful for bounded spaces (262ns O(1) lookup)

**Recommendation:** Use Simple evaluator for 99% of cases. It's already highly optimized.

### Phase 3: AWS Cedar Integration

**Objective:** Add AWS Cedar policy language support for ABAC scenarios.

**Achievements:**
- Full Cedar syntax parsing and evaluation
- ABAC attribute-based access control
- Integration with entity data store
- Performance: 10-50µs (acceptable for complex ABAC)

### Phase 4-5: Reaper DSL Development

**Objective:** Create native domain-specific language for policy definition.

**Features Implemented:**
- `.reap` file format with intuitive syntax
- Variable binding and comprehensions
- Collection operations (some, all, filter)
- String, math, and time functions
- Conditional expressions

**Performance:** Sub-microsecond for most policies.

### Phase 6: Multi-Layer Policy Architecture

**Objective:** Support complex multi-layer policy evaluation.

**Achievements:**
- Composite index support for complex queries
- View-based policy organization
- Multi-source data integration
- Dual-source scale testing (roles + attributes + resources)

### Phase 7: Service Architecture

**Objective:** Establish Agent + Platform two-service model.

**Achievements:**
- **Reaper Agent** (8080): High-throughput enforcement layer
- **Reaper Platform** (8081): Management and coordination layer
- Atomic hot-swapping for zero-downtime deployments
- Lock-free concurrent policy store

### Phase 8: Management Server

**Objective:** Add multi-tenant management capabilities.

**Features:**
- Organization-based multi-tenancy
- Bundle-based policy deployment workflow
- Agent registration and management
- PostgreSQL backend for persistence
- SSE-based event streaming to agents

### Phase 9: eBPF Integration (Experimental)

**Objective:** Kernel-level policy enforcement via eBPF LSM hooks.

**Implementation:**
- `reaper-ebpf-kern`: eBPF kernel program with LSM hooks
- BPF maps for policy lookup (<100ns fast path)
- `file_open` and `socket_connect` hooks
- Ring buffer for userspace communication

**Status:** Experimental, not production-ready.

## Key Technical Decisions

### 1. Lock-Free Architecture
Using `DashMap` for concurrent policy store enables millions of concurrent evaluations without lock contention.

### 2. String Interning
All entity strings deduplicated via `StringInterner` achieving ~60% memory savings.

### 3. Atomic Hot-Swapping
Policies replaced atomically using `Arc<T>` for zero-downtime updates.

### 4. Pluggable Evaluators
The `PolicyEvaluator` trait enables adding new policy languages without core changes.

### 5. Bundle Format (.rbb)
Binary policy bundles for fast loading and network transfer.

## Performance Benchmarks

| Metric | Target | Achieved |
|--------|--------|----------|
| Simple Policy Eval | < 1µs | 341ns |
| Cedar Policy Eval | < 50µs | 10-50µs |
| Policy Lookup | < 200ns | 50-200ns |
| Memory (10k entities) | < 50MB | ~25MB |
| Throughput | > 100k req/s | 2.9M req/s |

## Lessons Learned

1. **Measure First**: Benchmark everything before optimizing. Original "200x speedup" claims were measuring wrong metrics.

2. **Simple is Fast**: The straightforward Simple evaluator outperformed complex indexed/compiled versions.

3. **Memory Matters**: Direct JSON loading eliminated OOM at scale through 40% memory reduction.

4. **Lock-Free Wins**: DashMap-based architecture enables massive concurrency without contention.

5. **Honest Engineering**: Better to admit when optimizations fail than ship slow code claiming it's fast.

---

*This history document was consolidated from development archives during the production-ready cleanup phase.*
