# Compiling `input` Document Policies (R4-01 Phase B)

**Status:** DESIGN — the worked approach for round-4 plan 01 Phase B
(`plans/round-4/01-dsl-parity-and-fast-path.md`). Written before
implementation, per the plan's own discipline: this is the policy class most
directly comparable to OPA workloads (Kubernetes admission control,
Terraform plan checks, API-gateway request policies), and today it runs
entirely on the AST interpreter.
**Prerequisite shipped:** per-rule fallback (Phase A.2, `MixedReapEvaluator`)
— every slice below pays off immediately because a partially-covered policy
now runs mixed instead of falling back whole.

---

## 1. What the corpus actually uses (the target grammar subset)

Extracted from `policy-library/kubernetes/admission-control`,
`policy-library/terraform/s3-guardrails`, `policy-library/api-gateway/*` —
the shapes that must compile for the class to be fast, in frequency order:

1. **Comprehension over an input array with filters, bound to a variable:**
   ```reap
   bad := [c.image | c := input.request.object.spec.containers[_];
                     c.image.endswith(":latest")] &&
   first := bad[0] &&
   bad.count() > 0
   ```
   Filters seen: `.endswith(..)`, `.startswith(..)` (negated too), `== true`,
   `== null` on nested element paths (`c.securityContext.privileged`,
   `c.resources.limits`). Outputs seen: the element (`c`), an element path
   (`c.image`, `c.name`).
2. **Direct deep-path leaf tests:**
   `input.request.object.metadata.labels.owner == null`.
3. **Bound-variable follow-ups:** indexing (`bad[0]`), `.count()` compared
   to a literal.
4. **Check-mode messages:** `deny with message concat("…: ", first)` —
   message expressions over rule-bound variables.

Everything else `input` could theoretically do (object comprehensions over
input, method chains on input paths, cross-input joins) is out of the first
cut: unsupported shapes keep their per-RULE fallback, which A.2 has already
made cheap and observable.

## 2. How the interpreter does it today (what we must equal, and what we can beat)

Entry: `ReapAstEvaluator::evaluate_with_input(_named)` /
`check_with_input(request, Option<&serde_json::Value>)`
(`reap/ast_evaluator/mod.rs`). Mechanics per evaluation:

- The whole JSON document is converted to an `EvalValue` tree
  (`json_to_eval_value`) — one full-tree allocation pass per request.
- Every `input.a.b.c` access re-splits the dotted path at eval time
  (`navigate_eval_path`), per access, per element.
- Comprehension iteration CLONES each element into the binder variable;
  filters clone again through the generic condition walk.
- The check driver runs every deny rule with a CLONED base context per rule
  (`check_with_input`), then renders messages from the rule's variables.

Semantics to preserve exactly (all already total / fail-closed):

- Missing document, missing path, wrong-type traversal ⇒ `Null`; comparisons
  against `Null` are `false` (the OPA-`undefined` analog, made total).
- Type strictness: cross-type comparison ⇒ `false`, no coercion.
- Rule-scoped variables; messages see the binding of THEIR rule only.
- `allowed` composition in check mode: any deny violation ⇒ false; else
  default-allow ⇒ true; else any allow rule matches.

What the compiled path can beat, in expected-impact order: per-access path
re-parsing (pre-parse once at compile), per-element clones (bind
references), the generic AST dispatch per filter node (pre-lowered filter
pipeline), and the per-rule context clone in check mode.

## 3. Design

### 3.1 Value domain: navigate `serde_json::Value` directly, zero conversion

The compiled path does NOT build the `EvalValue` tree at all. Input values
live in their own domain — borrowed `&serde_json::Value` nodes — and never
touch the `DataStore` interner (no transient-interning churn, no unbounded
per-request strings; the same discipline the request path already enforces
with `lookup`-not-`intern`).

```rust
/// Pre-parsed at compile time; no string splitting at eval time.
enum PathSeg {
    Key(Box<str>),   // .field  /  ["field"]
    Index(usize),    // [0]
}
struct InputPath { segs: Box<[PathSeg]> }   // wildcard handled by the
                                            // iteration source, not the path
```

`InputPath::resolve(&doc) -> Option<&Value>` is a loop of `get()`s —
no allocation, no parsing. Missing anything ⇒ `None` ⇒ the leaf's
fail-closed `false`/`Null` semantics.

### 3.2 New compiled leaves (decision-mode)

Added to `CompiledCondition` (each one classified in `leaf_staticness` —
the exhaustive match forces it — as `Dynamic`; input is request-scoped):

```rust
/// input.<path> <op> <literal-or-null>  — covers ==, !=, >, >=, <, <=,
/// and the == null / != null existence idiom.
InputCompare { path: InputPath, op: NumericOp, target: CompiledCompareTarget }
/// input.<path>.<string-op>("lit") — contains/startswith/endswith.
InputStringOp { path: InputPath, op: StringOp, value: String }
```

Comparison semantics are defined by ONE function,
`json_cmp(&serde_json::Value, &CompiledCompareTarget) -> Option<Ordering
/ bool>`, whose behavior is pinned against the AST's
`json_to_eval_value`-then-compare result by a dedicated property test
(random JSON scalars × ops, both paths agree). This is where int/float/u64
edge cases live (serde_json `Number` vs `EvalValue::Int/Float`); the
property test — not code review — is the guarantee.

### 3.3 Comprehensions: reference-binding iteration

Extend the existing compiled comprehension machinery rather than invent a
parallel one:

- `CompiledIterationSource` gains `InputPath(InputPath)` — the source
  resolves to `&Value`; a non-array resolves to an empty iteration
  (fail-closed, matches AST).
- A new borrowed evaluation scope for input-bound variables:

  ```rust
  /// Lives for one rule evaluation; borrows from the request document.
  struct InputVars<'doc> { vars: FxHashMap<InternedString, &'doc Value> }
  ```

  The binder (`c := …[_]`) binds `&'doc Value` — **no element clone**.
  Filters (`c.image.endswith(..)`, `c.securityContext.privileged == true`)
  compile to a small filter pipeline over the borrowed element:
  `Vec<CompiledInputFilter>` where each filter is
  `{ elem_path: InputPath, test: InputTest }` (`InputTest` = the §3.2 leaf
  bodies, plus `Negated(..)`). No generic condition walk per element.
- Outputs project either the element itself or an element path, collected
  as `Vec<&'doc Value>`; the comprehension **result** binds into the
  existing rule-scoped variables map. Bridging rule: a collected result
  that later flows into the generic variable machinery (`bad.count()`,
  `bad[0]`, message rendering) is materialized ONCE at bind time into the
  existing `AttributeValue`/string forms — i.e. one conversion of the
  (usually tiny) *result*, never of the (large) *document*.
- `first := bad[0]` and `bad.count() > 0` then compile through the existing
  variable-comparison shapes — no new machinery (`bad[0]` on the
  materialized result; A.3's variable-cluster slices already cover the
  comparison forms these policies use, and land before or with this).

### 3.4 Where input enters the compiled evaluator

`EntityBindings<'a>` gains `input: Option<&'a serde_json::Value>` (one new
`None` at every existing construction site — mechanical). New entries on
`ReaperDSLEvaluator`:

```rust
pub fn evaluate_with_input_named(&self, request, input: Option<&serde_json::Value>)
    -> Result<(PolicyAction, Option<&str>), ReaperError>
pub fn check_with_input(&self, request, input: Option<&serde_json::Value>)
    -> Result<CheckResult, ReaperError>
```

`MixedReapEvaluator` gets the same pair, dispatching per rule exactly like
`decide()` (A.2's unknown-principal routing applies unchanged — input
policies typically have no principal, which routes to… see §3.6).

The serving surface today is `reaper-wasm` + CLI check flows (there is no
agent `/api/v1/check` endpoint yet; `PolicyRequest` deliberately carries no
input field). Those surfaces call `check_with_input` on the preferred
evaluator — after this phase, that call hits the compiled driver instead of
the interpreter. An agent check endpoint is a separate product item, out of
scope here.

### 3.5 Check-mode compiled driver

`ReaperDSLEvaluator::check_with_input`: iterate ALL compiled deny rules
(not first-match), evaluating each with a fresh rule scope; collect
`Violation { rule, message }`. Messages compile to:

```rust
enum CompiledMessagePart { Literal(String), Variable(InternedString) }
struct CompiledMessage { parts: Vec<CompiledMessagePart> }   // concat(...) lowered
```

rendered from the rule's variables after its condition matched — same
visibility rule as the interpreter (that rule's bindings only). The
`allowed` composition copies the interpreter's exactly (violations empty →
default / any-allow-matches).

### 3.6 The no-principal edge

Document policies mostly have no principal; A.2's mixed evaluator routes
unknown-principal requests to its embedded whole-AST interpreter. Two cases:

- **Fully-compiled input policy** (the goal state for the k8s/tf corpus):
  served by `ReaperDSLEvaluator` — which errors on unknown principals
  today. Phase B therefore relaxes the compiled entry ONLY for the new
  `*_with_input` entries: an unknown/absent principal binds a synthesized
  empty user entity (attribute reads `Null`, fail closed) instead of
  erroring — the same treatment resource and actor already get. The
  legacy `evaluate()` path keeps its error contract (no behavior change
  for existing deployments).
- **Mixed input policy:** unchanged A.2 routing (whole-AST) — correct,
  just not yet fast; it converges as coverage closes.

### 3.7 What deliberately does NOT compile in this phase

Object/set comprehensions over input, nested comprehension sources, method
chains on input paths beyond the §1 string ops, `input` in cross-entity
comparisons, and dynamic (variable) path segments. Each keeps its per-rule
fallback and its trigger stays in the REGO_GAP_ANALYSIS §4 inventory. The
fitness instrument (`examples/specialization_fitness.rs` reports
AST-fallback policies) re-runs at the end of the phase to publish the
before/after counts.

## 4. Gates (merge-blocking, same bar as A.2)

1. **Decision differential:** the input corpus (three library classes +
   synthetic shape matrix) × document matrix (well-formed, missing paths,
   wrong-typed nodes, empty arrays, no document at all) — compiled/mixed
   decisions ≡ interpreter decisions.
2. **Check differential incl. messages:** `CheckResult` — `allowed`,
   violation set, AND rendered message strings — byte-identical to the
   interpreter over the same matrix. (Messages are user-visible audit
   artifacts; "close enough" is not a contract.)
3. **Scalar-comparison property test:** `json_cmp` ≡ convert-then-compare
   across random JSON scalars × every op (the int/float/u64 pin, §3.2).
4. **Frozen corpus:** the three library policies enter
   `policy-library/frozen/` with document fixtures, so future evaluator
   changes cannot silently alter admission decisions.
5. **Bench:** ≥5× interpreter throughput on the k8s admission policy with a
   representative AdmissionReview document (target from the plan; measured,
   not asserted).
6. **Fitness re-run:** AST-fallback count across corpora published in the
   PR (expected: policy-library 4→~1, benchmark corpora 15→single digits).

## 5. Slices (each independently shippable behind the gates)

- **B.1 (S/M):** `InputPath` IR + `InputCompare`/`InputStringOp` leaves +
  `EntityBindings.input` threading + `evaluate_with_input_named` on the
  compiled/mixed evaluators + no-principal relaxation (§3.6) + gates 1/3.
  Lands: `owner_label_required`-class rules compile.
- **B.2 (M):** `CompiledIterationSource::InputPath` + `InputVars` borrowed
  scope + filter pipeline + projections + result materialization bridge +
  gate 1 extended to the full corpus. Lands: the comprehension rules —
  the bulk of the class.
- **B.3 (M):** compiled check driver + `CompiledMessage` + gates 2/4/5/6.
  Lands: the class is served compiled end-to-end; numbers published.

## 6. Risks & mitigations

- **Silent semantic drift between value domains** (serde_json vs EvalValue
  comparisons) — the class of bug most likely to survive review. Mitigation
  is structural: gate 3's property test plus gate 2's byte-identical
  messages; any drift is a red build, not a code-review catch.
- **Lifetime plumbing** (`&'doc Value` through the comprehension scope) —
  contained: `InputVars` lives inside one rule evaluation; nothing borrowed
  escapes it except via the materialization bridge, which copies.
- **Scope creep toward full JSON-query compilation** — §3.7 is the fence;
  anything outside §1's corpus shapes falls back per-rule, visibly, and
  waits for demand.
