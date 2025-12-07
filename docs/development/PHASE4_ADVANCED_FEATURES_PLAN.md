# Phase 4: Advanced Features - Implementation Plan

**Status**: 🚧 IN PROGRESS
**Start Date**: 2025-12-07
**Target Duration**: 2-3 weeks
**Priority**: Expand Reaper functionality before comprehensive integration testing

---

## Strategy

Build out advanced features to make Reaper feature-complete before writing comprehensive integration tests. This approach ensures:
- Integration tests cover ALL features (not just Tier 1)
- Avoid rewriting tests as new features are added
- More realistic end-to-end scenarios testing feature interactions

**Integration Testing**: Deferred until Phase 4 complete. Will include comprehensive behavior-driven tests covering:
- All built-in functions (Tier 1 + Tier 2)
- Feature interactions (e.g., time checks + string methods + comprehensions)
- Real-world policy scenarios at scale
- Performance benchmarks with full feature set

---

## Phase 4 Features

### 1. Time/Date Functions (Tier 2 Built-ins) - PRIORITY 1
**Duration**: 3-5 days
**Importance**: Critical for temporal policies

**Functions to Implement**:
```reap
// Current time
time::now_ns() -> Integer          // Nanoseconds since Unix epoch
time::now_ms() -> Integer          // Milliseconds since Unix epoch
time::now() -> Integer             // Seconds since Unix epoch

// Parsing
time::parse_rfc3339(s) -> Integer  // Parse ISO 8601 string to ns
time::parse_ns(s, format) -> Integer

// Formatting
time::format_rfc3339(ns) -> String
time::format(ns, format) -> String

// Arithmetic
time::add_ns(ns, duration) -> Integer
time::subtract_ns(ns, duration) -> Integer

// Comparison helpers
time::is_before(t1, t2) -> Boolean
time::is_after(t1, t2) -> Boolean
time::is_between(t, start, end) -> Boolean
```

**Use Cases**:
- Token expiration: `time::now_ns() < user.token_expires_at`
- Time windows: `time::is_between(time::now(), start, end)`
- Age verification: `time::now() - user.birthdate > age_threshold`

**Dependencies**: Use `chrono` crate (already in dependencies)

---

### 2. Regex Support - PRIORITY 2
**Duration**: 2-3 days
**Importance**: High - essential for pattern validation

**Functions to Implement**:
```reap
// String methods
s.matches(pattern) -> Boolean       // Test if string matches regex
s.find(pattern) -> String?          // Find first match
s.find_all(pattern) -> Array        // Find all matches
s.replace(pattern, replacement) -> String
s.replace_all(pattern, replacement) -> String

// Regex namespace
regex::is_valid(pattern) -> Boolean // Check if regex compiles
regex::escape(s) -> String          // Escape special chars
```

**Use Cases**:
- Email validation: `user.email.matches(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")`
- Phone validation: `user.phone.matches(r"^\+?[1-9]\d{1,14}$")`
- Pattern extraction: `user.role.find(r"admin_(\w+)")`

**Dependencies**: Use `regex` crate (already in dependencies)

---

### 3. String Intern Caching - PRIORITY 3
**Duration**: 2-3 days
**Importance**: Medium - performance optimization

**Approach**:
1. Add `StringInterner` to evaluator context
2. Cache results of `lower()`, `upper()`, `split()` by input string ID
3. Use reference counting to manage cache size

**Expected Speedup**: 2-5x for repeated string operations

**Example**:
```rust
// First call: computes and caches
"ADMIN".lower() -> "admin" (cached with key "ADMIN")

// Subsequent calls: retrieves from cache
"ADMIN".lower() -> "admin" (cache hit, ~1ns)
```

**Implementation**:
- Add `cache: HashMap<String, CachedValue>` to evaluator
- Check cache before computation
- Limit cache size (LRU eviction)

---

### 4. Math Functions - PRIORITY 2
**Duration**: 1-2 days
**Importance**: Medium-High for numeric policies

**Functions to Implement**:
```reap
// Math namespace
math::abs(n) -> Number           // Absolute value
math::round(n) -> Integer        // Round to nearest integer
math::floor(n) -> Integer        // Round down
math::ceil(n) -> Integer         // Round up
math::pow(base, exp) -> Number   // Exponentiation
math::sqrt(n) -> Float           // Square root
math::log(n) -> Float            // Natural logarithm
math::log10(n) -> Float          // Base-10 logarithm

// Min/max (already have as methods, add as functions)
math::min(a, b) -> Number
math::max(a, b) -> Number
math::clamp(n, min, max) -> Number
```

**Use Cases**:
- Budget calculations: `math::round(total * tax_rate)`
- Distance calculations: `math::sqrt(dx*dx + dy*dy)`
- Score normalization: `math::clamp(score, 0, 100)`

---

### 5. Advanced Collection Operations - PRIORITY 3
**Duration**: 2-3 days
**Importance**: Medium for complex policies

**Functions to Implement**:
```reap
// Array methods
arr.length() -> Integer          // Alias for count()
arr.first() -> Value?            // First element or null
arr.last() -> Value?             // Last element or null
arr.contains(value) -> Boolean   // Check membership
arr.index_of(value) -> Integer?  // Find index of value
arr.slice(start, end) -> Array   // Extract subarray
arr.reverse() -> Array           // Reverse order
arr.sort() -> Array              // Sort ascending
arr.unique() -> Set              // Remove duplicates

// Set methods (already have union/intersection/difference)
set.is_empty() -> Boolean
set.size() -> Integer            // Alias for count()

// Object methods
obj.keys() -> Array              // Get all keys
obj.values() -> Array            // Get all values
obj.has_key(key) -> Boolean      // Check if key exists
```

**Use Cases**:
- Permission ordering: `perms.sort().first()`
- Deduplication: `roles.unique().count()`
- Key validation: `user.metadata.has_key("verified")`

---

### 6. JSON/Object Manipulation - PRIORITY 3
**Duration**: 2-3 days
**Importance**: Medium for dynamic policies

**Functions to Implement**:
```reap
// JSON parsing/serialization
json::parse(s) -> Object         // Parse JSON string to object
json::stringify(obj) -> String   // Serialize object to JSON
json::is_valid(s) -> Boolean     // Check if string is valid JSON

// Object merging
obj.merge(other) -> Object       // Shallow merge
obj.deep_merge(other) -> Object  // Deep merge
```

**Use Cases**:
- Dynamic config: `config := json::parse(user.preferences)`
- Metadata merging: `all_data := user_data.merge(role_data)`

---

### 7. SIMD Aggregates (Optional) - PRIORITY 4
**Duration**: 3-4 days
**Importance**: Low - micro-optimization

**Approach**: Use SIMD for `sum()` on large numeric arrays (10K+ items)
**Expected Speedup**: 4-8x for very large arrays
**Trade-off**: Complexity vs benefit - may defer to Phase 5

---

## Implementation Order

### Week 1 (Days 1-5)
1. **Day 1-2**: Time/Date Functions
   - Implement core time functions
   - Add parser support for `time::` namespace
   - Unit tests for time functions

2. **Day 3-4**: Regex Support
   - Implement regex methods
   - Add parser support for `.matches()`, `.replace()`
   - Unit tests for regex functions

3. **Day 5**: Math Functions
   - Implement math namespace
   - Add parser support
   - Unit tests

### Week 2 (Days 6-10)
4. **Day 6-7**: Advanced Collection Operations
   - Implement array/set/object methods
   - Parser support
   - Unit tests

5. **Day 8-9**: JSON/Object Manipulation
   - Implement JSON namespace
   - Parser support
   - Unit tests

6. **Day 10**: String Intern Caching
   - Add caching layer to evaluator
   - Performance benchmarks

### Week 3 (Days 11-14)
7. **Day 11-12**: Integration & Polish
   - Fix any issues discovered
   - Update documentation
   - Performance profiling

8. **Day 13-14**: Comprehensive Integration Tests
   - End-to-end scenarios with ALL features
   - Real-world policy examples
   - Performance benchmarks
   - Behavior-driven tests

---

## Success Metrics

By end of Phase 4, Reaper should have:
- ✅ **~50 total built-in functions** (23 Tier 1 + ~27 Tier 2)
- ✅ **Time/date support** for temporal policies
- ✅ **Regex validation** for pattern matching
- ✅ **Math operations** for numeric policies
- ✅ **Advanced collections** for complex data manipulation
- ✅ **JSON support** for dynamic policies
- ✅ **Performance caching** for repeated operations
- ✅ **Comprehensive integration tests** covering all features

**Target Performance** (after optimizations):
- Simple policies: < 1µs
- Complex policies (all features): < 10µs
- Very complex policies: < 100µs

---

## Risk Assessment

**Low Risk**:
- Time/date functions (well-understood, chrono crate mature)
- Math functions (straightforward implementations)

**Medium Risk**:
- Regex support (performance implications of regex compilation)
- String caching (cache invalidation strategy)

**Mitigation**:
- Regex: Pre-compile patterns where possible, add compilation limits
- Caching: Use LRU with size limits, make it optional

---

**Next Step**: Start with Time/Date Functions (highest priority, most requested feature)
