# Work Item: Complete Compiled-Mode Coverage for All DSL Functions

**Status**: DONE
**Owner**: policy-engine
**Related**: `DSL_V2_DESIGN.md`, `COMPILER_LIMITATIONS.md`,
`crates/policy-engine/tests/compiled_ast_equivalence_tests.rs`

## Goal

Every DSL method/function should evaluate on the **compiled `ReaperDSLEvaluator`
(DSL v2)** fast path. The compiled evaluator is the preferred, sub-microsecond
path; the AST interpreter is the correctness-equivalent fallback.

## Outcome

All previously-falling-back functions now compile on the fast path:
`find`, `find_all`, `replace`, `values`, `has_key`, `any`, `all`, and
`intersection` / `difference` with **literal-array** arguments (the entity-attr
forms already compiled). `find_all` was an extra fallback the strengthened
equivalence suite surfaced during the work.

`compiled_ast_equivalence_tests.rs` now **asserts the compiled path is selected**
for every function it covers — `assert_fn_is` fails if `build_preferred` falls
back to AST — so a future compiler regression that drops a function back to the
interpreter fails the build. Compiled ≡ AST is still pinned for every function
(plus new deny/edge cases), and comprehensions remain a deliberate AST-only
fallback (`build_preferred_falls_back_to_ast`).

### How each was wired (for reference)
- **find / find_all**: `ExprType::RegexFind` / `RegexFindAll` →
  `CompiledExprType::RegexFind` (already existed) / `RegexFindAll` (new eval via
  `regex_cache` + `find` / `find_iter`).
- **replace**: new `ExprType`/`CompiledExprType::StringReplace` (regex
  `replace_all`, matching AST).
- **values**: new `SetValues` mirroring `SetKeys`.
- **has_key / any / all**: new `Condition` + `CompiledCondition` variants
  (`ObjectHasKey`, `CollectionAny`, `CollectionAll`) with truthiness matching
  `builtin_methods::method_any/all`; wired in both `compile_method_call` and
  `compile_entity_method_call`.
- **intersection / difference (literal args)**: the direct-path compiler arms now
  fall back from `extract_entity_attr` to `extract_string_array`, producing the
  existing `SetIntersection` / new `SetDifference` literal-value expr variants.

## Exact blockers (from the compile errors)

| Function | Blocker | Kind |
|---|---|---|
| `intersection`, `difference` | Compile fine with an **entity-attribute** arg (`user.a.intersection(user.b)`); a **literal-array** arg (`.intersection(["rust"])`) hits `extract_entity_attr(args[0])` which rejects `Literal(Array)` | Add a literal-arg variant (the chained-method path already carries `values`) + eval |
| `find` | `find()` "not supported for expression assignments"; the `RegexFind` compiled expr variant already exists | Wire the assignment compiler → existing variant |
| `replace` | `replace()` "not supported for expression assignments"; no compiled expr variant | New `ExprType`/`CompiledExprType` variant + eval (2 args: pattern, replacement) |
| `values` | "not supported for expression assignments"; `SetKeys` exists, `SetValues` does not | New variant mirroring `SetKeys` + eval |
| `has_key` | "not supported in compiled policies"; boolean method taking a string arg | New compiled condition + eval |
| `any`, `all` | "not supported in compiled policies"; boolean collection predicates | New compiled condition + eval (truthiness over items) |

## Plan (easy → hard, one function per change, each with its equivalence test)

1. **`find`** — pure wiring: route `MethodName::Find` in the expression-assignment
   compiler to the existing `RegexFind` compiled expr. Lowest risk.
2. **`replace`** — add `ExprType::StringReplace`/`CompiledExprType::StringReplace`
   (entity_type, attribute, pattern, replacement) + eval; wire the assignment
   compiler.
3. **`values`** — add `SetValues` mirroring `SetKeys`; wire + eval.
4. **`has_key`** — add a compiled condition `ObjectHasKey { entity_type,
   attribute, key }` + eval; wire `compile_method_call`.
5. **`any` / `all`** — add compiled conditions `CollectionAny`/`CollectionAll`
   (truthiness over List/Set items, matching `builtin_methods::method_any/all`)
   + eval; wire `compile_method_call`.
6. **`intersection` / `difference` with literal args** — extend the set-op expr
   variants with a literal-values form (reuse `extract_string_array`) + eval, so
   both entity-attr and literal args compile.

Each step:
- Adds the compiled variant(s) and eval.
- Removes the corresponding entry from the FALLBACK list.
- Extends `compiled_ast_equivalence_tests.rs` so the function is asserted to run
  on the COMPILED path (not just agree via fallback) — e.g. assert
  `build_preferred(...).evaluator_type() == "reaper_dsl"` for that policy.

## Definition of done

- All 24 functions compile; `build_preferred` never falls back for the DSL
  surface exercised by the equivalence suite.
- `compiled_ast_equivalence_tests.rs` asserts the compiled path is selected for
  every function, and compiled≡AST for all.
- No change to decision semantics (guaranteed by the differential).

## Why this is its own arc (not folded into an unrelated PR)

It touches the compiler core (`reap/compiler/**`) and the compiled evaluator
(`evaluators/reaper_dsl/**`) for each function. Landing it incrementally on its
own branch keeps each function's compiled path reviewable and independently
verifiable against the equivalence net, rather than bundled into a large
data-plane/CI PR.
