# Verification & Gates — Every Load-Bearing Claim Machine-Defended

> **Planning artifact only — no product code modified.** Round-3 remediation.
> Base: current `main`. This plan owns the *test harnesses and CI gates*; where a
> behavior is owned by a sibling round-3 plan it says so and takes only the
> machine-check.

**Readiness gate:** Blocks CONDITIONAL → READY. The correctness core is genuinely
strong, but the instrument that proves the oracle has teeth (mutation) is
advisory, the DSL builtins sit outside the oracle, the CLI (the customer CI/CD
entry point) has zero active integration tests, and "FAST" is defended only
*relatively* (p99, warm) — cold-start unmeasured, absolute SLA nightly-only at
250× loosening, p999 in no blocking gate. Several memory-invariant suites are
dead in CI. Every one of these is a *load-bearing product claim we cannot fully
stand behind*.
**Priority:** P1. Theme: make every claim on the "claims-we-can-stand-behind"
table (ready / correct-logic / fast) defended by a **named blocking** check so it
cannot regress silently.
**Findings closed:** Testing T1, T2, T3, T4, T5, T7, T8; Code/API R3-03;
Evolutionary E-06 (dead interner suites). Coordinates with round-3 plans 01
(tenant fitness function), 02 (C4 CLI build/release), 04 (DSL/builtins semantics).

---

## 1. Goal

Reproduce the review's **claims-we-can-stand-behind** table
(`reviews/round-3/06-testing.md:173-188`) with **every row at Confidence = High**,
each backed by a named blocking CI gate:

1. Promote **mutation testing** from advisory to blocking with a ratcheting
   surviving-mutant budget, and widen scope beyond evaluators/compiler to the
   **audit** and **tenant-isolation** planes.
2. Bring the **DSL builtins** (jwt/regex/time/comprehension/string/math) under a
   differential/property oracle — own the harness; semantics owned by plan 04.
3. Add a **tenant-isolation behavioral corpus** (A cannot read/mutate B's
   policies/data/decisions/webhooks) as a blocking regression suite.
4. Add **audit-invariant** tests: hash-chain tamper → detected; "no decision
   without a record" at the handler under saturation; bounded-buffer drop /
   backpressure.
5. Give **reaper-cli** an integration suite (exit-code + output contract) and
   ensure it builds on PR CI; re-enable/replace the disabled empty BDD stub.
6. Promote **absolute-SLA** and **cold-start** latency to blocking gates; gate
   **p999** (not just p99) on the served path; verify benches use representative
   policy/graph sizes.
7. Prove a **full-journey e2e** (create org → policy → data → deploy → promote →
   eval) exercises end to end in CI.
8. Wire the **three dead interner-bounding suites** into CI.
9. **Measure coverage** (tarpaulin) and set a floor on critical modules.

## 2. Current state (evidence) — file:line

- **Mutation is informational, cannot fail a build.** `mutation.yml:69-77` treats
  exit 2 (missed mutants found) as success; missed mutants only land in the step
  summary + artifact. No tracked budget file exists (`find -iname '*mutant*'` →
  nothing). Scope (`mutation.yml:52-56`) is evaluators + `reap/ast_evaluator/**` +
  `reap/compiler/**` + `data/relationships.rs` — it **excludes** `data/loader.rs`
  (delta upsert/delete), `decision_log.rs`, `decision_buffer.rs` (audit). A quoted
  mutation score is a score on evaluators+compiler+rebac only (Testing T1).
- **Builtins are outside the correctness oracle.** The generator atom set in
  `differential_parity_tests.rs:165-206` is attr/context/`in`/badge/`rebac::*`
  only — no `jwt::*`/`regex::*`/`time::*`/comprehension/string/math. Those are
  AST-only in places and covered by example tests: `jwt.rs:77` decodes a
  far-future `exp:4102444800` and never exercises signature-reject or the
  expiry boundary. A miscompiled authz builtin passes the curated set and ships
  (Testing T2 — "single most likely place for a correctness bug to reach
  production undetected").
- **CLI has zero active integration tests.** `tools/reaper-cli/tests/` holds only
  `cli_bdd_tests.rs.backup` (disabled; step bodies are empty `// ... will go here`
  stubs) + a `features/cli.feature`; the only live tests are 6 inline `#[cfg(test)]`
  units in `src/`. Nothing spawns the binary to assert the `test --expect deny`
  exit-code contract customers gate CI on (Testing T3; Code R3-03). The CLI *is*
  cross-target built in `release.yml:192`, but is not exercised on PR CI.
- **"FAST" defended only relatively.** `perf-gate.yml` is blocking, paired A/B,
  self-tested (+15% fails, `:70-71,138-139`), but the served-path HTTP gate
  compares `--metric p99_us` only (`perf-gate.yml:198`). Absolute SLA runs
  nightly, non-blocking, at `SLO_MULTIPLIER=250` and concurrency 4
  (`slo-harness.yml:9-19,96,121`) — "targeted p50 ≤ 2µs" is only ever asserted at
  ≤ 500µs. `slo.yaml` defines p999 rows but they are checked only by the 250×
  nightly. **Cold-start / first-request latency is measured nowhere** — every
  bench and harness warms up (`perf-gate.yml:180 --warmup 1000`); CLAUDE.md's
  `<100ms cold start` / `<10ms bundle load` are unfalsified (Testing T4).
- **Served == recorded not asserted at the handler.** Buffer drop-oldest is tested
  in isolation (`decision_buffer.rs:1587,1607-1615`) and the durable-loss
  fail-closed latch is tested, but no agent-handler test asserts N served
  decisions ⇒ N recorded rows (or N − drops) under load with capture on/off
  (Testing T7).
- **Three interner-bounding suites are dead in CI.** `ci.yml` runs `--workspace
  --lib` + explicitly named `--test` targets; only `context_interner_leak_tests`
  is named (`ci.yml:309`). `eval_interner_bounding_tests.rs`,
  `rebac_interner_bounding_tests.rs`, `migration_rename_interner_tests.rs` exist
  under `crates/policy-engine/tests/` but are **never executed** — the memory
  invariants behind the "~60% memory reduction" claim are advisory (Evolutionary
  E-06).
- **Coverage neither measured nor gated.** No `tarpaulin`/`llvm-cov`/`codecov` in
  `ci.yml`; uncovered modules are unknown, contradicting CLAUDE.md's coverage
  story (Testing T5).
- **Fuzz is totality-only.** `parse_reap.rs:15`, `compile_reap.rs:19` assert only
  "does not panic"; no differential (compiled ≡ AST) fuzz target (Testing T8).
- **e2e is thin but real.** `tests/e2e/tests/data_plane_e2e.rs` + `e2e_tests.rs`
  run via `reaper-e2e-tests` (`ci.yml:1080`); no single test walks
  org→policy→data→deploy→promote→eval end to end (Testing pyramid map).

## 3. Definition of Done — testable checkboxes

Reproduce `06-testing.md:173-188` with **every row High**, each backed by a named
gate. Row → gate mapping:

| Claim (table row) | New/promoted **blocking** gate |
|---|---|
| Correct ALLOW / DENY (builtins) | `builtin_differential_tests` in CI + in mutation scope (§4.2) |
| Tenant isolation | `tenant_isolation_tests` behavioral corpus, blocking (§4.3) |
| Audit completeness (no decision unlogged) | `audit_invariant_tests` served==recorded under saturation (§4.4) |
| Audit tamper-evidence | audit suites added to mutation scope (§4.1) |
| < Xµs eval (absolute) + > Yk rps | `slo-harness` promoted to blocking absolute at tightened multiplier + throughput assert (§4.6) |
| Cold-start < 100ms / bundle < 10ms | `cold_start_bench` with threshold, blocking (§4.6) |
| No perf regression (p999) | p999 added to blocking HTTP A/B (§4.6) |
| Delta-sync ≡ rebuild / memory-bound | 3 interner suites wired (§4.8) |

- [ ] `mutation.yml` **fails** when surviving-mutant count exceeds a committed,
      ratcheting budget file; scope includes `decision_log.rs`, `decision_buffer.rs`,
      `data/loader.rs`, and the tenant-scoping authz path. A hand-introduced missed
      mutant fails the run; the budget only ratchets down.
- [ ] A `builtin_differential_tests` suite runs each authz-relevant builtin
      (jwt/regex/time/comprehension/string/math) compiled-vs-AST-vs-oracle over
      generated inputs, incl. jwt forged-signature and expiry-boundary **deny**
      cases; it is named in CI and in mutation scope.
- [ ] `tenant_isolation_tests` proves tenant A gets 404/deny on B's
      policies/data/decisions/webhooks/CRs across every read+mutate verb; blocking.
- [ ] `audit_invariant_tests`: (a) hash-chain tamper/drop/tail-drop/forgery →
      detected; (b) handler-level N mixed requests ⇒ N recorded (or N − drop with
      drop counter) under saturation, capture on/off; (c) bounded buffer applies
      backpressure/drop-with-counter, never silent loss.
- [ ] `reaper-cli` integration suite (`assert_cmd`/`trycmd`) asserts exit codes +
      stdout for `eval`/`test`/`test-suite`/`validate`/`compile`/`bundle` on
      allow/deny/error fixtures; `cargo build -p reaper-cli` runs on PR CI; the
      empty BDD stub is re-enabled or deleted.
- [ ] `slo-harness` runs a **blocking** absolute job at a tightened multiplier on
      the served path incl. **p999**; the HTTP A/B gate also compares `p999_us`; a
      throughput floor is asserted (not just reported).
- [ ] `cold_start_bench` measures unwarmed first-request eval and bundle-load
      time with a committed threshold; blocking.
- [ ] A `full_journey_e2e` test drives create org → create policy → load data →
      deploy → promote → eval and asserts the final decision; named in CI.
- [ ] `eval_interner_bounding_tests`, `rebac_interner_bounding_tests`,
      `migration_rename_interner_tests` are named in the `integration-tests` job.
- [ ] `cargo tarpaulin` runs in CI, publishes coverage, and **fails** below a
      floor on critical modules (engine evaluators, decision log/buffer, tenant
      scoping, data loader).

## 4. Critical steps — ordered; what / where / verify

1. **Promote mutation to a blocking, ratcheting gate + widen scope.** Add a
   committed budget file (e.g. `mutation-budget.json` keyed by shard) recording the
   allowed missed-mutant count; change `mutation.yml:69-77` so exit 2 fails when
   missed > budget. Extend `-f` globs (`:52-56`) to `data/loader.rs`,
   `decision_log.rs`, `decision_buffer.rs`, and the management tenant-scoping
   module. Add the new audit + tenant suites (§4.3, §4.4) to the per-mutant
   `--test` list (`:61-67`).
   *Where:* `.github/workflows/mutation.yml`, new `mutation-budget.json`.
   *Verify:* a hand-loosened oracle leaves a surviving mutant → CI red; budget only
   ratchets down (a PR that raises it is flagged in review).

2. **Builtin differential/property oracle (own harness; semantics = plan 04).**
   Extend the parity generator (`differential_parity_tests.rs:165-206`) or add a
   sibling `builtin_differential_tests.rs`: generate arbitrary jwt/regex/time/
   comprehension/string/math expressions, evaluate compiled-vs-AST-vs an
   independent naive oracle, incl. negative jwt (forged sig, `exp` at/just-past
   `now`) and regex-on-resource deny cases. Requires the injectable clock made
   native-available (Testing T6, `clock.rs:21-41`) — coordinate with plan 04.
   *Where:* `crates/policy-engine/tests/builtin_differential_tests.rs`; `ci.yml`
   integration-tests job; mutation scope.
   *Verify:* mutating a builtin dispatch arm now produces a caught mutant; forged
   jwt test denies.

3. **Tenant-isolation behavioral corpus (blocking).** Consolidate/extend the
   existing cross-tenant 404 assertions (`integration_tests.rs:2377,2481,3020,
   3368,3642`) into a named `tenant_isolation_tests` target that sweeps every
   read+mutate verb for policies/data/decisions/webhooks/change-requests, asserting
   A→B is 404/deny. Plan 01 owns the architectural fitness function (no handler
   without a tenant scope); this plan owns the behavioral corpus it protects.
   *Where:* `services/reaper-management/tests/tenant_isolation_tests.rs`; `ci.yml`.
   *Verify:* removing a `WHERE org_id = $` clause fails a corpus case.

4. **Audit-invariant suite.** (a) Reuse the tamper tests (`decision_log.rs:1666,
   2118,2133`) under one named target. (b) Add an agent-handler test: fire N mixed
   allow/deny requests, assert recorded rows == served count (or == served − drop
   counter) with capture on and off, under saturation (fill the ring buffer). (c)
   Assert bounded-buffer backpressure/drop increments a counter and never loses
   silently (`decision_buffer.rs:1587`).
   *Where:* `services/reaper-agent/tests/audit_invariant_tests.rs`; `ci.yml`;
   mutation scope (§4.1).
   *Verify:* dropping the record call, or a buffer that silently overwrites, fails
   a case.

5. **reaper-cli integration suite + build/release.** Add `assert_cmd`/`trycmd`
   dev-deps; a `tests/cli_integration.rs` spawns the built binary over fixtures and
   asserts exit codes + stdout for each subcommand's allow/deny/error path
   (especially `test --expect deny` non-zero exit). Re-enable or delete
   `cli_bdd_tests.rs.backup`. Add `cargo build -p reaper-cli` (and run the suite)
   to PR CI. Coordinate with plan 02 C4, which owns making the CLI a first-class
   release artifact (`release.yml:192` already cross-builds it).
   *Where:* `tools/reaper-cli/tests/cli_integration.rs`, `tools/reaper-cli/Cargo.toml`;
   `ci.yml`.
   *Verify:* inverting the `--expect` exit logic fails a test.

6. **Absolute-SLA + cold-start + p999 blocking gates.** (a) Add a blocking
   `slo-harness` job (or promote the nightly) that asserts the served path at a
   *tightened* multiplier and includes the p999 rows already in `slo.yaml`; document
   the multiplier as the CI floor, not the SLA. (b) Add `--metric p999_us` to the
   blocking HTTP A/B (`perf-gate.yml:193-198`). (c) Add `cold_start_bench` measuring
   unwarmed first-request eval + bundle load, with a committed threshold. Confirm
   benches use N=10k policies / representative ReBAC graph, not toy inputs
   (`slo-harness.yml:86-91` generates 10k; verify the cold-start and p999 paths do
   too).
   *Where:* `slo-harness.yml`, `perf-gate.yml`, `crates/policy-engine/benches/`,
   `scripts/perf_ab_gate.py`.
   *Verify:* a synthetic first-request stall / p999 doubling fails CI; p99 stays flat.

7. **Full-journey e2e.** Extend `tests/e2e/tests/data_plane_e2e.rs` (or add
   `full_journey_e2e.rs`) to walk org create → policy create → data load → deploy →
   promote → eval, asserting the final decision reflects the promoted policy +
   pinned data. Keep it in the `reaper-e2e-tests` CI job (`ci.yml:1080`).
   *Verify:* breaking any stage (e.g. promotion not applied) fails the final assert.

8. **Wire the three dead interner suites.** Add `--test eval_interner_bounding_tests`,
   `--test rebac_interner_bounding_tests`, `--test migration_rename_interner_tests`
   to the `integration-tests` job (`ci.yml:1061-1094`), alongside the already-wired
   `context_interner_leak_tests`/`pruning_index_tests`.
   *Verify:* introducing an unbounded interner insert on a churn path fails a suite.

9. **Coverage measurement + floor.** Add a `cargo tarpaulin --workspace --out Xml`
   CI job; publish; fail below a per-module floor for engine evaluators,
   `decision_log.rs`/`decision_buffer.rs`, tenant scoping, `data/loader.rs`. Start
   the floor at *measured* current coverage and ratchet up (never down).
   *Where:* `ci.yml`, `tarpaulin.toml`.
   *Verify:* deleting a covered module's tests drops coverage below floor → red.

## 5. Dependencies

- **Plan 04 (DSL/builtins):** §4.2's oracle needs the native-injectable clock
  (`clock.rs`) and canonical builtin semantics plan 04 defines. Build the harness
  here; import the semantics from there. Land together.
- **Plan 01 (tenant fitness function):** §4.3's behavioral corpus complements plan
  01's structural "no un-scoped handler" assertion — they defend the same invariant
  from behavior and structure. Don't duplicate the scoping rule; cross-link.
- **Plan 02 (C4, CLI build/release):** §4.5 owns CLI *tests*; plan 02 owns the CLI
  as a *released, buildable* artifact. Share the `cargo build -p reaper-cli` CI step.
- **Infra:** §4.6's tightened absolute gate is more trustworthy on a
  dedicated/self-hosted runner (same dependency plan 08 Phase F noted); until then
  it uses a documented multiplier > 1 but < 250 and the paired A/B for p999.
- **§4.1 mutation scope** depends on §4.3/§4.4 suites existing first (they are the
  per-mutant test targets that give audit/tenant mutants something to be caught by).

## 6. Testing & verification

- **Gate self-tests (meta):** every new blocking gate ships with a proof it can
  fail — a hand-introduced missed mutant (§4.1), a loosened oracle (§4.2), a
  dropped `WHERE org_id` (§4.3), a dropped record call (§4.4), an inverted CLI exit
  (§4.5), a synthetic p999/cold-start regression (§4.6), a broken promotion (§4.7),
  an unbounded interner insert (§4.8). This mirrors `perf-gate.yml`'s existing
  `--self-test` discipline (`perf-gate.yml:70-71`).
- **Representative inputs:** all latency/throughput gates run at N=10k policies and
  a real 10k ReBAC graph (`slo-harness.yml:86-91,147`), never single-rule toys.
- **No-flake budget:** SSE/`sleep`-as-sync health polls in the harness (Testing
  "What I did NOT cover") are replaced with readiness polling in the new e2e/cli
  jobs to avoid timing flakes.

## 7. Effort & phasing — S/M/L

- **Phase A (M):** §4.5 CLI integration suite + §4.8 wire dead interner suites —
  highest leverage, lowest risk, restores two named-but-broken guarantees. Quick win.
- **Phase B (M):** §4.3 tenant corpus + §4.4 audit invariants (both feed C).
- **Phase C (M):** §4.1 promote+widen mutation (depends on B's suites).
- **Phase D (M–L):** §4.2 builtin oracle (depends on plan 04 clock/semantics).
- **Phase E (M, infra-gated):** §4.6 absolute-SLA/cold-start/p999 gates.
- **Phase F (S):** §4.7 full-journey e2e + §4.9 coverage floor.

## 8. Key decisions (ADR-style)

- **ADR-1: Mutation testing becomes a ratcheting *budget*, not a hard 0.** A
  green-field "0 surviving mutants" is unrealistic and would block on unreachable
  arms; a committed budget that can only ratchet down makes oracle rot fail the
  build while allowing pragmatic exceptions with a documented, dated entry.
  Consequence: the budget file is reviewed like a lockfile.
- **ADR-2: The builtin oracle owns the harness, plan 04 owns the semantics.** The
  differential engine (compiled-vs-AST-vs-naive) is a testing asset; the canonical
  jwt/regex/time behavior is a language decision. Split so neither plan blocks the
  other's merge beyond the shared clock seam.
- **ADR-3: Absolute latency is gated at a documented CI *floor multiplier*, with
  multiplier 1.0 reserved for dedicated hardware.** A shared-runner absolute p50 ≤
  2µs is physically impossible (TCP loopback floor ≈ 100×); gating at a tightened
  multiplier catches order-of-magnitude and tail regressions honestly, and the
  paired A/B (variance-cancelled) carries the sub-µs relative defense incl. p999.
  Consequence: the marketing "sub-µs" number stays scoped to the engine slice.
- **ADR-4: Cold-start is a first-class, separately-gated number.** Every existing
  path warms up, so the documented `<100ms`/`<10ms` claims are unfalsified. A
  dedicated unwarmed bench is the only way to defend them. Consequence: one new
  bench that must not accidentally get a warmup.
- **ADR-5: Behavioral corpus + structural fitness function are both kept.** Tenant
  isolation is defended by structure (plan 01: no un-scoped handler) *and* behavior
  (this plan: A→B is 404). Redundant by design — a bug that slips one is caught by
  the other. Consequence: two suites reference one invariant; cross-linked.

## 9. Risks & rollback

- **Risk: mutation gate is flaky/slow → developers route around it.** Mitigation:
  keep it nightly (not per-PR) but *failing*, sharded 4× with per-shard budgets and
  a 120s per-mutant timeout as today; only the budget comparison is new. Rollback:
  revert budget file, gate returns to advisory.
- **Risk: builtin oracle encodes a *wrong* canonical semantics → pins a bug.**
  Mitigation: the independent naive oracle is written from the spec (plan 04), not
  from the evaluator; disagreement between all three is a test failure, not a pin.
- **Risk: absolute SLA gate is noisy on shared runners → false reds.** Mitigation:
  tightened-but->1 multiplier + p999 via the paired A/B (variance-cancelled by
  construction), not raw absolute; provision a dedicated runner to tighten further.
  Rollback: relax multiplier (documented, dated), keep p999 A/B blocking.
- **Risk: CLI integration tests are slow (spawn per case).** Mitigation:
  `trycmd` snapshot batching + fixtures; run in its own PR job in parallel.
- **Risk: coverage floor becomes a merge nuisance.** Mitigation: floor starts at
  measured current, ratchets only up, excludes generated/bench code; a drop is the
  signal, not an absolute target. Rollback: report-only until the number stabilizes.
- **General rollback:** every gate here is additive and independently revertible to
  advisory; none changes product behavior, so a bad gate is a one-line workflow
  revert, not a code rollback.
