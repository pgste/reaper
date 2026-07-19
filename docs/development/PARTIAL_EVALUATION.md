# Partial Evaluation for the Compiled DSL Path

**Status:** Tier 1 shipped (Plan 06 Phase F, this document's sibling PR).
Tier 2 designed here, not yet implemented.
**Owner plan:** `plans/round-3/06-ga-hardening.md` §7 Phase F.

## 1. Context: what "partial evaluation" means here, and what already exists

Partial evaluation = doing at **deploy time** whatever part of policy
evaluation does not depend on the request, so the per-request loop only pays
for what is genuinely dynamic. The codebase currently has three related
artifacts, at very different levels of reality:

| Artifact | What it does | Serving-path status |
|---|---|---|
| `ReaperDSLEvaluator` compile pass (`reaper_dsl/compiler.rs`) | Lowers AST → `CompiledCondition` (pre-interned strings, pre-partitioned deny/allow, regex/membership caches) **+ tier-1 constant folding** | **PRIMARY** — serves every compiled `.reap` policy |
| Pruning index (Plan 08 A, R2-D2, R3-P2-1) | Statically bounds each policy's match set (resource-id tier + resource-type tier); evaluate-all only touches candidates | **LIVE** — this is partial evaluation *relocated into the index*: the data-dependent fact (resource → type) is resolved once per request, not once per policy |
| `partial_evaluation.rs` + `CompiledPolicyEvaluator` (`compiled_evaluator.rs`) | Generic `Condition` simplifier + Simple-rule-model evaluator taking a `static_context` | **NOT WIRED** — `EnhancedPolicy::build_compiled()` has no serving-path caller; only the `comparison_baseline_vs_compiled` example uses it |

### Tier 1 (shipped): structural constant folding

`compiler::fold_condition` runs once per rule at deploy, after
`compile_condition`:

- `!!x` → `x`
- `And`: splice nested `And`s (order-preserving), drop `true` conjuncts;
  a `false` conjunct folds the conjunction to `false`
- `Or`: splice nested `Or`s, drop `false` disjuncts; a `true` disjunct folds
  the disjunction to `true`
- Bottom-up, so inner constants propagate: `(false || true) && x` → `x`

**The one soundness subtlety — variable bindings.** Condition evaluation has
one side effect: `*Assignment` conditions write the rule-scoped `variables`
map, and a binding made inside one branch is visible to every *later*
condition of the same rule. An eliminating fold (dropping a child, replacing
a subtree with a constant) skips evaluations the runtime would have
performed. Example: `(let x = user.role && false) || x == "admin"` — folding
the left disjunct away leaves `x` unbound and flips the outcome. Every
eliminating fold is therefore gated on `binds_variables(subtree) == false`;
the binding-variant list is pinned by an exhaustiveness test
(`test_binds_variables_covers_every_assignment_variant`) so a new binding
variant cannot silently make the folds unsound. Splicing and `!!x` need no
guard (they change no evaluation set, only nesting).

Rules whose condition folds to `false` are **kept** (one cheap check per
request), not dropped: dropping would change `validate()`'s at-least-one-rule
behavior and rule-count metadata for `allow if false` policies that deploy
successfully today.

Correctness gates: the compiled-vs-AST equivalence differential
(`compiled_ast_equivalence_tests`), the parity/builtin oracles, and the fold
unit tests. Folding must never diverge the compiled evaluator from the AST
evaluator — the AST fallback deliberately does NOT fold.

## 2. Tier 2: deploy-time specialization against data — the problem

The tier-2 promise (what `partial_evaluation.rs`'s module docs describe) is
to pre-evaluate conjuncts whose truth is determined by **deploy-time data**,
so a 4-conjunct rule becomes a 2-conjunct rule at runtime.

### 2.1 Be honest about how much is actually static

In this engine, less than the classic OPA-style picture suggests. A conjunct
is specializable only if its truth does not depend on the request. Walking
the `CompiledCondition` leaves:

| Conjunct shape | Depends on | Specializable? |
|---|---|---|
| `action == "read"`, `resource == "x"` | request | No |
| `user.<attr> …` | request principal → entity | No (principal varies per request) |
| `resource.<attr> …` | request resource → entity | No (resource varies; the **type tier already captures the useful static residue** of this shape) |
| `context.<key> …`, `taint::trusted` | request context | No |
| ReBAC over `principal`/`resource` refs | request | No |
| ReBAC with **both refs literal** (`has_relation("team_a", "owns", "repo_1")`) | relationship graph only | **Yes** |
| Comparisons where **every operand is a literal or a literal-named entity's attribute** | DataStore only | **Yes** |
| Anything under an operator-declared **static context** (tenant id, region, deployment environment — fixed for the agent's lifetime) | agent config | **Yes** |

So tier 2's real surface is: (a) literal-entity attribute reads, (b)
literal-literal ReBAC checks, (c) agent-static context. These are the
"feature-flag conjunct" and "environment conjunct" patterns:
`deny if config.maintenance_mode == true && …`,
`allow if context.region == "eu" && …` under a pinned region. That is a real
but *narrow* win — the design below is sized accordingly, and tier 2 should
only be built if profiling shows these patterns in real policy corpora
(fitness check in §6).

### 2.2 Why the naive version is unsound: staleness

Anything pre-evaluated against the `DataStore` goes stale the moment the
store mutates — and the agent mutates it at runtime (`/api/v1/data`,
`/api/v1/data/stream`, clear-and-reload). A specialized policy that baked in
`config.maintenance_mode == false` keeps allowing after an operator flips the
flag: **fail-open**. This is the same class of defect the type tier's
same-store rule guards against, and it is why tier 2 must not ship without
the invalidation design below.

## 3. Tier 2 design: epoch-stamped specialization with fail-safe fallback

### 3.1 Data epoch

`DataStore` gains a monotonically increasing **data generation**:

```rust
pub struct DataStore {
    …
    /// Bumped on every mutation batch (insert/remove/clear/bulk load).
    data_epoch: AtomicU64,
}
pub fn data_epoch(&self) -> u64  // Relaxed load
```

Bump sites: `insert`, `remove`, `clear`, `DataLoader` bulk paths, and the
relationship graph's mutation entry points. Batch loads bump **once at the
end** (a half-loaded batch already isn't an atomic unit today; the epoch must
not make it look like one). The counter is cheap (one relaxed `fetch_add`
per mutation batch, not per entity).

### 3.2 Specialized-rule cache on the evaluator

```rust
struct SpecializedRules {
    /// DataStore::data_epoch() observed when this specialization was built.
    epoch: u64,
    /// Same shape as the general rules, with data-static conjuncts folded
    /// to Always / Not(Always) and then tier-1-folded.
    deny_rules: Vec<CompiledRule>,
    allow_rules: Vec<CompiledRule>,
}
// On ReaperDSLEvaluator:
specialized: ArcSwapOption<SpecializedRules>,
```

**Request path** (the only part that must stay nanosecond-cheap):

```text
let spec = self.specialized.load();
let rules = match &*spec {
    Some(s) if s.epoch == self.store.data_epoch() => (&s.deny_rules, &s.allow_rules),
    _ => (&self.compiled_deny_rules, &self.compiled_allow_rules),   // fail-safe
};
```

One relaxed atomic load + compare. On mismatch the request **falls back to
the general (unspecialized) rules immediately** — it never blocks on
re-specialization and never serves a stale specialization. Fail-safe
direction: the general rules are always present and always correct;
specialization is strictly an optimization overlay.

**Re-specialization** happens off the request path: after any data-load
handler completes (the same places that bump the epoch), and at deploy time.
Debounce: a re-specialize task snapshots the epoch first, builds, then CAS-
publishes only if the epoch is still current (a load racing in during the
build just leaves the fallback in place until the next trigger).

### 3.3 The specialization pass

A sibling of `fold_condition` (same file, same bottom-up shape):

```rust
fn specialize_condition(cond: &CompiledCondition, store: &DataStore) -> CompiledCondition
```

- The three §2.1-static leaf shapes are evaluated against `store` **using the
  exact evaluation functions the runtime uses** (`eval_attribute_comparison`
  etc. — never a reimplementation, so specialize-time truth ≡ eval-time
  truth by construction) and replaced with `Always` / `Not(Always)`.
- Every other leaf is copied unchanged.
- The result is passed through tier-1 `fold_condition`, which is what
  actually shortens rules (`false && …` → `false`, drop `true` conjuncts).
- The tier-1 **binding guard applies unchanged** — a specialized-away subtree
  containing an assignment must not be eliminated.
- Conservative default: any leaf not provably data-static is dynamic. An
  unrecognized shape can only cost speed, never correctness.

### 3.4 Interaction with the pruning index

`resource_pruning()` must keep answering from the **general** rules, not the
specialized overlay. The index is rebuilt only on policy deploy; if bounds
came from a specialization that data reloads later invalidate, the index
would need data-driven rebuilds — a much bigger machine than this design
wants. Cost of that choice: none for soundness (general bounds are a superset
of specialized bounds); a policy whose rule specializes to `false` still
occupies its index buckets — acceptable.

### 3.5 Config & rollback

- `REAPER_USE_SPECIALIZED_POLICIES` (default **false** for the first
  release, flipped to true one release later, mirroring how the pruning
  index shipped). Off ⇒ the overlay is never built or consulted; the eval
  path is byte-identical to today's.
- The epoch counter ships unconditionally (it is also independently useful
  as an observability signal: `data_epoch` in `/health`).

## 4. Soundness argument & test gates (hard gates, same bar as R3-P2-1)

1. **Differential (the merge gate):** for a corpus of policies × entity
   stores × requests, decisions/matched/rule-name from the specialized
   overlay must be identical to the general rules. Extends
   `compiled_ast_equivalence_tests`' harness: general vs specialized instead
   of compiled vs AST.
2. **Staleness property test:** for random sequences of {data mutation,
   evaluate}, every evaluate's outcome must equal a never-specialized
   engine's outcome. This is the test that would have caught the naive
   design: after any mutation, the stale overlay's epoch mismatches and the
   fallback serves the general rules.
3. **Epoch-bump completeness:** a mutation path that forgets to bump the
   epoch is the remaining fail-open hole. Pin with a test enumerating every
   public `DataStore` mutation entry point and asserting the epoch strictly
   increases across each (the mirror of the R3-P3-1 lock-step discipline —
   and like `binds_variables`, an exhaustiveness pin, so a new mutation
   method cannot ship unbumped).
4. **Binding preservation:** re-run the tier-1 binding-guard tests through
   `specialize_condition` (a specialized `false` must obey the same
   elimination rules).

## 5. Disposition of the legacy artifacts (closes R3-P3-3 properly)

`partial_evaluation.rs` + `CompiledPolicyEvaluator` are superseded by this
design: they operate on the Simple-rule string model, not `CompiledCondition`,
and their only consumer is one example. When tier 2 lands:

- Port `OptimizationStats` (conditions before/after, rules shortened) onto
  the specialization pass — it is the right observability surface.
- Delete `partial_evaluation.rs` and `CompiledPolicyEvaluator`, and re-point
  the `comparison_baseline_vs_compiled` example at the real evaluator with
  the flag on/off.

Until then they stay as-is (harmless, exercised by their own tests).

## 6. Fitness check before building tier 2

Tier 2 is worth its complexity only if real corpora contain the §2.1-static
shapes. Before implementation: instrument `specialize_condition` in dry-run
mode (count specializable leaves, log `OptimizationStats`) against the
policy-library and frozen-decision corpora plus at least one production-like
bundle. If < ~5% of rules shorten, park tier 2 and keep only the epoch
counter (observability) — the pruning index has already captured the bulk of
the win.

## 7. Phasing

- **F.1 (shipped):** tier-1 constant folding in the compiled path.
- **F.2:** `data_epoch` counter + `/health` exposure + bump-completeness test.
- **F.3:** dry-run `specialize_condition` + `OptimizationStats` + fitness
  measurement (§6). **Decision point.**
- **F.4:** overlay + fallback path behind `REAPER_USE_SPECIALIZED_POLICIES`,
  with gates §4.1-4.4.
- **F.5:** legacy artifact deletion (§5), flag default flip one release
  later.

Each step independently shippable; F.4 is the only one touching the eval hot
path and it is guarded by the flag and the differential gates.
