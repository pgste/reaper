# PR #3 — Reviewer's Guide

89 commits, ~357 files. This guide groups the work into independent arcs so you
can review one coherent slice at a time and know which automated net backs each.
It is descriptive, not a substitute for reading the diff — but it tells you where
the risk is and what to trust.

## How to review this efficiently

1. **Trust the differential nets, then spot-check.** The correctness-critical
   arcs (DSL evaluator, delta sync) are pinned by *differential* suites that
   assert two independent implementations agree, or that an incremental result
   equals a from-scratch rebuild. If those pass, the surface area you must read
   by hand shrinks a lot. The three to know:
   - `compiled_ast_equivalence_tests` — compiled evaluator ≡ AST evaluator for
     every DSL function.
   - `delta_sync_differential_tests` — delta-applied store ≡ freshly-rebuilt
     store.
   - `differential_parity_tests` / `check_mode_differential_tests` — oracle
     parity for the policy library.
2. **Review in arc order below** (roughly dependency order). Each arc is
   self-contained; you can stop after any of them.
3. **The one consolidated CI comment** on the PR is the live status — one
   pass/fail with per-suite counts and performance numbers. Green = 1047 tests +
   Docker images build.

## Arcs

### 1. Baseline security & hot-path perf (the original review)
Where the branch started: a prioritized security+perf review and its first
remediations.
- **Commits:** `e9356c4` (review doc), `6c339a6` (decision-cache correctness),
  `57baacb` (parity-gated OPA benchmark), `aa9ec47` (default-deny semantics),
  `9b19d40` (allocator/LTO/off-thread logging), `7a75b5f` + `72a0c38` (Phase 1
  security: tenant isolation, OAuth token storage, CSRF, rate limiting, JWKS
  SSRF/aud, mTLS, secret enforcement), `52ca81a` (UDS socket perms).
- **Read for:** the security fixes are the highest-value manual review here —
  tenant isolation and token storage especially. Small, self-contained diffs.

### 2. Compiled-evaluator performance
Tightening the sub-microsecond path.
- **Commits:** `7b39079`, `6f04880`, `159ca28` (borrow interned values, drop
  per-eval resolves), `80d931f`, `0457489` (agent handler: fast rng ids, cached
  metric handles), throughput harnesses (`6445829`, `fda976f`, `2069841`,
  `b918dce`, `d80e19c`).
- **Net:** `compiled_ast_equivalence_tests` + criterion benches. Perf-only;
  decisions unchanged.

### 3. DSL v2 — RBAC + ABAC + ReBAC + document policies
The policy language and both evaluators.
- **Commits:** `c1ab8aa` (design), `d2698ec` (structured `input` + check mode),
  `3e9f4d5` (`/api/v1/check`), `b18b154` (first-class ReBAC graph + bounded
  traversal in BOTH evaluators), `8860548` (OPA-derived policy library),
  `f4d274c` (JWT/auth-artifact interpretation, CLI), `9f97c0c` + `28b598c`
  (type-strict contract, comprehension totality, mutation/perf/release gates),
  `fc60704` (mutation-kill tests).
- **Key files:** `crates/policy-engine/src/evaluators/reaper_dsl/**`,
  `src/reap/ast_evaluator/**`.
- **Net:** `compiled_ast_equivalence_tests` (37), `differential_parity_tests`,
  `check_mode_differential_tests`. **This is the correctness core** — the
  differential suites are what make it reviewable.

### 4. Decision logging pipeline
OPA-style audit off the hot path.
- **Commits:** `8e6d430` (design), `6858bd5`, `61db6d3`, `4572c7e`, `5959f9b`
  (sharded ring capture, 4.9x concurrent), `5188233` (masking/pseudonymization/
  AES-256-GCM), `2dd23c6` + `69fde79` + `596a937` (ClickHouse query API, capture
  modes), `c4387ac` (port/bind unification).
- **Read for:** the crypto in `5188233` (encryption of logged fields) and that
  capture stays off the eval hot path (`5959f9b`).

### 5. Bundle signing
Supply-chain integrity for policy bundles.
- **Commits:** `dffb055` (Ed25519+SHA-256), `5512ce3` (ECDSA P-256 variant),
  `511204a` (agent verifies before hot-swap, fail closed), `e734e49` (management
  signs at creation), `d0789de` (`reaper-cli keygen`), `dc354c4` (deploy-status
  confirmation), `90e4df9` (audit doc).
- **Read for:** `511204a` — signature verification must fail closed before any
  hot-swap.

### 6. Data plane — D1/D2 (authorization data model + durable delta sync)
The largest functional arc: clients manage the data that drives their policies.
- **Commits:** `7d666df` (design), `4976749` (D1: managed authz data, verified
  sync, staleness budgets), `eaf34a1` (traversal direction / fail-closed /
  save-path), `b7231fb` (reaper-sync replication + replica-lag), `d25a635` (D2
  core: delta primitives + delta==rebuild gate), `03b9f6a` (durable change log +
  delta pull API), `2f54300` (agent delta apply + self-retrying sync),
  `4546745` (transactional outbox), `0b59f12` (process-level E2E — caught two
  production bugs), `932774c` (commit tightening).
- **Key files:** `services/reaper-agent/src/handlers/data.rs`,
  `services/reaper-sync/**`, `services/reaper-management/src/sync/**`,
  `crates/policy-engine/src/data/**`.
- **Net:** `delta_sync_differential_tests` (delta ≡ rebuild) and the
  process-level E2E (real binaries, kill/respawn). **Trust these** — they caught
  a referential-cascade hole and two production bugs router-level tests couldn't
  see.

### 7. PostgreSQL backend
One query codebase over SQLite and Postgres.
- **Commits:** `28eb0b0` (`$n` placeholders), `dc183c1` (AnyPool client +
  embedded versioned migrator), `124594f` (entire suite on real PG 16),
  `68e6f49` (E2E on PG too), `b71ee42` (`pg_notify` bridge), `045a204` (review
  doc `docs/development/POSTGRES_CLIENT_REVIEW.md`).
- **Read for:** the migrator (checksum-drift refusal) and that SQLite stays the
  zero-config default. **Net:** whole suite runs on both backends via
  `REAPER_TEST_DATABASE_URL`.

### 8. Operations & Kubernetes
Making the data plane operable.
- **Commits:** `3874353` (stale-replica alerting + change-log retention),
  `96e7ec6` (cold-start readiness gate `REAPER_DATA_REQUIRE_SYNC`), `39f8fd0`
  (`/ready` machine-readable body — the SDK health contract), `74e5e84` (Data
  Protection PEP design — design only, no code).
- **Note:** Helm templates are inspection-verified only (helm was unavailable in
  the dev container) — a `helm template` in CI is a reasonable follow-up ask.

### 9. DSL v2 count bug + full-function equivalence
A real compiled-path correctness bug and the net that now prevents its class.
- **Commits:** `aaf8442` (`user.skills.count()` compiled to a variable lookup →
  denied users who should pass; now compiles to `CountOp`), `41e1f95`
  (equivalence coverage for **every** DSL function), `7fac11e` (work-item doc for
  remaining compiled-mode functions).
- **Read `aaf8442` closely** — it's the kind of silent authorization divergence
  the equivalence suite exists to catch.

### 10. Memory hardening (this session)
Bounded memory on the data-distribution and eval paths.
- **Commits:** `6330d72` + `8226a33` (memory test reports actual store heap +
  load peak, not process RSS), `c7f6889` (**streaming JSON load** — cuts the
  ingest peak ~308→228 MB, byte-identical store), `1124bfb` + `06bbfd2`
  (data-distribution memory example + CI surfacing), `26e0bfb` (**refcounted
  interner + index pruning** — bounded memory under delta churn: 98 MB → 3.69 MB
  in the stress test), `4807421` (**context.\* eval-path leak fix** — request
  values compared by content, not interned).
- **Key files:** `crates/policy-engine/src/data/interning.rs` (refcount + pin
  safety), `src/data/store.rs` (release-on-remove + index pruning),
  `src/data/loader.rs` (streaming + counted interning),
  `src/evaluators/reaper_dsl/mod.rs` (context comparison).
- **Read for:** `interning.rs` — the pin-safety invariant is the crux (a string a
  compiled policy references can never be evicted; `intern()` pins,
  `intern_counted()`/`release()` reclaim). **Net:** the three differential suites
  above stay green under eviction, plus `context_interner_leak_tests` and the new
  interner unit tests. Full write-up: `docs/development/DATA_DISTRIBUTION_MEMORY.md`.

### 11. CI / build infrastructure
Not product code, but it gates everything.
- **Commits:** `603e893` (one consolidated pass/fail PR comment), `8043f06` +
  `3a90e2e` (mgmt-integration/equivalence/eval-microbench in the build; skipped
  pipeline no longer reads as PASSED), `d169bd3` (clippy clean), `05c3683` +
  `1dcf3a1` (Docker COPY fixes), `52d9532` + `32ec4f5` + `f88af7d` (toolchain to
  1.94, eopa via ghcr), `91fc7a2` (BDD skips cleanly), `755db2e` (stream mega
  data load), `f0deb8a` (per-service Docker cache scope).

## Where the risk concentrates (scrutinize these)
- **Fail-closed paths:** signature verification before hot-swap (`511204a`),
  staleness `enforce` mode (all-deny + `/ready` 503), scoped-binding fail-closed
  (`eaf34a1`), null semantics (`9f97c0c`).
- **The interner pin invariant** (`interning.rs`, `26e0bfb`) — the one place a
  bug would be a use-after-free-style wrong decision. Backed by the differential
  suites + dedicated unit tests.
- **Transactional outbox** (`4546745`) — mutation + sequence + change-log must
  commit or roll back together.

## Known residuals / follow-ups (documented, not blockers)
- ReBAC-subject churn is pinned (not reclaimed until snapshot rebuild) —
  narrow workload; see `DATA_DISTRIBUTION_MEMORY.md`.
- Helm templates inspection-verified only — add `helm template` to CI.
- Compiled-mode coverage for 8 remaining DSL functions is a tracked work item
  (`COMPILED_FUNCTIONS_WORKITEM.md`); AST fallback is decision-equivalent today.
- Data Protection PEP (`74e5e84`) is design only — no implementation in this PR.
