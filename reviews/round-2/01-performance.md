# Reaper — Performance Review, Round 2 (Persona 1: Systems Performance Engineer)

**Auditor mandate:** hostile, enterprise/bank-grade re-review. Verify that Round-1's
perf closures (Plan 08, "Engine performance to SLA," claimed shipped via PRs #33–34)
are *real* and find what is newly wrong or still missed. Scope, in priority order:
the single-eval hot path (`evaluators/reaper_dsl/`, `engine/`, agent `evaluate.rs`),
audit isolation, sidecar cold-start/reload, and system-level gates/observability. I
did **not** re-audit Cedar internals, the control-plane request path, or the eBPF crate
beyond where they touch eval.

---

## VERDICT: CONDITIONAL — no P0, one P1, three P2, five P3.

The two Round-1 P1s are **genuinely fixed, not cosmetically**: the served evaluate-all
path now queries a resource pruning index and hard-caps fan-out
(`engine/mod.rs:342`, `evaluate.rs:231-307`), and batch eval runs on
`spawn_blocking` + rayon behind a pre-evaluation count cap
(`evaluate.rs:1003-1082`, `940-949`). The decision cache is N-way sharded with an
allocation-free commutative fingerprint (`decision_cache.rs:79-113,128-147`), ReBAC BFS
reuses thread-local scratch under a per-evaluation node budget
(`relationships.rs:54-75,296-319`), worker threads are configurable
(`main.rs:143-149`), and mimalloc is the agent's global allocator (`main.rs:18-19`).
Audit capture is now not just isolated but has a real mandatory fail-closed mode with a
signed hash-chain (`decision_buffer.rs`, `decision_log.rs`). This is strong work.

It is **not** yet READY for a bank. The single P1 is that the headline of Plan 08 — a
"stated, *measured* latency SLO" for the served path — is **unproven**: the SLO
load-harness is deferred by the plan's own admission, and the blocking regression gate
exercises two in-process micro-benchmarks, not the request-total path the SLA is defined
on. You cannot sign off an authorization-latency SLA you have never measured under
representative load. Behind that sit real design gaps: the pruning index is inoperative
for the DSL — the *mandated* policy language — so the evaluate-all SLO row is unmet at
scale for real policies; mandatory-audit `Block` mode does a blocking channel send
directly on a tokio worker; and the shipped-but-unwired arena / indexed-engine /
partial-evaluation subsystems mean the at-scale story rests on unmeasured assumptions.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R2-P1-1 | P1 | `plans/08-…md:16-17`; `.github/workflows/perf-gate.yml:70-78`; `evaluate.rs:574,899` | The served-path SLO (§3 table: p50/p99/p999 at 10k policies × 5k rps) is **never measured**. The load harness is deferred; the blocking gate benches only `policy_evaluation_bench` + `rebac_bench` (in-process criterion micro-benches), not the request-total handler at scale. The request-total histogram exists but nothing drives it under representative load in CI. | The central Plan-08 claim ("meet a *measured* SLO") is unverified. A bank cannot approve a latency SLA on unmeasured assumptions; regressions in deser/serialize/cache/audit on the real path are ungated. | Build the deferred load harness (extend `benchmarks/reaper-vs-opa` HDR path to the §3 N×M points); assert request-total p99/p999 against the table in CI; add e2e/reload/audit-capture benches to the gate. |
| R2-P2-1 | P2 | `engine/mod.rs:148-162,342-356`; `simple.rs:1-16,115-120`; `settings.rs:352-354` | The Phase-A pruning index prunes **only `PolicyLanguage::Simple`** (`index_terms` returns `None` for DSL/Cedar → always `unprunable`). Simple is **deprecated** ("Real policies must use the Reaper DSL"). So for DSL, `candidate_policy_ids` returns the *whole set*; at >256 (`default_max_candidate_policies`) evaluate-all returns blanket `candidate_cap_exceeded` denials, and with the cap raised it degrades to a full scan **plus a new per-request O(N log N) `sort`+`dedup`**. | The SLO row "Evaluate-all via pruning index (few candidates match): 10k policies ≤25µs p99" is unachievable for the mandated language. Closure is real only for the deprecated language. Evaluate-all workloads on DSL either mass-deny (availability incident) or run O(N log N)/req. | Extract resource predicates from DSL rules (wire `partial_evaluation.rs`) so DSL policies are prunable; document the SLO evaluate-all row as Simple-only until then; make the cap error observable/typed distinctly from a policy-miss deny. |
| R2-P2-2 | P2 | `decision_buffer.rs:680-692`; `decision_log.rs:746-751`; `evaluate.rs:542,870` | Mandatory-audit **`Block`** mode does a *blocking* `SyncSender::send` inside `buffer.log()`, which is called **inline on the tokio async worker** (not `spawn_blocking`). If the durable sink stalls and the 65,536-deep queue fills, request-serving workers park on the send. Docs undersell this as merely "slower" tail latency. | A bank that mandates zero audit loss will pick `Block`; audit-sink slowness then converts into reactor starvation / head-of-line blocking across unrelated requests on that worker — a serving outage triggered by disk pressure. This is the exact class Round-1 P1-2 removed for batch. | Move the blocking hand-off off the reactor (`spawn_blocking`, or an async bounded channel awaited from the handler); or drop `Block` and keep only the default non-blocking `FailClosed`. Fix the doc to state it blocks the reactor. |
| R2-P2-3 | P2 | `evaluate.rs:152-167,186-198,216-228,241-253,269-303` (+ fast: `712-755`) | The request-total SLA histogram (`metrics.duration.observe`) is observed only on the success path (`:574`) and cache-hit path (`:371-375`). **Every early-return deny** — `data_stale`, `policy_not_found`, `evaluate_all_disabled`, `candidate_cap_exceeded`, `no_policies` — returns without observing into the SLA series, on both endpoints. | During a denial storm (stale-data gate tripped, misconfig, attack) the request-total latency dashboard goes silent even as the agent serves millions of denies — the SLA series, which *is* the bank's latency signal, understates load and can hide an incident. | Observe request-total on every return path (including denies); denies are served requests. |
| R2-P3-1 | P3 | `arena.rs` (whole file); `reaper_dsl/mod.rs:1639` | The bumpalo arena ("zero-allocation evaluation loops," Layer-1 goal) is **dead on the hot path**: `with_arena`/`reset_arena`/`prewarm_arena` are never called by any evaluator (grep: only `lib.rs` re-exports). The DSL allocates a fresh `variables` `HashMap` per eval instead. | Unrealized optimization; per-request heap churn the arena was built to remove; dead maintenance surface. | Route DSL per-eval scratch (variables map, transient strings) through `with_arena_reset`, or delete `arena.rs` if abandoned. |
| R2-P3-2 | P3 | `indexed_engine.rs`, `optimized_engine.rs`, `partial_evaluation.rs` | Still referenced only by benches/examples (grep of `services/` = empty). ADR-1 promised "reconcile or delete the standalone variant once the served index is proven." `optimized_engine` wraps `PartialEvaluator` — the one technique that could make DSL prunable (see R2-P2-1) — and sits unused. | Dead code; the most useful unwired asset (partial eval) is exactly what R2-P2-1 needs. | Either wire partial-eval into the served index path or delete the trio; do not carry three parallel engines. |
| R2-P3-3 | P3 | `evaluate.rs:171,209,665,695` | Round-1 **P3-1 is marked closed** in Plan 08 ("Findings closed: … P3-1") but the code still heap-allocates `let policy_ids: Vec<Uuid> = vec![id]` for the common **targeted** single-policy path — no `SmallVec<[Uuid;1]>`. | Minor per-request heap alloc on the hottest (targeted) path; and a plan overclaim. | `SmallVec<[Uuid;1]>` as planned, or evaluate a single id without the Vec. |
| R2-P3-4 | P3 | `evaluate.rs:1011-1082`; `plans/08-…md:163` | No cap on **concurrent** batch requests. Each batch spawns a `spawn_blocking` task that fans out on the global rayon pool; the plan's own risk note ("add a concurrency limit on batch tasks") was not shipped. | A flood of ≤1000-request batches oversubscribes the 512-thread blocking pool and thrashes the rayon pool — an availability vector distinct from the per-batch cap. | Bound concurrent batch tasks (semaphore) as the plan specified. |
| R2-P3-5 | P3 | `.github/workflows/perf-gate.yml:70-78` | The blocking gate benches 2 of 9 criterion suites; no hot-swap/reload, cache-contention, decision-capture, data-scaling, or e2e bench is gated. The paired A/B design is sound; coverage is not. | Regressions in reload latency, audit capture, cache contention, and the full handler are invisible to the gate. | Add the reload/e2e/caching/data-scaling benches to the paired gate. |

---

## Detailed findings

### R2-P1-1 — The served-path SLO is unproven; the gate guards micro-benches, not the SLA

**Evidence.** Plan 08's own STATUS banner concedes the closure is incomplete:
> "The SLO load-harness (validating the §3 SLO table end-to-end) is the one deferred
> follow-up; the enforcement gate that guards it is in place." (`plans/08-…md:16-17`)

But the "enforcement gate" (`perf-gate.yml:70-78`) runs exactly two in-process criterion
suites against the compiled engine:
```yaml
cargo bench -p policy-engine --bench policy_evaluation_bench -- --save-baseline base
cargo bench -p policy-engine --bench rebac_bench          -- --save-baseline base
```
Neither drives `POST /api/v1/messages` end to end. The request-total histogram the SLA is
defined on (`evaluate.rs:574,899`, into `reaper_decision_duration_seconds`) is populated
only by a live agent under HTTP load — which nothing in CI generates. So the §3 table
(p50 ≤2µs / p99 ≤10µs / p999 ≤50µs targeted at 10k×5k; evaluate-all ≤25µs p99;
ReBAC ≤75µs p99; batch ≤1ms/call p99) is a **design aspiration, never a measurement**.

**Why P1 for a bank.** The deliverable of Plan 08 is a *measured* SLA on authorization
latency. An SRE org cannot approve an SLA it has never observed under representative
policy counts, request rates, and concurrency, and cannot detect regressions in the parts
of the path the micro-benches skip (JSON deser at `evaluate.rs:130/620`, ~5 String clones
at `:312-319`, cache probe, decision-log entry build at `:496-542`, response serialize at
`:559`). The engine-slice number is real and excellent; the request-total SLA is
unvalidated.

**Remediation.** The HDR-percentile harness in `benchmarks/reaper-vs-opa` already computes
request-total p50/p99/p999. Extend it to the §3 N×M points (10k policies, targeted +
evaluate-all + ReBAC + batch), run it against a real agent in CI (or nightly on a
dedicated runner), and assert the table. Add reload and audit-capture benches to the
blocking gate so the *served* path — not just the compiled walk — is regression-guarded.

---

### R2-P2-1 — Pruning index is inoperative for the DSL (the mandated language)

**Trace.** Evaluate-all (`evaluate.rs:258` / fast `:724`) calls
`state.policy_engine.candidate_policy_ids(resource)`. That function
(`engine/mod.rs:342-356`) returns the resource bucket **plus every `unprunable`
policy**, then `sort()`+`dedup()`s. What lands in `unprunable` is decided by
`index_terms` (`engine/mod.rs:148-162`):
```rust
fn index_terms(policy: &EnhancedPolicy) -> Option<Vec<String>> {
    if policy.language != PolicyLanguage::Simple {
        return None;               // DSL, Cedar -> ALWAYS unprunable
    }
    ...
}
```
Only `Simple` policies are ever bucketed by resource. And `simple.rs:1-16` states
plainly: *"This evaluator … cannot express RBAC or ABAC … Real policies must use the
Reaper DSL … Do not build new functionality on it."* So in any real (DSL) deployment
**every** policy is `unprunable`, and `candidate_policy_ids` returns the full set.

**Consequences at scale (10k DSL policies).**
- With the default cap (`default_max_candidate_policies() = 256`, `settings.rs:352-354`),
  every evaluate-all request has `candidate_ids.len() == 10_000 > 256` →
  `candidate_cap_exceeded` **deny** (`evaluate.rs:287-304`). Evaluate-all becomes a blanket
  denial generator — a correctness-visible availability failure for that mode.
- Raise the cap to 10k and each request now pays a full `list`-equivalent **plus a new
  O(N log N) `sort`+`dedup` over 10k ids** (`engine/mod.rs:353-354`) — arguably worse
  per-request CPU than the Round-1 linear clone it replaced, and nowhere near the
  ≤25µs p99 SLO row.

The soundness argument for pruning (only non-matching literal-resource Simple rules are
dropped) is correct *for Simple* — I verified `matches_rule` is exact-only
(`simple.rs:115-120`, `rule.resource == "*" || == request.resource`), so the exact-string
index is safe. But note a latent trap: the index's correctness is silently coupled to
that exact-match semantics; the `// TODO: Add glob patterns` at `simple.rs:117` would make
the index unsound (missed policy → fail-open deny) if implemented without updating
`index_terms`. There is no test or type-level guard enforcing that coupling.

**Remediation.** Wire `partial_evaluation.rs` (already in-tree, unused) to statically
extract resource predicates from DSL rules so DSL policies gain concrete index terms;
until then, document the evaluate-all SLO row as Simple-only and default the cap behavior
to a typed 4xx rather than a silent authorization deny. Add a property test asserting
`index_terms` and the evaluator's match semantics agree.

---

### R2-P2-2 — Mandatory-audit `Block` mode blocks the async reactor

**Evidence.** `buffer.log()` is invoked inline in the async handlers
(`evaluate.rs:542`, `evaluate.rs:870`) — not inside `spawn_blocking`. Inside `log()`:
```rust
// decision_buffer.rs:681-688
let block = self.config.audit_required
    && self.config.on_audit_unavailable == OnAuditUnavailable::Block;
if block {
    if tx.send(WriterMsg::Entry(arc.clone())).is_err() {   // BLOCKING send
        self.audit.note_loss();
    }
} else if tx.try_send(...).is_err() { self.audit.note_loss(); }
```
`tx` is a `sync_channel(65_536)` (`:419`). In `Block` mode a saturated queue makes the
**request-serving tokio worker** park on `send()`. `decision_log.rs:748-751` describes
this only as trading "tail latency under sink pressure … only a slower one" — it is
worse: it stalls the reactor thread, so *other* futures multiplexed on that worker (health
checks, unrelated evals, denies) are blocked too.

The default is the safe non-blocking `FailClosed` (latch → 503 + drain, `:746-747`), which
is correct and is what a bank *should* run. But `Block` is offered precisely to guarantee
zero audit loss — the property a regulated deployment is most likely to demand — and in
that configuration audit-sink slowness becomes a serving outage.

**Remediation.** Perform the durable hand-off off the reactor: either wrap the `Block`
send in `tokio::task::spawn_blocking`, or replace the `sync_channel` with a bounded async
channel (`tokio::sync::mpsc`) and `.await` the send from the handler with a timeout that
degrades to `FailClosed`. Correct the doc comment to state that `Block` can stall the
runtime.

---

### R2-P2-3 — SLA latency histogram omits every early-return deny

`metrics.duration.observe(start_time.elapsed())` runs only at `evaluate.rs:574` (success)
and `:371-375` (cache hit). The deny early-returns —
`data_stale` (`:152-167`), `policy_not_found` (`:186-198`, `:216-228`),
`evaluate_all_disabled` (`:241-253`), `no_policies` (`:269-283`),
`candidate_cap_exceeded` (`:287-303`) — and the fast-path equivalents (`:712-755`) return
before any duration observation. Counters (`ERRORS_TOTAL`, `DENIALS_TOTAL`) do fire, so the
events are countable, but the **request-total latency series is blind to them**. For an SRE
org whose primary latency signal is `reaper_decision_duration_seconds`, a stale-data or
misconfig denial storm shows healthy/empty latency while the agent denies at line rate.
Observe request-total on all return paths.

---

## Emerging-techniques assessment (special assignment)

Judged against *this* code, with what is already done noted so as not to re-recommend it.

**Already shipped (verified in source):** mimalloc global allocator (`main.rs:18-19`);
sonic-rs SIMD JSON on the fast path and loaders (`evaluate.rs:620`, `fast_parse.rs`);
`memchr::memmem` for DSL substring ops (`string_eval.rs:17`); thread-local pre-compiled
regex cache (`regex_cache.rs`); arc-swap RCU policy store (`engine/mod.rs:94`); sharded
decision cache with commutative fingerprint (`decision_cache.rs`); interned `u32`
node/relation ids + sorted inline `SmallVec` adjacency for ReBAC (`relationships.rs:40`).
This is a genuinely modern baseline — the low-hanging fruit is gone.

**Not used, assessed for fit:**

- **Bytecode / flattened compiled conditions.** The DSL executes a *recursive* AST walk
  over `CompiledCondition` (`reaper_dsl/mod.rs:1646-1681`, `evaluate_compiled_condition`).
  Flattening to a linear bytecode / register VM (evaluate in a `for` over an op array)
  removes call overhead, improves branch prediction and I-cache behavior, and bounds stack
  depth — a real p999 win for deep conditions. **Fits well.** **Cranelift/JIT: reject** —
  sub-µs policies don't amortize JIT compile latency, and it adds a code-gen security
  surface a bank will challenge. Bytecode interpreter is the right altitude.
- **Partial evaluation (`partial_evaluation.rs`, in-tree, unwired).** The single
  highest-leverage unused asset: it is what makes DSL policies statically prunable and
  closes R2-P2-1. **Fits — wire it.**
- **`phf` perfect hashing.** Relations are "pinned, bounded vocabulary"
  (`relationships.rs:95`); a load-time-built phf over the schema vocabulary beats DashMap
  for read-mostly relation/permission lookups (collision-free, cache-friendlier). Modest,
  real. Must be built at load (vocabulary isn't const) — still a read-side win.
- **Arena / object pooling (`bumpalo`, `arena.rs`, in-tree, unwired).** Directly removes
  the per-eval `variables` HashMap and transient-string churn (R2-P3-1). **Fits — wire it.**
- **CSR / SoA / succinct relationship layout.** ReBAC keeps four DashMaps of
  `SmallVec<[EntityId;4]>` (`relationships.rs:81-89`); at millions of tuples the per-key
  node overhead dominates. A read-optimized CSR (u32 offset array + neighbor array) built
  at snapshot-swap time, with DashMap retained for the write-hot delta path, cuts memory
  and improves traversal cache locality. **Fits at scale; watch delta rebuild cost.**
- **io_uring / thread-per-core (glommio/monoio).** Repo already offers UDS + a sharded
  runtime knob. For a policy sidecar dominated by CPU (compiled walk), not I/O, this is
  low ROI versus its maturity/ecosystem cost. **Defer.**
- **rkyv zero-copy / borrowed serde.** The fast path already borrows request fields via
  sonic-rs; the standard path clones ~5 Strings (`evaluate.rs:312-319`). Borrowing those
  (or arena-allocating them) is cheaper and lower-risk than adopting rkyv for the request
  wire format. **Prefer the borrow/arena route.**
- **PGO / BOLT.** Reasonable for a ns-scale release binary once the SLO harness exists to
  measure the gain; without R2-P1-1's harness you can't quantify it. **Sequence after the
  harness.**

**Top 3 changes to most improve p999 (tied to code):**
1. **Wire `partial_evaluation.rs` into the served index so DSL policies are prunable**
   (`engine/mod.rs:148`, `partial_evaluation.rs`). Removes the O(N)/O(N log N) evaluate-all
   tail and the candidate-cap denial cliff for the mandated language (R2-P2-1).
2. **Flatten the compiled-condition tree to bytecode + run it over the arena**
   (`reaper_dsl/mod.rs:1646-1689`, `arena.rs`). Kills recursion overhead and per-eval heap
   allocation — the two biggest remaining contributors to the DSL walk's tail.
3. **Make the decision cache truly bypassable for cheap evaluators** (`decision_cache.rs`
   — the module's own docs say a shard `RwLock` read often costs more than re-evaluating a
   sub-µs DSL policy). A seqlock/atomic-generation read, or a per-policy "don't cache"
   flag, removes lock-word bouncing from the DSL fast path's p999.

**Top 2 changes to most reduce memory (tied to code):**
1. **CSR/SoA relationship representation at snapshot time** (`relationships.rs:81-89`).
   Four DashMaps of `SmallVec` per (entity,relation) is the dominant ReBAC memory cost at
   scale; a compressed read-side adjacency (u32 offsets + neighbor blob) materially shrinks
   it. (mimalloc is already global — that lever is spent.)
2. **Blob-backed interner** (`interning.rs`): store interned strings as
   `(offset,len)` into one growable arena instead of per-entry `String` headers — removes
   ~24 bytes/string overhead at millions of entities, on top of the existing refcounted
   eviction.

---

## Absence checks performed (falsifiable)

- **DSL execution model — compiled, not re-parsed.** `evaluate_with_match`
  (`reaper_dsl/mod.rs:1534-1689`) walks pre-partitioned `compiled_deny_rules` /
  `compiled_allow_rules` of `CompiledCondition`; deny-first, short-circuit. Confirmed.
- **`reset_traversal_budget` is actually wired** — called at every evaluator entry:
  `reaper_dsl/mod.rs:1540`, `compiled_evaluator.rs:230`, `ast_evaluator/mod.rs:88`. So the
  per-eval ReBAC budget does not silently drain-and-fail-closed in production (a bug I
  specifically checked for). The budget resets per policy-evaluate; combined with the
  candidate cap this bounds a request (`relationships.rs:47-54`).
- **Hot-swap = RCU, readers never block.** Full-set load builds a fresh `ActiveSet` and
  `store()`s the arc-swap (`engine/mod.rs:377-409`); the pruning index is built into the
  new set *before* the swap (`:386-393`) so readers never see a half-built index. Confirmed.
  (Round-1's noted single-deploy-vs-full-swap race — `deploy_policy` mutates the loaded set
  in place, `engine/mod.rs:222-240` — persists but is a distribution/correctness concern,
  not perf.)
- **Cache staleness across deploy.** Generation captured before eval
  (`evaluate.rs:328-332`), checked on insert (`decision_cache.rs:281`); sharded epoch
  invalidation clears + bumps (`:232-239`). Race-safe.
- **Audit isolation + mandatory mode.** Capture gated before allocation
  (`decision_buffer.rs:529`), per-thread sharded uncontended ring push (`:697-708`),
  JSON+I/O on the writer thread (`:574-610`), bounded `sync_channel(65_536)` drop-and-count
  in the default path (`:689-691`), signed hash-chain checkpoints (`Checkpointer`),
  mandatory fail-closed latch + `audit_gate` 503 (`evaluate.rs:110-117`). Round-1's
  "no fail-closed-if-audit-unavailable" gap is **closed** (`OnAuditUnavailable::FailClosed`
  default). The `Block` variant is the new hazard (R2-P2-2). *Note:* `protection.apply`
  (mask/hash/AES) runs **inline** in `log()` (`decision_buffer.rs:648-653`), so
  encryption-at-capture is on the request thread when configured — acceptable (gated,
  opt-in) but worth knowing.
- **Zero-copy request context.** `EvalContext` borrows action/resource/context
  (`reaper_dsl/mod.rs:1635`); unknown resource entity synthesized on the **stack**, not
  `Arc::new` (`:1605-1616`); transient interned strings reclaimed by `ScratchGuard`
  (`:1546`). Confirmed real.
- **Worker threads configurable.** `runtime::Builder::new_multi_thread().worker_threads(n)`
  when `performance.worker_threads > 0`, else auto; `REAPER_WORKER_THREADS` override; logged
  at startup (`main.rs:143-149,199-209`). Confirmed (P2-3 closed).
- **Benches exist** — 9 criterion suites (`benches/`), but only 2 are in the blocking gate
  (R2-P3-5), and none is an end-to-end served-path bench (R2-P1-1).

## What's done well (≤5)

- Round-1 P1-1/P1-2 are **substantively** fixed: pruning index + fan-out cap, and batch on
  `spawn_blocking`+rayon behind a pre-eval count cap — not paper closures.
- Audit path matured from "isolated" to "isolated **and** provably complete when required":
  mandatory fail-closed latch, signed hash-chain, contiguous signed checkpoints, capture-time
  data protection — genuinely bank-grade audit mechanics.
- ReBAC traversal is bounded, cycle-safe, integer-keyed, thread-local-scratch, with a
  per-evaluation node budget and lock-drop-before-recurse — a correct DoS guard.
- The perf gate's **paired A/B same-runner** design with a `--self-test` proving a synthetic
  +15% fails is a thoughtful answer to shared-runner variance (its weakness is coverage,
  not method).
- A modern performance baseline is genuinely in place — mimalloc, sonic-rs, memchr,
  arc-swap RCU, sharded cache, interned u32 ReBAC — few things left are cheap wins.
