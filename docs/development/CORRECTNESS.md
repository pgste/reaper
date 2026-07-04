# Reap DSL Correctness Program

How we know policy decisions are RIGHT — not just that two implementations
agree, and not just that the examples we thought of pass. This document is
the map of the verification arc: the semantics contract, the layers that
enforce it, what each layer has caught, and the protocol for changing
evaluator behavior without regressing.

## The semantics contract

### Decision model
1. **Deny wins.** Any matching deny rule ⇒ Deny, regardless of allow rules.
2. Otherwise any matching allow rule ⇒ Allow.
3. Otherwise the policy default.

Two laws follow and are enforced as properties:
- **Rule-order invariance** — shuffling rules never changes any decision.
- **Deny monotonicity** — adding a deny rule can never turn Deny into Allow.

### Type-strict, total comparisons (no coercion)
Comparisons never error and never coerce:
- `==` is true only for a same-typed equal value (Int/Float cross-numeric
  is the one exception). `user.level == "5"` does NOT match `Int(5)`.
- `!=` is true only for a PRESENT value that differs — a different type
  differs by definition; a missing value satisfies neither (see below).
- Ordering (`>`, `>=`, `<`, `<=`) is true only over two numbers; anything
  else is false — never an evaluation error (a live policy degrades toward
  deny, it does not fail the request).
- Wildcard/existential (`arr[*] == x`): typed element equality; a present
  scalar of a non-matching element type matches no element.

Pinned by the mixed-type `badge` attribute in the differential generator
(string/int/bool/absent) — re-introducing the old Int→String coercion is
caught within one default run (the shrunk counterexample is committed in
the proptest-regressions file).

### Null / undefined semantics (fail closed)
A missing attribute, entity, context key, variable, or input path evaluates
to **Null**, and Null satisfies **no** comparison except an explicit
presence check:

| Expression (`x` missing)     | Result | Why |
|------------------------------|--------|-----|
| `x == "v"`, `x == 5`, …      | false  | nothing to compare |
| `x != "v"`, `x != 5`, …      | false  | **absence must never satisfy an inequality guard** |
| `x > / >= / < / <= n`        | false  | no ordering with Null |
| `x == null`                  | true   | explicit presence check |
| `x != null`                  | false  | explicit presence check |
| `arr[*] ==/!= y`, arr missing| false  | no comparison took place |

The dangerous case is `!=`: if absence satisfied `resource.class != "secret"`,
every entity *lacking* the attribute would pass the guard — default-open.
This exact bug existed in BOTH evaluators (they agreed with each other) and
was caught only by the independent oracle (layer 3 below).

Consequence for compilation: **`!=` must always compile natively, never as
`Not(Equal)`** — `Not` inverts the fail-closed miss into a pass. All NotEqual
compile sites now emit native negated conditions (`NumericOp::NotEqual`,
`StringOp::Lower/UpperNotEquals`, `WildcardComparison{negated}`,
`VariableNotEqualsLiteral`, `VariableAttrNotEqualsLiteral`).

Note: explicit `!(a == b)` is user-written negation and keeps pure
complement semantics — with `a` missing, `!(a == b)` is true. That is the
documented difference between `!=` (null-safe, fail-closed) and `!(==)`.

### Compiled/AST contract
- If a policy **compiles**, the compiled evaluator and the AST interpreter
  must return identical decisions for every request (and identical error
  kinds when they error).
- If a construct is **not supported** by the compiler, compilation must FAIL
  (the engine falls back to the AST evaluator) — it must never silently
  compile to different semantics. Audit outcome: comparison-result
  assignments from `var.attr` (`x := p.active == true`) used to silently
  drop the binding and compile as a bare guard; they are now rejected at
  compile time and served by the AST evaluator.

### Check mode (documents: Terraform / K8s / any input JSON)
- `violations` = every **deny** rule whose condition holds against `input`
  (each with its rendered `with message`).
- `allowed` = no violations AND (default allow OR any allow rule matches).
- Check mode is AST-only by design.

## The verification layers

| Layer | File | What it proves | Typical catch |
|-------|------|----------------|---------------|
| 1. Unit tests (546+) | `src/**` | each eval primitive, incl. fail-closed edges of every native NotEqual | regressions in one primitive |
| 2. Golden corpus | `policy-library/**` + `tests/policy_library_tests.rs` | 13 real-world scenarios (RBAC/ABAC/ReBAC/JWT/Terraform/K8s/combined), 76+ pinned decisions, compiled-vs-AST parity per case | behavior drift on realistic policies |
| 3. **Differential + oracle (authz)** | `tests/differential_parity_tests.rs` | random policies × worlds × 81 requests: compiled == AST == independent oracle; order-invariance; deny-monotonicity. Covers attribute compares (str/num), cross-entity, context, membership, and **ReBAC** (`rebac::related`/`reachable` over random edges, group membership, nesting — oracle does its own BFS) | miscompiles AND both-wrong-together semantics bugs |
| 4. **Differential + oracle (check mode)** | `tests/check_mode_differential_tests.rs` | random Terraform/K8s-ish documents × random deny/allow policies over `input.*` paths: violation SET, messages, and allowed flag vs an independent oracle; hammers absence paths (missing fields, missing parents, missing document) | fail-open input guards, violation/message drift |
| 5. Suite runners | `tests/integration_runner.rs`, `tests/comparison_runner.rs` + `test-fixtures/**` | YAML-declared expectations against committed fixtures; runners now HARD-FAIL on missing data files instead of silently evaluating an empty store | fixture rot, environment-dependent green |
| 6. BDD / Gherkin | `tests/gherkin_tests.rs`, `*/tests/features/` | readable scenario specs | spec-level drift |

| 7. Mutation testing (nightly) | `.github/workflows/mutation.yml` | cargo-mutants injects ~2,000 bugs (flipped operators, deleted match arms, forced returns) across both evaluators, the reap compiler, and the ReBAC graph; every surviving mutant is a bug the suites would not catch | holes in layers 1–6 themselves |
| 8. Release gate | `.github/workflows/release.yml` (correctness-gate job) | no tag ships without the functional suite + differential suites at 2000 cases + bench smoke | releasing a regression |
| 9. Perf regression tracking | `.github/workflows/perf-tracking.yml` | criterion micro-benchmarks (evaluation + ReBAC hot paths) tracked per push; >30% regression fails the run and comments; main pushes update the gh-pages baseline | silent latency regressions on the primary (compiled) path |

Layers 3–4 are the gate: they generate what nobody thought to write down.
Every failure shrinks to a minimal counterexample and is saved to
`*.proptest-regressions`, which is committed — every past failure re-runs
first on every future run, forever. Layer 7 measures the layers themselves:
if a mutant survives, the missing test gets written (the nightly summary
lists every survivor).

## Running it

```bash
# Everything
cargo test -p policy-engine

# The differential gate, at scale (CI runs 500; use 1000+ before releases)
PROPTEST_CASES=1000 cargo test -p policy-engine --release \
    --test differential_parity_tests --test check_mode_differential_tests

# Golden corpus only
cargo test -p policy-engine --test policy_library_tests
```

CI (`.github/workflows/ci.yml`, integration-tests job) runs the library,
both suite runners, and both differential suites at `PROPTEST_CASES=500`
on every push.

Note: the harnesses read `PROPTEST_CASES` explicitly — proptest's default
env handling is overridden the moment a config sets `cases:`, which
silently ignored the variable before.

## Protocol for changing evaluator behavior

1. If the change touches comparison/decision semantics, update the ORACLE
   first (it is the spec) — in the same PR, with a row in the table above
   if the contract changes.
2. Run both differential suites at `PROPTEST_CASES=1000` release.
3. Any shrunk counterexample that reflects a real spec change gets a
   dedicated unit test with a comment naming the semantics rule.
4. Never delete `*.proptest-regressions` entries.
5. New compiler condition types: native negation variants from day one if
   the operator can face missing data; silent `_ => false` arms in eval
   functions are only acceptable when `false` is the fail-closed answer for
   BOTH the operator and its negation.

## What this has caught so far (all found by layers 2–4)

1. Compiled context cross-entity comparison miscompile (denied everything).
2. Compiled context null-check miscompile.
3. **Fail-open `!=` on missing attributes in BOTH evaluators** (oracle catch).
4. Compiled `Not(Equal)` NotEqual sites re-introducing fail-open after the
   AST fix (parity catch).
5. Silent dropped binding in `x := var.attr == lit` compilation (audit).
6. Suite runners silently passing/failing against an absent data file.

### Comprehension contract (total iteration)
- A missing or non-collection iteration source is an EMPTY collection —
  the comprehension yields empty, the assignment still binds, and the rule
  continues. (The compiled evaluator used to fail the rule outright on a
  missing source; aligned to the AST contract and pinned by
  `comprehension_tests.rs` on BOTH evaluators.)
- Assignments always succeed as conditions — emptiness is checked with
  `.count()` guards (`bad := [...] && bad.count() > 0`), the conftest
  pattern used across the policy library.
- Library coverage: `terraform/s3-guardrails` and
  `kubernetes/admission-control` exercise comprehensions over `input`
  documents; `combined/release-gates` exercises them over entity data with
  compiled/AST parity enforced per case.

## Known gaps / next steps

- ~~Type-coercion divergence~~ RESOLVED: contract is type-strict total
  comparisons (above); compiled literal arms, `compare_attr_values`, the
  wildcard eval, and AST `compare_numeric` all aligned; pinned by
  mixed-type worlds in the generator.
- ~~Mutation testing not wired~~ RESOLVED: nightly sharded cargo-mutants
  run over the decision-logic scope; burn down survivors from the nightly
  summary.
- Collection methods (`.first()`, `.count()` on strings) and JWT builtins
  are exercised by layers 1–2 but not yet generated in layer 3/4 grammars.
- Comprehension GRAMMAR is not yet generated in layers 3–4 (covered by
  layers 1–2 + mutation); a comprehension atom for the generators is the
  natural next extension.
- `platform_bdd_tests` requires live services (`make platform` + `make
  agent`) and fails, rather than skips, without them.
