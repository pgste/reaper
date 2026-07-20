# DSL Parity & Fast Path — Closing the Compiled/AST Cliff and the Rego Authoring Gaps

> **STATUS: 📋 PLANNED** — round-4. Source analysis:
> `docs/development/REGO_GAP_ANALYSIS.md` (construct matrix, fallback-trigger
> inventory, ranked candidates). This plan operationalizes its P1/P2 items and
> writes the anti-roadmap into the language contract. P3 items are listed as
> parked, not scheduled.

**Competitive gate:** The headline claim vs OPA is speed — yet the policy
class most directly comparable to OPA workloads (`input`-document policies:
terraform, kubernetes admission, api-gateway) runs entirely on the AST
interpreter today, and one exotic construct anywhere in a policy silently
de-optimizes the *whole policy* (per-policy fallback). Separately, the two
authoring features Rego users hit in week one (helper predicates, imports)
do not exist. This plan closes those in risk order.
**Priority:** Phase A/B = P1 (competitive), Phase C = P1 (adoption),
Phase D = P2, Phase E = P3 (parked).
**Findings closed:** REGO_GAP_ANALYSIS §4 (bifurcation), §5 P1/P2 lists,
§6 (anti-roadmap documentation).

---

## 1. Goal

1. **No silent slow path.** A policy either compiles whole, or the *rules*
   that need the interpreter run on it while their siblings stay compiled —
   and the deploy surface reports which (observable, never silent).
2. **The `input` policy class is fast.** Terraform/k8s/api-gateway-style
   policies run on the compiled path with decisions AND violation messages
   identical to the AST evaluator.
3. **Rego-parity authoring:** helper predicates and imports, under the
   existing totality and compatibility contracts (no recursion; language-
   version bump + frozen corpus for any semantic surface).
4. **The stdlib gaps that block real policies are closed:** `net::cidr_*`,
   encoding, infix arithmetic, quantifier sugar.
5. **The refusals are contract, not accident:** `http.send`-class I/O,
   nondeterminism, unification, value-producing rules — documented in the
   language reference as deliberate, sellable properties.

Non-goals: loosening the entity contract (REGO_GAP_ANALYSIS §6 resolves
this); the filter API (own design doc, demand-gated); tier-2 specialization
(parked with recorded verdict).

## 2. Current state (evidence)

- **Per-policy fallback:** `Policy::build_preferred`
  (`crates/policy-engine/src/reap/mod.rs:133-146`) — `compile_policy` is
  all-or-nothing; any `Err` sends the entire policy to `ReapAstEvaluator`.
- **`input` never compiles:** 5 unconditional fallback sites
  (`reap/compiler/mod.rs:130`, `compiler/comprehension.rs:96`,
  `compiler/comparison/{mod.rs:159, entity.rs:23, membership.rs:46}`). All
  `input`-based policy-library policies are AST-only (fitness run 2026-07-20:
  4/18 in policy-library, 15/60 across benchmark corpora).
- **Mechanical fallback clusters** (complete inventory in
  REGO_GAP_ANALYSIS §4): variable-shape comparisons
  (`compiler/comparison/variable.rs:238,247,254,260,328` …), method calls in
  expressions/comprehension outputs (`compiler/expression.rs:319,371,422,483`,
  `compiler/comprehension.rs:32,54,75`), literal-value assignment
  (`compiler/mod.rs:158`), indexed non-equality
  (`compiler/comparison/entity.rs:39,73`), dynamic-id ReBAC
  (`compiler/mod.rs:503`), dynamic-key taint.
- **No functions/imports:** no grammar production (`reap.pest`); `package`
  is a metadata string (`reap/mod.rs:159-165`); "Imports" is a declared
  future enhancement (`docs/reference/reap-language.md:622-628`).
- **Stdlib:** no `net::`, no base64/url decode, no infix arithmetic
  (`math::` builtins only), no named `some`/`every` sugar.
- **Existing gates to reuse:** compiled-vs-AST differential
  (`tests/compiled_ast_equivalence_tests.rs`), frozen decision corpus +
  CHECKSUMS (`policy-library/frozen/`, `DSL_COMPATIBILITY.md`), builtin
  differential oracles, SLO harness.

## 3. Definition of done (visible behaviors)

- [ ] **Anti-roadmap is contract.** `docs/reference/reap-language.md` gains a
  "Deliberate non-features" section (no mid-decision I/O, no
  nondeterministic builtins, no unification, no value-producing rules, no
  unbounded traversal, no in-language mocking) with the rationale for each;
  REGO_GAP_ANALYSIS is linked as the comparison source.
- [ ] **Per-rule fallback.** A policy with one uncompilable rule keeps every
  other rule on the compiled path. Deny-overrides ordering is preserved
  across the mixed set (all deny rules — compiled or AST — before any allow).
  The build result reports per-rule mode; the agent exposes
  `compiled_rules`/`ast_rules` counts per policy (deploy response + metrics).
  Differential gate extended to mixed-mode policies.
- [ ] **Mechanical clusters compile.** Every §2 mechanical trigger either
  compiles (with a new/extended `CompiledCondition` shape) or is
  re-documented as deliberate with a tracking note. Fallback-trigger count
  measured before/after; the compiled-vs-AST differential covers each closed
  shape with targeted cases.
- [ ] **`input` compiles.** A compiled input-access plan (pre-parsed path
  segments, wildcard positions, comprehension sources over input) evaluates
  the policy-library terraform/kubernetes/api-gateway policies on the
  compiled path with identical decisions AND identical check-mode violation
  messages to the AST evaluator (differential + frozen corpus additions).
  Benchmarked ≥5× faster than AST on those policies.
- [ ] **Helper predicates + imports ship behind a language-version bump.**
  `func` definitions (non-recursive, depth-counted, call-graph DAG enforced
  at parse) usable in conditions; `import` of a `.reap` library file;
  bundle format carries imported units; frozen corpus gains cases; old
  engines fail closed on the new language version (existing rule).
- [ ] **P2 builtins land additively:** `net::cidr_contains`/`cidr_overlaps`,
  `base64::decode`/`encode`, `url::decode`, infix arithmetic on numbers
  (`+ - * /` with the existing type-strictness), named `some`/`every` sugar
  desugaring to existing semantics, `object.get(key, default)`, `flatten`,
  `to_number`. Each with builtin-differential oracle cases and frozen-corpus
  entries. No decision of any existing frozen case changes.

## 4. Critical steps — ordered

**Phase A — stop the silent cliff (S/M)**
1. "Deliberate non-features" section in the language reference (S).
2. Per-rule fallback: `compile_policy` returns per-rule results; new
   mixed-mode policy evaluator holds compiled rules + an AST sub-evaluator
   for the remainder; deny-precedence ordering proven across the mix;
   per-rule mode surfaced in deploy response/metrics/logs (M).
3. Close the mechanical clusters, one PR per cluster, each behind the
   differential: literal assignment → variable comparisons (var↔entity-attr,
   var-attr↔var-attr) → method-comparison shapes → expression-assignment
   builtin calls → indexed non-equality (M total; each slice S).

**Phase B — compile `input` (M/L)**
4. Compiled input path IR: `Vec<PathSeg>` (`Key`, `Index`, `Wildcard`),
   pre-resolved at compile; evaluation walks the request's input JSON once
   per path with no string re-parsing (M).
5. Input-anchored `CompiledCondition` shapes: compare/membership/string-op
   over an input path; comprehension iteration sourced from an input path;
   violation-message expressions over input (M).
6. Gates: differential over the input corpus (decisions + messages), frozen
   corpus additions, bench row (compiled ≥5× AST on library input policies),
   fitness re-run to confirm the AST-fallback count drops (S).

**Phase C — functions & imports (L; language-version bump)**
7. Grammar + AST: `func name(params) := <condition-or-expr>`; call-graph
   DAG check at parse (recursion = parse error); depth accounting includes
   function bodies (M).
8. Imports: `import "path" as ns` resolving at *load* time (bundle build /
   file load), imported units embedded in the compiled artifact — no
   runtime file I/O (M).
9. Compile pass inlines function bodies (totality makes inlining always
   terminating); AST evaluator gets the same semantics; language version
   bumped; frozen corpus + CHECKSUMS extended; `DSL_COMPATIBILITY.md`
   waiver recorded (M).

**Phase D — P2 builtins (M, additive slices)**
10. `net::` (cidr parse/contains/overlaps, IPv4+IPv6) (S/M).
11. Encoding (`base64::`, `url::decode`) (S).
12. Infix arithmetic desugaring to checked numeric ops; division-by-zero →
    `null` (fail-closed, documented) (M).
13. `some x in coll { … }` / `every x in coll { … }` sugar desugaring to
    existing `[_]`/comprehension-count semantics (S/M).
14. `object.get`/`flatten`/`to_number` (S).

**Phase E — P3 (PARKED, demand-gated):** schema annotations, string
interpolation, raw strings, `jwt::verify` (JWKS-configured), glob builtin.

## 5. Key decisions (ADR-style)

- **ADR-1: Per-rule fallback before `input` compilation.** The cliff is the
  liability; per-rule fallback shrinks it for every mixed policy immediately
  and creates the mixed-mode machinery Phase B's partial coverage will need
  anyway. Consequence: a "compiled" policy is no longer a binary property —
  observability (per-rule counts) ships in the same phase, and the
  differential gains a mixed-mode axis.
- **ADR-2: Inline functions at compile, never a call stack.** Totality (DAG
  + depth cap) makes inlining always terminate; the compiled path stays a
  flat condition walk with zero new runtime machinery; the AST path may
  interpret calls directly. Consequence: pathological inlining growth is
  bounded by the existing nesting cap applied *post-inline*.
- **ADR-3: `input` stays request-scoped JSON — no schema requirement.** The
  loose island keeps its OPA-equivalent flexibility; compilation
  pre-resolves *paths*, not types. Type strictness at the leaves is the
  existing rule (mismatch → false). Consequence: no schema work blocks
  Phase B; schemas remain P3.
- **ADR-4: Additive-only stdlib growth, frozen-corpus enforced.** Every new
  builtin/operator lands with oracle + frozen cases; anything that would
  change an existing decision is a language-version event per
  `DSL_COMPATIBILITY.md`. Consequence: velocity bounded by the corpus
  process — accepted, that process is the product's compatibility story.

## 6. Phasing & effort

| Phase | Size | Independently shippable | Gate |
|---|---|---|---|
| A (cliff) | S+M+M | per slice | compiled-vs-AST differential (+ mixed-mode axis) |
| B (`input`) | M/L | yes | differential incl. messages, frozen corpus, bench ≥5× |
| C (func/imports) | L | yes | language-version bump machinery, frozen corpus |
| D (builtins) | M | per builtin | builtin oracles, frozen corpus |
| E (P3) | — | parked | demand |

Start: Phase A step 1+2 (anti-roadmap doc + per-rule fallback), then A.3
slices in trigger-count order.
