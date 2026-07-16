# Round 3 — Subagent 6: Testing Guru

**Scope:** Test-suite *quality* (not count) as the basis for standing behind Reaper's three
load-bearing claims — READY, correct DECISION LOGIC, FAST. UI out of scope. Priority order per
repo map: eval hot path > API surface > distribution/promotion > audit > persistence.

---

## VERDICT: CONDITIONAL

No P0. The correctness core is defended by genuinely good machinery — a compiled/AST **differential
property suite with an independent human-auditable oracle** (`differential_parity_tests.rs`),
matching oracle suites for check-mode and delta-sync, real behavioral BDD, tamper-evidence and
fail-closed tests. That is materially above the industry norm and I could not find a core
authz/audit/isolation invariant with *zero* test. But several P1s block a regulated deploy: the
safety net that measures oracle *adequacy* (mutation testing) is **informational, not gated**; the
DSL **builtins** (jwt/regex/time/comprehension/string/math) sit *outside* the differential oracle
and rely on example tests only; the **CLI** — a documented CI/CD entry point — has **zero active
integration tests**; and the "FAST" claim is defended only in *relative* terms (regression gates),
with **cold-start unmeasured**, **absolute SLA gated only nightly at a 250x-loosened threshold**, and
**p999 not gated by any blocking check**.

---

## Exec summary (≤10 lines)

1. Correctness oracle quality is high **where the oracle reaches** — but it reaches a curated DSL
   subset (attrs, cross-eq, context, type-strict comparison, rebac direct/reachable). jwt/regex/time/
   comprehension/string/math builtins have no differential/property oracle. (P1)
2. Mutation testing exists, is well-scoped to decision logic, but is **explicitly non-blocking**
   (`mutation.yml:69-77`): surviving mutants never fail anything, no tracked budget. The teeth are
   optional. (P1)
3. CLI has **0 active integration tests**; its BDD is a `.backup` stub with empty step bodies. The
   `test --expect deny` exit-code contract customers gate CI on is unverified end-to-end. (P1)
4. "FAST" is defended only as *relative* regression (`perf-gate.yml`, blocking, p99). Absolute SLA
   runs nightly non-blocking at **250x** loosening (`slo-harness.yml:13-16`); **cold-start latency is
   measured nowhere** despite documented `<100ms` / `<10ms` claims. p999 not in any blocking gate. (P1)
5. Coverage is **not measured or gated in CI** (no tarpaulin in `ci.yml`), so uncovered modules are
   unknown/unquantified — contradicting CLAUDE.md's coverage story. (P2)
6. Native clock is **not injectable** (`clock.rs:32` — injection is wasm32-only), so time-based authz
   boundary conditions (jwt exp, is_before/after vs now) can't be pinned deterministically on the
   production target. (P2)
7. Done well: differential+oracle suites, delta-sync≡rebuild, concurrent-hotswap consistency,
   fail-closed staleness gate, hash-chain tamper detection, cross-tenant 404 matrix.

---

## Findings table

| ID | Severity | Location | Finding | Impact | Recommendation |
|---|---|---|---|---|---|
| T1 | P1 | `mutation.yml:69-77` | Mutation testing is informational: exit 2 (missed mutants) tolerated, no threshold, no tracked budget file | The metric that proves the oracle suites have teeth cannot fail a build; oracle rot ships silently | Gate on a missed-mutant budget (ratchet-down), fail nightly when budget regresses |
| T2 | P1 | `differential_parity_tests.rs:165-206`; builtins `reap/ast_evaluator/builtin_functions/{jwt,regex,time}.rs` | Differential+oracle suites cover a curated DSL subset; **builtins (jwt/regex/time/comprehension/string/math) are excluded** from both the oracle and compiled/AST parity | A semantic/compile bug in an authz-relevant builtin (jwt scope, regex on resource) can pass example tests and ship | Extend the generator to builtins, or add per-builtin differential/property suites with an oracle; add negative jwt-verify (forged/expired) tests |
| T3 | P1 | `tools/reaper-cli/tests/cli_bdd_tests.rs.backup` (disabled, empty steps); `tools/reaper-cli/src/*` (6 inline unit tests only) | CLI has no active integration test; no test spawns the binary and asserts exit code/output for `eval`/`test`/`test-suite`/`validate`/`compile` | The CI/CD gating tool customers script against is unverified end-to-end; a wrong `--expect deny` exit code regresses silently | Add an integration test that invokes the built binary over fixtures and asserts exit codes + stdout for allow/deny/error |
| T4 | P1 | `slo-harness.yml:13-16,96`; `perf-gate.yml:198` | Absolute SLA checked only nightly, non-blocking, at 250x loosening; blocking gate is relative p99 only; **cold-start measured nowhere** | The headline "sub-µs / <100ms cold start" claims have no blocking absolute defense; a 2x tail or first-request regression is not caught by a required check | Add a cold-start/first-request bench; gate p999 in the blocking HTTP A/B; tighten nightly multiplier as data accrues |
| T5 | P2 | `.github/workflows/ci.yml` (no tarpaulin); CLAUDE.md "make coverage" | Coverage is neither measured nor gated in CI | Uncovered modules are unknown; the coverage story is aspirational | Run tarpaulin/llvm-cov in CI, publish, gate critical crates on a floor |
| T6 | P2 | `clock.rs:21-41` (`#[cfg(target_arch="wasm32")]` on injection); `builtin_functions/time.rs:15-19` | Clock injection is wasm32-only; native uses `SystemTime::now()` directly. `time.rs` error text even points to an injector that doesn't exist on native | Time-based authz boundary edges (jwt exp, is_before/after vs now) can't be pinned → tested only with far-future constants (`jwt.rs:77`), boundaries untested | Make the injected clock native-available behind a test seam; add expiry-boundary deny tests |
| T7 | P2 | `services/reaper-agent/tests/*`; `decision_buffer.rs:1587` | Buffer drop/fail-closed tested in isolation; no agent-handler test asserts **served-count == recorded-count** end-to-end under load (allow+deny, capture on/off) | "No decision without a record" is proven for the durable-loss latch but not for the hot-path capture invariant under saturation | Add a handler-level test: N mixed requests → assert N decision rows (or N-drop with drop counter) |
| T8 | P2 | `parse_reap.rs:15`, `compile_reap.rs:19` | Fuzz targets assert only "does not panic" (by design); no differential fuzz. Full-grammar equivalence rests on proptest generators that cover a narrower grammar than arbitrary-byte fuzzing | Deep/rare grammar shapes are checked for totality but not for *correctness* | Add a structured/differential fuzz target (arbitrary-derived AST → compiled≡AST) |
| T9 | P3 | `services/reaper-management/tests/integration_tests.rs` (no `billing` test) | Billing subsystem has no integration test (quota does) | Metering regressions ship untested; low authz risk | Add billing integration coverage before GA |

---

## Detailed findings (P1)

### T1 — Mutation testing has no teeth (informational only)
`mutation.yml` is well-designed in scope: it mutates both evaluators, the reap compiler, and the
ReBAC graph (`mutation.yml:52-56`) and re-runs the *high-signal* suites per mutant — the differential
parity, check-mode differential, policy-library golden corpus, compiled-evaluator, comprehension,
rebac, and input-document suites (`:61-67`). That is exactly the right instrument to measure whether
the oracle suites actually constrain behavior.

But it cannot fail:
```
# Exit 2 = missed mutants found (report, don't hard-fail the nightly ...)
if [ "$MUTANTS_EXIT" != "0" ] && [ "$MUTANTS_EXIT" != "2" ]; then ... exit  # mutation.yml:69-77
```
Missed mutants are summarized to `$GITHUB_STEP_SUMMARY` and uploaded as an artifact, never gated.
There is **no tracked missed-mutant budget** anywhere (`find -iname '*mutant*'` → nothing; no
threshold in `.github/` or `docs/`). Consequence: a future change that weakens an oracle (or adds
un-mutation-covered logic) surfaces only if a human reads a nightly summary. For a product that
markets decision correctness, the adequacy metric must ratchet. Also note the scope **excludes**
`data/loader.rs` (delta upsert/delete correctness) and `decision_log.rs`/`decision_buffer.rs` (audit)
— a high mutation score here is a score on evaluators+compiler+rebac only, not on the data or audit
planes. Say that plainly when quoting any mutation number.

### T2 — Builtins are outside the correctness oracle
The differential suite is genuinely strong within its world: `differential_parity_tests.rs` renders
random valid policies, runs **compiled vs AST vs an independent naive oracle** (`:425-448`), and pins
meta-laws (rule-order invariance `:680-686`, deny-monotonicity `:688-694`) plus type-strict
comparison over a mixed-type `badge` (`:512-541`). `check_mode_differential_tests.rs` and
`delta_sync_differential_tests.rs` add the same oracle discipline for the check path and for
delta≡rebuild.

The generator's atom set (`:165-206`) is: string/num attr compare, cross-entity eq, context eq/notnull,
`in` array, badge type-strictness, `rebac::related`, `rebac::reachable`. It does **not** generate
`jwt::*`, `regex::*`, `time::*`, comprehensions, string ops, math, or json builtins. Those are
covered only by example unit tests (e.g. `jwt.rs` has ~3 tests; the visible one decodes claims with a
far-future `exp:4102444800` at `jwt.rs:77` and does not exercise signature-reject or expiry-boundary)
and BDD features (`regex_validation.feature`, `time_based_policies.feature`, `comprehensions.feature`).
Several of these builtins are **AST-only** (no compiled twin), so even the compiled/AST parity net
doesn't cover them. A miscompile or semantic edge in a builtin that feeds an authz decision (jwt
scope check, regex resource match) can pass the curated example set and ship. This is the single most
likely place for a correctness bug to reach production undetected.

### T3 — CLI has no active integration tests
`tools/reaper-cli/tests/` contains only `cli_bdd_tests.rs.backup` (disabled) whose step bodies are
empty stubs (`given/when/then` all `// ... will go here`) and a `cli.feature`. The only live tests are
**6 inline unit tests** split between `src/main.rs` (2) and `src/airgap.rs` (4) — helper-level, not
command-surface. Nothing builds the binary and asserts the exit-code/stdout contract for
`eval`/`test`/`test-suite`/`validate`/`bundle`/`compile`. CLAUDE.md documents `reaper-cli test
--expect <allow|deny>` as the CI/CD integration point; a regression that inverts or drops the non-zero
exit on a failed assertion would break every downstream pipeline gating on it, silently. For a tool
whose entire job is to be a gate, the gate itself is untested.

### T4 — "FAST" is defended relatively, not absolutely; cold-start unmeasured
The relative machinery is excellent: `perf-gate.yml` is **blocking on PRs**, runs a paired A/B on the
**same runner** (base vs head, `:73-91`), has a **self-test that proves a synthetic +15% regression
fails** (`:70-71`), and gates both the engine slice (criterion) and the **served-path HTTP total**
(JSON deser + lookup + cache + audit + serialize, `:115-198`). That genuinely defends against
regressions. Gaps in the *absolute* claim:
- **Absolute SLA is nightly + non-blocking + 250x-loosened.** `slo-harness.yml:13-16` scales every
  `slo.yaml` threshold by `SLO_MULTIPLIER=250` and never blocks PRs (`:20-22`). So "targeted p50 ≤ 2µs"
  is only ever asserted at ≤500µs on a shared runner. That catches order-of-magnitude regressions,
  not the sub-µs claim. Multiplier=1 (the real SLA) is documented as "only meaningful on dedicated
  hardware" and is not run anywhere in CI.
- **p999 is not in any blocking gate.** The blocking HTTP A/B compares `--metric p99_us`
  (`perf-gate.yml:198`) only. `slo.yaml` defines p999 rows (`:28,35,40,45`) but they're checked only by
  the non-blocking 250x nightly. Tail-latency regressions — exactly what bites authz under load — are
  not required-check-gated.
- **Cold-start / first-request latency is measured nowhere.** Every bench and harness warms up
  (`policy_evaluation_bench.rs:84`, `--warmup 1000` in `perf-gate.yml:180`, `--warm-up-time` in
  `ci.yml:1321`). CLAUDE.md claims `<100ms cold start` and `<10ms bundle load` — no test or bench
  produces those numbers. The claim is unfalsified.

---

## Absence checks (module named + what I searched for and did not find)

- **Builtin differential oracle:** searched `differential_parity_tests.rs` / `check_mode_differential_tests.rs`
  atom generators for `jwt`/`regex`/`time`/comprehension — **not present**. No `*builtin*differential*`
  or `*builtin*proptest*` suite exists.
- **jwt signature-reject / expiry-boundary tests:** searched `jwt.rs` for a test asserting a forged or
  expired token is rejected at the boundary — **not found**; the present test decodes a far-future
  token (`jwt.rs:73-84`).
- **CLI end-to-end exit-code test:** searched `tools/reaper-cli/tests` and `src` for a test invoking the
  built binary — **not found** (only 6 helper unit tests; BDD is a disabled empty stub).
- **Cold-start / first-request bench:** searched all `benches/`, `slo.yaml`, and workflows for
  `cold`/`first-request`/first-eval timing — **not found** (all paths warm up).
- **Coverage measurement in CI:** searched `ci.yml` for `tarpaulin`/`llvm-cov`/`codecov` — **not found**
  (only unrelated "coverage" mentions).
- **Native clock injection:** searched `clock.rs` — injection API is `#[cfg(target_arch="wasm32")]`
  only (`:25-41`); no native test seam.
- **Served==recorded invariant at the agent handler:** searched agent `tests/` for a test asserting the
  count of served decisions equals recorded rows under load — **not found** (buffer drop is tested in
  isolation at `decision_buffer.rs:1587`; durable-loss fail-closed latch is tested).
- **Mutation budget/threshold file:** `find -iname '*mutant*'` and grep for a score threshold — **not
  found**; mutation is purely informational.
- **Billing integration test:** searched management `tests/*.rs` for `billing` — **not found** (quota is
  covered at `integration_tests.rs` `quota_enforcement_...`).

**Positive absence-check results (invariants I looked for AND found):**
tenant isolation (`integration_tests.rs:2377,2481,3020,3368,3642` — cross-tenant bundle/policy/CR/scim/
audit → 404), hash-chain tamper-evidence incl. dropped-entry and hidden-tail-drop
(`decision_log.rs:1666,2118,2133`), delta≡rebuild (`delta_sync_differential_tests.rs`,
`integration_tests.rs` `migration_apply_atomic_and_delta_equals_rebuild`), concurrent hot-swap
consistency with both allow+deny observed mid-swap (`concurrent_hotswap_tests.rs:224-225,235`),
fail-closed cold-start & staleness (`failure_modes_tests.rs:33,54`), buffer drop-oldest exact oracle
(`decision_buffer.rs:1607-1615`).

---

## Claims-we-can-stand-behind table

| Claim | Evidence (tests/gates) | Confidence | What's missing to reach High |
|---|---|---|---|
| Correct ALLOW | differential+oracle (`differential_parity_tests.rs`), BDD rbac/abac/multilayer positives, compiled-evaluator suite | **High** (curated DSL) / **Med** (builtins) | Builtin differential oracle (T2) |
| Correct DENY | oracle deny-phase + deny-monotonicity law (`:435,688-694`), BDD negative scenarios, check-mode violation-set oracle | **High** (curated DSL) / **Med** (builtins) | Builtin & jwt/regex deny paths (T2) |
| Tenant isolation | cross-tenant 404 matrix across bundle/policy/CR/scim/audit (`integration_tests.rs`) | **High** | — (per-org rate-limit + storage isolation also asserted) |
| Audit completeness (no decision unlogged) | durable-loss fail-closed latch; buffer drop counters | **Med** | Handler-level served==recorded under load (T7) |
| Audit tamper-evidence | signed-checkpoint hash chain detects tamper/drop/tail-drop/forgery (`decision_log.rs:1666-2133`) | **High** | — |
| Hot-swap correctness under load | multi-thread swap, consistency, no torn reads (`concurrent_hotswap_tests.rs`) | **High** | — |
| < Xµs eval (absolute) | criterion benches; nightly SLO at 250x | **Low** | Absolute gate on isolated hardware; tighten multiplier (T4) |
| > Yk rps | slo-harness *reports* achieved rps, target not asserted (`slo-harness.yml:17-19`) | **Low** | A load gate that asserts throughput (T4) |
| No perf regression (relative) | blocking paired A/B, self-tested +15% (`perf-gate.yml`) | **High** (p99) / **Low** (p999) | p999 in blocking gate (T4) |
| Cold-start < 100ms / bundle < 10ms | none | **None** | Add a cold-start bench (T4) |
| Delta-sync ≡ full rebuild | property differential + management migration test | **High** | — |

---

## Test-pyramid map (rough proportions)

```
                 e2e (2 files, tests/e2e; run-e2e-tests.sh)           ~1%   THIN but real
        integration (agent 11, mgmt 3 incl. 104-test integration_tests.rs,
                     engine ~40, sdk/wasm/sync/core)                  ~30%   STRONG on mgmt+engine
   property/differential+oracle (parity, check-mode, delta-sync)      small but HIGHEST-value tier
              BDD (20 .feature files, real allow/deny oracles)        ~15%
        unit / #[cfg(test)] inline (dominant mass across all crates)  ~55%
                 fuzz (2 targets, PR-smoke + nightly) — totality only
                 mutation (nightly, informational) — adequacy, ungated
```

**Biggest inversion/gap:** the pyramid is *healthy* for the engine and control plane (the 62.9k-LOC
management crate is well covered by inline tests + a 104-test `integration_tests.rs` spanning
migrations, promotions, dual-control, freeze, quota, scim/sso, revocation, tenant isolation — the repo
map's "only 3 tests/ files" undersells it). The real hole is at the **edges**: the CLI (T3, zero active
integration) and the **absolute-perf / cold-start** rung (T4, no gate). And the adequacy layer
(mutation) that would police the whole pyramid is switched to advisory (T1).

---

## Top 5 "would ship broken silently today"

1. **A miscompiled/misbehaving authz builtin.** A `jwt::verify(...).scope` or `regex::matches(resource)`
   used in a deny rule returns the wrong boolean. *Missing test:* a builtin differential/property suite
   (arbitrary jwt/regex/time inputs → oracle) + jwt forged/expired-boundary deny tests. Today: example
   tests + AST-only path, no parity, no oracle. (T2)
2. **A weakened oracle that stops constraining the evaluator.** Someone edits an evaluator + loosens a
   test; mutation would flag it but nightly mutation is informational. *Missing test:* a gated
   missed-mutant budget. (T1)
3. **A CLI exit-code regression.** `reaper-cli test --expect deny` returns 0 on a failed assertion,
   silently greening customer pipelines. *Missing test:* an integration test spawning the binary and
   asserting exit codes/stdout for allow/deny/error. (T3)
4. **A cold-start / first-request latency blow-up.** A new per-startup allocation or lazy-compile stall
   pushes first-request latency past the documented budget. *Missing test:* a cold-start bench
   (unwarmed first-eval / bundle-load timing) with a threshold. (T4)
5. **A p999 tail regression under load.** A new per-request allocation doubles p999 while p99 stays flat;
   the blocking gate is p99-only. *Missing test:* p999 in the blocking HTTP A/B gate (thresholds
   already exist in `slo.yaml`). (T4)

---

## What's done well (≤5)

1. **Differential + independent-oracle property testing** (`differential_parity_tests.rs`,
   `check_mode_differential_tests.rs`, `delta_sync_differential_tests.rs`) — three implementations
   cross-checked, meta-laws pinned, delta≡rebuild proven. This is the strongest correctness asset in
   the repo and above industry norm.
2. **Blocking paired-A/B perf gate with a self-test** (`perf-gate.yml`) — cancels runner variance by
   construction and continuously verifies its own sensitivity; gates the served path, not just the
   micro-bench.
3. **Audit tamper-evidence is really tested** — signed hash-chain detects tamper, dropped entry, hidden
   tail-drop, forged signature, and enforces durable-loss fail-closed (`decision_log.rs`,
   `decision_buffer.rs`).
4. **Fail-closed & concurrency invariants** — cold-start and staleness both deny
   (`failure_modes_tests.rs`); hot-swap under many threads observes both allow and deny with no torn
   state (`concurrent_hotswap_tests.rs`).
5. **Control-plane integration breadth** — `integration_tests.rs` (104 async tests) exercises
   migrations/idempotency/rollback, promotion/dual-control/freeze/ServiceNow, quota, scim/sso,
   revocation, and cross-tenant isolation with concrete oracles.

---

## What I did NOT cover

- Did not execute any test/bench (static review of test source, CI YAML, and code-under-test only).
- Did not audit Cedar-path evaluator test quality beyond noting `cedar_integration_tests.rs` exists.
- Did not deep-read every one of the 20 `.feature` files or all 104 management integration tests —
  sampled rbac.feature (real allow/deny oracles) and the management test-name index.
- Did not assess eBPF, WASM parity, or SDK test depth beyond the map.
- Flake analysis is inferential (SSE timing, `sleep`-as-sync in `slo-harness.yml`/`perf-gate.yml`
  health polls, DashMap ordering) — I did not run the suite repeatedly to quantify flake rates.
- Coverage percentages are unquantifiable because CI does not measure coverage (T5).
