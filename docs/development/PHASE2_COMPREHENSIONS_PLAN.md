# Phase 2: Comprehensions - Implementation Plan

**Start Date**: 2025-12-06
**Status**: 🚧 IN PROGRESS
**Estimated Duration**: 3-4 weeks
**Performance Target**: < 10µs for 100 iterations

---

## Executive Summary

Phase 2 adds **Rego-compatible comprehensions** to Reaper DSL, enabling powerful collection transformations:
- **Set Comprehensions**: `{x | condition}` - Build unique sets with O(1) deduplication
- **Array Comprehensions**: `[x | condition]` - Build ordered lists
- **Object Comprehensions**: `{key: value | condition}` - Build maps/dictionaries

**Why Critical**: Comprehensions are a **core Rego idiom** - most real-world policies use them for:
- Building permission sets from roles
- Filtering entities by attributes
- Transforming data structures
- Multi-step reasoning (user → roles → permissions)

---

## Design Overview

### Comprehension Syntax

```rego
// Set comprehension - produces unique values
admin_users := {u.name | u := data.users[_]; "admin" in u.roles}

// Array comprehension - preserves order, allows duplicates
admin_names := [u.name | u := data.users[_]; "admin" in u.roles]

// Object comprehension - builds key-value maps
user_map := {u.id: u.name | u := data.users[_]}
```

### Components

```
┌─────────────────────────────────────────────────────────┐
│  Comprehension Expression                               │
├─────────────────────────────────────────────────────────┤
│  {  output_expr  |  iterator  ;  condition  }          │
│                                                         │
│  output_expr:  What to collect (u.name, u.id: u.name) │
│  iterator:     Variable binding (u := data.users[_])   │
│  condition:    Filter (optional, "admin" in u.roles)   │
└─────────────────────────────────────────────────────────┘
```

---

## Phase 2.1: AST Extensions

### New AST Types

**File**: `crates/policy-engine/src/reap/ast.rs`

```rust
/// Comprehension types
#[derive(Debug, Clone, PartialEq)]
pub enum Comprehension {
    /// Set comprehension: {expr | iterator; condition}
    Set {
        output: Box<Expr>,
        iterator: Iterator,
        condition: Option<Box<Condition>>,
    },
    /// Array comprehension: [expr | iterator; condition]
    Array {
        output: Box<Expr>,
        iterator: Iterator,
        condition: Option<Box<Condition>>,
    },
    /// Object comprehension: {key: value | iterator; condition}
    Object {
        key: Box<Expr>,
        value: Box<Expr>,
        iterator: Iterator,
        condition: Option<Box<Condition>>,
    },
}

/// Iterator binding
#[derive(Debug, Clone, PartialEq)]
pub struct Iterator {
    /// Variable name (e.g., "u")
    pub variable: String,
    /// Collection to iterate (e.g., data.users[_])
    pub collection: CollectionExpr,
}

/// Collection expression
#[derive(Debug, Clone, PartialEq)]
pub enum CollectionExpr {
    /// Entity attribute: user.roles, data.users
    EntityAttr(EntityAttr),
    /// Literal array/set/object
    Literal(Value),
    /// Variable reference
    Variable(String),
    /// Nested comprehension
    Comprehension(Box<Comprehension>),
}

/// Expression (for output/key/value)
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal value
    Literal(Value),
    /// Entity attribute access
    EntityAttr(EntityAttr),
    /// Variable reference
    Variable(String),
    /// Object construction: {key: value}
    Object(Vec<(Expr, Expr)>),
    /// Function call (future)
    FunctionCall { name: String, args: Vec<Expr> },
}
```

**Estimated Effort**: 4-6 hours

---

## Phase 2.2: Parser Extensions

### Grammar Updates

**File**: `crates/policy-engine/src/reap.pest`

```pest
// Comprehension expressions
comprehension = {
    set_comprehension |
    array_comprehension |
    object_comprehension
}

set_comprehension = {
    "{" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)? ~ "}"
}

array_comprehension = {
    "[" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)? ~ "]"
}

object_comprehension = {
    "{" ~ expr ~ ":" ~ expr ~ "|" ~ iterator ~ (";" ~ condition)? ~ "}"
}

iterator = {
    ident ~ ":=" ~ collection_expr
}

collection_expr = {
    comprehension |          // Nested comprehension
    entity_attr ~ "[" ~ "_" ~ "]" |  // Wildcard iteration
    entity_attr |            // Direct attribute
    value |                  // Literal
    ident                    // Variable
}

expr = {
    comprehension |
    entity_attr |
    value |
    ident
}
```

### Parser Implementation

**File**: `crates/policy-engine/src/reap/parser.rs`

```rust
fn parse_comprehension(pair: Pair<Rule>) -> Result<Comprehension, ReaperError> {
    match pair.as_rule() {
        Rule::set_comprehension => {
            let mut inner = pair.into_inner();
            let output = Box::new(parse_expr(inner.next().unwrap())?);
            let iterator = parse_iterator(inner.next().unwrap())?;
            let condition = inner.next().map(|p| Box::new(parse_condition(p))).transpose()?;

            Ok(Comprehension::Set {
                output,
                iterator,
                condition,
            })
        }
        Rule::array_comprehension => {
            // Similar to set
        }
        Rule::object_comprehension => {
            let mut inner = pair.into_inner();
            let key = Box::new(parse_expr(inner.next().unwrap())?);
            let value = Box::new(parse_expr(inner.next().unwrap())?);
            let iterator = parse_iterator(inner.next().unwrap())?;
            let condition = inner.next().map(|p| Box::new(parse_condition(p))).transpose()?;

            Ok(Comprehension::Object {
                key,
                value,
                iterator,
                condition,
            })
        }
        _ => Err(ReaperError::ParseError("Invalid comprehension".into()))
    }
}

fn parse_iterator(pair: Pair<Rule>) -> Result<Iterator, ReaperError> {
    let mut inner = pair.into_inner();
    let variable = inner.next().unwrap().as_str().to_string();
    let collection = parse_collection_expr(inner.next().unwrap())?;

    Ok(Iterator {
        variable,
        collection,
    })
}
```

**Estimated Effort**: 8-12 hours (parser is complex)

---

## Phase 2.3: Evaluator Implementation

### Comprehension Evaluation

**File**: `crates/policy-engine/src/evaluators/reaper_dsl.rs`

```rust
impl ReaperDSLEvaluator {
    /// Evaluate a comprehension expression
    fn evaluate_comprehension(
        &self,
        comp: &Comprehension,
        user: &Entity,
        resource: &Entity,
        context: &HashMap<String, String>,
        variables: &mut HashMap<String, AttributeValue>,
    ) -> Result<AttributeValue, ReaperError> {
        match comp {
            Comprehension::Set { output, iterator, condition } => {
                self.evaluate_set_comprehension(output, iterator, condition, user, resource, context, variables)
            }
            Comprehension::Array { output, iterator, condition } => {
                self.evaluate_array_comprehension(output, iterator, condition, user, resource, context, variables)
            }
            Comprehension::Object { key, value, iterator, condition } => {
                self.evaluate_object_comprehension(key, value, iterator, condition, user, resource, context, variables)
            }
        }
    }

    /// Set comprehension - O(n) with HashSet deduplication
    fn evaluate_set_comprehension(
        &self,
        output: &Expr,
        iterator: &Iterator,
        condition: &Option<Box<Condition>>,
        user: &Entity,
        resource: &Entity,
        context: &HashMap<String, String>,
        variables: &mut HashMap<String, AttributeValue>,
    ) -> Result<AttributeValue, ReaperError> {
        let collection = self.evaluate_collection(
            &iterator.collection,
            user,
            resource,
            context,
            variables
        )?;

        // Get items from collection (List, Set, or Object values)
        let items = self.get_collection_items(&collection)?;

        // Pre-allocate with expected capacity
        let mut result_set = HashSet::with_capacity(items.len());

        for item in items {
            // Bind iterator variable
            variables.insert(iterator.variable.clone(), item.clone());

            // Evaluate condition (if present)
            if let Some(cond) = condition {
                if !self.evaluate_condition(cond, user, resource, context, variables)? {
                    continue; // Skip this item
                }
            }

            // Evaluate output expression
            let output_value = self.evaluate_expr(output, user, resource, context, variables)?;
            result_set.insert(output_value);

            // Clear iterator variable for next iteration
            variables.remove(&iterator.variable);
        }

        Ok(AttributeValue::Set(result_set))
    }

    /// Array comprehension - O(n) with ordered collection
    fn evaluate_array_comprehension(
        &self,
        output: &Expr,
        iterator: &Iterator,
        condition: &Option<Box<Condition>>,
        user: &Entity,
        resource: &Entity,
        context: &HashMap<String, String>,
        variables: &mut HashMap<String, AttributeValue>,
    ) -> Result<AttributeValue, ReaperError> {
        let collection = self.evaluate_collection(
            &iterator.collection,
            user,
            resource,
            context,
            variables
        )?;

        let items = self.get_collection_items(&collection)?;
        let mut result_vec = Vec::with_capacity(items.len());

        for item in items {
            variables.insert(iterator.variable.clone(), item.clone());

            if let Some(cond) = condition {
                if !self.evaluate_condition(cond, user, resource, context, variables)? {
                    continue;
                }
            }

            let output_value = self.evaluate_expr(output, user, resource, context, variables)?;
            result_vec.push(output_value);

            variables.remove(&iterator.variable);
        }

        Ok(AttributeValue::List(result_vec))
    }

    /// Object comprehension - O(n) with HashMap construction
    fn evaluate_object_comprehension(
        &self,
        key_expr: &Expr,
        value_expr: &Expr,
        iterator: &Iterator,
        condition: &Option<Box<Condition>>,
        user: &Entity,
        resource: &Entity,
        context: &HashMap<String, String>,
        variables: &mut HashMap<String, AttributeValue>,
    ) -> Result<AttributeValue, ReaperError> {
        let collection = self.evaluate_collection(
            &iterator.collection,
            user,
            resource,
            context,
            variables
        )?;

        let items = self.get_collection_items(&collection)?;
        let mut result_map = HashMap::with_capacity(items.len());

        for item in items {
            variables.insert(iterator.variable.clone(), item.clone());

            if let Some(cond) = condition {
                if !self.evaluate_condition(cond, user, resource, context, variables)? {
                    continue;
                }
            }

            // Evaluate key and value expressions
            let key_value = self.evaluate_expr(key_expr, user, resource, context, variables)?;
            let value_value = self.evaluate_expr(value_expr, user, resource, context, variables)?;

            // Extract string key (for Object/HashMap)
            let key_string = self.extract_string_key(&key_value)?;
            let interner = self.store.get_interner();
            let key_interned = interner.intern(&key_string);

            result_map.insert(key_interned, value_value);

            variables.remove(&iterator.variable);
        }

        Ok(AttributeValue::Object(result_map))
    }

    /// Helper: Get items from a collection
    fn get_collection_items(&self, collection: &AttributeValue) -> Result<Vec<AttributeValue>, ReaperError> {
        match collection {
            AttributeValue::List(vec) => Ok(vec.clone()),
            AttributeValue::Set(set) => Ok(set.iter().cloned().collect()),
            AttributeValue::Object(map) => Ok(map.values().cloned().collect()),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!("Cannot iterate over {:?}", collection)
            })
        }
    }
}
```

**Performance Optimizations**:
1. **Pre-allocation**: `HashSet::with_capacity(items.len())`
2. **Early termination**: Skip items that fail condition
3. **String interning**: Keys in object comprehensions use InternedString
4. **Zero-copy when possible**: Clone only when necessary

**Estimated Effort**: 12-16 hours

---

## Phase 2.4: Integration with Condition System

### Update Condition Enum

**File**: `crates/policy-engine/src/evaluators/reaper_dsl.rs`

Add comprehensions as a condition type:

```rust
pub enum Condition {
    // ... existing conditions ...

    /// Comprehension result check
    ComprehensionCheck {
        comprehension: Comprehension,
        check: ComprehensionCheckType,
    },
}

pub enum ComprehensionCheckType {
    /// Check if result is non-empty
    NonEmpty,
    /// Check if result contains value
    Contains(Value),
    /// Check result count
    Count { op: Operator, value: i64 },
}
```

**Example Usage**:
```reap
rule admin_access {
    allow if {
        // Check if user has admin role
        admins := {u.name | u := data.users[_]; "admin" in u.roles},
        user.name in admins
    }
}
```

**Estimated Effort**: 4-6 hours

---

## Phase 2.5: Testing Strategy

### Unit Tests

**File**: `crates/policy-engine/tests/comprehension_tests.rs` (already exists from earlier work!)

Add new tests for:
1. Set comprehension with filter
2. Array comprehension with filter
3. Object comprehension with filter
4. Nested comprehensions
5. Comprehension with no filter
6. Comprehension with empty result
7. Comprehension with wildcard iterator
8. Comprehension with variable binding

**Example Test**:
```rust
#[test]
fn test_set_comprehension_with_filter() {
    let store = setup_test_store();

    // Add users with roles
    store.insert(Entity::new("user1", "User")
        .with_attr("name", "alice")
        .with_attr("roles", AttributeValue::Set(
            hashset!["admin".into(), "user".into()]
        )));

    store.insert(Entity::new("user2", "User")
        .with_attr("name", "bob")
        .with_attr("roles", AttributeValue::Set(
            hashset!["user".into()]
        )));

    // Parse comprehension: {u.name | u := users[_]; "admin" in u.roles}
    let policy = r#"
        policy test {
            version: "1.0.0",
            default: deny,

            rule admin_check {
                admins := {u.name | u := users[_]; "admin" in u.roles},
                allow if user.name in admins
            }
        }
    "#;

    let evaluator = build_evaluator(policy, store);

    // Test with alice (should be in admins set)
    let request = PolicyRequest::new("alice", "read", "resource1");
    assert_eq!(evaluator.evaluate(&request).unwrap(), PolicyAction::Allow);

    // Test with bob (should NOT be in admins set)
    let request = PolicyRequest::new("bob", "read", "resource1");
    assert_eq!(evaluator.evaluate(&request).unwrap(), PolicyAction::Deny);
}
```

**Estimated Effort**: 8-12 hours

---

## Phase 2.6: Example Policies

### Real-World Examples

**File**: `crates/policy-engine/examples/comprehension_examples.reap`

```reap
policy rbac_with_comprehensions {
    version: "1.0.0",
    description: "RBAC using comprehensions for role → permission mapping",
    default: deny,

    // Build admin user set
    rule admin_full_access {
        admins := {u.name | u := data.users[_]; "admin" in u.roles},
        allow if user.name in admins
    }

    // Build permission map
    rule role_based_access {
        user_perms := {
            p.action |
            r := user.roles[_];
            role := data.roles[_];
            role.name == r;
            p := role.permissions[_];
            p.resource == resource.id
        },
        allow if context.action in user_perms
    }

    // Filter by attribute
    rule active_users_only {
        active_users := [u.name | u := data.users[_]; u.status == "active"],
        allow if user.name in active_users
    }

    // Build lookup map
    rule department_access {
        dept_map := {u.id: u.dept | u := data.users[_]},
        user_dept := dept_map[user.id],
        allow if user_dept == resource.department
    }
}
```

**Estimated Effort**: 4-6 hours

---

## Phase 2.7: Performance Validation

### Benchmark Tests

**File**: `crates/policy-engine/examples/benchmark_comprehensions.rs` (already exists!)

Update with Rego-style comprehension benchmarks:

```rust
#[tokio::main]
async fn main() {
    println!("Phase 2 Comprehension Performance Validation");
    println!("===========================================\n");

    // Test 1: Set comprehension with filter
    test_set_comprehension_100_items();

    // Test 2: Array comprehension with filter
    test_array_comprehension_100_items();

    // Test 3: Object comprehension
    test_object_comprehension_100_items();

    // Test 4: Nested comprehension
    test_nested_comprehension();

    // Test 5: Complex multi-step policy
    test_complex_rbac_comprehension();
}

fn test_set_comprehension_100_items() {
    let store = setup_store_with_100_users();

    let policy = r#"
        policy test {
            rule check {
                admins := {u.name | u := data.users[_]; "admin" in u.roles},
                allow if user.name in admins
            }
        }
    "#;

    let evaluator = build_evaluator(policy, store);

    // Warmup
    for _ in 0..10 {
        evaluator.evaluate(&request).unwrap();
    }

    // Benchmark
    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        evaluator.evaluate(&request).unwrap();
    }
    let elapsed = start.elapsed();

    let avg_ns = elapsed.as_nanos() / iterations;
    let target_ns = 10_000; // 10µs

    println!("Test 1: Set Comprehension (100 items)");
    println!("  Average: {} ns ({:.2} µs)", avg_ns, avg_ns as f64 / 1000.0);
    println!("  Target:  {} ns ({} µs)", target_ns, target_ns / 1000);
    println!("  Status:  {}", if avg_ns < target_ns { "✅ PASS" } else { "❌ FAIL" });
    println!();
}
```

**Performance Targets**:
- Set comprehension (100 items): < 10µs
- Array comprehension (100 items): < 10µs
- Object comprehension (100 items): < 15µs
- Nested comprehension (10x10): < 50µs

**Estimated Effort**: 6-8 hours

---

## Success Criteria

### Must Have ✅
- [ ] Set comprehensions work with filters
- [ ] Array comprehensions work with filters
- [ ] Object comprehensions work with key:value
- [ ] Wildcard iterator `[_]` works
- [ ] Variable binding in iterators works
- [ ] Conditions in comprehensions work
- [ ] All unit tests pass (100% pass rate)
- [ ] Performance: < 10µs for 100 iterations

### Should Have 🎯
- [ ] Nested comprehensions (comprehension inside comprehension)
- [ ] Comprehensions work with entity attributes
- [ ] Comprehensions work with variables
- [ ] Example policies demonstrate real-world usage

### Nice to Have 💡
- [ ] Multiple iterators in one comprehension (advanced)
- [ ] Short-circuit evaluation for early termination
- [ ] Parallel evaluation for large collections (Rayon)

---

## Estimated Timeline

| Task | Duration | Dependencies |
|------|----------|--------------|
| **2.1: AST Extensions** | 4-6 hours | None |
| **2.2: Parser Extensions** | 8-12 hours | 2.1 |
| **2.3: Evaluator Implementation** | 12-16 hours | 2.1, 2.2 |
| **2.4: Condition Integration** | 4-6 hours | 2.3 |
| **2.5: Testing** | 8-12 hours | 2.3 |
| **2.6: Examples** | 4-6 hours | 2.3 |
| **2.7: Performance Validation** | 6-8 hours | All |
| **Documentation** | 4-6 hours | All |

**Total**: 50-72 hours (1.5-2 weeks full-time, 3-4 weeks part-time)

---

## Risks & Mitigation

### Risk 1: Parser Ambiguity
**Issue**: Set vs Object comprehension syntax overlap
**Mitigation**: Disambiguate by looking for `:` in output expression

### Risk 2: Performance Regression
**Issue**: Nested loops could be slow
**Mitigation**:
- Pre-allocate collections
- Early termination on condition failure
- Benchmark continuously

### Risk 3: Memory Usage
**Issue**: Large comprehensions could allocate significant memory
**Mitigation**:
- Streaming evaluation (future optimization)
- Lazy evaluation for chained comprehensions
- Document memory characteristics

---

## Next Phase Preview

After Phase 2 completes, we'll move to **Phase 3: Essential Built-ins**:
- Aggregates: `count()`, `sum()`, `max()`, `min()`
- Strings: `concat()`, `contains()`, `split()`
- Objects: `object.get()`, `object.keys()`
- Time: `time.now_ns()`, `time.parse_ns()`

Comprehensions + Built-ins = **80% of real-world Rego policies** ✨

---

**Status**: Ready to begin implementation
**Next Step**: Phase 2.1 - AST Extensions
