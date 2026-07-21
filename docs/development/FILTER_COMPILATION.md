# Policy→Filter Compilation (List Authorization)

**Status:** DESIGN — not scheduled. Written as the follow-through on the
Phase F.3 finding (`PARTIAL_EVALUATION.md` §6.1): per-decision partial
evaluation is not worth building in this engine because full evaluation is
already nanosecond-scale — but the *same specialization machinery*, applied
per **list query** instead of per decision, addresses a capability the
engine does not have at any speed: answering **"which resources may this
principal act on?"** without evaluating every resource.
**Decision input:** §10 (fitness check) and §11 (phasing + recommendation).

---

## 1. The problem: list authorization

Point evaluation answers `can(alice, read, doc123)?` in <1µs. It does not
answer the question every list endpoint, search page, and bulk job actually
asks:

> `SELECT * FROM documents WHERE <alice may read it> LIMIT 50`

Today a consumer has two bad options:

1. **Fetch-then-filter:** pull N candidate rows from the application
   database, point-evaluate each. Cost is O(N) evaluations *plus* O(N) data
   transfer for rows that will be thrown away. At N = 10⁶ and 0.5µs/eval
   that is ~0.5s of pure evaluation — and the data transfer dwarfs it.
   Pagination breaks too: "LIMIT 50 after filtering" may require fetching
   unbounded prefixes.
2. **Duplicate the policy in SQL by hand:** the WHERE clause and the policy
   drift apart, silently. This is the exact failure mode a policy engine
   exists to remove.

The filter model gives a third option: **compile the policy, for one
concrete (principal, action, context), into a residual predicate over the
unknown resource** — then answer it either natively against the agent's own
indexed DataStore, or hand it to the application's database as a
parameterized WHERE clause.

## 2. Prior art, and where this design sits

| System | Mechanism | Strengths | Gaps |
|---|---|---|---|
| **OPA / EOPA "data filters"** | Partial evaluation with `input.resource` unknown → residual Rego → translators emit SQL/UCAST | Mature idea; arbitrary Rego over knowns | Residual is interpreted Rego structures; translation is a separate library layer; OPA's own `data` is an unindexed JSON tree — no native fast ListObjects; verify-phase discipline left to the integrator |
| **Zanzibar-family (OpenFGA `ListObjects`, SpiceDB `LookupResources`)** | Reverse expansion over the relationship graph | Native, fast object listing | Relationships only — no ABAC (attribute predicates) in the same answer |
| **This design** | Ground known leaves of the *compiled* condition via the real evaluators, fold with tier-1, emit a small residual IR; answer in-engine via the multi-index store + reverse relationship index, or translate to parameterized SQL with a mandatory verify contract | One residual covers ABAC **and** ReBAC; in-engine backend needs no integration work; residual compiles in µs because it operates on `CompiledCondition`, not an AST | Bounded by what the compiled DSL path supports (AST-fallback policies are out of scope, §8) |

The distinctive claim: **Reaper agents already hold the entity data in a
multi-indexed store** (`crates/policy-engine/src/data/store.rs` —
`attribute_index`, `composite_index`, and the doubly-indexed
`RelationshipGraph`). OPA cannot answer "list what alice can read" natively
at index speed because its data model has no indexes; we can. SQL pushdown
then becomes the *second* backend for data that never enters the agent, not
the only path.

## 3. Semantic contract

For a fixed query `q = (principal, action, context)` and the deployed policy
set, define:

```text
Permitted(q) = { r ∈ Resources | evaluate(principal, action, r, context) = Allow }
```

The filter API must return **exactly** `Permitted(q)` (ids backend) or a
predicate/rows contract that composes to exactly `Permitted(q)` (pushdown
backend, §7). Both directions are load-bearing:

- **Soundness (no leaked rows):** nothing outside `Permitted(q)` is ever
  returned. A leak is an authorization bypass.
- **Completeness (no dropped rows):** everything in `Permitted(q)` is
  returned. Silent drops corrupt application behavior invisibly.

Deny precedence and the default decision compose per the engine's existing
semantics (`ReaperDSLEvaluator`: deny rules first, then allow, else
default):

```text
Permitted(r) = ¬DenyMatch(r) ∧ (AllowMatch(r) ∨ default = allow)
DenyMatch(r)  = ∨ residual(deny_rule_i, q)(r)
AllowMatch(r) = ∨ residual(allow_rule_j, q)(r)
```

Multi-policy evaluate-all composes the same way across the candidate policy
set (the pruning index's type tier already narrows which policies can
possibly match a given `resource_type` — reused as-is, §6.9).

## 4. Core machinery: per-query grounding

This is the parked tier-2 pass (`PARTIAL_EVALUATION.md` §3.3) with one
substitution: instead of specializing at *deploy* time against *data* (which
F.3 measured as an empty surface), specialize at *query* time against the
**known request coordinates** — principal, action, context — leaving only
`resource` unknown. F.3 proved the per-decision version has nothing to bite
on; the per-list version bites on *every* resource-anchored leaf, which the
corpus is full of.

### 4.1 Dependence analysis

Generalize F.3's `leaf_staticness` (exhaustive-match pinned,
`reaper_dsl/compiler.rs`) into a dependence set per leaf:

```rust
bitflags Dependence: PRINCIPAL | RESOURCE | ACTION | CONTEXT | ACTOR | VARIABLES | DATA
```

- The classifier stays an **exhaustive match with no wildcard** — a new
  `CompiledCondition` variant fails compilation until its dependence is
  declared. Same pin discipline as `binds_variables` and F.3.
- `VARIABLES` covers every `*Assignment` and `Variable*` shape: any rule
  containing them is **not residualizable** (rule-scoped binding order
  cannot be split across a ground/residual boundary) and takes the fallback
  path (§6.6 / §7.4). Conservative, never wrong.

### 4.2 Grounding pass

Per rule, walk the compiled condition bottom-up:

- Leaf with `RESOURCE ∉ deps`: **evaluate now**, with the exact runtime
  functions (`eval_attribute_comparison`, `eval_time_operation`, the ReBAC
  lookups, …) bound to the query's principal/context/actor — never a
  reimplementation, so ground truth ≡ eval-time truth by construction (the
  same rule tier-2 §3.3 set). Replace with `Always` / `Not(Always)`.
- Leaf with `RESOURCE ∈ deps`: **translate to a residual atom** (§5). A
  leaf that mixes resource with principal/context (e.g.
  `resource.owner == user.id`, `has_relation(principal, "owner", resource)`)
  resolves the known side to a *constant* first:
  `resource.owner == "alice"`, `RelatedTo("alice", "owner")`.
- Untranslatable resource leaf: `Opaque` atom (§5) — forces the verify
  path, never guesses.
- Run the result through the REAL tier-1 `fold_condition`. The residual
  falls out: rules whose ground part is false vanish (`false ∧ … → false`);
  rules that don't mention the resource collapse to a constant.

Cost: one grounding pass per rule per list query — microseconds, amortized
over the whole result set instead of paid per row.

## 5. Residual IR

A small closed predicate language over one unknown resource:

```rust
enum ResidualAtom {
    /// resource.id == "x" (from ResourceIdEquals)
    IdEquals(InternedString),
    /// resource.<attr> <op> <const>   (op: Eq/Ne/Gt/Ge/Lt/Le)
    AttrCompare { attr: InternedString, op: NumericOp, value: ResidualConst },
    /// <const> ∈ resource.<attr>  (MembershipTest / wildcard forms)
    AttrContains { attr: InternedString, value: ResidualConst },
    /// resource.<attr> string ops (contains/starts/ends; regex carries the pattern)
    AttrString { attr: InternedString, op: StringOp, value: String },
    /// principal-side-resolved ReBAC: resource ∈ related_to(subject, relation)
    /// (kind + via + max_depth preserved for the graph walk)
    RelatedTo { subject: InternedString, relation: InternedString, kind: RebacKind, .. },
    /// A resource-dependent leaf the IR cannot express (comprehensions over
    /// resource collections, SameEntityAttrCompare on resource, …).
    /// Carries the original CompiledCondition for the verify path.
    Opaque(Box<CompiledCondition>),
}
enum Residual { True, False, Atom(ResidualAtom), And(Vec<Residual>), Or(Vec<Residual>), Not(Box<Residual>) }
```

Design rules:

- **`Opaque` is a first-class citizen, not an error.** It is the honesty
  valve: anything the IR cannot express degrades to per-candidate point
  evaluation of exactly that subtree, so coverage grows leaf-shape by
  leaf-shape without ever being wrong.
- The IR is backend-neutral: the ids backend interprets it against the
  DataStore; the SQL backend prints it; `mode=ir` returns it verbatim (JSON)
  for external translators — the analog of OPA's residual, but already
  normalized and constant-folded.

## 6. Backend A: in-engine ids (`mode=ids`) — the differentiator

Answers `Permitted(q)` directly from the agent's DataStore. Two-phase:

1. **Index phase — narrow candidates.** Interpret the residual as set
   algebra over entity-id sets:
   - `IdEquals` → singleton.
   - `AttrCompare{Eq}` / `AttrContains` (string consts) → one
     `attribute_index` / `composite_index` lookup (100–300ns) — these
     indexes exist today and are exactly (attr, value) → id-set.
   - `RelatedTo` → the reverse relationship index
     (`RelationshipGraph::related_to`, plus the bounded BFS for
     reachable/inherited kinds under the existing traversal budget).
   - `And` → intersect; `Or` → union.
   - Non-indexable atoms (`Gt/Lt` ranges, string prefix, regex, `Opaque`)
     and any `Not(...)` contribute **no narrowing** (treated as "all of the
     resource type") — they are handled by phase 2. `Not` is deliberately
     never answered by set complement in phase 1: complement of an index
     set over the store's live population is a moving target; the verify
     phase makes it exact.
   - The candidate universe is bounded first by `resource_type` (the
     `type_index`), reusing the pruning-index type tier's discipline.
2. **Verify phase — make it exact.** Point-evaluate the surviving
   candidates with the ordinary evaluator (ground truth). Because phase 1
   only ever *narrows from a superset*, and phase 2 is the real evaluator,
   the result is exactly `Permitted(q)` **regardless of how coarse phase 1
   was**. Worst case (all atoms non-indexable) degrades gracefully to
   evaluate-all-of-type — never to a wrong answer.

Additional contract points:

- **Epoch stamping (F.2 pays off):** the response carries
  `data_epoch` observed at the start; if it changed by the end, the agent
  retries once, then returns the result flagged `epoch_moved: true`. List
  answers are inherently snapshots; the flag makes the race visible instead
  of pretended away.
- **Budgets:** `max_candidates` (server-configured, like
  `max_candidate_policies`) caps the phase-2 set; exceeding it returns a
  structured `candidates_exceeded` error (fail closed, mirror of
  `candidate_cap_exceeded`) telling the caller to push down (§7) or narrow
  by type.
- **Pagination:** deterministic order (sorted interned ids), keyset cursor
  (`after_id`), same envelope as the management API's `Paginated`.
- **Rules with `VARIABLES`:** the whole rule skips residualization and is
  evaluated in the verify phase only (its phase-1 contribution is "no
  narrowing"). Sound by the same superset argument.

## 7. Backend B: pushdown translation (`mode=sql`)

For resource populations that never enter the agent (the application's own
database). The output is a **pre-filter + verify contract**, not a bare
string:

```jsonc
{
  "sql": "(doc_type = $1 AND (owner_id = $2 OR visibility = $3))",
  "params": ["report", "alice", "public"],
  "exact": false,          // true ⇒ rows need no verification
  "unverified_atoms": 1,   // count of atoms widened out of the filter
  "data_epoch": 42
}
```

- **Parameterized always.** Literals are emitted as placeholders + a params
  array; the translator never string-interpolates a value. (Injection
  safety is structural, not a sanitization step.)
- **Attribute→column mapping is operator-supplied config** (per resource
  type: `attr "owner" → column "owner_id"`, typed). An unmapped attribute
  is not an error: its atom **widens**.
- **Widening discipline (the soundness core):** the pushed filter must be a
  provable **superset** of `Permitted(q)`. Any atom the dialect/mapping
  cannot express is *widened to `TRUE` in positive position and pushed down
  into `exact:false`* — never dropped from a disjunction (which would leak
  nothing but drop rows? no: dropping a disjunct *shrinks* — completeness
  violation; widening a conjunct *grows* — safe). Formally: widening
  replaces untranslatable subtrees with `TRUE` under an even number of
  negations and `FALSE` under odd, which always yields a superset.
  `exact:true` is set only when zero atoms were widened AND the residual
  had no `Opaque` — only then may the caller skip verification.
- **Verification:** when `exact:false`, the caller MUST point-evaluate
  returned rows (one `POST /api/v1/messages` batch — the batch endpoint
  exists). The SDK wraps this so the app sees one call:
  `client.filter_rows(q, rows)`.
- **ReBAC atoms:** resolved in-engine to an id-set first
  (`related_to(subject, relation)`), emitted as `col IN ($n…)` when the set
  is under a size cap (config, e.g. 1k); over the cap the atom widens.
  (A join-table mapping for app-side edges is a later extension; out of
  scope for the first cut.)
- **Dialects:** start with one neutral ANSI subset + params style flag
  (`$n` / `?`). No ORM adapters in-engine; `mode=ir` is the extension point
  for external translators (UCAST/Prisma layers can consume the IR).

## 8. Scope boundaries (honest limits)

- **Compiled path only.** AST-fallback policies (`input` document access,
  `jwt::decode`, …) are out of tier-2's reach for the same reason as
  before; a filter query against a policy set containing them fails with a
  structured error naming the policies (fail closed, visible). Corpus
  measurement of how much this excludes: the F.3 instrument already counts
  it (4/18 in policy-library).
- **`resource_type` is strongly recommended** and may be made mandatory in
  v1: it bounds phase-1/phase-2 universes and matches how list endpoints
  actually work ("list *documents*").
- **Context trust (taint):** `TaintTrusted` leaves are ground (context is
  known) — evaluated in the grounding pass with the same provenance rules
  as point eval.
- **The answer is a snapshot.** Same guarantee point evaluation gives; the
  epoch stamp makes it auditable.

## 9. Reuse map (why this is cheaper than it looks)

| Existing asset | Role here |
|---|---|
| F.2 `data_epoch` (shipped) | Snapshot stamping + retry-on-move |
| F.3 `leaf_staticness` + exhaustiveness pin (shipped) | Seed of the dependence classifier (§4.1) — same file, same discipline |
| Tier-1 `fold_condition` + binding guard (shipped) | Residual normalization; `VARIABLES` fallback rule |
| Tier-2 §3.3 grounding principle (designed) | "Evaluate known leaves with the real eval functions" — unchanged, retargeted |
| Pruning index type tier (shipped) | Candidate-universe bounding; the filter is its inverse (it answers "which policies for this resource", this answers "which resources for this policy") |
| `attribute_index` / `composite_index` / `related_to` (shipped) | Phase-1 set algebra — zero new index structures |
| Batch evaluate endpoint + evaluate-all (shipped) | Verify phase, both backends |
| SLO harness + differential-test patterns (shipped) | The gates (§10) drop into existing harnesses |

New surface: the IR (+ its interpreter and SQL printer), the grounding walk,
one agent endpoint, config for column mappings. No new storage, no new
indexes, no hot-path changes to point evaluation.

## 10. Gates and fitness check (same bar as F.3/R3-P2-1)

**Correctness gates (merge-blocking):**

1. **The differential:** for random (policy corpus × entity stores ×
   queries), `filter(q)` ids ≡ brute-force `{r | evaluate(q, r) = Allow}`
   over every entity of the type. Property test, both backends (`mode=sql`
   executed against in-CI SQLite with a generated column mapping).
2. **Deny-precedence composition:** targeted cases where allow and deny
   residuals overlap, including default-allow and default-deny policies.
3. **Widening soundness:** property test that `exact:false` SQL results are
   always a superset (never a proper subset) of permitted rows.
4. **Epoch race:** mutate the store mid-filter; assert the flag/retry
   behavior, never a silently mixed snapshot.
5. **Fallback completeness:** corpora with `VARIABLES` rules and `Opaque`
   atoms must produce identical results to brute force (the graceful-
   degradation path is the one most likely to rot; it gets the same
   differential).

**Fitness check (before building, mirror of §6 discipline):** extend the
F.3 instrument to report, per corpus: % of rules fully residualizable, % of
resource-anchored leaves index-answerable (phase-1-effective), % requiring
verify-only. Build G.2 only if the phase-1-effective share is meaningful
(else the in-engine backend is just evaluate-all with extra steps — note
evaluate-all + type tier may already be an acceptable v0 for small stores).
**Product fitness is the real gate:** this is a *capability* (new API),
not an optimization — it should be pulled by a consumer (SDK list helper,
management list endpoints, a design partner) rather than pushed.

## 11. Phasing & effort

- **G.1 — Dependence classifier + residual IR + grounding + differential
  gate.** Engine-only, no API. The gate is the deliverable. (M)
- **G.2 — In-engine ids backend + `POST /api/v1/filter` (`mode=ids`) +
  budgets/pagination/epoch + SLO row.** First shippable user value. (M)
- **G.3 — SQL printer + column-mapping config + widening discipline +
  SQLite differential + SDK verify helper (`mode=sql`, `mode=ir`).** (M–L)
- **G.4 — ReBAC depth/perf work: reachable/inherited reverse expansion
  tuning, id-set caps, `IN`-list vs join strategies.** (S–M, demand-driven)

Each phase independently shippable; G.1's differential gate protects all of
them. Nothing touches the point-evaluation hot path at any phase.

## 12. Comparison summary (for the build/no-build decision)

- **vs OPA:** OPA's partial evaluation + data-filter translators are the
  same *idea*; the differences are (a) our residual is computed over
  already-compiled conditions with real-evaluator grounding (µs, not ms;
  no semantic gap between residual and runtime), (b) OPA has no native
  indexed ids backend — with OPA you must push down or fetch-then-filter;
  our agents can answer id-lists locally, (c) our verify contract is
  explicit in the API (`exact`, `unverified_atoms`) instead of
  integrator folklore.
- **vs OpenFGA/SpiceDB:** they answer ListObjects natively for pure
  relationships; we answer ABAC + ReBAC in one residual against one store.
  We do not (and should not) replicate their scale story for billion-edge
  graphs; our reverse expansion is bounded by the existing traversal
  budget.
- **The honest anti-claim:** if no consumer needs list authorization, this
  is shelf-ware with a maintenance cost — the fitness check (§10) exists
  for exactly that reason.
