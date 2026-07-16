# Reaper — Performance Review, Round 3 (Persona 1: Systems Performance Engineer)

**Auditor mandate:** hostile, bank-grade third-pass re-review. Verify Round-2's perf
closures are *real*, and find what is newly wrong on the surfaces that grew since (F1
agentic authz / capability gate, the served-path SLO harness, the DSL pruning index).
Priority order per the shared rules: single-eval hot path (`evaluators/reaper_dsl/`,
`engine/`, agent `evaluate.rs`) > API eval surface > distribution touchpoints > audit
isolation > system-level gates. I did **not** re-audit Cedar internals, the
control-plane request path, the sync/promotion pipeline, or eBPF beyond where they
touch eval.

---

## VERDICT: READY (P2s only) — with two conditions before GA / before enabling F1 at scale

No P0, **no P1**. This is a real change from Rounds 1–2. Every prior blocking finding
is closed **in code, not on paper**, and I verified each against source:

- **R2-P1-1 (unmeasured served SLO) — CLOSED.** There is now a **blocking** paired
  A/B *HTTP* gate that drives the request-total path (`perf-gate.yml:115-198`,
  `http-slo-ab` job: base vs head agents, interleaved, same runner, `--http-threshold
  1.25` on request-total p99) *and* a nightly absolute SLO harness over a real release
  agent across all four §3 rows (`slo-harness.yml`). The SLA is now measured, not
  asserted.
- **R2-P2-1 (DSL unprunable) — SUBSTANTIALLY closed** for literal-resource DSL: the
  compiled evaluator now extracts `resource == "literal"` constraints via a sound
  set-algebra walk (`reaper_dsl/mod.rs:1844-1917`, delegated through
  `PolicyEvaluator::resource_index_terms`). Remaining gap is narrower and is my new
  **P3-1** below (ABAC/ReBAC shapes are still unprunable by construction).
- **R2-P2-2 (mandatory `Block` blocks the reactor) — CLOSED.** The eval hot path now
  uses `log_durable` (`decision_buffer.rs:922-957`): `try_send` only (never a blocking
  `send`) + an async `oneshot` ack under `DURABLE_ACK_TIMEOUT`, awaited off-reactor.
  The blocking `Block` branch survives only in `log()` and is documented as no longer
  reached from the async path (`decision_buffer.rs:893-896`).
- **R2-P2-3 (denies skip the SLA histogram) — CLOSED.** `observe_early_return`
  (`evaluate.rs:138-145`) feeds every early-return deny and the 503 audit-gate into the
  histogram under a constant `early_deny` label; both endpoints call it on all return
  paths.
- Decision cache sharded + allocation-free commutative fingerprint
  (`decision_cache.rs`), ReBAC per-eval budget with thread-local scratch
  (`relationships.rs:54-75,282-355`), `SmallVec<[Uuid;1]>` on the targeted path,
  configurable worker threads, and a 16 MB per-eval body limit distinct from the 256 MB
  bulk limit (`main.rs:656`) are all present and correct.

What keeps this **conditional in spirit**: the two marquee *new* surfaces since Round 2
each carry a P2. (1) The pruning index — the mechanism the whole evaluate-all SLO rests
on — is inoperative for exactly the policy shapes the product is sold on (ABAC/ReBAC);
it only prunes exact-resource-id policies. (2) The F1 capability gate does an **uncached
ed25519 verification inline on the async reactor for every agentic request**, an
asymmetric-cost DoS vector and a throughput ceiling that has never been load-tested. Fix
both before GA and before a bank turns on agentic auth at scale. Because the *primary,
recommended* enforcement mode (targeted policy-id/name lookup, non-agentic) is unaffected
by either — it is O(1) lookup + one compiled walk and genuinely meets the sub-µs claim —
neither rises to P1.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R3-P2-1 | P2 | `reaper_dsl/mod.rs:1867-1917`; `engine/mod.rs:337-351` | The pruning index only extracts terms from `resource == "literal"` (`ResourceIdEquals`). Every ABAC/ReBAC predicate (`resource.type == …`, `resource.owner == principal`, `has_relation(...)`, wildcards, dynamic ids) yields `None` → **unprunable**. Real ABAC/ReBAC policies constrain resources by *attribute/relation*, not literal id, so at scale they are **all** unprunable. | For the mandated languages, evaluate-all returns the whole unprunable set + an **O(U log U) `sort`/`dedup` per request** (`engine/mod.rs:348-349`); at 10k such policies the 256 `max_candidate_policies` cap fires → blanket `candidate_cap_exceeded` denials (availability incident), or with the cap raised, O(U) eval + O(U log U) sort/request — far above the ≤25µs evaluate-all SLO row. | Wire `partial_evaluation.rs` (in-tree, unused) to extract resource *type/prefix* buckets, or add a resource-type index tier so ABAC policies are prunable by type; until then document the evaluate-all SLO row as "literal-resource policies only" and keep evaluate-all `false` (as it defaults). |
| R3-P2-2 | P2 | `capability_gate.rs:71`; `verify.rs:142-158`; `capability.rs:320-380` | The F1 capability gate runs `verify_capability` (ed25519 `verify_raw` + canonical-message serialization) **inline on the tokio worker** for every agentic request, with **no verification-result cache**. | ~30-50µs CPU crypto on the reactor per agentic request; the *same* capability is re-verified on every call (a 5k-rps agent burns ~0.25 core/s on redundant crypto); asymmetric-cost DoS — a caller (authenticated, but any tenant) sends garbage-signature capabilities that each cost a full verify before rejection, with no rate limit. Not `spawn_blocking`. | Cache verification verdicts keyed by `(capability_id, key_id, signature, minute-bucket)` with the revocation-generation folded in; and/or move the verify to `spawn_blocking` above a small size threshold; add a per-tenant cap-verify rate limit. Load-test the agentic path before a bank enables F1. |
| R3-P3-1 | P3 | `engine/mod.rs:497-511`; `arc-swap` load | `evaluate_set` calls `self.get_policy(id)` per candidate — one `ArcSwap::load` (guard alloc + acquire) **per policy** in an evaluate-all fan-out, instead of loading the `ActiveSet` once and reusing it. | N arc-swap loads for N candidates; measurable at 256-candidate fan-outs, redundant with the guard the loop could hold. | Load `self.active.load()` once at the top of `evaluate_set` and index the snapshot directly. |
| R3-P3-2 | P3 | `arena.rs` (whole file); `reaper_dsl/mod.rs:1699` | The bumpalo arena is **still dead on the hot path** (grep: `with_arena`/`reset_arena` unused outside `lib.rs`/`arena.rs`). The DSL still allocates a fresh `std::collections::HashMap` (default SipHash) for `variables` per eval — lazily, so only policies that bind variables pay it, but with the slow hasher. | R2-P3-1 not addressed; per-eval heap + SipHash churn for variable-using policies; dead maintenance surface. | Route the `variables` map through the arena (or at least `FxHashMap`), or delete `arena.rs`. |
| R3-P3-3 | P3 | `services/` grep = empty | `indexed_engine.rs` / `optimized_engine.rs` / `partial_evaluation.rs` are **still unwired** into any service (R2-P3-2 unresolved). `partial_evaluation.rs` is exactly the asset R3-P2-1 needs. | Three parallel engines carried; the most useful (partial eval) sits unused while the pruning gap it would close stays open. | Wire partial-eval into the served index path or delete the trio. |
| R3-P3-4 | P3 | `evaluate.rs:1214`; no `Semaphore` in agent | No cap on **concurrent** batch requests (R2-P3-4 unresolved). Each batch spawns a `spawn_blocking` task fanning out on the global rayon pool; the per-batch count cap (1000) does not bound concurrency. | A flood of concurrent ≤1000-item batches oversubscribes the blocking pool and thrashes rayon — an availability vector distinct from the per-batch cap. | Bound concurrent batch tasks with a `tokio::sync::Semaphore`. |
| R3-P3-5 | P3 | `perf-gate.yml:80-88`; `fast_evaluate_policy:725-730` | (a) The blocking *criterion* gate still benches only 2 suites (`policy_evaluation_bench`, `rebac_bench`) — no reload/caching/data-scaling bench (R2-P3-5); the new `http-slo-ab` job mitigates the served-path gap but not these. (b) `looks_agentic` runs **three full-body `memmem` scans** per fast-path request. | (a) Regressions in reload/cache-contention/data-scaling invisible to the blocking gate. (b) O(3·body) dispatch cost on the "fast" lane; negligible for small bodies, linear for large ones. | Add the reload/caching/data-scaling benches to the paired criterion gate; cap `looks_agentic`'s scan length or gate it on a cheap first-byte/size check. |

---

## Detailed findings

### R3-P2-1 — Pruning index only prunes literal-resource policies; ABAC/ReBAC stay unprunable

**Trace.** Evaluate-all (`evaluate.rs:328` / fast `:887`) calls
`candidate_policy_ids(resource)` → `engine/mod.rs:337`, which returns the resource
bucket **plus every `unprunable` policy**, then `sort()`+`dedup()`s. Membership of the
bucket vs `unprunable` is decided by `index_terms` → `resource_index_terms()` →
`compiled_resource_index_terms()` (`reaper_dsl/mod.rs:1844`). That walk is **sound** — I
verified the set algebra (`condition_resource_constraint`, `:1867-1917`): the only
compiled leaf that binds the *request resource identity* is
`CompiledCondition::ResourceIdEquals`; `And` → intersection of bounded children, `Or` →
union iff every child is bounded, everything else → `None` (unbounded). A resource
absent from the union provably makes every rule non-matching, and the set combiner
treats non-matching as non-decisive — so pruning is correct and can never fail open.

**The problem is coverage, not soundness.** A realistic ABAC/ReBAC rule —
`allow when resource.type == "document" and user.dept == resource.dept`, or
`allow when has_relation(resource, "viewer", user)` — contains **no `ResourceIdEquals`
leaf**. Every predicate is an attribute compare or a relation check, each returning
`None`. `condition_resource_constraint` therefore returns `None` for the rule, `?`
propagates it, and `compiled_resource_index_terms` returns `None` → the policy is
`unprunable` (`engine/mod.rs:170-173`). Only policies phrased as literal
`resource == "/admin/x"` (RBAC-over-paths) gain concrete terms.

**Consequences at 10k realistic DSL policies (evaluate-all mode).**
- `unprunable.len() == 10_000`. `candidate_policy_ids` copies all 10k ids, then
  `ids.sort(); ids.dedup()` — **~O(10⁴·log 10⁴) ≈ 130k comparisons on every request**,
  a per-request cost the Round-1 fix was supposed to remove.
- With the default cap (`max_candidate_policies = 256`, `settings.rs:362`), every
  evaluate-all request has `candidate_ids.len() = 10_000 > 256` →
  `candidate_cap_exceeded` **deny** (`evaluate.rs:358`). Evaluate-all becomes a blanket
  denial generator for the mandated languages.
- Raise the cap to 10k and each request pays the full O(N) fan-out *plus* the O(N log N)
  sort — nowhere near the ≤25µs p99 evaluate-all SLO row.

**Why only P2.** `allow_evaluate_all` **defaults `false`** (`settings.rs:347`) and
ADR-2 explicitly frames policy-less fan-out as a DoS amplifier. The **recommended and
default** enforcement mode is *targeted* (caller supplies `policy_id`/`policy_name`):
that path is an O(1) `DashMap` lookup + one compiled walk and fully meets the sub-µs
claim. So this bites only the opt-in, discouraged mode — but it bites it for exactly the
use case the product headlines, which is why it must be fixed before the evaluate-all
SLO row can be honestly claimed.

**Remediation.** Add a resource-*type*/prefix index tier (extract `resource.type == T`
and literal prefixes from compiled conditions via `partial_evaluation.rs`) so ABAC
policies bucket by type instead of collapsing to `unprunable`; replace the per-request
`sort`+`dedup` over `unprunable` with a pre-sorted, swap-time-built id slice held in the
`ActiveSet`; and until type-pruning ships, document the evaluate-all SLO row as
"literal-resource policies only."

---

### R3-P2-2 — Capability gate does uncached ed25519 verification inline on the reactor

**Trace.** `evaluate_policy` calls `capability_gate::enforce(...)` at `evaluate.rs:207`
— directly in the async handler, **not** inside `spawn_blocking`. When a capability is
present, `enforce` calls `state.bundle_verifier.verify_capability(cap, now)`
(`capability_gate.rs:71`), which does:

```rust
// verify.rs:152-156
let revoked = self.revocation.capability_revocations(now)?;   // RwLock read + Arc::clone (cheap)
cap.verify_at(key, expected_key_id, now, &revoked)            // ed25519 verify (~30-50µs CPU)
```

and `verify_at` (`capability.rs:348-350`) hex-decodes the signature, rebuilds the
canonical message, and runs `verifying_key.verify_raw(...)` — a full ed25519
verification — **on every agentic request**, with no memoization of the verdict.

**Impact.**
1. **Reactor-blocking crypto.** ~30-50µs of pure CPU on a tokio worker per agentic
   request. For a product whose eval slice is sub-µs, the crypto is 50-1000× the eval
   cost and it runs on the same threads serving unrelated evals/health — the same *class*
   of head-of-line concern that R2-P2-2 removed for audit, just shorter per event.
2. **Redundant re-verification.** An agent presenting the same capability on every call
   re-verifies an identical signature every time. At 5k rps that is ~0.15-0.25 core/s of
   wasted crypto per agent.
3. **Asymmetric-cost DoS.** A capability with a *garbage* signature still costs a full
   ed25519 verify before it is rejected. A caller can cheaply generate such payloads;
   there is no per-caller cap-verify rate limit. The `/api/v1/messages` route is behind
   `bearer_jwt`, so this is an *authenticated* DoS (any tenant with a token), which
   bounds but does not remove it.

**Why only P2.** It fires only when a capability is presented and the operator has armed
agentic auth (F1); the work always completes (unlike an unbounded blocking send); and the
endpoint requires a JWT. But for a fleet that *mandates* agentic capabilities — the exact
posture F1 is built for — this is a throughput ceiling and an availability vector that has
never been load-tested (the SLO harness scenarios are non-agentic).

**Remediation.** Cache the *positive* verdict keyed on
`(capability.id, key_id, signature_bytes, expiry, revocation_generation)` with a short
TTL and eviction on revocation-list update — turning steady-state agentic traffic into a
hash lookup. Additionally, offload `verify_raw` to `spawn_blocking` when batching or above
a size threshold, and add a per-tenant capability-verification rate limit. Add an
agentic scenario (capability-per-request) to `slo-harness.yml` so this path gets a number.

---

## Absence checks performed (falsifiable)

- **DSL still compiled, not re-parsed.** `evaluate_with_match`
  (`reaper_dsl/mod.rs:1553-1729`) walks pre-partitioned `compiled_deny_rules` /
  `compiled_allow_rules`; deny-first, short-circuit; `reset_traversal_budget()` at entry
  (`:1559`), `ScratchGuard` reclaims transient interned strings (`:1565`), resource/actor
  entities synthesized on the **stack** not `Arc::new` (`:1624-1683`). Confirmed.
- **Pruning-index soundness.** Verified the set-algebra composition in
  `condition_resource_constraint` (`:1867-1917`) is a superset of each rule's true-set;
  `?`-propagation guarantees an unrecognized shape yields `None` (unprunable), never a
  spurious `Some` (which would fail-open). Property tests exist
  (`reaper_dsl/tests.rs:301-408`: any resource outside `Some(terms)` must be
  non-decisive). Correct.
- **Hot-swap = RCU, readers never block.** Full-set load builds a fresh `ActiveSet`
  with the pruning index built *before* the `ArcSwap::store` (`engine/mod.rs:372-404`);
  single-policy deploy mutates the loaded set's `DashMap` in place (`:206-261`). No
  `RwLock` on the read path. Confirmed.
- **Durable audit is non-blocking.** `log_durable` uses `try_send` + async `oneshot`
  ack under `DURABLE_ACK_TIMEOUT` (`decision_buffer.rs:937-956`); a full queue / dropped
  writer / timeout all → `note_loss` + `false` → handler returns 503 and observes the SLA
  histogram (`evaluate.rs:644-653`). Best-effort `log()` stays fire-and-forget
  drop-and-count. Mandatory stdout-only config is rejected at construction, not
  per-request (`evaluate.rs:1395-1408`). R2-P2-2 fully closed.
- **Early-return denies observed.** `observe_early_return` (`evaluate.rs:138-145`) is
  called on `data_stale`, `capability_rejected`, `policy_not_found`,
  `evaluate_all_disabled`, `no_policies`, `candidate_cap_exceeded`, fast-path
  `parse_error`, and the audit-gate 503 — both endpoints. R2-P2-3 closed.
- **Decision cache sharded, no per-probe alloc.** N-way sharded (`decision_cache.rs`),
  `context_fold`/`provenance_fold` commutative — no sorted-key `Vec` per probe; actor +
  taint provenance folded into the 128-bit fingerprint (cross-actor / cross-taint
  poisoning tested, `:419-473`). Generation captured before eval, checked on insert —
  race-safe across deploy. Confirmed.
- **ReBAC bounded + budgeted.** `TRAVERSAL_NODE_BUDGET = 4096` per traversal,
  `EVAL_TRAVERSAL_BUDGET = 4×` per evaluation, reset at each evaluator entry; BFS uses
  thread-local scratch, clones the `EdgeList` and drops the DashMap guard before
  recursing into `hit` (`relationships.rs:342-352`) — cycle-safe, lock-drop-before-
  recurse. Confirmed.
- **Served-path SLO now measured.** Blocking `http-slo-ab` paired A/B on request-total
  p99 (`perf-gate.yml:115-198`) + nightly absolute harness over a real agent across all
  four §3 rows (`slo-harness.yml`). R2-P1-1 closed (the harness multiplier of 250 means
  nightly catches only order-of-magnitude regressions on shared runners — the blocking
  PR protection is the paired A/B, which cancels machine variance by construction).
- **Config tunability.** `worker_threads`, `max_batch_requests`, `allow_evaluate_all`,
  `max_candidate_policies`, `use_pruning_index` all in `PerformanceConfig` with env
  overrides (`settings.rs:303-349`, `config/mod.rs:164-188`). Sensible defaults
  (evaluate-all off, pruning on, batch cap 1000, candidate cap 256). Confirmed.
- **Eval body limit separated from bulk limit.** 16 MB `route_layer` on eval routes vs
  256 MB global (`main.rs:656`); batch count cap enforced pre-eval with 413
  (`evaluate.rs:1125-1134`). R1-P1-2 body-limit half closed; concurrency half still open
  (R3-P3-4).

**Did NOT cover:** Cedar evaluator internals, control-plane (`reaper-management`)
request path, sync/promotion pipeline performance, eBPF crate, WASM build. Data-plane
write APIs read only where they intersect eval (datastore lookups on the log path).

## What's done well (≤5)

- The three Round-2 P1/P2 blockers (unmeasured SLO, DSL-unprunable, reactor-blocking
  mandatory audit) are **substantively** closed — verified in source, not cosmetic.
- The served-path SLO is now defended by a *blocking* paired-A/B HTTP gate that measures
  request-total p99 — the right instrument for a latency-headline product.
- DSL pruning-index term extraction is a genuinely careful, provably-sound set-algebra
  walk with property tests asserting it can never fail open.
- Durable-audit-before-serve is non-blocking by construction (`try_send` + async ack +
  bounded timeout) and fails closed on every unavailability mode.
- A modern baseline remains intact (mimalloc, sonic-rs, memchr, arc-swap RCU, sharded
  cache, interned-u32 ReBAC, stack-synthesized entities); the cheap wins are spent.

## Top 3 changes to most improve p999

1. **Make ABAC/ReBAC policies prunable** (R3-P2-1): add a resource-*type* index tier via
   `partial_evaluation.rs`, and pre-sort the `unprunable` id slice at swap time to kill
   the per-request O(N log N) sort. Removes the evaluate-all candidate-cap denial cliff
   and O(N) fan-out for the mandated languages.
2. **Cache capability verifications and get the crypto off the reactor** (R3-P2-2):
   memoize positive verdicts keyed on signature + revocation generation; `spawn_blocking`
   the cold-path verify. Turns steady-state agentic traffic into a hash lookup and closes
   the asymmetric-cost DoS.
3. **Load `ActiveSet` once per `evaluate_set`** (R3-P3-1) and add the reload/caching/
   data-scaling benches to the blocking criterion gate (R3-P3-5a): removes N redundant
   arc-swap loads on fan-out and closes the regression-visibility gaps outside the two
   currently-gated suites.
