# Engine Performance to SLA

**Readiness gate:** Blocks CONDITIONAL → READY for at-scale/regulated deployments. The sub-µs headline is real only for a 1-rule policy in isolation; the *served* path does not hold it at policy scale, and a policy-less request is a DoS amplifier.
**Priority:** P1 (P1-1 served-engine linear scan + P1-2 batch runtime blocking are the two that break the SLA; P2/P3 are throughput/tail refinements).
**Findings closed:** Synth #10; Perf P1-1, P1-2, P2-1, P2-2, P2-3, P2-4, P3-1, P3-2, P3-3, P3-4.

---

## 1. Goal

Make the *served* agent path meet a stated, measured latency SLO at realistic policy counts and request rates, and lock it with a regression gate:
1. **Wire a resource/action pruning index into the served engine** so a request evaluates only candidate policies, not all of them, and eliminate the per-request full-set `Arc`-clone.
2. **Cap unbounded fan-out** — a policy-less "evaluate-all" request must not be a 1-request→N-eval amplifier.
3. **Get CPU-bound batch work off the tokio worker** (`spawn_blocking` + rayon) and cap batch size independently of the 256 MB body limit.
4. **Shard the decision cache** so it stops serializing cores on low-hit workloads, and drop the per-probe `Vec` allocation.
5. **Export consistent end-to-end latency histograms** on both the standard and fast endpoints (currently one measures the engine slice, the other total).
6. **Make tokio worker threads configurable** for cgroup-limited sidecars.
7. **Reuse thread-local scratch in ReBAC traversal** instead of allocating fresh `FxHashSet`+`VecDeque` per condition.
8. **A statistically sound benchmark regression gate** (the current `perf-tracking.yml` runs on shared runners with a 30% threshold — noisy and blind to sub-30% regressions on a nanosecond-scale product).

## 2. Current state (evidence) — file:line

- **Served engine linear-scans and clones the full policy set on evaluate-all.** `handlers/evaluate.rs:258-278`: when no `policy_id`/`policy_name` is given, calls `state.policy_engine.list_policies()` →
  ```rust
  // engine/mod.rs:222-229
  pub fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>> {
      self.active.load().policies.iter()
          .map(|entry| entry.value().clone())   // Arc clone per policy, every request
          .collect()                             // Vec alloc sized to policy count
  }
  ```
  then evaluates every id (deny-overrides), and each `engine.evaluate` (`engine/mod.rs:281-334`) linearly scans that policy's rules. `policy.name.clone()` runs on every evaluate incl. non-matching policies (`engine/mod.rs:329`, Perf P3-2). Cost at 10k policies × 5k rps ≈ 5×10⁷ Arc-clones/s + 5×10⁷ evaluator calls/s, ~400 MB/s alloc churn for the candidate `Vec` alone (Perf P1-1).
- **Indexed/optimized engines exist but are NOT wired into the agent.** `crates/policy-engine/src/indexed_engine.rs` (`IndexedPolicyEngine` with `deploy_policy`/`evaluate`/`get_index_stats`, index built per-principal/resource) and `optimized_engine.rs` exist and are referenced only by benches/examples (`benches/optimization_phases_bench.rs`, `examples/comparison_indexed_vs_linear.rs`). The agent's `AgentState` constructs a plain `PolicyEngine` (`services/reaper-agent/src/state.rs:24 pub policy_engine: PolicyEngine`). `grep IndexedEngine services/reaper-agent/src` → nothing (Perf P1-1).
- **Batch endpoint blocks the async runtime.** `handlers/evaluate.rs:775-890` `batch_evaluate_policy` is an `async fn` that runs the eval loop inline (`.iter().map(...).collect()`), sequential — despite the doc comment claiming rayon parallelism. No `spawn_blocking`/rayon in the agent. Bounded only by the global `DefaultBodyLimit::max(256 * 1024 * 1024)` (`main.rs:538`), so a 256 MB payload of tiny requests = millions of synchronous evals on one worker thread (Perf P1-2).
- **Decision cache is a single global lock.** `decision_cache.rs:90-99`: `cache: RwLock<FxHashMap>` + `order: Mutex<VecDeque>`. Every miss takes the global write lock + order mutex; `fingerprint` (`decision_cache.rs:54-72`) allocates a `Vec<&String>` of context keys and sorts it on **every** get/insert. On low-hit workloads this serializes all cores — a net loss vs the lock-free engine (Perf P2-1).
- **Inconsistent latency metrics, no end-to-end histogram on standard path.** Standard endpoint observes only the engine slice (`evaluate.rs:382-385`, feeding `reaper_decision_duration_seconds`); fast endpoint observes total (`evaluate.rs:692`). Same Prometheus series fed two different measurements; standard path excludes JSON deser (`:158`), ~5 String clones (`:283-290`), cache probe, log-entry build (`:454-485`), response serialization (`:502`) (Perf P2-2). Histogram infra exists with sub-µs buckets (`observability.rs:43-49`).
- **Worker threads not configurable.** `services/reaper-agent/src/main.rs:127` `#[tokio::main]` — worker threads default to host CPU count, no Reaper config knob. A cgroup-limited sidecar over-subscribes (Perf P2-3).
- **ReBAC traversal allocates fresh scratch per condition.** `data/relationships.rs:209-240` + `bfs_reaches` (`:246+`): each traversal condition clones an `EdgeList` and (in BFS) allocates a fresh `FxHashSet` visited + `VecDeque` queue; node budget `TRAVERSAL_NODE_BUDGET = 4096` is per-traversal, so per-request cost = (#rebac conditions) × (up to 4096 nodes) — unbounded across many conditions (Perf P3-4).
- **Perf gate weak.** `.github/workflows/perf-tracking.yml` runs criterion on shared `ubuntu-latest` with a 130% (30%) alert threshold — noisy or blind on a ns-scale product (Perf P2-4). (Repo-map notes it is comment-only as of recent work — either way it does not gate.)

## 3. Definition of Done — testable checkboxes (incl. quantified SLO targets)

**SLO table (served path, `POST /api/v1/messages`, warm, single agent, request-total latency incl. deser+serialize):**

| Scenario | Policies (N) | Load (M rps) | p50 | p99 | p999 |
|---|---|---|---|---|---|
| Targeted (policy_id given), simple DSL | 10,000 | 5,000 | ≤ 2 µs | ≤ 10 µs | ≤ 50 µs |
| Evaluate-all via pruning index (few candidates match) | 10,000 | 5,000 | ≤ 5 µs | ≤ 25 µs | ≤ 100 µs |
| ABAC/ReBAC (bounded traversal) | 10,000 | 2,000 | ≤ 15 µs | ≤ 75 µs | ≤ 300 µs |
| Batch (100 reqs/call, spawn_blocking+rayon) | 10,000 | 500 calls | ≤ 200 µs/call | ≤ 1 ms/call | ≤ 5 ms/call |

(Engine-slice-only p99 for a matched compiled DSL policy stays < 1 µs — reported as a *separate* series, not the SLA.)

- [ ] Served evaluate-all no longer calls `list_policies()`; it queries a pruning index and evaluates only candidate policies. Benchmark at N=10k with 3 matching policies shows evaluator invocations ≈ 3, not 10,000 (assert via `get_index_stats` / a counter).
- [ ] No per-request `Vec<Arc<EnhancedPolicy>>` full-set clone on the served path (verified by an allocation-count test or `dhat` on the eval path; alloc/request bounded by candidate count, not N).
- [ ] Policy-less "evaluate-all" is either rejected by default (config `allow_evaluate_all=false`) or hard-capped at `max_candidate_policies` after pruning; exceeding the cap returns a typed error, not an N-eval fan-out. Test: a request matching >cap candidates returns the cap error.
- [ ] Batch eval runs on `spawn_blocking` + rayon; a `max_batch_requests` cap (e.g. 1,000) is enforced before evaluation and returns 400/413 when exceeded, independent of body size. Test: a 2,000-request batch is rejected; a valid batch does not stall a concurrent single-eval (measured: p99 of a parallel single-eval stream stays within target while a large batch runs).
- [ ] Decision cache is sharded (N-way, e.g. 64 shards) with per-shard lock + eviction; `fingerprint` computes context hash without an intermediate sorted `Vec` (commutative combiner, as `scope_hash` at `decision_cache.rs:286`). Contention benchmark at 32 threads / low hit-rate shows insert throughput scales with cores (no global-writer serialization).
- [ ] Both `/api/v1/messages` and `/api/v1/fast-messages` observe **request-total** latency into `reaper_decision_duration_seconds`; the engine-slice is exported as a distinct series (e.g. `reaper_engine_eval_seconds`). Test asserts both endpoints feed the total series and the two series differ on the standard path.
- [ ] `worker_threads` is configurable via Reaper config/env (e.g. `REAPER_WORKER_THREADS`), defaulting sensibly per profile; sidecar profile does not over-subscribe a 2-vCPU cgroup. Verified by startup log + a config test.
- [ ] ReBAC BFS reuses thread-local `visited`/`queue` scratch (cleared, not reallocated) per condition; a per-request traversal budget caps total nodes across all conditions. Alloc test shows zero fresh `FxHashSet`/`VecDeque` allocation per condition after warm-up.
- [ ] A regression gate runs on a dedicated/self-hosted runner **or** uses statistical gating (`critcmp` + multiple samples + confidence interval) with a threshold ≤ 10%; a synthetic 15% regression fails CI.

## 4. Critical steps — ordered; per step what/where(files)/verify

1. **Add a resource/action pruning index to the served engine and wire it into the agent.**
   - What: Either (a) promote `IndexedPolicyEngine` (`indexed_engine.rs`) into `AgentState`, or (b) add an index (`DashMap<(action|resource-key), SmallVec<PolicyId>>`) alongside the existing `ActiveSet` in `engine/mod.rs`, rebuilt in `replace_all_policies` (`engine/mod.rs:238-265`) and single-policy deploy. Replace the `list_policies()` call in the served evaluate-all branch with an index lookup returning only candidate ids. Return `Arc<str>` policy name or defer `policy.name.clone()` (`engine/mod.rs:329`, P3-2).
   - Where: `crates/policy-engine/src/engine/mod.rs:222-334`, `crates/policy-engine/src/indexed_engine.rs`, `services/reaper-agent/src/handlers/evaluate.rs:258-278,636-644`, `services/reaper-agent/src/state.rs:24`.
   - Verify: bench `benches/` at N=10k, 3 matching → evaluator invocation counter ≈ 3; `dhat` shows no full-set Vec clone. Correctness: differential test vs the current linear-scan engine (same allow/deny for a corpus of requests). Confirm hot-swap safety — index rebuilt atomically within the arc-swap (readers never see a partial index).

2. **Cap unbounded fan-out.**
   - What: Add config `allow_evaluate_all` (default false) and `max_candidate_policies`. When a request has no policy id/name, require the index to return ≤ cap candidates or reject with a typed error; document evaluate-all as a non-default mode.
   - Where: `services/reaper-agent/src/handlers/evaluate.rs:258-278`, agent config (`config/settings.rs`).
   - Verify: request that matches >cap candidates returns the cap error, not an N-eval; unit + integration test.

3. **Move batch eval off the async worker and cap it.**
   - What: Wrap the loop (`evaluate.rs:838`) in `tokio::task::spawn_blocking`, parallelize with `rayon::par_iter` (making the doc comment true). Add `max_batch_requests` config, enforced before evaluation. Apply a **smaller** `DefaultBodyLimit` to the eval routes than to the data-load routes via a per-route layer (currently one global 256 MB at `main.rs:538`).
   - Where: `services/reaper-agent/src/handlers/evaluate.rs:768-890`, `services/reaper-agent/src/main.rs:490-538`.
   - Verify: 2,000-request batch rejected; while a large valid batch runs, a concurrent single-eval stream's p99 stays within the SLO table (head-of-line blocking gone). Add `rayon` to agent deps.

4. **Shard the decision cache; kill the per-probe Vec.**
   - What: Replace `RwLock<FxHashMap>` + `Mutex<VecDeque>` (`decision_cache.rs:90-99`) with an N-way sharded structure (e.g. `DashMap` or array of `Mutex<Shard{map,order}>`), sharded by fingerprint. In `fingerprint` (`decision_cache.rs:54-72`) replace the sorted `Vec<&String>` with an order-independent commutative combiner (XOR/add of per-entry hashes, mirroring `scope_hash` at `:286`). Keep epoch-generation invalidation semantics (`decision_cache.rs:149-168`) intact.
   - Where: `crates/policy-engine/src/decision_cache.rs`.
   - Verify: 32-thread low-hit contention bench shows near-linear insert scaling; alloc test shows zero per-probe Vec; existing cache correctness/epoch tests still pass.

5. **Unify and complete latency histograms.**
   - What: On the standard endpoint, observe request-total (start at handler entry, before deser at `:158`; stop after serialization at `:502`) into `reaper_decision_duration_seconds`. Keep engine-slice as a separate series `reaper_engine_eval_seconds`. Ensure the fast endpoint (`:692`) feeds the same total series. Add p50/p99/p999-friendly buckets covering µs..ms.
   - Where: `services/reaper-agent/src/handlers/evaluate.rs:364,382-385,507,692`, `crates/policy-engine/src/observability.rs:43-49`.
   - Verify: scrape `/metrics` under load; standard-path total histogram > engine-slice histogram; both endpoints comparable. Dashboard shows honest p99.

6. **Make worker threads configurable.**
   - What: Replace `#[tokio::main]` (`main.rs:127`) with an explicit `runtime::Builder::new_multi_thread().worker_threads(cfg)`; read from config/env `REAPER_WORKER_THREADS`, default per profile (sidecar: min(cpus, 2-4); service: cpus).
   - Where: `services/reaper-agent/src/main.rs:127`, agent config.
   - Verify: startup log reports configured count; config test; a 2-vCPU cgroup run doesn't over-subscribe.

7. **Thread-local scratch + per-request budget in ReBAC.**
   - What: In `bfs_reaches` (`relationships.rs:246+`), use `thread_local!` `RefCell<(FxHashSet, VecDeque)>` cleared per call instead of fresh allocation; avoid cloning the full `EdgeList` where a borrow suffices (`:212-213`). Track a per-request traversal node budget (sum across conditions), not just per-traversal.
   - Where: `crates/policy-engine/src/data/relationships.rs:200-260`.
   - Verify: alloc test — zero fresh set/queue alloc per condition after warm-up; cycle-safety + budget-exactness tests (`relationships.rs:346,397`) still pass; adversarial multi-condition policy stays within the per-request budget.

8. **Harden the perf regression gate.**
   - What: Move `perf-tracking.yml` criterion runs to a dedicated/self-hosted runner **or** adopt statistical gating: run N samples, compare to committed baselines with `critcmp` + a confidence interval, fail on >10% regression outside noise. Make it blocking (not comment-only).
   - Where: `.github/workflows/perf-tracking.yml`, `crates/policy-engine/benches/*`, committed baseline artifacts.
   - Verify: inject a synthetic 15% slowdown → CI red; noise-level (<5%) variation → green.

## 5. Dependencies

- **Steps 1-2 (index + cap)** are the foundation; step 5 (honest metrics) must land alongside so the SLO can actually be measured — otherwise the dashboard understates p99 and the gate is meaningless.
- **Step 3 (batch)** depends on adding `rayon` to `reaper-agent` deps and per-route body limits (touches the same `main.rs` router as the auth plan's route changes — coordinate).
- **Step 8 (gate)** depends on step 5's metrics and on a dedicated runner being provisioned (infra dependency outside the codebase).
- **Correctness parity**: step 1's index must be validated against the existing linear engine via the differential proptest harnesses already in the repo (`differential_parity_tests.rs`).
- **Config plumbing** (steps 2, 3, 6) shares the agent config module — batch cap, evaluate-all flag, and worker threads land together.

## 6. Testing & verification

- **Differential correctness**: indexed vs linear engine on a request corpus (reuse `examples/comparison_indexed_vs_linear.rs` + proptest) — identical decisions.
- **Alloc/contention micro-benchmarks**: `dhat`/counter-based alloc tests for the candidate list (step 1), cache probe (step 4), ReBAC scratch (step 7); 32-thread cache contention bench (step 4).
- **Load/SLO harness**: drive `/api/v1/messages` at the SLO-table N×M points, record HDR-histogram request-total p50/p99/p999 (the `benchmarks/reaper-vs-opa` harness already has HDR percentiles — extend it to the scale scenarios).
- **Head-of-line-blocking test**: concurrent single-eval stream + large batch → single-eval p99 within SLO (step 3).
- **Metrics test**: scrape `/metrics`, assert total-vs-slice series semantics (step 5).
- **Regression gate self-test**: synthetic 15% slowdown fails CI (step 8).
- **Hot-swap under load**: deploy/replace policies while driving load — no partial-index observation, decisions stay correct (step 1 + epoch cache invalidation, `decision_cache.rs:163-168`).

## 7. Effort & phasing — S/M/L

- **Phase A (L):** Served pruning index + wiring + fan-out cap (steps 1-2). Largest and highest-impact; needs correctness parity work.
- **Phase B (M):** Batch off-runtime + cap + per-route body limits (step 3).
- **Phase C (M):** Sharded decision cache + fingerprint fix (step 4).
- **Phase D (S):** Metrics unification (step 5) and worker-thread config (step 6).
- **Phase E (S):** ReBAC thread-local scratch + per-request budget (step 7).
- **Phase F (S, infra-gated):** Statistical/dedicated-runner perf gate (step 8).
- P3-1 (`SmallVec<[Uuid;1]>` for single-policy id set, `evaluate.rs:198,286-290`) and P3-3 (`with_resolved` instead of `resolve`, `reaper_dsl/mod.rs:319-327`) are trivial follow-ons folded into Phase A/E.

## 8. Key decisions (ADR-style)

- **ADR-1: Add an index to the existing `PolicyEngine` rather than swap in `IndexedPolicyEngine` wholesale.** The served engine's arc-swap/RCU hot-swap (`engine/mod.rs:238-265`) and epoch-cache invalidation are correct and battle-tested; grafting a pruning index preserves them. Adopting `IndexedPolicyEngine` outright risks losing that hot-swap safety. Consequence: some duplication with `indexed_engine.rs`; reconcile or delete the standalone variant once the served index is proven.
- **ADR-2: Evaluate-all is opt-in and capped, not the default enforcement mode.** A policy-less request evaluating every policy is a DoS amplifier and semantically ambiguous. Default `allow_evaluate_all=false`. Consequence: callers must specify a policy or accept the cap — documented as a behavior change.
- **ADR-3: `spawn_blocking` + rayon for batch, with an explicit request-count cap.** CPU-bound loops must not run on tokio workers. Consequence: adds `rayon` dep and a small task-spawn overhead per batch (amortized across many requests).
- **ADR-4: Shard the cache; keep it opt-in.** The cache stays `Option` (blast-radius containment, `state.rs:30`) but sharding removes the global-writer serialization so enabling it is never a throughput regression. Consequence: slightly higher memory (per-shard order queues).
- **ADR-5: Report two latency series — request-total (the SLA) and engine-slice (the marketing number) — and never conflate them.** Consequence: dashboards and the SLO gate use request-total; the < 1 µs claim is explicitly scoped to the engine slice.
- **ADR-6: Statistical/dedicated-runner perf gate over a fixed 30% threshold on shared runners.** A ns-scale product cannot be regression-gated on noisy shared hardware. Consequence: either infra cost (self-hosted runner) or multi-sample CI time.

## 9. Risks & rollback

- **Risk: pruning index returns wrong candidate set → missed deny (fail-open) or wrong allow.** This is the highest-severity risk (authorization correctness). Mitigation: differential parity tests as a hard merge gate; ship the index behind a config flag (`use_pruning_index`) that falls back to the linear scan; canary in shadow-eval mode comparing both engines before cutover. Rollback: flip the flag.
- **Risk: index rebuild cost regresses deploy/hot-swap latency.** Mitigation: build the new index inside the same arc-swap step as `replace_all_policies` so readers never block; benchmark deploy latency at N=10k. Rollback: flag off.
- **Risk: batch `spawn_blocking` starves the blocking pool under many concurrent batches.** Mitigation: bounded rayon pool + `max_batch_requests` + a concurrency limit on batch tasks. Rollback: revert to inline (accepting P1-2) is possible but reintroduces the DoS — instead keep the cap even if rayon is reverted.
- **Risk: sharded cache changes eviction semantics (per-shard FIFO ≠ global FIFO).** Mitigation: document that eviction is per-shard approximate-FIFO; correctness unaffected (cache is advisory, epoch-invalidated). Rollback: flag back to the global map.
- **Risk: metrics change breaks existing dashboards/alerts keyed on the old series meaning.** Mitigation: introduce the new engine-slice series name additively; update dashboards before repurposing `reaper_decision_duration_seconds`. Rollback: keep both series during transition.
- **Risk: thread-local ReBAC scratch retains large capacity across requests (memory).** Mitigation: clear-not-drop with a periodic shrink threshold. Rollback: revert to per-call allocation (accepting P3-4).
- **General rollback:** every performance change is behind a config flag and validated by differential/correctness tests before the flag defaults on; the linear engine remains the always-available fallback.
