# Reaper Policy Compiler - Known Limitations

**Date**: 2025-12-08
**Context**: Phase 7 Integration Testing

## Overview

During Phase 7 integration testing setup, we discovered several features that are **not yet supported** by the compiled policy evaluator (`ReaperDSLEvaluator`). These features work correctly with the AST evaluator (`ReapAstEvaluator`) but cause compilation errors.

## Unsupported Features in Compiled Evaluator

### 1. Function Call Assignments
**Status**: ❌ Not Supported

```reap
rule example {
    allow if {
        now := time::now_ns() &&  // ❌ ERROR: Expression assignments not supported
        user.token_expires_at > now
    }
}
```

**Error Message**:
```
Expression assignments (e.g., function calls) are not yet supported in compiled policies.
Variable 'now' uses an expression which requires direct AST evaluation.
```

### 2. Function Calls in Conditions
**Status**: ❌ Not Supported

```reap
rule example {
    allow if {
        time::is_after(user.token_expires_at, 1765180000000000000)  // ❌ ERROR
    }
}
```

**Error Message**:
```
Expression-based conditions (e.g., function calls like is_string(x)) are not yet supported in compiled policies.
```

### 3. Context Entity
**Status**: ❌ Not Supported

```reap
rule example {
    allow if {
        context.action == "read" &&  // ❌ ERROR: Context entity not supported
        user.role == "admin"
    }
}
```

**Error Message**:
```
Context entity not yet supported
```

## Workaround: Use AST Evaluator

For policies that require these features, use `ReapAstEvaluator` instead of compiling:

```rust
// Instead of:
let evaluator = policy.build(store)?;  // Tries to compile

// Use:
let evaluator = policy.build_ast_evaluator(store);  // Direct AST evaluation
```

## Impact on Integration Tests

### Time-Based Policies
- ❌ Cannot test time functions (`time::now_ns()`, `time::is_after()`, etc.)
- ❌ Cannot test time arithmetic or comparisons
- **Reason**: Requires function calls in conditions

### Recommended Test Strategy
1. **For compiled evaluator**: Test simple attribute comparisons only
2. **For AST evaluator**: Test all advanced features (time, regex with caching, etc.)
3. **Future**: Implement `PolicyEvaluator` trait for `ReapAstEvaluator` with thread-safe caching

## Thread Safety Issue with AST Evaluator

`ReapAstEvaluator` currently uses `RefCell<HashMap>` for regex caching, which is **not `Sync`**. To implement the `PolicyEvaluator` trait (which requires `Send + Sync`), we need to:

1. Replace `RefCell` with `Mutex` or `RwLock` for thread-safe caching
2. Implement all required trait methods:
   - `evaluate(&self, request) -> Result<PolicyAction>`
   - `validate(&self) -> Result<()>`
   - `evaluator_type(&self) -> &str`

## Roadmap

### Short-term (Phase 7)
- ✅ Document limitations
- ⏳ Create integration tests for features supported by compiled evaluator
- ⏳ Skip time-based tests until AST evaluator integration is complete

### Medium-term (Phase 8)
- Implement thread-safe caching for ReapAstEvaluator
- Implement PolicyEvaluator trait for ReapAstEvaluator
- Enable full feature testing via AST evaluation

### Long-term (Phase 9+)
- Enhance compiler to support function calls and context entity
- Compile all advanced features to optimized bytecode

## Files Affected

- `crates/policy-engine/src/reap/compiler.rs` - Compiler implementation
- `crates/policy-engine/src/reap/ast_evaluator.rs` - AST evaluator (needs PolicyEvaluator impl)
- `crates/policy-engine/tests/features/integration/time_based_policies.feature` - Integration tests (blocked)
- `crates/policy-engine/examples/policies/time_policy.reap` - Example policy (cannot compile)

## Compiled-mode function coverage: COMPLETE

Much of this document predates DSL v2. The compiled-mode coverage gap is now
**closed**: every DSL method/function the equivalence suite covers runs on the
compiled fast path (`find`, `find_all`, `replace`, `values`, `has_key`, `any`,
`all`, and `intersection`/`difference` with literal-array args were the last to
land). `compiled_ast_equivalence_tests.rs` asserts `build_preferred` selects the
compiled evaluator for each — a regression that drops back to AST fails the
build. See:

**→ `COMPILED_FUNCTIONS_WORKITEM.md`** (Status: DONE)

Comprehensions and the expression/function features listed earlier in this
document (time function calls, `context` entity in some positions) remain a
deliberate AST-only fallback; the AST path stays decision-equivalent, guaranteed
by the equivalence suite.

## Conclusion

The compiled evaluator is currently suitable for:
- ✅ Simple attribute comparisons (`user.role == "admin"`)
- ✅ Boolean logic (`&&`, `||`, `!`)
- ✅ Set operations (`in`)
- ✅ Null checks (`!= null`)

For advanced features (time functions, regex, complex expressions), use AST evaluator directly until compiler support is added.
