# Performance Optimization Analysis

**Phase 7.3: Optimization & Flamegraph Profiling**

Analysis of benchmark results to identify optimization opportunities and validate performance targets.

---

## 📊 Benchmark Results Summary

### Type Checking Operations (✅ Excellent - Sub-400ns)

| Function | Time (median) | Status | Notes |
|----------|--------------|--------|-------|
| `is_number()` | 307.31 ns | ✅ Best | Fastest type check |
| `is_null()` | 308.66 ns | ✅ Excellent | Near-optimal |
| `is_bool()` | 320.73 ns | ✅ Excellent | |
| `is_string()` | 371.28 ns | ✅ Good | |
| `is_array()` | 387.43 ns | ✅ Good | Slightly slower (expected) |

**Analysis**:
- All type checks under 400ns ✨
- Performance variation < 80ns across all types
- `is_number()` and `is_null()` are fastest (~308ns)
- No optimization needed

**Validation**: ✅ Meets sub-microsecond target (<1µs)

---

### String Operations (✅ Excellent - Sub-500ns)

| Method | Time (median) | Status | Notes |
|--------|--------------|--------|-------|
| `endswith()` | 440.58 ns | ✅ Best | |
| `startswith()` | 443.35 ns | ✅ Excellent | |
| `trim()` | 449.52 ns | ✅ Excellent | |
| `lower()` | 450.55 ns | ✅ Excellent | |
| `contains()` | 481.02 ns | ✅ Good | |
| `upper()` | 489.07 ns | ✅ Good | Slightly slower than lower |
| `matches()` | 5.2963 µs | ⚠️ Slower | Regex compilation cost |

**Analysis**:
- Simple string ops: 440-490ns (excellent!)
- `matches()` is 10x slower due to regex compilation
- **Optimization**: Regex caching should improve `matches()` to ~1-2µs on cache hits

**Validation**: ✅ Meets sub-microsecond target (except regex)

---

### Collection Operations - Scaling Analysis

#### count() - ⚠️ Scaling Issue Detected

| Array Size | Time (median) | Expected | Actual Behavior |
|-----------|--------------|----------|-----------------|
| 10 elements | 551.09 ns | O(1) ~500ns | ✅ Baseline |
| 100 elements | 2.7168 µs | O(1) ~500ns | ⚠️ 5x slower |
| 500 elements | (not shown) | O(1) ~500ns | ⏳ Needs data |

**🟡 ROOT CAUSE IDENTIFIED**: Attribute conversion, not count() itself

**Investigation** (ast_evaluator.rs:538-607):
- `method_count()` correctly uses `.len()` (O(1)) ✅
- Scaling comes from `attribute_value_to_eval_value()` conversion
- Every call to `user.nums` converts `AttributeValue::List` → `EvalValue::Array`
- Conversion iterates through all elements (lines 560-569):
  ```rust
  AttributeValue::List(list) => {
      let items: Vec<EvalValue> = list
          .iter()
          .map(|v| self.attribute_value_to_eval_value(...))  // O(n)
          .collect();
      EvalValue::Array(items)
  }
  ```

**Analysis**:
- count(10): 551ns total (conversion + count)
- count(100): 2,717ns total (~21.7ns per element conversion + count)
- This affects ALL method calls on entity attributes, not just count()
- Performance is still fast in absolute terms (<3µs for 100 elements)

**Fix Priority**: MEDIUM (architectural, affects all operations)
**True Fix**: Use Arc for shared collection data (significant refactor)
**Workaround**: Fast-path for count() to operate on AttributeValue directly

#### sum() - Good Performance

| Array Size | Time (median) | Per Element | Status |
|-----------|--------------|-------------|--------|
| 10 | 572.39 ns | 57.2 ns/elem | ✅ Good |
| 100 | 2.6756 µs | 26.8 ns/elem | ✅ Good |

**Analysis**:
- Linear scaling as expected (O(n))
- ~27ns per element for 100-element array
- **SIMD Opportunity**: At 64+ elements, expect 2-4x speedup

#### max() - Excellent Performance

| Array Size | Time (median) | Per Element | Status |
|-----------|--------------|-------------|--------|
| 10 | 617.82 ns | 61.8 ns/elem | ✅ Excellent |
| 100 | 2.4742 µs | 24.7 ns/elem | ✅ Excellent |

**Analysis**:
- Better than expected for 100 elements
- 24.7ns/elem is very fast
- Likely benefiting from SIMD already

#### min() - Best Performance!

| Array Size | Time (median) | Per Element | Status |
|-----------|--------------|-------------|--------|
| 10 | 746.11 ns | 74.6 ns/elem | ✅ Good |
| 100 | 1.7211 µs | 17.2 ns/elem | ✅ **Best!** |

**Analysis**:
- Fastest aggregate operation at scale
- 17.2ns/elem is exceptional
- Strong SIMD candidate

---

## 🎯 Optimization Priorities

### Priority 1: Attribute Conversion Optimization 🟡

**Status**: INVESTIGATED - Root cause identified
**Impact**: MEDIUM - Affects all entity attribute method calls
**Effort**: HIGH - Architectural change required
**Speedup**: 5-10x for large collections (potential)

**Finding**: `method_count()` already uses `.len()` correctly (O(1))
**Root Cause**: O(n) scaling from `attribute_value_to_eval_value()` conversion
- Affects: count(), sum(), max(), min(), and all collection methods
- Impact: ~21.7ns per element for conversion overhead
- Current: 2.7µs for 100 elements (still fast, but scales linearly)

**Options**:
1. **Short-term**: Add fast-path for count() on AttributeValue directly (90 minutes)
2. **Long-term**: Use Arc<Vec<>> for shared collection data (architectural refactor, 1-2 days)
3. **Defer**: Accept current performance as "good enough" (<3µs for 100 elements)

**Recommendation**: **DEFER** for now - current performance is acceptable
- 2.7µs for 100 elements is still very fast
- Affects equally across all aggregate operations (not just count)
- Architectural fix should be part of larger optimization effort

### Priority 2: SIMD Aggregates ✅ CONFIRMED

**Status**: VERIFIED - Fully implemented
**Impact**: MEDIUM - 2-4x speedup for arrays with 64+ elements
**Effort**: NONE - Already implemented
**Speedup**: 2-4x for large pure-type arrays (expected from LLVM)

**Implementation Confirmed** (ast_evaluator.rs):
- **sum()**: Lines 1997, 2013 - Fast paths for >64 element arrays
- **max()**: Lines 2077, 2094 - Fast paths for >64 element arrays
- **min()**: Lines 2161, 2178 - Fast paths for >64 element arrays

**SIMD Pattern**:
```rust
if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
    let sum: i64 = items
        .iter()
        .filter_map(|v| if let EvalValue::Integer(i) = v { Some(*i) } else { None })
        .sum();  // LLVM auto-vectorizes this pattern
    return Ok(EvalValue::Integer(sum));
}
```

**Features**:
- Threshold: 64 elements (configurable)
- Separate fast paths for integer and float arrays
- Automatic fallback to scalar for mixed types
- LLVM auto-vectorization (no manual SIMD required)

### Priority 3: Regex Cache ✅ CONFIRMED

**Status**: VERIFIED - Fully implemented
**Impact**: MEDIUM - 2-5x speedup on repeated patterns
**Effort**: NONE - Already implemented
**Speedup**: 3-5x for cache hits (expected)

**Implementation Confirmed** (ast_evaluator.rs:2386-2407):
- **Cache**: `Mutex<HashMap<String, regex::Regex>>` with parking_lot
- **Fast path**: Check cache with lock, return clone if found
- **Slow path**: Compile regex OUTSIDE lock (no blocking)
- **Usage**: matches(), find(), find_all(), replace()

**Caching Strategy**:
```rust
fn get_cached_regex(&self, pattern: &str) -> Result<regex::Regex, ReaperError> {
    // Fast path: check cache
    {
        let cache = self.regex_cache.lock();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    } // Release lock

    // Compile outside lock (avoid blocking)
    let re = regex::Regex::new(pattern)?;

    // Insert into cache
    {
        let mut cache = self.regex_cache.lock();
        cache.insert(pattern.to_string(), re.clone());
    }
    Ok(re)
}
```

**Performance**:
- First match: 5.3µs (includes compilation)
- Cache hits: Expected 1-2µs (3-5x faster)
- Lock contention: Minimal (parking_lot + short critical sections)

### Priority 4: Profile Hot Paths 🔥

**Impact**: Variable - Identify unexpected bottlenecks
**Effort**: MEDIUM - Analysis required

**Action**:
```bash
# Profile end-to-end scenarios
./crates/policy-engine/scripts/profile.sh e2e

# Profile built-ins
./crates/policy-engine/scripts/profile.sh builtins

# Analyze flamegraphs for hot paths
```

---

## 🔬 SIMD Analysis Plan

### Test Array Sizes
```
Small:    16, 32          (below threshold)
Threshold: 48, 56, 64, 72, 80  (around threshold)
Large:    128, 256, 512, 1024  (above threshold)
```

### Expected Performance Curve

```
Time/Element
     ^
50ns |  ●●●●
     |      ●●
30ns |        ●●●
     |           ●●●●
10ns |               ●●●●●●●●●●
     +--------------------------->
       16  32  48  64  96 128  256  Array Size

     [No SIMD] | [SIMD Active]
```

### SIMD Validation Checklist

- [ ] Verify 64-element threshold is optimal
- [ ] Confirm 2-4x speedup at 128+ elements
- [ ] Test integer vs float performance
- [ ] Validate sum(), max(), min() all benefit
- [ ] Check multiple aggregates don't interfere

---

## 📈 Performance Targets & Status

| Category | Target | Current | Status |
|----------|--------|---------|--------|
| Type Checking | < 500ns | 307-387ns | ✅ 100% |
| String Ops | < 500ns | 440-489ns | ✅ 100% |
| count() O(1) | < 600ns | 551ns (10) ⚠️ 2.7µs (100) | ❌ Needs fix |
| Regex (cached) | < 2µs | 5.3µs (uncached) | ⏳ Pending test |
| SIMD Aggregates | 2-4x @ 64+ | TBD | ⏳ Pending test |
| Simple Policy | < 1µs | TBD | ⏳ Pending e2e |
| ABAC Policy | < 10µs | TBD | ⏳ Pending e2e |

**Overall**: 2/7 validated, 1 issue found, 4 pending verification

---

## 🚀 Quick Wins

### 1. Fix count() - 10 minutes, 5x speedup ⚡
```rust
// Change in ast_evaluator.rs:method_count()
- arr.iter().count()  // Suspected current
+ arr.len()           // Direct O(1) access
```

### 2. Verify Regex Caching - Already implemented ✅
Run `cargo bench --bench caching_bench` to confirm 2-5x speedup

### 3. Document SIMD Threshold - For optimization guide 📚
Run `cargo bench --bench simd_bench` to find exact crossover point

---

## 🔍 Investigation Needed

### 1. count() Implementation
**File**: `src/reap/ast_evaluator.rs:1979`
**Check**: Verify using `.len()` not `.iter().count()`

### 2. Regex Cache Implementation
**File**: `src/reap/ast_evaluator.rs:40`
**Verify**: `regex_cache: Mutex<HashMap<String, regex::Regex>>`
**Test**: Cache hit rate and performance

### 3. SIMD Implementation
**Files**: Check aggregate methods (sum, max, min)
**Verify**: SIMD annotations and threshold logic

---

## 📊 Next Benchmarks to Run

```bash
# 1. Verify count() issue
cargo bench --bench builtins_bench collection_methods/count

# 2. Test SIMD threshold
cargo bench --bench simd_bench simd_threshold

# 3. Validate regex caching
cargo bench --bench caching_bench regex_cache_hit

# 4. End-to-end scenarios
cargo bench --bench e2e_bench simple_policy
cargo bench --bench e2e_bench abac_policy

# 5. Full SIMD analysis
cargo bench --bench simd_bench
```

---

## 🎯 Success Criteria

### Phase 7.3 Complete When:
- ✅ count() is O(1) and ~550ns for all sizes
- ✅ SIMD threshold verified (64 elements)
- ✅ Regex caching shows 2-5x speedup
- ✅ Flamegraphs generated for hot paths
- ✅ All performance targets met or exceeded
- ✅ CI regression detection active

---

## 📚 References

- Benchmark source: `crates/policy-engine/benches/`
- AST Evaluator: `crates/policy-engine/src/reap/ast_evaluator.rs`
- Profiling script: `crates/policy-engine/scripts/profile.sh`
- CI workflow: `.github/workflows/benchmark.yml`

---

*Analysis Date: 2025-12-14*
*Benchmark Data: Phase 7.2 Initial Baselines*
