# Reaper Policy Engine

High-performance, multi-language policy evaluation engine for authorization decisions.

## Features

### 🚀 Multi-Language Support

Reaper supports multiple policy languages through an extensible evaluator architecture:

| Language | Performance | Use Case | Status |
|----------|------------|----------|--------|
| **Simple** | ~1-2 µs | High-throughput APIs, service mesh | ✅ Production |
| **Cedar** | ~1-50 ms | ABAC, schema validation, AWS compat | ✅ Production |
| **Custom** | TBD | Compile-time optimization, Rust DSL | 🔮 Planned |

### ⚡ Performance Characteristics

#### Simple Policy Language
- **Latency**: Sub-microsecond (600K+ ops/sec)
- **Use When**: Performance is critical
- **Features**: Wildcard matching, first-match-wins
- **Example**: API gateway, rate limiting

#### Cedar Policy Language
- **Latency**: 1-50 milliseconds (700+ ops/sec)
- **Use When**: Rich policy expression needed
- **Features**: ABAC, schema validation, AWS Cedar compatible
- **Example**: Document management, multi-tenant SaaS

### 🔧 Core Features

- **Hot-Swapping**: Zero-downtime policy updates using atomic operations
- **Lock-Free**: DashMap-based storage for concurrent access
- **Type-Safe**: Rust's type system ensures correctness
- **Extensible**: Easy to add new policy languages via `PolicyEvaluator` trait
- **Observable**: Built-in tracing and metrics

## Quick Start

### Simple Policy (High Performance)

```rust
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule};

// Create engine
let engine = PolicyEngine::new();

// Define simple rules
let rules = vec![
    PolicyRule {
        action: PolicyAction::Allow,
        resource: "api/*".to_string(),
        conditions: vec![],
    },
];

// Create and deploy policy
let policy = EnhancedPolicy::new(
    "my-policy".to_string(),
    "API access control".to_string(),
    rules,
);

engine.deploy_policy(policy.clone())?;

// Evaluate requests
let request = PolicyRequest {
    resource: "api/users".to_string(),
    action: "read".to_string(),
    context: HashMap::new(),
};

let decision = engine.evaluate(&policy.id, &request)?;
// decision.evaluation_time_ns: ~1000 ns
```

### Cedar Policy (Expressive ABAC)

```rust
use policy_engine::{EnhancedPolicy, PolicyEngine, PolicyLanguage};

let cedar_policy = r#"
    permit(
        principal,
        action == Action::"read",
        resource
    ) when {
        principal.role == "viewer"
    };
"#;

let policy = EnhancedPolicy::new_with_language(
    "cedar-rbac".to_string(),
    "Role-based access control".to_string(),
    PolicyLanguage::Cedar,
    cedar_policy.to_string(),
)?;

engine.deploy_policy(policy.clone())?;

// Evaluate with context
let mut context = HashMap::new();
context.insert("principal".to_string(), "alice".to_string());

let request = PolicyRequest {
    resource: "document-123".to_string(),
    action: "read".to_string(),
    context,
};

let decision = engine.evaluate(&policy.id, &request)?;
// decision.evaluation_time_ns: ~1_000_000 - 50_000_000 ns
```

## Architecture

### PolicyEvaluator Trait

The foundation for multi-language support:

```rust
pub trait PolicyEvaluator: Send + Sync + Debug {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError>;
    fn validate(&self) -> Result<(), ReaperError>;
    fn evaluator_type(&self) -> &str;
    fn metadata(&self) -> Option<EvaluatorMetadata>;
}
```

### Current Implementations

1. **SimplePolicyEvaluator**
   - Wildcard pattern matching
   - First-match-wins
   - Sub-microsecond latency
   - Zero-copy evaluation

2. **CedarPolicyEvaluator**
   - AWS Cedar integration
   - Rich ABAC support
   - Schema validation
   - Entity model support

3. **Custom DSL** (Planned)
   - Rust macro-based
   - Compile-time optimization
   - Procedural macros
   - Zero-cost abstractions

## Examples

Run the multi-language demo:

```bash
cargo run --example multi_language_demo
```

See `examples/cedar_policies.md` for Cedar policy examples.

## Adding Your Own Language

Implement the `PolicyEvaluator` trait:

```rust
use policy_engine::{PolicyEvaluator, PolicyAction, PolicyRequest};

#[derive(Debug)]
pub struct MyCustomEvaluator {
    // Your policy representation
}

impl PolicyEvaluator for MyCustomEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        // Your evaluation logic
        Ok(PolicyAction::Allow)
    }

    fn validate(&self) -> Result<(), ReaperError> {
        // Validate policy syntax/semantics
        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "my-custom-language"
    }
}
```

Then add it to `PolicyLanguage` enum and `EnhancedPolicy::build_evaluator()`.

## Performance Benchmarks

Run benchmarks:

```bash
cargo bench --package policy-engine
```

Expected results (on modern hardware):
- Simple evaluation: < 1 microsecond (p99)
- Cedar evaluation: 1-50 milliseconds
- Hot-swap: < 100 nanoseconds
- Concurrent access: Lock-free, no contention

## Testing

```bash
# Unit tests
cargo test --package policy-engine

# BDD tests (Cucumber)
cargo test --package policy-engine --test policy_bdd_tests
```

## Design Principles

1. **Performance First**: Lock-free data structures, zero-copy patterns
2. **Type Safety**: Leverage Rust's type system for correctness
3. **Extensibility**: Plugin architecture for new languages
4. **Backward Compatibility**: Simple policies always supported
5. **Production Ready**: Tracing, metrics, error handling

## Roadmap

- [x] Simple policy language
- [x] Cedar integration
- [x] Multi-language architecture
- [x] Hot-swapping
- [x] Lock-free evaluation
- [ ] Custom Reaper DSL with compile-time optimization
- [ ] Policy versioning and rollback
- [ ] Audit logging
- [ ] OPA Rego support (optional)
- [ ] WebAssembly policy support

## License

MIT OR Apache-2.0
