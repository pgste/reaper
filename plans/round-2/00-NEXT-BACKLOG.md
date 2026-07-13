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

## Workstream B — Propagation surface safety  *(the strongest pillar's two holes)*

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

## Workstream C — Data-plane API hardening

- **C1 — Paginate the ABAC/ReBAC list endpoints.** *Closes CODE R2-01.* Entity /
  role-binding / tuple lists return everything with no `LIMIT` — the biggest
  tables. Route through the existing `PageQuery`/`Paginated` keyset (as `policies`
  already does). **Effort S.** Availability foot-gun; cheap fix.
- **C2 — Turn optimistic concurrency from dormant to enforced.** *Closes CODE
  R2-02, R2-03.* `require_if_match` defaults false (warn-only); policy ETag is
  content-hash so metadata edits silently lose updates. Flip the GA default to
  true; derive ETag from a row-version/`updated_at` every write bumps. **Effort
  S.**
- **C3 — Version-guard + idempotency on `apply_migration`.** *Closes CODE R2-04,
  R2-05.* Plan 12's migration apply has no `model_version` optimistic guard and no
  Idempotency-Key — concurrent migrations clobber, retried timeouts double-apply +
  double-propagate. Add the guard (the plan already carries `model_before`) and
  wrap in `idempotency::run`. **Effort S.**
- **C4 — Typed DTOs + error model on the contract.** *Closes CODE R2-06, R2-08.*
  Datastore/decisions/replay/audit handlers return untyped `Json<Value>`;
  `ProblemDetails` isn't `ToSchema` and omits `instance`. Add DTOs + schema-lint so
  the published contract is client-codegen-clean. **Effort M.**

---

## Workstream D — Measured SLO + eval finishing

- **D1 — Build the deferred load harness and gate the request-total SLO.** *Closes
  PERF R2-P1-1.* The headline sub-µs SLA is never measured on the real HTTP path;
  the perf gate benches only 2 in-process micro-suites. Extend the reaper-vs-opa
  HDR harness to the N×M points, assert request-total p99/p999 in CI. **Effort M.**
  Without this, "READY" rests on unmeasured assumptions.
- **D2 — Make the DSL prunable (wire partial evaluation).** *Closes PERF R2-P2-1.*
  The pruning index only prunes the *deprecated* Simple language; DSL evaluate-all
  mass-denies past 256 policies or degrades to O(N log N)/req. `partial_evaluation.rs`
  — the exact fix — is in-tree but dead. **Effort M.**
- **D3 — Observe request-total latency on every return path.** *Closes PERF
  R2-P2-3.* Denies (stale-data, policy-not-found, cap-exceeded) skip the SLA
  histogram, so denial storms go invisible on the dashboard. **Effort S.**
- **D4 — Reconcile or delete the parallel engines / dead arena.** *Closes PERF
  R2-P3-1/2/3.* `arena.rs`, `indexed_engine.rs`, `optimized_engine.rs` are
  bench-only; `SmallVec` on the targeted single-policy path was claimed but not
  shipped. Wire partial-eval (feeds D2) or delete the trio; stop carrying three
  engines. **Effort S–M.** Pairs naturally with D2.

---

## Workstream E — Enterprise / compliance finishing

- **E1 — Native SIEM export connectors.** *Closes PROD R2-2.* NDJSON/JSON only;
  no Kafka/Splunk-HEC/CEF/OCSF. Ship Vector sink configs + an OCSF field mapping +
  a push export API. **Effort M.** Bank SOC onboarding blocker.
- **E2 — GDPR subject-erasure endpoint.** *Closes PROD R2-3.* No erase-by-subject
  anywhere. Build erase over ClickHouse + DataStore with a legal-hold guard.
  **Effort M.** UK DPA-2018 DSAR gap.
- **E3 — Signed air-gap export/import.** *Closes PROD R2-4, SEC (air-gap half of
  R2-10 round-1).* `compile` doesn't sign; CLI deploy sends no signature. Add
  `bundle export --sign` / `import --verify` + agent checksum report. **Effort M.**
- **E4 — Real multi-tenant quota enforcement.** *Closes PROD R2-9.* Plan-limit
  quotas are advisory (`UsageMetrics` hardcoded 0). Wire real usage counts +
  enforce at agent-register / policy-create + per-tenant rate ceilings.
  **Effort M.** Only matters for the multi-tenant SaaS shape; single-tenant
  enterprise can defer.

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
