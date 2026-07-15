# Reaper — Round-2 Remediation Backlog (candidate, un-sequenced by decision)

Derived from `reviews/round-2/05-SYNTHESIS.md` and the five reviewer reports.
This is a **decision menu**, not a committed plan: pick the order. Round 2's
verdict is **CONDITIONAL** (0 P0, 11 P1) — everything below is finishing work on
pillars that already exist, not new architecture. Round-1's 12-plan roadmap is
shipped; this is the next, much smaller set.

**How to read effort:** S ≈ days, M ≈ 1–3 weeks, L ≈ 3–6 weeks (one engineer).
**"Closes"** cites the finding IDs from `reviews/round-2/`.

---

## The six workstreams at a glance

| WS | Theme | Items | Severity peak | Total effort | Why it matters |
|----|-------|-------|---------------|--------------|----------------|
| A | Audit integrity operationalization | A1–A5 | P1 (P0-adjacent default) | M–L | The #1 bank-board rejection; 3 reviewers converge here |
| B | Propagation surface safety | B1–B2 | P1 | S–M | Strongest pillar's two remaining safety holes |
| C | Data-plane API hardening | C1–C4 | P1 | S–M | DoS + lost-update on the biggest tables |
| D | Measured SLO + eval finishing | D1–D4 | P1 | M | The headline latency claim is unproven |
| E | Enterprise / compliance finishing | E1–E4 | P1 | M–L | SOC onboarding + DSAR blockers |
| F | Strategic bet (forward, optional) | F1–F2 | — | L+ | Where the category is going, not where it is |

---

## Workstream A — Audit integrity operationalization  *(COMPLETE — A1-A5 landed)*

The chain + signed checkpoints are real and tested; the *operationalization* is
missing. This is the cluster a regulator's "prove the log is complete and
unaltered" question lands on.

- **A1 — Ship a store-backed audit verifier (CLI + endpoint).** *Closes SEC R2-2,
  PROD R2-10.* `verify_chain`/`verify_checkpoint` exist but are called only from
  unit tests. Build `reaper audit verify` (CLI) + `GET …/audit/verify` that pull
  `decisions`+`checkpoints` from ClickHouse and prove a range intact. **Effort M.**
  Prereq for A2.
- **A2 — Make the chain verifiable from the queryable store.** *Closes SEC R2-2.*
  `seq` is assigned pre-enqueue and can diverge from writer order; ClickHouse
  `ORDER BY` + ReplacingMergeTree dedup doesn't preserve write order. Persist a
  monotonic *writer-assigned* order key and verify by it, not by query order.
  **Effort M.** Couples with A1.
- **A3 — Independent immutable checkpoint anchor + cross-boot linkage.** *Closes
  SEC R2-3.* Enable the S3/WORM checkpoint sink by default (it's commented out in
  `vector.toml:97-107`); link boots with a signed genesis `chain_id` so an insider
  can't delete a boot's decisions+checkpoints together undetectably. **Effort M.**
  This is the piece that flips A from P1 to "P0-adjacent closed."
- **A4 — Durable-before-serve for mandatory-audit mode.** *Closes SEC R2-4, and
  removes PERF R2-P2-2's reactor-stall.* Today decisions are served before the
  async writer persists; `Block` mode blocks the tokio reactor. Add a durable
  backpressuring path (WAL or async bounded channel off the reactor) so mandatory
  mode has no served-allow loss window and no head-of-line stall. **Effort M.**
- **A5 — Redaction-on-by-default posture + redactable `resource`.** *Closes SEC
  R2-5.* PII redaction is all opt-in and `resource` has no redaction path. Make
  redaction policy explicit at enable time, allow `resource` redaction, ship a
  GDPR-compliant default profile. **Effort S.**

---

## Workstream B — Propagation surface safety  *(COMPLETE — B1-B2 landed)*

- **B1 — Fine-grained authz on rollout/rollback/approve-wave/cancel/pin.** *(landed)* *Closes
  SEC R2-1.* These gate only on org-membership/Admin — any token (incl. read-only
  service tokens) can roll the fleet. Require a deploy scope
  (`BundlePromote`/new `DeploymentWrite`) + `OrgAdmin` fallback, mirroring
  `change_requests.rs` authorize. **Effort S.** High value / low cost.
- **B2 — Autonomous auto-rollback control loop.** *(landed)* *Closes PROD R2-1.* Thresholds +
  signal + trigger + action all exist (`rollback_config.rs:274-364`); missing only
  a supervising loop (no `main.rs` task is a rollout supervisor). Reuse the
  existing advisory-lock leader-election. Ship in `monitor` mode, then arm
  `enforce` per namespace. **Effort M.** *The product reviewer's single most
  important next move* — the last safety gap with no operational workaround.
  *Landed as `deployment/supervisor.rs`: leader-elected loop (advisory key
  `ROLLOUT_SUPERVISOR`), shared trigger evaluation
  (`DeploymentService::evaluate_rollback_trigger`), `mode: monitor|enforce` on
  the rollback config (migration 023/0016, default monitor), loop guard via
  `rollouts.triggered_by='auto_rollback'`, audit actions
  `deployment.auto_rollback[_triggered]`, SSE `auto_rollback_triggered`,
  counter `reaper_management_auto_rollbacks_total`, and read-only
  `GET /orgs/{org}/rollouts/{id}/rollback-status`.*

---

## Workstream C — Data-plane API hardening  *(COMPLETE — C1-C4 landed)*

- **C1 — Paginate the ABAC/ReBAC list endpoints.** *(landed)* *Closes CODE R2-01.* Entity /
  role-binding / tuple lists return everything with no `LIMIT` — the biggest
  tables. Route through the existing `PageQuery`/`Paginated` keyset (as `policies`
  already does). **Effort S.** Availability foot-gun; cheap fix.
  *Landed: `list_entities_page`/`list_bindings_page`/`list_tuples_page` keyset
  over `(created_at, id)` with the `LIMIT n+1` sentinel; the three HTTP lists
  return the uniform `Paginated` envelope (default 50 / hard max 200 → 400);
  the unbounded repo methods remain for internal snapshot/plan builders only.*
- **C2 — Turn optimistic concurrency from dormant to enforced.** *(landed)* *Closes CODE
  R2-02, R2-03.* `require_if_match` defaults false (warn-only); policy ETag is
  content-hash so metadata edits silently lose updates. Flip the GA default to
  true; derive ETag from a row-version/`updated_at` every write bumps. **Effort
  S.**
  *Landed: `server.require_if_match` defaults **true** (`REAPER_REQUIRE_IF_MATCH=false`
  is the one-release opt-down); new `policies.row_version` column (migration
  024/0017) bumped by every UPDATE; ETag = `content_hash.r{row_version}` and the
  SQL guard is `AND row_version = $expected`, so metadata races 412 instead of
  clobbering. Docs: `docs/api/VERSIONING.md` §5.*
- **C3 — Version-guard + idempotency on `apply_migration`.** *(landed)* *Closes CODE R2-04,
  R2-05.* Plan 12's migration apply has no `model_version` optimistic guard and no
  Idempotency-Key — concurrent migrations clobber, retried timeouts double-apply +
  double-propagate. Add the guard (the plan already carries `model_before`) and
  wrap in `idempotency::run`. **Effort S.**
  *Landed: the model UPDATE carries `AND model_version = $expected` inside the
  apply transaction (mismatch → full rollback → 409 naming expected vs actual);
  the endpoint runs under `idempotency::run` scope `datastore.migrate` keyed on
  the transform fingerprint, mirroring `start_rollout` — a retried timeout
  replays the stored response instead of double-applying/double-publishing.*
- **C4 — Typed DTOs + error model on the contract.** *(landed)* *Closes CODE R2-06, R2-08.*
  Datastore/decisions/replay/audit handlers return untyped `Json<Value>`;
  `ProblemDetails` isn't `ToSchema` and omits `instance`. Add DTOs + schema-lint so
  the published contract is client-codegen-clean. **Effort M.**
  *Landed: all 32 `Json<Value>` handlers across datastore/decisions/replay/audit
  now return named `#[derive(Serialize, ToSchema)]` DTOs (wire shapes unchanged —
  the existing integration tests pin them); `ProblemDetails` is a published
  component with the RFC 9457 `instance` member (= request path) and a
  `request_id` extension, stamped by the idempotent `problem_instance` middleware
  (inside `build_served_router` + outside the auth gateway, so extractor and
  gateway rejections carry it too); `tests/api_contract.rs` gained
  `contract_is_publishable` — hard-fails on a missing/incomplete ProblemDetails
  schema, on untyped 4xx bodies or any documentation regression in the typed
  groups, and ratchets summary/4xx-coverage/success-body checks over the rest of
  the surface (baselines 0/129/94 — lower, never raise).*

---

## Workstream D — Measured SLO + eval finishing  *(COMPLETE — D1-D4 landed; forward perf follow-ups parked, see "Parked / deferred" below)*

- **D1 — Build the deferred load harness and gate the request-total SLO.** *(landed)* *Closes
  PERF R2-P1-1.* The headline sub-µs SLA is never measured on the real HTTP path;
  the perf gate benches only 2 in-process micro-suites. Extend the reaper-vs-opa
  HDR harness to the N×M points, assert request-total p99/p999 in CI. **Effort M.**
  Without this, "READY" rests on unmeasured assumptions.
  *Landed: `benchmarks/reaper-vs-opa` gained `slo-harness` (four §3 scenarios —
  slo-targeted / slo-evaluate-all / slo-rebac / slo-batch — HDR request-total
  p50/p99/p999 over HTTP against a real agent, with per-scenario decision
  probes), `generate-data policy-set --language dsl|simple` (10k-distinct-policy
  sets; the evaluate-all row uses Simple because the pruning index doesn't
  prune DSL yet — see D2), and a checked-in `slo.yaml` encoding the §3 table
  with `--slo-multiplier` semantics (1.0 = the real SLA on dedicated hardware).
  CI: nightly absolute assertion in `slo-harness.yml` (multiplier 250 on shared
  runners, documented as an observed starting point; artifacts uploaded), and a
  PR-blocking paired A/B HTTP job (`http-slo-ab` in `perf-gate.yml`) comparing
  merge-base vs head request-total p99 via `perf_ab_gate.py --http-ab`
  (median-ratio 1.25x + disjoint-samples rule, self-tested).*
- **D2 — Make the DSL prunable (wire resource-literal extraction).** *(landed)*
  *Closes PERF R2-P2-1.* The pruning index previously pruned only the *deprecated*
  Simple language; DSL evaluate-all mass-denied past 256 policies or degraded to
  O(N log N)/req. **Effort M.**
  *Landed: added a `PolicyEvaluator::resource_index_terms() -> Option<Vec<String>>`
  trait method (default `None` = unprunable = always a candidate) so match-set
  soundness lives next to each evaluator's own semantics — closing the review's
  "latent trap" where index correctness was coupled to Simple's exact-match in a
  separate `engine::index_terms` function. The compiled `ReaperDSLEvaluator`
  (PRIMARY path) overrides it: it walks each compiled rule's `CompiledCondition`
  and extracts request-resource literals from `ResourceIdEquals` leaves
  (`resource == "lit"`), composing `And` (intersection of bounded children),
  `Or` (union iff every child bounded), `Not(Always)`→∅, `Always`→unbounded;
  ANY unbounded rule ⇒ whole policy `None`. Attribute/action/rebac/time/string/
  variable predicates and dynamic resource ids are all unbounded (`None`), so it
  can never fail open. `SimplePolicyEvaluator` moved its exact-match logic into
  the same trait method; the `ReapAstEvaluator` fallback and Cedar keep the safe
  default `None`. `engine::index_terms` now delegates to
  `policy.get_evaluator().ok()?.resource_index_terms()` — same downstream
  indexing (`index_policy`/`candidate_policy_ids`/`unprunable`, sort+dedup).
  Deviation from the original plan: implemented as a compiled-condition walk in
  the evaluator (the mandated compiled-first path) rather than by wiring
  `partial_evaluation.rs`. Tests: 12 unit tests on
  `ReaperDSLEvaluator::resource_index_terms` incl. the REQUIRED soundness
  differential (`test_ridx_soundness_differential`: every resource outside the
  reported terms is verified non-decisive under real `evaluate_matched`), plus
  engine-level `dsl_policies_are_prunable_by_resource_literal` /
  `dsl_or_of_literals_buckets_both` in `pruning_index_tests.rs`. AST-side
  extraction is a documented follow-up.*
- **D3 — Observe request-total latency on every return path.** *(landed)* *Closes PERF
  R2-P2-3.* Denies (stale-data, policy-not-found, cap-exceeded) skip the SLA
  histogram, so denial storms go invisible on the dashboard. **Effort S.**
  *Landed: both `/api/v1/messages` and `/api/v1/fast-messages` observe
  request-total into `reaper_decision_duration_seconds` on every early-return
  deny (data_stale, policy_not_found, evaluate_all_disabled, no_policies_loaded,
  candidate_cap_exceeded, fast-path parse_error) under the constant
  `early_deny` label, plus the audit-gate and audit-persist 503s; the timer now
  starts before the first possible return. Engine-slice series untouched.
  Test: `sla_histogram_deny_paths_tests.rs` (exact sample-count deltas per
  path + success-path label sanity).*
- **D4 — Reconcile or delete the parallel engines / dead arena.** *(landed)*
  *Closes PERF R2-P3-1/2/3.* Deleted the three unwired experiments —
  `arena.rs` (its `with_arena`/`ArenaString` API was never called by any
  evaluator), `indexed_engine.rs` (self-documented as 6–8× **slower** than the
  linear baseline — DashMap overhead), and `optimized_engine.rs` (wrapped the
  two) — plus their bench (`optimization_phases_bench`) and three comparison
  examples. `partial_evaluation.rs` was **kept**: the review listed it in the
  trio, but its `Condition` enum + `PartialEvaluator` are live on the served
  compiled path (`compiled_evaluator.rs`), so it is not dead. Shipped
  `SmallVec<[Uuid; 1]>` on the targeted single-policy path (the id no longer
  heap-allocates; evaluate-all still spills to heap via `.into()`, unchanged).
  The *served* pruning index (`engine::candidate_policy_ids`, resource buckets)
  is a separate, wired, working structure — not one of the deleted engines; its
  DSL gap is D2's job. **Effort S.** Done before D2.

---

## Workstream E — Enterprise / compliance finishing  *(COMPLETE — E1, E2, E3, E4 all landed. See the per-plan STATUS docs.)*

- **E1 — Native SIEM export connectors.** *(COMPLETE — all 4 slices landed; see
  `plans/round-2/E1-siem-connectors.md` STATUS.)* *Closes PROD R2-2.* Shipped OCSF
  (Authorize Session 3003) + CEF shaping in `policy-engine`, Vector sink overlay
  (Kafka/Splunk-HEC/S3 + OCSF transform), the control-plane push-export connectors
  API (`audit:export` scope), and an optional low-latency agent streaming sink.
  **Effort M.** Bank SOC onboarding blocker.
- **E2 — GDPR subject-erasure endpoint.** *(COMPLETE — `POST /orgs/{org}/audit/erasure`
  live + all three follow-ups landed; see `E2-subject-erasure.md` STATUS)*
  *Closes PROD R2-3.* Erase-by-subject over ClickHouse (redact-in-place,
  hold-honoring, chain-preserving; pseudonymised-column matching via
  request-supplied salt) + DataStore cascade, with immutable-surface exemptions
  disclosed on a receipt and a queryable `audit_erasure_requests` history
  (`GET …/audit/erasures`). **Effort M.** UK DPA-2018 DSAR gap.
- **E3 — Signed air-gap export/import.** *(COMPLETE — see
  `plans/round-2/E3-airgap-signing.md` STATUS.)* *Closes PROD R2-4, SEC (air-gap
  half of R2-10 round-1).* Shipped `reaper bundle export` (v2-signed `.rbb` +
  `.sig` sidecar), `bundle import` (offline verify + deploy-with-signature +
  attestation), `bundle deploy` sidecar auto-attach, and the agent checksum report
  (`bundle_hash` on `list_policies` + `bundle attest`). **Effort M.**
- **E4 — Real multi-tenant quota enforcement.** *(COMPLETE.)* *Closes PROD R2-9.*
  Wired real usage counts (was `UsageMetrics` hardcoded 0), plan tiers persisted
  in `Organization.settings` with per-org overrides, resource-quota enforcement at
  agent-register / policy-create (`402` over limit), and a per-tenant request-rate
  ceiling (`api_per_org_per_minute` → `429`) on those paths. `GET /orgs/{org}/billing`
  exposes real usage. `src/quota/` + `docs/deployment/TENANT_QUOTAS.md`. **Effort M.**

---

## Workstream F — Strategic bet  *(forward-looking, from `06-future-architecture.md`)*

Not remediation — a wager on where authorization is going. The strategist's case:
Reaper is hardening a commoditizing eval core while the differentiating axis sits
unclaimed. Only pursue if the tactical P1s above are funded first.

- **F1 — Authorization for non-human / agentic AI actors.** Attenuated short-lived
  capabilities, per-request LLM context taint, cheap allow-path explainability, an
  MCP adapter. Every existing strength (sub-µs eval, ReBAC, provenance) re-values
  upward here. **Effort L+.** The one axis where Reaper would *lead* rather than
  race.
- **F2 — Wasm eval build target.** The source tree is ~80% ready; unlocks
  edge/browser/multi-language embedding and rides the OPA→wasm / Cedar→wasm trend.
  **Effort L.** Enables F1's distribution.

---

## Parked / deferred — forward perf follow-ups (saving for later)

Not on the committed path. These are optional eval-engine refinements, gated on a
profile that actually shows the engine slice (not the HTTP/loopback floor)
mattering — today the served DSL path is ~1µs while the wire floor is ~150µs, so
none of these move the needle yet. Captured here so the intent isn't lost as
orphan code.

- **P-1 — Wire partial evaluation into the served compiled DSL path.**
  `crates/policy-engine/src/partial_evaluation.rs` exists but is unwired: its
  `Condition` primitives are reached only through `CompiledPolicyEvaluator::compile`
  (`compiled_evaluator.rs`), which **no served path calls** (the DSL serve path is
  `ReaperPolicy::build_preferred` → `ReaperDSLEvaluator`, not `build_compiled`), and
  its DSL/Cedar `partial_evaluate` is a TODO no-op. To make it real: constant-fold
  static `principal.*`/`resource.*` conditions at *deploy* time in the compiled
  DSL v2 evaluator, and add it to the perf A/B gate. **Only worth it if profiling
  shows compiled-condition evaluation dominates — it currently does not.** Effort M.
- **P-2 — AST-side resource-index extraction (D2 follow-up).** D2 made the
  *compiled* `ReaperDSLEvaluator` prunable via `resource_index_terms()`; the
  `ReapAstEvaluator` fallback still returns `None` (always a candidate). Extend the
  same soundness-preserving extraction to the AST evaluator so AST-tier DSL policies
  also prune. Compiled is the primary path, so this is strictly secondary. Effort S.

Sibling work (both eval-perf); if either is picked up, do it as a "D5" slice with a
profiling justification, not as remediation.

---

## Recommended sequence (a starting proposal — override freely)

The reviewers' own priority signals point one way; here's a defensible ordering
that front-loads the bank-board rejections and the cheapest high-value wins:

1. **B1 + C1 + C2 + C3 + D3** — the "one focused week" bundle: all S-effort, they
   close a P1 (fleet-authz) and the lost-update/pagination/observability cluster
   that a bank load-tests first. Highest value-per-day on the board.
2. **Workstream A (A1→A2→A3→A4→A5)** — the dominant theme and rejection #1. Do it
   as a unit; A3 is the piece that flips it to "P0-adjacent closed." Largest single
   lift.
3. **B2** — autonomous auto-rollback. The product reviewer's top pick; closes the
   last safety gap on the flagship pillar. Do right after A so the fleet is both
   governed (B1) and self-healing (B2).
4. **D1 + D2/D4** — prove the SLO, then make the DSL prunable and retire the dead
   engines. Turns "unmeasured claim" into a gated number.
5. **Workstream E (E1, E2 first)** — SIEM + GDPR erasure are the concrete SOC/DSAR
   onboarding blockers; E3/E4 as the buyer profile demands.
6. **Workstream F** — only once CONDITIONAL → READY is banked. The strategic bet,
   not the compliance sprint.

**Single most important next move (if you pick just one):** B2 — close the
autonomous auto-rollback loop. It is ~90% built, has no operational workaround,
and is the last safety gap on the product's strongest, most-differentiated pillar.
**Cheapest high-value move:** B1 — a deploy-scope check, S-effort, closes a P1
privilege-escalation on the propagation surface.

---

*Planning doc only — no code changed. Findings and evidence live in
`reviews/round-2/`. Nothing here is committed work until you sequence it.*
