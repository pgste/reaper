# Phase 2: Comprehensions - Design Document

**Status:** 🚧 In Progress
**Target:** Rego-compatible comprehensions for data transformation

---

## Overview

Comprehensions are a powerful feature in Rego that allow **collecting and transforming data** from collections in a single expression. This phase adds three types of comprehensions:

1. **Set Comprehensions**: `{expr | iteration; filters}`
2. **Array Comprehensions**: `[expr | iteration; filters]`
3. **Object Comprehensions**: `{key: value | iteration; filters}`

---

## Rego Syntax Examples

### Set Comprehension
```rego
# Collect names of all admin users
admin_names := {u.name | u := data.users[_]; "admin" in u.roles}

# Result: {"alice", "bob", "carol"}  (unique values, unordered)
```

### Array Comprehension
```rego
# Collect email addresses of all users
user_emails := [u.email | u := data.users[_]]

# Result: ["alice@example.com", "bob@example.com"]  (ordered, may have duplicates)
```

### Object Comprehension
```rego
# Create user ID -> name mapping
user_map := {u.id: u.name | u := data.users[_]}

# Result: {"user-1": "alice", "user-2": "bob"}  (key-value pairs)
```

### Complex Filters
```rego
# Collect senior developer emails
senior_dev_emails := [u.email |
    u := data.users[_];
    "developer" in u.roles;
    u.years_experience >= 5
]
```

---

## Design Decisions

### 1. Syntax Choice

**Option A: Rego-compatible (semicolon separator)**
```reap
{u.name | u := users[_]; "admin" in u.roles}
```

**Option B: Reaper-style (comma separator)**
```reap
{u.name | u := users[_], "admin" in u.roles}
```

**Decision:** Use **semicolon separator** for Rego compatibility.

**Rationale:**
- Exact syntax match with Rego
- Semicolons clearly separate iteration from filters
- Minimal learning curve for Rego users

### 2. AST Structure

```rust
/// Comprehension expression
pub enum Comprehension {
    /// Set comprehension: {expr | iteration; filters}
    Set {
        output: Box<Expr>,
        iterator: Iterator,
        filters: Vec<Condition>,
    },

    /// Array comprehension: [expr | iteration; filters]
    Array {
        output: Box<Expr>,
        iterator: Iterator,
        filters: Vec<Condition>,
    },

    /// Object comprehension: {key: value | iteration; filters}
    Object {
        key: Box<Expr>,
        value: Box<Expr>,
        iterator: Iterator,
        filters: Vec<Condition>,
    },
}

/// Iterator specification
pub struct Iterator {
    /// Variable name to bind each element to
    pub variable: String,

    /// Collection to iterate over
    pub collection: EntityAttr,
}

/// Expression type for output
pub enum Expr {
    /// Literal value: "admin", 42, true
    Literal(Value),

    /// Variable reference: u.name, role
    Variable(String),

    /// Attribute access: u.name, user.role
    AttributeAccess {
        variable: String,
        attribute: String,
    },

    /// Indexed access: u.roles[0]
    IndexedAccess {
        variable: String,
        attribute: String,
        index: Index,
    },
}
```

### 3. Grammar Design

```pest
// Comprehensions
set_comprehension = { "{" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)* ~ "}" }
array_comprehension = { "[" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)* ~ "]" }
object_comprehension = { "{" ~ expr ~ ":" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)* ~ "}" }

// Iterator
iterator = { ident ~ ":=" ~ entity_attr }

// Expression
expr = {
    literal_value |
    attribute_access |
    indexed_access |
    variable_ref
}

attribute_access = { ident ~ "." ~ ident }
variable_ref = { ident }
literal_value = { string | integer | float | boolean }
```

### 4. Evaluation Strategy

#### Set Comprehension Evaluation

```rust
fn evaluate_set_comprehension(
    output: &Expr,
    iterator: &Iterator,
    filters: &[Condition],
    context: &EvaluationContext,
) -> HashSet<AttributeValue> {
    let mut result = HashSet::new();

    // Get collection to iterate over
    let collection = get_collection(&iterator.collection, context);

    // Iterate over each element
    for element in collection.iter() {
        // Bind element to iterator variable
        let mut iteration_context = context.clone();
        iteration_context.set_variable(&iterator.variable, element.clone());

        // Evaluate filters
        let all_filters_pass = filters.iter().all(|filter| {
            evaluate_condition(filter, &iteration_context)
        });

        if all_filters_pass {
            // Evaluate output expression and add to set
            let output_value = evaluate_expr(output, &iteration_context);
            result.insert(output_value);
        }
    }

    result
}
```

**Performance Characteristics:**
- **Time Complexity:** O(n * f) where n = collection size, f = filter complexity
- **Space Complexity:** O(m) where m = output set size
- **Early Filtering:** Filters evaluated lazily, stops at first failure
- **Set Deduplication:** Automatic via HashSet

#### Array Comprehension Evaluation

Similar to set comprehension but uses `Vec` instead of `HashSet`:
- Preserves order
- Allows duplicates
- Faster insertion (no hashing)

#### Object Comprehension Evaluation

```rust
fn evaluate_object_comprehension(
    key_expr: &Expr,
    value_expr: &Expr,
    iterator: &Iterator,
    filters: &[Condition],
    context: &EvaluationContext,
) -> HashMap<AttributeValue, AttributeValue> {
    let mut result = HashMap::new();

    for element in collection.iter() {
        let mut iteration_context = context.clone();
        iteration_context.set_variable(&iterator.variable, element.clone());

        if filters.iter().all(|f| evaluate_condition(f, &iteration_context)) {
            let key = evaluate_expr(key_expr, &iteration_context);
            let value = evaluate_expr(value_expr, &iteration_context);
            result.insert(key, value);
        }
    }

    result
}
```

### 5. Performance Optimization

#### Optimization 1: Early Filter Evaluation
```rust
// Evaluate cheapest filters first
filters.sort_by_key(|f| filter_cost(f));

// Stop at first filter failure
let pass = filters.iter().all(|f| evaluate_condition(f, ctx));
```

#### Optimization 2: Pre-allocate Collections
```rust
// Pre-allocate with capacity hint
let mut result = HashSet::with_capacity(collection.len());
```

#### Optimization 3: Inline Small Comprehensions
```rust
// For small collections (< 10 elements), inline evaluation
if collection.len() < 10 {
    return inline_evaluate_comprehension(...);
}
```

#### Optimization 4: Parallel Evaluation (Future)
```rust
// For large collections (> 1000 elements), use rayon
use rayon::prelude::*;
collection.par_iter()
    .filter(|elem| filters_pass(elem))
    .map(|elem| evaluate_output(elem))
    .collect()
```

---

## Implementation Plan

### Phase 2.1: AST Extensions
- Add `Comprehension` enum to AST
- Add `Expr` enum for output expressions
- Add `Iterator` struct
- Update `Value` to support comprehension results

### Phase 2.2: Grammar Updates
- Add set comprehension rule
- Add array comprehension rule
- Add object comprehension rule
- Add iterator rule
- Add expression rules

### Phase 2.3: Parser Implementation
- Implement `parse_set_comprehension()`
- Implement `parse_array_comprehension()`
- Implement `parse_object_comprehension()`
- Implement `parse_iterator()`
- Implement `parse_expr()`

### Phase 2.4: Evaluator Implementation
- Add comprehension evaluation to ReaperDSLEvaluator
- Implement `evaluate_expr()`
- Implement `evaluate_set_comprehension()`
- Implement `evaluate_array_comprehension()`
- Implement `evaluate_object_comprehension()`
- Add iteration context management

### Phase 2.5: Testing
- Unit tests for each comprehension type
- Tests with filters
- Tests with complex expressions
- Tests with nested attribute access
- Performance benchmarks

### Phase 2.6: Documentation & Examples
- Example policies
- Performance analysis
- Completion documentation

---

## Use Cases

### Use Case 1: RBAC Role Collection
```reap
policy rbac_with_comprehensions {
    version: "1.0.0",
    default: deny,

    rule admin_users {
        // Collect all admin usernames
        admin_names := {u.name | u := data.users[_]; "admin" in u.roles}

        allow if user.name in admin_names
    }
}
```

### Use Case 2: Permission Aggregation
```reap
policy permission_aggregation {
    version: "1.0.0",
    default: deny,

    rule aggregate_permissions {
        // Collect all unique permissions from all user roles
        all_perms := {perm |
            role := user.roles[_];
            perm := data.role_permissions[role][_]
        }

        allow if context.action in all_perms
    }
}
```

### Use Case 3: Resource Filtering
```reap
policy resource_filter {
    version: "1.0.0",
    default: deny,

    rule accessible_resources {
        // Get list of all accessible resource IDs
        accessible := [r.id |
            r := data.resources[_];
            r.owner == user.id ||
            user.id in r.shared_with
        ]

        allow if resource.id in accessible
    }
}
```

### Use Case 4: Attribute Mapping
```reap
policy attribute_mapping {
    version: "1.0.0",
    default: deny,

    rule user_dept_map {
        // Create user ID -> department mapping
        dept_map := {u.id: u.department | u := data.users[_]}

        allow if dept_map[user.id] == resource.department
    }
}
```

---

## Performance Targets

| Comprehension Type | Collection Size | Target Latency |
|-------------------|----------------|----------------|
| Set (small) | 10 elements | < 1 µs |
| Set (medium) | 100 elements | < 10 µs |
| Set (large) | 1000 elements | < 100 µs |
| Array (small) | 10 elements | < 1 µs |
| Array (medium) | 100 elements | < 10 µs |
| Object (small) | 10 elements | < 1 µs |
| Object (medium) | 100 elements | < 10 µs |

**With Filters:**
- Add ~50-100ns per filter evaluation
- Early termination should keep < 2x base latency

---

## Comparison with Rego

| Feature | Rego | Reaper DSL (Target) |
|---------|------|-------------------|
| Set comprehension | ✅ | ✅ Phase 2.1-2.4 |
| Array comprehension | ✅ | ✅ Phase 2.1-2.4 |
| Object comprehension | ✅ | ✅ Phase 2.1-2.4 |
| Nested comprehensions | ✅ | ⚠️ Future (Phase 2.7) |
| Multiple iterators | ✅ `{x | a := ...; b := ...}` | ⚠️ Future (Phase 2.8) |
| Performance | ~100-500 µs | Target: 1-100 µs (5-10x faster) |

---

## Technical Challenges

### Challenge 1: Variable Scoping
**Problem:** Comprehension variables must not leak into outer scope

**Solution:** Create isolated iteration context per comprehension
```rust
let mut iteration_context = context.clone();
iteration_context.enter_comprehension_scope();
// ... evaluate comprehension
iteration_context.exit_comprehension_scope();
```

### Challenge 2: Memory Management
**Problem:** Large comprehensions can allocate significant memory

**Solution:**
- Stream evaluation for large collections (> 1000 elements)
- Early capacity estimation
- Memory limits

### Challenge 3: Performance
**Problem:** Nested iterations can be O(n²) or worse

**Solution:**
- Filter optimization (evaluate cheap filters first)
- Index-based iteration where possible
- Parallel evaluation for large collections (future)

---

## Success Criteria

- ✅ Parse all three comprehension types correctly
- ✅ Evaluate comprehensions with correct semantics
- ✅ Support filters in comprehensions
- ✅ Support complex output expressions
- ✅ Meet performance targets for small/medium collections
- ✅ 100% test coverage
- ✅ Example policies demonstrating all patterns
- ✅ Comprehensive documentation

---

## Next Steps

1. **Implement AST extensions** (Phase 2.1)
2. **Update grammar** (Phase 2.2)
3. **Implement parser** (Phase 2.3)
4. **Implement evaluator** (Phase 2.4)
5. **Add comprehensive tests** (Phase 2.5)
6. **Document and benchmark** (Phase 2.6)

**Estimated Effort:** 3-4 hours for complete implementation
**Risk Level:** Medium (complex feature, but well-defined)
**Dependencies:** Phase 1 complete (variables, wildcards, collections)

---

**Phase 2 Design Document**
**Status:** Ready for implementation
**Next:** Phase 2.1 - AST Extensions
