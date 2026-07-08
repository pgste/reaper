# Reaper — Performance Review (Persona 1: Systems Performance Engineer)

**Scope:** eval hot path (agent handler → engine → compiled DSL → decision → response),
DSL execution model, data-structure complexity, ReBAC traversal, hot-swap concurrency,
tail-latency contributors, sidecar transport, and audit-pipeline isolation. I did **not**
cover Cedar internals, the control-plane (`reaper-management`) request path, or the eBPF
crate. Data-plane write APIs were read only where they intersect the eval path.

---

## VERDICT: CONDITIONAL (no P0; two P1s)

The core engineering is genuinely good: the DSL is **compiled at load time** to an interned
`CompiledCondition` tree (not re-parsed per request), lookups are lock-free (`arc-swap` +
`DashMap`), the ReBAC graph is integer-keyed with a bounded BFS, and the audit path is
structurally off the eval loop with fire-and-forget capture and a bounded, drop-on-full
writer queue. The sub-microsecond claim is real **for the compiled-condition walk in
isolation**.

It does **not** survive production unqualified. Two things break the headline at scale:
(1) the **served** engine still does an O(policies × rules) linear scan with a full
Arc-clone of the policy set per request, and the indexed/optimized engine variants that
exist in the tree are **not wired into the agent**; (2) the batch endpoint runs an
**unbounded synchronous CPU loop on the tokio worker** behind a 256 MB body limit — a
single request can monopolize a runtime thread. Neither corrupts authorization or loses
audit silently, so neither is a P0, but both must be fixed before a regulated at-scale
deployment.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| P1-1 | P1 | `engine/mod.rs:222-229,281-334`; agent `evaluate.rs:258-278,636-644` | "Evaluate-all" path clones **every** policy Arc (`list_policies`) then evaluates all of them; within a policy, rules are a linear scan. Indexed/optimized engines exist but are **not** used by the agent. | Latency scales O(n_policies·n_rules); silently violates the sub-µs SLA at scale; a policy-less request is a DoS amplifier (1 request → N evals). | Wire a resource/action-pruning index into the served engine; cap/deny unbounded fan-out; index rules. |
| P1-2 | P1 | `evaluate.rs:775-890` + `main.rs:538` | Batch endpoint runs a **synchronous** eval loop directly on the async worker (not rayon/`spawn_blocking` despite the doc claim), with only the global **256 MB** body limit bounding batch size. | One request monopolizes a tokio worker; head-of-line blocking / availability DoS on few-core sidecars. | `spawn_blocking` + rayon; add a per-endpoint request-count cap distinct from the data-load body limit. |
| P2-1 | P2 | `decision_cache.rs:90-99,176-239,54-72` | Decision cache is a **single global** `RwLock<FxHashMap>` + `Mutex<VecDeque>`; every miss takes the global write lock + order mutex; every probe allocates+sorts a `Vec` of context keys. | On low-hit-rate workloads the cache serializes all cores and adds an alloc per request — a net throughput loss vs no cache. | Shard (e.g. `DashMap`/sharded LRU); avoid the per-probe Vec; document it as a Cedar-only win. |
| P2-2 | P2 | headline vs `evaluate.rs:166,364,382-385` and `metrics_cache.rs:43` | Exported `evaluation_time_microseconds`/`reaper_decision_duration_seconds` on the **standard** endpoint measure only the engine slice, excluding JSON deser, ~5 String clones, cache probe, log-entry build, and response serialization. The **fast** endpoint observes total time — inconsistent. | Latency dashboards understate true request latency; the two endpoints aren't comparable. | Observe end-to-end on both; export both engine-slice and request-total histograms. |
| P2-3 | P2 | `main.rs:127` (`#[tokio::main]`), no worker-thread config | Worker threads default to host CPU count; not configurable via Reaper config. A cgroup-limited sidecar may over-subscribe. | Thread over-subscription, context-switch tail latency in constrained sidecars. | Expose `worker_threads` in config; default sensibly per sidecar/service profile. |
| P2-4 | P2 | `.github/workflows/perf-tracking.yml` | Perf gate runs criterion on **shared `ubuntu-latest`** with a 130% (30%) alert threshold. | Micro-bench variance on shared runners makes a 30% gate either noisy or blind to <30% regressions on a nanosecond-scale product. | Run perf on a dedicated/self-hosted runner or use statistical gating (e.g. `critcmp` with CI + multiple samples). |
| P3-1 | P3 | `evaluate.rs:198,286-290` | Per-request `Vec<Uuid>` allocation for the policy-id set even for the single-policy case; `PolicyRequest` clones `resource`/`action`/principal. | Minor per-request heap churn. | `SmallVec<[Uuid;1]>`; borrow where the evaluator allows. |
| P3-2 | P3 | `engine/mod.rs:329` | `policy.name.clone()` (String alloc) on **every** `evaluate()`, incl. non-matching policies in a set. | Small alloc per policy per request. | Return `Arc<str>` name or defer the clone to the caller that needs it. |
| P3-3 | P3 | `reaper_dsl/mod.rs:319-327` | `ActionEquals`/`ResourceIdEquals` use `interner.resolve()` (Arc<str> clone, atomic inc/dec) instead of `with_resolved()`. | Atomic refcount churn on a common comparison. | Use `with_resolved` (already exists, `interning.rs:236`). |
| P3-4 | P3 | `relationships.rs:209-240` | Each ReBAC traversal condition allocates a fresh `FxHashSet` + `VecDeque`, and clones each `EdgeList` per hop; per-request cost = (#rebac conditions)·(up to 4096 nodes). Node budget is per-traversal, not per-request. | Bounded but the per-request multiplier across many rebac conditions is unbounded; adversarial data+policy can reach µs–ms. | Reuse thread-local scratch for visited/queue; consider a per-request traversal budget. |

---

## Detailed findings

### P1-1 — Served engine is O(policies × rules) with a full policy-set clone; indexed engines unused

**Hot path traced.** `POST /api/v1/messages` → `evaluate_policy` (`evaluate.rs:156`). When no
`policy_id`/`policy_name` is supplied (`evaluate.rs:258-277`, and the fast path
`evaluate.rs:636-644`), the handler calls `state.policy_engine.list_policies()`:

```rust
// engine/mod.rs:222
pub fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>> {
    self.active.load().policies.iter()
        .map(|entry| entry.value().clone())   // Arc clone per policy, every request
        .collect()                             // Vec alloc sized to policy count
}
```

Then `evaluate_policy_set` (`evaluate.rs:88-142`) loops over **all** ids calling
`engine.evaluate` on each (deny-overrides), and each `engine.evaluate`
(`engine/mod.rs:281-334`) does a linear scan of that policy's rules. Within the compiled DSL,
deny and allow rules are pre-partitioned but still **linearly scanned**
(`reaper_dsl/mod.rs:1636-1671`).

**Cost model.** At 10k policies × 5k rps, the policy-less path alone is 5k × 10k = 5×10⁷
Arc-clones/s plus 5×10⁷ evaluator invocations/s — before any rule work. A `Vec<Arc>` of 10k
elements (~80 KB) is allocated and dropped every request: ≈ 400 MB/s of alloc churn at 5k rps
just for the candidate list. The sub-µs number (measured on a single 1-rule policy) does not
describe this regime at all.

**The kicker:** the repo map notes `indexed_engine.rs` and `optimized_engine.rs` exist, but
`grep IndexedEngine|OptimizedEngine services/reaper-agent/src` returns **nothing** — the agent
constructs a plain `PolicyEngine` (`state.rs`). The optimization work is dead relative to the
served path.

**Remediation.** (1) Build a resource/action → candidate-policy index in the served engine so
a request evaluates only relevant policies; (2) reject or hard-cap policy-less "evaluate all"
requests (they should not be a normal enforcement mode); (3) index rules within a policy
(e.g. by action/resource prefix) rather than linear scan; (4) if the indexed engine is
production-ready, wire it into `AgentState`.

---

### P1-2 — Batch endpoint blocks the async runtime with unbounded synchronous work

`batch_evaluate_policy` (`evaluate.rs:775`) is an `async fn` that does the entire CPU-bound
loop **inline on the tokio worker**:

```rust
// evaluate.rs:838
let results: Vec<Value> = requests.iter().enumerate()
    .map(|(i, req)| { /* eval + cache probe per request */ })
    .collect();
```

This is **sequential**, contradicting the doc comment two lines above:
`evaluate.rs:768` — *"evaluates multiple policy requests in parallel using rayon"* — there is
no rayon and no `spawn_blocking` (`grep rayon|par_iter|spawn_blocking services/reaper-agent/src`
finds only that comment). The loop therefore:

1. runs to completion on one runtime worker, starving every other future scheduled there
   (head-of-line blocking; on a 1–2 vCPU sidecar this stalls unrelated evals), and
2. is bounded only by the **global 256 MB body limit** (`main.rs:538`, deliberately large for
   bulk entity loads) — there is no per-endpoint request-count cap. A 256 MB payload of tiny
   requests is millions of synchronous evaluations on one thread.

**Remediation.** Move the loop to `tokio::task::spawn_blocking` and parallelize with rayon (as
documented); add an explicit `max_batch_requests` cap enforced before evaluation; apply a
smaller `DefaultBodyLimit` to the eval endpoints than to the data-load endpoints via a
per-route layer.

---

### P2-1 — Decision cache is a global-lock hotspot and allocates per probe

`DecisionCache` (`decision_cache.rs:90-99`) is a single `RwLock<FxHashMap>` plus a
`Mutex<VecDeque>` for FIFO order — **not** sharded. `get` (`:176`) takes the global read lock
(a shared cache-line under parking_lot; readers don't block each other but do bounce the lock
word across cores at high rps). `insert` (`:205`) takes the **global write lock and the order
mutex**, so on a low-hit-rate / high-cardinality workload — where every miss triggers an
insert — all cores serialize on one writer. Enabling the cache there is a net loss versus the
lock-free engine it's meant to accelerate.

Additionally, `fingerprint` (`:54-72`) allocates a `Vec<&String>` of context keys and sorts it
on **every** get and insert — an allocation on the hot path whenever caching is enabled.

The cache is `Option` (opt-in, `state.rs:30`), which contains the blast radius, and the module
docs correctly note it only helps expensive evaluators. But the write-lock-per-miss
serialization and per-probe allocation are undocumented traps. **Remediation:** shard the map
(DashMap or an N-way sharded LRU), fold the FIFO order into the shard, and hash context without
the intermediate Vec (iterate with an order-independent commutative combiner as `scope_hash`
already does at `:286`).

---

### P2-2 — Latency metric measures a slice on the standard endpoint, total on the fast endpoint

Standard endpoint: `metrics.duration.observe(latency_seconds)` where
`latency_seconds = total_eval_time_ns / 1e9` (`evaluate.rs:382-385`) — engine slice only, and
the `evaluation_time_microseconds` field likewise (`:507`). Fast endpoint:
`metrics.duration.observe(total_time.as_secs_f64())` (`:692`) — full request. The same
Prometheus series `reaper_decision_duration_seconds` (`observability.rs:43`) is fed two
different measurements depending on endpoint, and both understate real p99 on the standard path
(JSON deser at `:158`, principal/resource/context String clones at `:283-290`, cache probe,
decision-log entry construction `:454-485`, response serialization `:502`). For a latency-
headline product, publish **request-total** histograms and keep the engine-slice as a separate
series.

---

## Absence checks performed (falsifiable)

- **DSL execution model** — Confirmed compiled, not interpreted per request:
  `build_evaluator_with_data` (`policy.rs:199-215`) compiles `.reap` to the DSL-v2 evaluator at
  deploy time via `build_preferred`; `ReaperDSLEvaluator::new` (`reaper_dsl/mod.rs:216-260`)
  pre-interns strings, pre-compiles regex, and partitions rules at construction. Runtime
  `evaluate` (`:1531`) walks the compiled tree. **Cache invalidation on update:** evaluator is
  rebuilt on `update_content`/`deploy_policy`; the decision cache is epoch-invalidated
  (`decision_cache.rs:163-168`) and the eval path captures the generation *before* evaluating
  (`evaluate.rs:299-303`) — verified race-safe.
- **Hot-swap = RCU, readers never block** — `arc-swap` full-set swap in `replace_all_policies`
  (`engine/mod.rs:238-265`); single-policy deploy mutates the loaded `ActiveSet` in place
  (`:123-171`). No `RwLock` on the read path. `default_policy` `RwLock` read
  (`:290`) is lazy (`or_else`), taken only on a miss. **Perf: no writer-starvation on eval.**
  (Note: single-policy deploy racing a full swap can lose the single deploy — a correctness
  concern deferred to the distribution reviewer, not a perf issue.)
- **ReBAC bound** — `TRAVERSAL_NODE_BUDGET = 4096` (`relationships.rs:44`), enforced in
  `bfs_reaches` (`:222`) with `FxHashSet` visited + depth bound; cycle-safe (test `:397`),
  budget exactness tested (`:346`). Guard-drops the shard lock before recursing (`:227`) so a
  traversal never holds a lock across `hit`. Sufficient and correct; see P3-4 for the
  cross-condition multiplier.
- **Audit isolation** — Verified structurally off the eval loop. Capture is gated by
  `should_log` before any allocation (`evaluate.rs:440`, `decision_buffer.rs:276`); ring push
  is a same-thread **sharded** uncontended lock (`decision_buffer.rs:387-392`); the file writer
  is a bounded `sync_channel(65_536)` with `try_send`, **dropping+counting** on full, never
  blocking (`:380-384`, `WRITER_QUEUE_CAPACITY:93`); JSON serialization happens on the writer
  thread, not the request (`:306-320`). Ring is capacity-bounded (drops oldest, counted) — **no
  unbounded buffering, no blocking**. Drops are observable via `writer_dropped`/`dropped_entries`
  stats and gauges (`observability.rs`). **Gap (compliance, cross-cutting):** there is **no
  fail-closed "deny if audit unavailable" mode** — regulated deployments requiring guaranteed
  audit cannot enforce it; capture is best-effort. Flag for the audit reviewer.
- **Histograms exist** — `reaper_decision_duration_seconds` HistogramVec with sub-µs buckets
  `[100ns … 1ms]` (`observability.rs:43-49`), per-policy handles cached (`metrics_cache.rs`).
  Queue depth / drops exposed as gauges. Good — but see P2-2 on what is measured.
- **Benchmarks exist** — criterion benches for eval, complex policy, data scaling, caching,
  ReBAC, e2e, SIMD (`crates/policy-engine/benches/*`), plus agent/platform/core benches. Perf
  CI gate exists (`perf-tracking.yml`, `fail-on-alert: true` @130%) — but weak on shared runners
  (P2-4). The repo map's "comment-only" note appears stale for this file.
- **Zero-copy request context** — `EvalContext` borrows action/resource/context
  (`reaper_dsl/mod.rs:145-173`), avoiding a HashMap clone per eval; resource entity synthesized
  on the **stack** not `Arc::new` (`:1595-1606`); transient interned strings reclaimed via
  `ScratchGuard` (`:92-123`) so high-cardinality results don't grow the shared interner. These
  are real, correct optimizations.

## What's done well (≤5)

- DSL is compiled once at load into an interned condition tree; runtime is a branch table over
  integer IDs, not string hashing (`reaper_dsl/mod.rs`, `interning.rs`).
- Read path is genuinely lock-free RCU (`arc-swap` + `DashMap`); readers never block during
  hot-swap.
- Audit capture is correctly isolated: gated, sharded, fire-and-forget, bounded, drops-counted,
  serialization deferred to a writer thread.
- ReBAC traversal is bounded, cycle-safe, integer-keyed, with lock-drop-before-recurse — a
  correct DoS guard.
- Thoughtful micro-optimizations with rationale in comments (stack-buffer UUID encoding,
  gated tracing, lazy default-policy read, transient-string reclamation, cached metric handles).

## Top 3 changes to most improve p999

1. **Wire a pruning index into the served engine** and cap/deny "evaluate-all" (P1-1) —
   removes the O(n_policies) scan and the per-request full-set Arc-clone that dominate tail
   latency for any multi-policy tenant.
2. **Get CPU-bound work off the async worker** — `spawn_blocking` + rayon for batch, plus a
   per-endpoint request/size cap (P1-2) — eliminates head-of-line blocking that spikes p999 for
   unrelated requests sharing a worker.
3. **Shard or gate the decision cache and drop the per-probe Vec alloc** (P2-1) — removes the
   global-write-lock serialization on low-hit workloads so the cache stops being a p999
   regression.
