# Policy Engine Performance Benchmarks

**Phase 7.2: Performance Profiling & Benchmarking**

Comprehensive benchmark suite for establishing performance baselines and detecting regressions.

## 📊 Benchmark Suite Overview

### 1. Built-in Functions (`builtins_bench.rs`)

**Purpose**: Establish baseline performance for all built-in functions

**Categories**:
- **Type Checking** (5 benchmarks): `is_string`, `is_number`, `is_bool`, `is_array`, `is_null`
- **String Methods** (7 benchmarks): `lower`, `upper`, `trim`, `contains`, `startswith`, `endswith`, `matches`
- **Collection Methods** (12 benchmarks): `count`, `sum`, `max`, `min` across 10/100/500 element arrays
- **Time Functions** (4 benchmarks): `now_ns`, `parse_rfc3339`, `add_ns`, `is_before`
- **JSON Functions** (2 benchmarks): `is_valid`, `parse`

**Total**: 30 individual benchmarks

**Run**: `cargo bench --bench builtins_bench`

### 2. Regex Caching (`caching_bench.rs`)

**Purpose**: Measure regex compilation cache effectiveness (2-5x speedup target)

**Benchmarks**:
- **Cache Hit**: Repeated use of same regex pattern (email validation)
- **Cache Miss**: Different regex patterns (3 variants)
- **Complexity**: Simple vs medium vs complex regex patterns
- **Operations**: `matches()` vs `find()` performance
- **Repeated Usage**: Cache effectiveness over 10/100/1000 iterations

**Total**: 13 benchmarks

**Run**: `cargo bench --bench caching_bench`

### 3. SIMD Aggregates (`simd_bench.rs`)

**Purpose**: Measure SIMD performance for numeric aggregates (2-4x speedup target at 64+ elements)

**Array Sizes**: 16, 32, 64, 128, 256, 512, 1024 elements

**Benchmarks**:
- **sum()**: Integer sum across all sizes
- **max()**: Maximum value across all sizes
- **min()**: Minimum value across all sizes
- **count()**: Constant-time operation verification
- **Int vs Float**: SIMD performance comparison
- **Threshold**: Performance at 48, 56, 64, 72, 80 elements (SIMD activation point)
- **Multiple Aggregates**: Combined sum/max/min in single policy

**Total**: 50+ benchmarks

**Run**: `cargo bench --bench simd_bench`

### 4. End-to-End Evaluation (`e2e_bench.rs`)

**Purpose**: Real-world policy evaluation scenarios

**Scenarios**:
- **Simple Policies**: Allow/deny all, single attribute check
- **ABAC Policies**: 1, 2, and 4 condition policies
- **Multi-Rule**: 1, 5, 10, 20 rule policies
- **Comprehensions**: Small/medium/nested array filtering
- **Time-Based**: Time comparisons and range checks
- **String Operations**: Contains, transformation chains, regex
- **JSON Operations**: Validation and parsing
- **Real-World**: Document access control scenario

**Total**: 30+ benchmarks

**Run**: `cargo bench --bench e2e_bench`

### 5. Policy Engine (`policy_evaluation_bench.rs`)

**Purpose**: Core engine performance (pre-existing)

**Benchmarks**:
- Simple/complex policy evaluation
- Policy hot-swapping
- Concurrent access patterns
- Memory efficiency
- Realistic workloads
- Sub-microsecond latency targets

**Run**: `cargo bench --bench policy_evaluation_bench`

## 🎯 Performance Targets

### Sub-Microsecond Operations (<1µs)
- Type checking functions: **~300-400ns** ✅
- Simple attribute checks: **<1µs**
- String methods (lower/upper/trim): **~440-490ns** ✅
- Collection count(): **~550ns** ✅

### Microsecond Range (1-10µs)
- Regex matches (simple): **~5.3µs** ✅
- Small comprehensions: **<10µs**
- Time operations: **<5µs**
- JSON validation: **<5µs**

### SIMD Performance
- **Target**: 2-4x speedup at 64+ elements
- Aggregates (sum/max/min) on 500 elements: **<2µs**
- Count() should be O(1): **~550ns** regardless of size ✅

### Regex Caching
- **Target**: 2-5x speedup on cache hits
- First compilation: One-time cost
- Subsequent matches: Cache hit performance

## 📈 Baseline Results (Initial)

### Type Checking (Sub-400ns)
```
is_string    : 371.28 ns
is_number    : 307.31 ns
is_bool      : 320.73 ns
is_array     : 387.43 ns
is_null      : 308.66 ns
```

### String Methods
```
lower        : 450.55 ns
upper        : 489.07 ns
trim         : 449.52 ns
contains     : 481.02 ns
startswith   : 443.35 ns
endswith     : 440.58 ns
matches      : 5.2963 µs (regex compilation + match)
```

### Collection Methods (10 elements)
```
count        : 551.09 ns
sum          : (running...)
max          : (running...)
min          : (running...)
```

## 🚀 Running Benchmarks

### Run All Benchmarks
```bash
cargo bench --workspace
```

### Run Specific Benchmark Suite
```bash
cargo bench --bench builtins_bench
cargo bench --bench caching_bench
cargo bench --bench simd_bench
cargo bench --bench e2e_bench
cargo bench --bench policy_evaluation_bench
```

### Run Specific Benchmark
```bash
cargo bench --bench builtins_bench type_check
cargo bench --bench simd_bench simd_sum
cargo bench --bench e2e_bench abac_policy
```

### Test Mode (Verify Benchmarks Work)
```bash
cargo bench --bench builtins_bench -- --test
```

### Save Baseline
```bash
cargo bench --bench builtins_bench -- --save-baseline main
```

### Compare to Baseline
```bash
cargo bench --bench builtins_bench -- --baseline main
```

## 📊 Benchmark Output

Criterion generates detailed reports in `target/criterion/`:
- **HTML Reports**: Visual charts and statistics
- **Baseline Comparisons**: Performance regression detection
- **Statistical Analysis**: Outliers, variance, confidence intervals

### View Reports
```bash
open target/criterion/report/index.html
```

## 🔍 Performance Profiling

### Flamegraph Generation
```bash
# Install flamegraph
cargo install flamegraph

# Profile a benchmark
cargo flamegraph --bench e2e_bench -- --bench
```

### CPU Profiling with perf
```bash
# Linux only
cargo bench --bench e2e_bench -- --profile-time=5
```

## 📝 Benchmark Design Principles

1. **Isolation**: Each benchmark tests a single operation
2. **Pre-loading**: Policy parsing and data loading happen in setup
3. **Black Box**: Use `std::hint::black_box` to prevent optimization
4. **Statistical Rigor**: 100 samples per benchmark for accuracy
5. **Minimal Overhead**: Tiny policies and data for focused measurement

## 🎨 Adding New Benchmarks

### Create Benchmark File
```rust
// benches/my_bench.rs
use criterion::{criterion_group, criterion_main, Criterion};

fn my_benchmark(c: &mut Criterion) {
    c.bench_function("my_test", |b| {
        b.iter(|| {
            // Code to benchmark
        })
    });
}

criterion_group!(benches, my_benchmark);
criterion_main!(benches);
```

### Register in Cargo.toml
```toml
[[bench]]
name = "my_bench"
harness = false
```

### Run It
```bash
cargo bench --bench my_bench
```

## 🔧 Optimization Targets

Based on benchmark results, optimization priorities:

1. **Regex Caching**: Ensure 2-5x speedup on repeated patterns
2. **SIMD Aggregates**: Verify 64-element threshold is optimal
3. **String Operations**: Already excellent (<500ns)
4. **Type Checking**: Already excellent (<400ns)
5. **Comprehensions**: Monitor scaling with array size

## 📊 CI Integration

### Performance Regression Detection
```yaml
# .github/workflows/bench.yml
- name: Run benchmarks
  run: cargo bench --workspace -- --save-baseline main

- name: Compare to main
  run: cargo bench --workspace -- --baseline main
```

### Alerts
- >5% slowdown: Warning
- >10% slowdown: Failure
- <5% speedup: Track improvements

## 🎯 Phase 7.2 Status

✅ **Completed**:
- Built-in functions benchmark suite (30 benchmarks)
- Regex caching benchmark suite (13 benchmarks)
- SIMD aggregate benchmark suite (50+ benchmarks)
- End-to-end evaluation benchmark suite (30+ benchmarks)
- All benchmarks registered in Cargo.toml
- All benchmarks compile successfully
- Initial baseline data collection in progress

**Total**: 120+ individual performance benchmarks

**Next Steps**:
- Complete baseline data collection
- Flamegraph profiling for hot paths
- CI/CD integration for regression detection
- Optimization based on bottlenecks identified

## 📚 Resources

- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Flamegraph Guide](https://github.com/flamegraph-rs/flamegraph)

---

*Generated for Reaper Policy Engine - Phase 7.2: Performance Profiling & Benchmarking*
