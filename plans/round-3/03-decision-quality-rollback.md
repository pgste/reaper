# Decision-Quality Auto-Rollback

> **STATUS: 📝 PLANNED (round-3)** — closes the one open P1 from
> `reviews/round-3/04-product-architecture.md` (R3-1). The autonomous
> auto-rollback supervisor already exists, is leader-elected, loop-guarded,
> monitor/enforce-gated, audited and metered (`deployment/supervisor.rs`,
> `main.rs:215`). This plan does **not** rebuild that loop. It replaces the
> *signal* feeding its trigger: today the trigger fires on bundle-**apply**
> failure rate (agents that couldn't fetch/verify/install a bundle); it must
> also fire on runtime **decision quality** (a valid policy that installs
> cleanly on 100% of agents and then denies/allows wrongly). Reconnect the
> right input; keep everything downstream.

**Readiness gate:** CONDITIONAL → READY (operational-resilience pillar). This is the single P1 blocking a clean regulated-bank deploy per the round-3 product review.
**Priority:** P1 (S–M — the loop, leader election, monitor/enforce, rollback action, and the live allow/deny signal all already exist; the two are simply not connected).
**Findings closed:** Product **R3-1** (auto-rollback keys on distribution success, not policy behaviour) + review mini-design **T-A**.

---

## 1. Goal

Make the flagship "safe-to-act" auto-rollback react to **what a deployed policy decides**, not only to **whether the bundle installed**. Concretely:

1. **Feed the agent decision buffer's live allow/deny/eval-error rate and latency SLO breach back to the control plane** as first-class health/telemetry (the heartbeat already carries `decisions_allow`/`decisions_deny`/`p99_latency_us`; extend it with an eval-error count and wire it into the rollback trigger).
2. **Define decision-quality rollback signals:** an eval-error-rate spike (absolute threshold — an eval error is never intended), an allow/deny-ratio shift beyond a per-policy baseline band (baseline-delta — a *legitimate* policy change is allowed to move deny rates), and a p99/p999 latency SLO breach.
3. **Wire these signals into the EXISTING trigger** (`evaluate_rollback_trigger`) as an additional arm: `should_rollback = deploy_failure_trip OR decision_quality_trip`. Do not add a second loop.
4. **Canary/progressive rollout** compares candidate-vs-baseline decisions on mirrored traffic and auto-halts on divergence beyond a threshold, reusing the in-tree **counterfactual replay engine** (`services/reaper-management/src/replay/mod.rs`) so canary and production can never combine policies differently.
5. **Guardrails against false-positive rollback storms:** min-sample gate, a measurement window, hysteresis/dampening, an operator override (monitor-first, per-env arming), and a full audit trail of every auto-rollback action with a `decision_quality` reason.

This closes review mini-design **T-A** (`reviews/round-3/04-product-architecture.md:147-155`).

---

## 2. Current state (evidence) — file:line

- **The trigger reads the deploy-apply signal only.** `DeploymentService::evaluate_rollback_trigger` builds `summary = AgentDeploymentRepository::get_summary(rollout.id)` (`deployment/service/mod.rs:581-582`) and computes `should_rollback = summary.failure_rate() > config.error_rate_threshold` (`:602-603`). `failure_rate()` is `failed / total_agents` over the `DeploymentSummary` statuses `{pending, deploying, deployed, failed, acknowledged}` (`domain/agent_deployment.rs:120-146`) — a *distribution* outcome. A policy that installs on every agent shows `failed = 0`, `failure_rate = 0.0`, and the supervisor reports "within threshold" regardless of what it decides.
- **The supervisor loop is complete and correct — only its input is wrong.** `deployment/supervisor.rs:145` calls the same `evaluate_rollback_trigger`; the monitor arm (`:150-199`) audits + SSE + counter, the enforce arm (`:201-266`) cancels then rolls back with the `auto_rollback` loop-guard marker. Nothing here needs changing except what `eval.should_rollback` is derived from.
- **The live decision signal already exists on the agent.** The decision buffer tracks `allow_count`/`deny_count` (`crates/policy-engine/src/decision_buffer.rs:419-440`, `DecisionBufferStats`); the agent exposes `GET /api/v1/decisions/stats` (`services/reaper-agent/src/handlers/decisions.rs:170`, `main.rs:708`) returning `allows`/`denies`/`p99_evaluation_time_ns` (`services/reaper-agent/src/types.rs:292-306`).
- **The agent already ships allow/deny + latency to the control plane every heartbeat.** `collect_metrics` populates `decisions_allow`, `decisions_deny`, `p50_latency_us`, `p99_latency_us` (`services/reaper-agent/src/management/sync.rs:422-483`, `management/types.rs:56-89`); the control plane stores them via the heartbeat handler (`services/reaper-management/src/api/agents.rs:407-456`, `agent_repo.update_metrics → agent_metrics_latest`, `db/repositories/agent.rs:375-460`, table `db/migrations/002_namespaces.sql:99-113`).
- **The gap in the reported signal: no eval-error count.** Agent eval errors (`policy_not_found`, `candidate_cap_exceeded`, `evaluate_all_disabled`, fast-path `parse_error`) are early-return JSON errors (`services/reaper-agent/src/handlers/evaluate.rs:739-915`) observed in the SLA histogram but **not** counted as a decision-quality metric or shipped in the heartbeat. `DecisionBufferStats`/`AgentMetrics` carry no `eval_error` field.
- **No baseline is stored per rollout.** The `Rollout` row (`domain/deployment.rs:158-176`) has no pre-rollout decision-quality baseline, so "a spike vs before this deploy" is currently undefinable.
- **The counterfactual engine exists and is reusable for canary diffing.** `services/reaper-management/src/replay/mod.rs` re-evaluates historical traffic through `PolicyEngine::evaluate_set` (the agent's own serving function) against a pinned bundle + data version and reports allow→deny / deny→allow flip counts — exactly the candidate-vs-baseline diff a canary needs.
- **Config is per-namespace and env-resolvable.** `RollbackConfig` (`domain/agent_deployment.rs:198-230`) already carries `error_rate_threshold`, `window_seconds`, `min_requests`, and `mode` (monitor|enforce), resolved namespace-first then org-default (`deployment/service/mod.rs:560-565`), migration `db/migrations/023_rollout_supervisor.sql`.

---

## 3. Definition of Done — testable checkboxes

- [ ] A **deployed-but-wrong policy** (clean apply — `failed = 0` on every agent — but denying/erroring a spiking share of requests) triggers an **automatic revert within a bounded window** in an integration test (monitor observes, enforce reverts), driven entirely through the existing supervisor pass.
- [ ] The agent counts and reports an **eval-error** metric alongside allow/deny; the heartbeat + `agent_metrics_latest` carry it; `GET /api/v1/decisions/stats` exposes it.
- [ ] `evaluate_rollback_trigger` gains a **decision-quality arm** and returns `should_rollback = deploy_failure_trip OR decision_quality_trip`, with a distinct machine-readable `reason`/trip-kind so audit shows *why* it fired.
- [ ] `RollbackConfig` is extended with decision-quality knobs — `eval_error_rate_threshold`, `denial_delta_threshold`, `latency_p99_slo_us`, `min_decisions`, `decision_window_seconds` — all **optional/defaulted so existing behaviour is unchanged** and **configurable per environment** (namespace-first resolution, unchanged).
- [ ] A **pre-rollout baseline** (allow/deny ratio, eval-error rate, p99) is captured on the rollout at start so "spike" and "ratio shift" are well-defined; baseline-delta is used for denials, absolute thresholds for eval-errors and latency.
- [ ] **Canary divergence auto-halt:** a progressive rollout compares candidate-vs-baseline decisions on mirrored traffic via the counterfactual engine and halts (→ existing rollback) when flip-rate exceeds a threshold, without a human polling.
- [ ] **Guardrails proven in tests:** below `min_decisions` never trips; a legitimate deny-rate change *within* the baseline band does not trip (no false-positive storm); hysteresis prevents flap; an operator can override (monitor mode / disable per env); the loop-guard still prevents re-rolling-back remediation.
- [ ] **Signals are observable:** allow/deny/eval-error rate and latency-breach are exported as Prometheus metrics/histograms on both the agent and control plane; `AUTO_ROLLBACKS_TOTAL` gains a `decision_quality` trigger label; every trip emits an audit entry with the decision-quality reason and the measured-vs-baseline numbers.

---

## 4. Critical steps — ordered

Each step: **what / where(files) / verify / schema**.

### Step 1 — Agent: count eval-errors and export them (S)
- **What:** Add an `eval_errors` `AtomicU64` to the agent stats and increment it on every eval-error early-return (`policy_not_found`, `candidate_cap_exceeded`, `evaluate_all_disabled`, `no_policies_loaded`, fast-path `parse_error`). Surface it on `GET /api/v1/decisions/stats` (`DecisionStats.eval_errors`) and, ideally, as a `DecisionBufferStats.eval_error_count`.
- **Where:** `services/reaper-agent/src/handlers/evaluate.rs:739-915` (increment at each early return, alongside the existing `observe_early_return`); `services/reaper-agent/src/types.rs:292-306` (`DecisionStats`); optionally `crates/policy-engine/src/decision_buffer.rs:419-440`.
- **Verify:** unit test — driving each early-return path bumps `eval_errors` by one; a clean allow/deny does not.
- **Schema:** none (in-memory counters).

### Step 2 — Ship the decision-quality signal in the heartbeat (S)
- **What:** Extend `AgentMetrics` (agent + control-plane mirror) with `eval_errors` (and reuse the already-present `decisions_allow`/`decisions_deny`/`p99_latency_us`). Populate it in `collect_metrics`. Persist it in `agent_metrics_latest`.
- **Where:** `services/reaper-agent/src/management/types.rs:56-89` + `management/sync.rs:455-483`; `services/reaper-management/src/domain/agent.rs:104-141`; `db/repositories/agent.rs:375-460`; new migration **027_agent_decision_quality.sql** — `ALTER TABLE agent_metrics_latest ADD COLUMN eval_errors INTEGER DEFAULT 0` (+ `latency_p999_us` if the agent reports it).
- **Verify:** integration — a heartbeat carrying `eval_errors` lands in `agent_metrics_latest` and is readable via `get_metrics`. `#[serde(default)]` keeps old agents compatible.
- **Schema:** 027 (additive, nullable/defaulted — down-migration drops the column).

### Step 3 — Per-rollout decision-quality baseline (M)
- **What:** At `start_rollout`, snapshot the target namespace's current aggregate decision quality (allow/deny ratio, eval-error rate, p99 across the namespace's agents from `agent_metrics_latest`) onto the rollout row. This defines the "before" against which post-rollout deltas are measured. A rollout with no prior traffic records an empty baseline (→ absolute thresholds only until `min_decisions` accrues).
- **Where:** `domain/deployment.rs:158-176` (add `baseline_decision_quality: Option<serde_json::Value>` or typed columns); `deployment/service/mod.rs` start path; migration **028_rollout_decision_baseline.sql**.
- **Verify:** unit — baseline captured at start reflects namespace metrics; absent-traffic → empty baseline, not a divide-by-zero.
- **Schema:** 028 (additive column on `rollouts`).

### Step 4 — Extend `RollbackConfig` with decision-quality thresholds (S–M)
- **What:** Add optional fields: `eval_error_rate_threshold: Option<f64>` (absolute), `denial_delta_threshold: Option<f64>` (baseline-delta), `latency_p99_slo_us: Option<f64>` (absolute), `min_decisions: u32`, `decision_window_seconds: u32`. All `None`/defaulted so the current deploy-apply behaviour is byte-for-byte unchanged when unset. Resolution stays namespace-first → org-default (so it is **per-environment configurable** via the env's bound namespace).
- **Where:** `domain/agent_deployment.rs:198-242` (`RollbackConfig` + `UpdateRollbackConfig`); `api/deployments/rollback_config.rs` (handler validation, 400 on nonsense); migration **029_rollback_decision_thresholds.sql** (ALTER `rollback_configs`).
- **Verify:** unit — config round-trips; unset decision-quality fields ⇒ trigger behaves exactly as today.
- **Schema:** 029 (additive columns, defaulted).

### Step 5 — Add the decision-quality arm to the trigger (M)
- **What:** In `evaluate_rollback_trigger`, after the existing deploy-apply computation, compute a decision-quality trip from the target namespace's live metrics vs the rollout baseline (Step 3), gated by `min_decisions`/`decision_window_seconds`: `eval_error_rate > eval_error_rate_threshold` (absolute) **OR** `denial_rate − baseline_denial_rate > denial_delta_threshold` (delta) **OR** `p99 > latency_p99_slo_us`. Return `should_rollback = deploy_failure_trip OR decision_quality_trip` and enrich `RollbackTriggerEvaluation` with the trip kind + measured/baseline numbers. **The supervisor's monitor/enforce arms, leader election, loop guard, cancel+rollback action are untouched.**
- **Where:** `deployment/service/mod.rs:557-627` (extend the evaluation; add a decision-metrics read alongside `get_summary`); `api/deployments/types.rs` (`RollbackTriggerEvaluation` fields); `deployment/supervisor.rs:145-266` reads the enriched eval unchanged (thread the trip kind into the audit `details` + the `AUTO_ROLLBACKS_TOTAL` label).
- **Verify:** unit — clean apply + eval-error spike ⇒ `should_rollback = true` with `reason = decision_quality`; clean apply + healthy decisions ⇒ false; deploy-apply failure still trips as before.
- **Schema:** uses 028/029.

### Step 6 — Canary candidate-vs-baseline divergence auto-halt (M)
- **What:** For progressive/canary strategies, run a bounded counterfactual diff — mirror the canary window's real requests through the counterfactual engine against the *baseline* bundle and compare flip counts (allow→deny / deny→allow) against a divergence threshold. On breach, halt the wave and route into the existing rollback (same terminal action as Step 5). This makes the canary an active gate, not just a slow rollout.
- **Where:** reuse `services/reaper-management/src/replay/mod.rs` (`evaluate_set`-based diff); invoke from the supervisor pass or the wave-advance gate (`deployment/service/helpers.rs` approval/confirmation loop) for canary rollouts; threshold from `RollbackConfig` (Step 4).
- **Verify:** integration — a canary whose candidate flips > threshold of decisions vs baseline auto-halts and rolls back; an equivalent-behaviour candidate proceeds.
- **Schema:** none new (replay reads existing decision + data-version tables).

### Step 7 — Guardrails: hysteresis, dampening, override, audit (S–M)
- **What:** (a) `min_decisions` + window gate (already in Step 4/5) — never trip on thin samples; (b) hysteresis — require the breach to persist across ≥N consecutive supervisor ticks before enforce acts, so a one-tick blip doesn't storm; (c) operator override — `monitor` stays the default (observe the decision-quality trip for a release before arming `enforce` per env), plus a per-env disable; (d) audit — every trip writes an audit entry with trip kind, measured vs baseline, window, and (enforce) the rollback rollout id, reusing the existing `DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED`/`DEPLOYMENT_AUTO_ROLLBACK` actions.
- **Where:** `deployment/supervisor.rs:150-266` (persist-across-ticks counter beside the existing `flagged` set; audit `details`); `metrics.rs:134` (`AUTO_ROLLBACKS_TOTAL` gains a `trigger` label = `deploy_apply|decision_quality`; add decision-rate gauges/histograms).
- **Verify:** unit — a single-tick spike below the persist count does not enforce; a sustained spike does; monitor mode never acts; audit row carries the decision-quality reason.
- **Schema:** none.

### Step 8 — Observability + docs (S)
- **What:** Export agent decision allow/deny/eval-error rate + latency as Prometheus metrics; export the control-plane trigger evaluation (current vs baseline, trip kind) as gauges/histograms; document the new signals, thresholds, per-env configuration, and monitor→enforce arming in the operations guide.
- **Where:** agent `metrics`; `services/reaper-management/src/metrics.rs`; `docs/deployment/OPERATIONS_GUIDE.md`.
- **Verify:** `/metrics` on both planes shows the new series; a runbook describes arming enforce per environment.
- **Schema:** none.

---

## 5. Dependencies

- **The existing supervisor is the required termination point** (`deployment/supervisor.rs`, `deployment/service/mod.rs:557-627`, `main.rs:215`). This plan is a signal swap + additive trigger arm, **not** a new loop — reuse leader election, monitor/enforce, loop guard, cancel+rollback verbatim.
- **The heartbeat metrics channel** (`api/agents.rs:407-456` → `agent_metrics_latest`) is the transport for the live signal; Steps 1–2 extend it. No new agent→control-plane channel is introduced.
- **The counterfactual replay engine** (`services/reaper-management/src/replay/mod.rs`, `PolicyEngine::evaluate_set`) is the canary-diff primitive (Step 6) — reuse it so canary and production evaluate identically. (Its ephemerality is R3-3's concern, not this plan's; a short-lived canary diff does not need durability.)
- **Per-env resolution** rides the existing namespace-first `RollbackConfig` lookup and the first-class `Environment→namespace` binding (Plan 10) — decision-quality thresholds are configurable per environment for free.
- **Migrations 027/028/029** must precede the code paths that read the new columns.
- **Auth:** the rollback-config and rollout endpoints already sit behind the default-deny gateway + org scope; new config fields inherit it.

---

## 6. Testing & verification

- **Unit:** eval-error counter increments per early-return (Step 1); config round-trip with decision-quality fields unset ⇒ identical to today (Step 4); trigger arm truth table — clean-apply + eval-error spike / deny-delta breach / p99 breach each ⇒ trip, healthy ⇒ no trip, below `min_decisions` ⇒ no trip (Step 5); baseline capture with and without prior traffic (Step 3); hysteresis persist-count (Step 7).
- **Integration (management, real DB):** simulate a rollout where every agent reports `failed = 0` but heartbeats a rising eval-error / deny rate → `run_supervisor_pass` in monitor audits once, in enforce cancels the rollout and starts a marked rollback — the **DoD headline test** (extends the existing `deployment/supervisor.rs` tests, which already fabricate rollouts + agent metrics).
- **Canary divergence (integration):** a canary whose candidate diverges beyond threshold vs baseline (via the replay engine) auto-halts and rolls back; an equivalent candidate proceeds through waves unchanged.
- **Regression:** the three existing supervisor tests (`enforce_mode_cancels…`, `monitor_mode_audits_once…`, `below_threshold_or_disabled…`) pass unchanged — the deploy-apply trip still works when decision-quality fields are unset.
- **False-positive/storm negative tests:** a legitimate deny-rate change within the baseline band does not trip; a one-tick blip below the persist count does not enforce; monitor mode never acts.
- **Observability:** assert the new Prometheus series and the `AUTO_ROLLBACKS_TOTAL{trigger="decision_quality"}` label are emitted; assert the audit entry records trip kind + measured/baseline.

---

## 7. Effort & phasing — S/M/L

- **Phase A — Signal (P1).** Step 1 (S) + Step 2 (S) + Step 8 partial. Get the eval-error/allow/deny/latency signal reported and observable end-to-end. Cheap, independently valuable. **~S.**
- **Phase B — Trigger (P1, the required fix).** Step 3 (M) + Step 4 (S–M) + Step 5 (M) + Step 7 (S–M). The decision-quality arm wired into the existing trigger with baseline + guardrails; ship in **monitor** first, arm **enforce** per env. Closes R3-1's DoD. **~M.**
- **Phase C — Canary gate (P1→P2).** Step 6 (M) + Step 8 remainder. Active candidate-vs-baseline canary halt via the replay engine. **~M.**

Cheapest slice that closes R3-1: **Phase A + Phase B** (signal reported + trigger arm + baseline + guardrails, monitor-then-enforce). The canary diff (Phase C) is the strong-form upgrade and can follow.

**Overall: S–M** — matches the review's sizing; the loop, leader election, monitor/enforce, rollback action, and the live allow/deny signal all already exist.

---

## 8. Key decisions (ADR-style)

### ADR-1 — Reconnect the input; do not rebuild the loop
- **Decision:** Extend `evaluate_rollback_trigger` with a decision-quality arm and OR it with the existing deploy-apply trip; leave the supervisor loop, leader election, monitor/enforce, loop guard, and cancel+rollback action untouched.
- **Why:** The round-3 review is explicit that "the loop is right; connect it to the decision-quality signal" (`reviews/round-3/04-product-architecture.md:84,154`). The supervisor is the review's strongest pillar; rebuilding it would add risk for no benefit. The trigger is the *one* component consuming the wrong signal.
- **Rejected:** a parallel decision-quality supervisor (duplicates leader election + audit + loop-guard; two competing revert paths).

### ADR-2 — Absolute threshold for eval-errors and latency, baseline-delta for denials
- **Decision:** Eval-error-rate and p99/p999 breaches use **absolute** thresholds; the allow/deny-ratio shift uses a **baseline-delta** band captured at rollout start.
- **Why:** An eval-error or an SLO breach is never an intended effect of a policy change, so an absolute line is correct and simple. But a *legitimate* policy change is often *supposed* to move deny rates — an absolute deny threshold would punish intended tightening/loosening. Baseline-delta catches "this deploy *flipped* behaviour" without penalising deliberate change (`reviews/round-3/04:155`).
- **Rejected:** absolute deny-rate threshold (false-positives on every legitimate tightening); pure ML anomaly detection (opaque, unauditable for a regulated buyer).

### ADR-3 — Reuse the heartbeat as the signal transport; reuse the counterfactual engine for canary
- **Decision:** Carry allow/deny/eval-error/latency to the control plane on the **existing heartbeat** (`agent_metrics_latest`), and reuse the **counterfactual replay engine** for candidate-vs-baseline canary diffing.
- **Why:** The heartbeat already ships allow/deny/latency and is the agent's health channel — adding one counter is minimal surface. The replay engine already re-evaluates via `PolicyEngine::evaluate_set`, guaranteeing canary and production combine policies identically — building a second diff path would risk divergent semantics.
- **Rejected:** a new agent→control-plane decision-metrics stream (redundant channel); a bespoke canary evaluator (semantic-drift risk vs the served path).

### ADR-4 — Monitor-first, per-environment arming
- **Decision:** Decision-quality thresholds default off; when set, `monitor` is the default mode; `enforce` is armed per environment (namespace-first `RollbackConfig`).
- **Why:** A brand-new autonomous revert signal must be observable and trusted before it acts in prod; per-env arming lets a bank watch the decision-quality trip on a release, then enable enforce on prod deliberately — matching the existing monitor/enforce design and Plan 10's env model.

---

## 9. Risks & rollback

- **Risk: false-positive rollback storm.** A noisy or legitimately-changing deny rate auto-reverts good policy. **Mitigation:** `min_decisions` + measurement window + baseline-delta (not absolute) for denials + hysteresis (persist across N ticks) + monitor-first arming; all thresholds per-env tunable.
- **Risk: signal blind spot from thin/quiet traffic.** A low-QPS namespace never accrues `min_decisions`, so a bad policy slips. **Mitigation:** the canary counterfactual diff (Step 6) exercises real historical traffic independent of live volume; document that decision-quality auto-rollback needs representative traffic and pair it with canary for low-QPS envs.
- **Risk: attribution error.** Namespace-level aggregate metrics blamed on the wrong rollout if two changes overlap. **Mitigation:** baseline captured at *this* rollout's start; the supervisor already loop-guards its own remediation; window-scoped comparison bounds cross-rollout bleed.
- **Risk: added heartbeat/DB load.** One extra counter per heartbeat + baseline read at start. **Mitigation:** negligible — reuses the existing metrics upsert and a single indexed read; no new hot-path cost on the agent eval slice.
- **Risk: enforce reverts during a legitimate incident-response push.** An operator intentionally shipping a stricter deny policy triggers a deny-delta trip. **Mitigation:** monitor-first, per-env enforce arming, operator override/disable, and the audit trail showing the measured-vs-baseline numbers so the operator can retune.
- **Rollback plan:** migrations 027/028/029 are additive (defaulted columns) — down-migrations drop them without touching `rollouts`/`agents`/`rollback_configs` semantics. With all decision-quality fields unset, `evaluate_rollback_trigger` is byte-for-byte today's deploy-apply behaviour, so the feature is dark by default and revertible by clearing config. Because the arm terminates in the existing rollback action, existing namespace/org rollback recovery is unchanged.
