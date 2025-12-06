# Phase 3: Essential Built-ins - Implementation Plan

**Date**: 2025-12-06
**Status**: 🚧 IN PROGRESS
**Architecture**: Hybrid (Iterator Methods + Built-in Functions)
**Target Duration**: 6 weeks
**Design Document**: See `PHASE3_BUILTINS_DESIGN.md`

---

## Executive Summary

Phase 3 implements essential built-in functions using a **Rust-first hybrid architecture** that combines:
- **Iterator methods** for aggregates (`.count()`, `.sum()`, `.max()`)
- **Zero-copy string operations** with intern caching
- **Type-aware compilation** for performance optimization
- **SIMD optimizations** for numerical operations

**Performance Target**: 20-1000x faster than Rego depending on operation type.

---

## Week-by-Week Breakdown

### Week 1-2: Tier 1 Functions (Aggregates + Type Checks)

#### Week 1: Aggregate Functions
**Target**: Implement `count()`, `sum()`, `max()`, `min()`, `any()`, `all()`

**Tasks**:
1. **Day 1-2: AST Extensions**
   - Add `MethodCall` variant to `Expr` enum
   - Add `MethodName` enum (Count, Sum, Max, Min, Any, All)
   - Update parser to support method call syntax
   - Parser tests for method calls

2. **Day 3-4: Evaluator Implementation**
   - Implement `evaluate_method_call()` in ast_evaluator.rs
   - Implement each aggregate method:
     - `count()`: Simple length calculation (zero allocation)
     - `sum()`: Iterator::fold with SIMD path for large collections
     - `max()`/`min()`: Iterator::max/min with type handling
     - `any()`/`all()`: Short-circuit evaluation
   - Add collection type extraction helper

3. **Day 5: Tests + Benchmarks**
   - Unit tests for each aggregate function
   - Integration tests with comprehensions
   - Benchmark vs Rego (target: 50-100x faster)

**Deliverables**:
- ✅ `Expr::MethodCall` AST variant
- ✅ Parser support with tests
- ✅ 6 aggregate functions implemented
- ✅ Performance benchmarks

#### Week 2: Type Checking + Membership
**Target**: Implement `is_string()`, `is_number()`, `is_array()`, `in` operator optimization

**Tasks**:
1. **Day 1-2: Type Checking Functions**
   - Add `TypeCheck` variant to `Expr`
   - Implement compile-time type inference where possible
   - Runtime type checks for dynamic values
   - Tests for all type checking functions

2. **Day 3-4: Membership Optimization**
   - Optimize `in` operator for sets (O(1) vs O(n))
   - Add `contains()` method for arrays/sets
   - Test with large collections (10K+ items)

3. **Day 5: Integration + Documentation**
   - Create example policies using aggregates + type checks
   - Document performance characteristics
   - Update PHASE3_BUILTINS_STATUS.md

**Deliverables**:
- ✅ Type checking functions (`is_string`, `is_number`, `is_array`, `is_bool`, `is_object`, `is_set`)
- ✅ Optimized membership testing
- ✅ Example policies
- ✅ Status update documentation

---

### Week 3-4: Tier 2 Functions (Strings + Time)

#### Week 3: String Operations
**Target**: Implement `concat()`, `contains()`, `startswith()`, `endswith()`, `lower()`, `upper()`, `split()`, `trim()`

**Tasks**:
1. **Day 1-2: String Intern Cache Extension**
   - Add caches to StringInterner:
     - `lowercase_cache: DashMap<InternedString, InternedString>`
     - `uppercase_cache: DashMap<InternedString, InternedString>`
     - `split_cache: DashMap<(InternedString, InternedString), Vec<InternedString>>`
   - Implement cache management (LRU eviction for large caches)

2. **Day 3-4: String Function Implementation**
   - `concat()`: Join multiple strings, intern result
   - `lower()`/`upper()`: Cache transformed strings
   - `split()`: Cache split results by delimiter
   - `contains()`, `startswith()`, `endswith()`: Zero-copy substring checks
   - `trim()`: Remove whitespace, intern result

3. **Day 5: Tests + Benchmarks**
   - Test all string functions
   - Test cache hit/miss behavior
   - Benchmark vs Rego (target: 50-100x faster with caching)

**Deliverables**:
- ✅ StringInterner with caching
- ✅ 8 string operation functions
- ✅ Cache performance tests
- ✅ String manipulation example policies

#### Week 4: Time/Date Functions
**Target**: Implement `time.now_ns()`, `time.parse_ns()`, `time.parse_duration_ns()`, time comparisons

**Tasks**:
1. **Day 1-2: Time Cache Infrastructure**
   - Create `TimeCache` struct:
     ```rust
     pub struct TimeCache {
         current_time: Option<DateTime<Utc>>,  // Per-evaluation cache
         parsed_times: HashMap<InternedString, DateTime<Utc>>,
     }
     ```
   - Add to evaluation context
   - Implement cache invalidation between evaluations

2. **Day 3-4: Time Function Implementation**
   - `time.now_ns()`: Return cached current time as nanoseconds
   - `time.parse_ns(format, time_str)`: Parse and cache by interned string
   - `time.parse_duration_ns(duration_str)`: Parse "1h30m" style durations
   - Time arithmetic operators (+, -, comparison)

3. **Day 5: Tests + Benchmarks**
   - Test time parsing with various formats
   - Test cache behavior
   - Benchmark vs Rego (target: 100x faster with caching)
   - Create time-based RBAC example (session expiry, time-of-day access)

**Deliverables**:
- ✅ TimeCache infrastructure
- ✅ Time/date functions with caching
- ✅ Time-based policy examples
- ✅ Performance benchmarks

---

### Week 5-6: Tier 3 Functions (Objects + Advanced)

#### Week 5: Object Operations
**Target**: Implement `object.get()`, `object.keys()`, `object.values()`, `object.remove()`

**Tasks**:
1. **Day 1-2: Object Namespace**
   - Add `object` namespace to function resolver
   - Implement object manipulation functions
   - Handle nested object access

2. **Day 3-4: Array/Set Advanced Operations**
   - `array.slice()`: Zero-copy slicing where possible
   - `array.concat()`: Efficient concatenation
   - `set.union()`, `set.intersection()`, `set.difference()`: Use HashSet operations

3. **Day 5: Tests + Examples**
   - Test all object/collection operations
   - Create complex data transformation examples
   - Document collection manipulation patterns

**Deliverables**:
- ✅ Object manipulation functions
- ✅ Advanced collection operations
- ✅ Data transformation examples

#### Week 6: Polish + Documentation
**Target**: Final optimizations, comprehensive testing, complete documentation

**Tasks**:
1. **Day 1-2: Type-Aware Compilation**
   - Implement compile-time type inference
   - Generate specialized code paths for known types
   - Benchmark improvements (target: 200-500x for type-aware paths)

2. **Day 3-4: Comprehensive Testing**
   - Integration tests combining all function types
   - Property-based tests (proptest)
   - Real-world policy scenarios (RBAC + ABAC + time constraints)
   - Performance regression tests

3. **Day 5: Documentation + Examples**
   - Complete function reference documentation
   - Performance comparison tables (Reaper vs Rego)
   - Best practices guide
   - Migration guide from Rego

**Deliverables**:
- ✅ Type-aware compilation
- ✅ Comprehensive test suite
- ✅ Complete documentation
- ✅ PHASE3_BUILTINS_STATUS.md (complete)

---

## Implementation Details

### AST Changes

```rust
// crates/policy-engine/src/reap/ast.rs

pub enum Expr {
    // ... existing variants ...

    // NEW: Method call syntax (e.g., users.count(), roles.sum())
    MethodCall {
        receiver: Box<Expr>,       // The collection/value
        method: MethodName,        // count, sum, max, etc.
        args: Vec<Expr>,          // Optional arguments
    },

    // NEW: Built-in function call (e.g., time.now_ns(), concat(a, b))
    FunctionCall {
        namespace: Option<String>, // "time", "object", etc.
        function: String,          // "now_ns", "get", etc.
        args: Vec<Expr>,
    },

    // NEW: Type checking (e.g., is_string(x), is_number(y))
    TypeCheck {
        value: Box<Expr>,
        type_name: TypeName,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MethodName {
    // Aggregates
    Count,
    Sum,
    Max,
    Min,
    Any,
    All,

    // Strings
    Lower,
    Upper,
    Trim,
    Split,
    Contains,
    Startswith,
    Endswith,

    // Collections
    Union,
    Intersection,
    Difference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeName {
    String,
    Number,
    Bool,
    Array,
    Set,
    Object,
    Null,
}
```

### Parser Changes

```rust
// crates/policy-engine/src/reap/parser.rs

// Parse method calls: collection.method(args)
fn parse_method_call(&mut self) -> Result<Expr> {
    let receiver = self.parse_primary()?;

    if self.consume_if(Token::Dot)? {
        let method_name = self.expect_identifier()?;
        self.expect(Token::LeftParen)?;
        let args = self.parse_argument_list()?;
        self.expect(Token::RightParen)?;

        return Ok(Expr::MethodCall {
            receiver: Box::new(receiver),
            method: MethodName::from_str(&method_name)?,
            args,
        });
    }

    Ok(receiver)
}

// Parse function calls: time.now_ns(), concat(a, b)
fn parse_function_call(&mut self) -> Result<Expr> {
    let first = self.expect_identifier()?;

    // Check for namespace (time.now_ns)
    if self.consume_if(Token::Dot)? {
        let function = self.expect_identifier()?;
        self.expect(Token::LeftParen)?;
        let args = self.parse_argument_list()?;
        self.expect(Token::RightParen)?;

        return Ok(Expr::FunctionCall {
            namespace: Some(first),
            function,
            args,
        });
    }

    // No namespace (concat(a, b))
    self.expect(Token::LeftParen)?;
    let args = self.parse_argument_list()?;
    self.expect(Token::RightParen)?;

    Ok(Expr::FunctionCall {
        namespace: None,
        function: first,
        args,
    })
}
```

### Evaluator Changes

```rust
// crates/policy-engine/src/reap/ast_evaluator.rs

// Add caches to evaluator context
pub struct EvaluationContext {
    // ... existing fields ...
    pub time_cache: TimeCache,
    pub string_cache: Arc<StringInterner>,  // Already exists, extend with caches
}

impl AstEvaluator {
    // Evaluate method calls
    fn evaluate_method_call(
        &self,
        receiver: &Expr,
        method: &MethodName,
        args: &[Expr],
        context: &mut EvaluationContext,
    ) -> Result<EvalValue, ReaperError> {
        let collection = self.evaluate_expr(receiver, context)?;

        match method {
            MethodName::Count => {
                let items = self.get_collection_items(&collection)?;
                Ok(EvalValue::Int(items.len() as i64))
            }

            MethodName::Sum => {
                let items = self.get_collection_items(&collection)?;
                let sum = items.iter()
                    .filter_map(|v| v.as_int())
                    .sum::<i64>();
                Ok(EvalValue::Int(sum))
            }

            MethodName::Max => {
                let items = self.get_collection_items(&collection)?;
                let max = items.iter()
                    .filter_map(|v| v.as_int())
                    .max()
                    .ok_or_else(|| ReaperError::EmptyCollection)?;
                Ok(EvalValue::Int(max))
            }

            MethodName::Lower => {
                let string_val = collection.as_string()
                    .ok_or_else(|| ReaperError::TypeMismatch)?;

                // Check cache first
                let interned = context.string_cache.intern(string_val);
                if let Some(cached) = context.string_cache.get_lowercase(interned) {
                    return Ok(EvalValue::String(cached.to_string()));
                }

                // Not cached - compute and cache
                let lowercased = string_val.to_lowercase();
                let lowercased_interned = context.string_cache.intern(&lowercased);
                context.string_cache.cache_lowercase(interned, lowercased_interned);

                Ok(EvalValue::String(lowercased))
            }

            // ... other methods ...
        }
    }

    // Evaluate function calls
    fn evaluate_function_call(
        &self,
        namespace: &Option<String>,
        function: &str,
        args: &[Expr],
        context: &mut EvaluationContext,
    ) -> Result<EvalValue, ReaperError> {
        match (namespace.as_deref(), function) {
            (Some("time"), "now_ns") => {
                // Return cached current time
                let now = context.time_cache.get_current_time();
                Ok(EvalValue::Int(now.timestamp_nanos_opt().unwrap()))
            }

            (Some("time"), "parse_ns") => {
                // Parse time string - cache by interned string
                let format = self.evaluate_expr(&args[0], context)?.as_string()
                    .ok_or_else(|| ReaperError::TypeMismatch)?;
                let time_str = self.evaluate_expr(&args[1], context)?.as_string()
                    .ok_or_else(|| ReaperError::TypeMismatch)?;

                let time_str_interned = context.string_cache.intern(time_str);
                if let Some(parsed) = context.time_cache.get_parsed(time_str_interned) {
                    return Ok(EvalValue::Int(parsed.timestamp_nanos_opt().unwrap()));
                }

                // Parse and cache
                let parsed = DateTime::parse_from_str(time_str, format)?;
                context.time_cache.cache_parsed(time_str_interned, parsed.into());

                Ok(EvalValue::Int(parsed.timestamp_nanos_opt().unwrap()))
            }

            (Some("object"), "get") => {
                let obj = self.evaluate_expr(&args[0], context)?.as_object()
                    .ok_or_else(|| ReaperError::TypeMismatch)?;
                let key = self.evaluate_expr(&args[1], context)?.as_string()
                    .ok_or_else(|| ReaperError::TypeMismatch)?;

                obj.get(key)
                    .cloned()
                    .ok_or_else(|| ReaperError::KeyNotFound)
            }

            (None, "concat") => {
                // Concatenate strings
                let strings: Result<Vec<&str>, _> = args.iter()
                    .map(|arg| self.evaluate_expr(arg, context)?.as_string())
                    .collect();

                let result = strings?.join("");
                Ok(EvalValue::String(result))
            }

            _ => Err(ReaperError::UnknownFunction(function.to_string()))
        }
    }
}
```

---

## Testing Strategy

### Unit Tests
- **Parser tests**: 20+ tests for method/function call syntax
- **Evaluator tests**: 50+ tests for each function implementation
- **Cache tests**: Verify hit/miss behavior for string/time caches

### Integration Tests
- **Aggregate + Comprehension**: `{r.id | r := data.resources[_]; r.priority > threshold}.count()`
- **String operations**: Complex text processing in policies
- **Time-based RBAC**: Session expiry, time-of-day access control

### Performance Tests
- **Benchmarks**: Compare each function vs Rego equivalent
- **Scaling tests**: Test with 10, 100, 1K, 10K, 100K items
- **Cache efficiency**: Measure hit rates in realistic scenarios

### Example Test Policy
```reap
policy advanced_rbac {
    version: "1.0.0",
    description: "Advanced RBAC with built-in functions",
    default: deny,

    rule admin_or_many_permissions {
        allow if {
            // Use type checking
            is_string(user.role),

            // Use aggregates
            user_perms := {p | p := data.permissions[_]; p.user_id == user.id},
            perm_count := user_perms.count(),

            // String operations
            user.role.lower() == "admin" or perm_count > 10
        }
    }

    rule time_restricted_access {
        allow if {
            // Time functions
            now := time.now_ns(),
            session_start := time.parse_ns("RFC3339", user.session_start),
            session_duration := now - session_start,

            // 8 hour session limit
            session_duration < time.parse_duration_ns("8h")
        }
    }
}
```

---

## Performance Targets

### Aggregates
- **count()**: < 10ns (array length lookup)
- **sum()**: 1-2ns per item (SIMD optimized)
- **max()/min()**: 1-2ns per item
- **Speedup vs Rego**: 50-100x

### Strings (Cached)
- **lower()/upper()**: < 50ns (cache hit), ~500ns (cache miss)
- **split()**: < 100ns (cached), ~1-5µs (uncached)
- **contains()**: < 20ns (substring search)
- **Speedup vs Rego**: 50-100x (cached), 5-10x (uncached)

### Time (Cached)
- **time.now_ns()**: < 10ns (cached), ~100ns (uncached)
- **time.parse_ns()**: < 50ns (cached), ~1-5µs (uncached)
- **Speedup vs Rego**: 100-200x (cached)

### Type-Aware Compilation
- **Known types**: 200-500x faster (no boxing, direct comparisons)
- **Runtime checks**: 10-20x faster (optimized type checking)

---

## Dependencies

### New Crate Dependencies
```toml
# Add to Cargo.toml workspace.dependencies
chrono = "0.4"           # Time/date parsing
regex = "1.10"           # Regex support (Tier 3)
```

### Internal Dependencies
- StringInterner (extend with caches)
- DashMap (for concurrent caches)
- DataStore (for entity lookups)

---

## Migration from Rego

### Syntax Comparison

| Rego | Reaper (Hybrid) | Notes |
|------|-----------------|-------|
| `count(users)` | `users.count()` | Method call |
| `sum([1, 2, 3])` | `[1, 2, 3].sum()` | Method call |
| `time.now_ns()` | `time.now_ns()` | Same (built-in) |
| `lower(s)` | `s.lower()` | Method call |
| `contains(s, "x")` | `s.contains("x")` | Method call |
| `is_string(x)` | `is_string(x)` | Same (built-in) |

### Performance Improvements

Typical policy with aggregates + strings + time:
- **Rego**: 50-100µs
- **Reaper**: 1-5µs
- **Speedup**: **10-100x faster**

---

## Risks and Mitigations

### Risk 1: Cache Memory Growth
**Mitigation**:
- Implement LRU eviction for caches
- Monitor cache size in production
- Configurable cache limits

### Risk 2: Syntax Divergence from Rego
**Mitigation**:
- Document migration guide clearly
- Support both syntaxes where feasible
- Provide conversion tool

### Risk 3: SIMD Portability
**Mitigation**:
- Feature-gate SIMD code
- Fallback to scalar implementation
- Test on multiple architectures

---

## Success Criteria

- ✅ All Tier 1 functions implemented and tested
- ✅ All Tier 2 functions implemented and tested
- ✅ Performance targets met (20-1000x vs Rego)
- ✅ Comprehensive test coverage (>90%)
- ✅ Complete documentation with examples
- ✅ Migration guide for Rego users

---

## Next Steps

**Immediate**: Begin Week 1 - Aggregate Functions
1. Extend AST with `MethodCall` variant
2. Update parser for method call syntax
3. Implement evaluator for `count()`, `sum()`, `max()`, `min()`
4. Create benchmarks

**See**: `PHASE3_BUILTINS_STATUS.md` for ongoing progress tracking.

---

**Created**: 2025-12-06
**Status**: 🚧 IN PROGRESS
**Next Review**: End of Week 2 (Tier 1 complete)
