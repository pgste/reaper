# Reaper Repo Map — Review Round 3 (2026-07-16)

Orientation for all round-3 review subagents. **This is the THIRD review pass.**
Round 1 (`reviews/00`–`05`) returned NOT READY → spawned a 12-plan remediation
roadmap (`plans/00-ROADMAP.md`), all shipped. Round 2 (`reviews/round-2/`)
re-reviewed the closures. This round adds three NEW lenses on top of the
original four personas:

- **05 — Evolutionary Architect (Fowler-style):** future direction, optionality,
  evolvability, fitness functions, tech-debt trajectory, where the architecture
  will crack under the *next* order of magnitude of scale/features.
- **06 — Testing Guru:** can we *stand behind* the readiness, correctness, and
  performance claims? Grade the test pyramid, oracle quality, coverage of the
  hot path and audit invariants, flake/determinism, and whether the perf stars
  are actually defended by gates.
- **07 — CI/CD Expert ("iexpert"):** is CI/CD constructed so we test in the
  right place, at the right layer, and build+publish in the right places —
  efficiently (caching, matrix shape, blocking-vs-advisory placement, supply
  chain, release provenance)?

Your job: review the CURRENT tree. Verify prior closures are real (not
cosmetic), find what is NEWLY wrong or was missed, and — for the new personas —
assess the dimensions above. The prior findings live in `reviews/*.md` and
`reviews/round-2/*.md`; shipped scope per plan is in each `plans/NN-*.md`.

Write your report to your designated file under `reviews/round-3/`. **Do not fix
anything — review only, with `file:line` evidence.**

## Workspace (Rust 2021, resolver 2, ~185k LOC Rust)

Root `Cargo.toml` sets `[workspace.lints.clippy] unwrap_used/expect_used =
"deny"` (crates opt in via `[lints] workspace = true`; `clippy.toml` allows the
idiom in `#[cfg(test)]`). `fuzz/` and `benchmarks/reaper-vs-opa` are excluded
from the default workspace.

| Member | ~LOC | Role | Integration test files |
|---|---|---|---|
| `crates/policy-engine` | ~80.6k | Eval engine: DSL, evaluators, DataStore, decision log | 36 |
| `crates/reaper-core` | ~2.9k | Shared types/traits, `bundle_signing.rs`, `revocation.rs`, `capability.rs`, `config/` | 2 |
| `crates/reaper-sdk` | ~1.2k | Client SDK | 1 |
| `crates/reaper-wasm` | small | WASM build of the engine (browser demo) | 1 |
| `crates/reaper-ebpf` | small | Experimental eBPF (Linux only) | 1 |
| `services/reaper-management` | ~62.9k | Multi-tenant control plane (axum, sqlx Any: SQLite dev / Postgres prod) | **3** |
| `services/reaper-agent` | ~13.1k | Enforcement sidecar/service (:8080) | 9 |
| `services/reaper-platform` | ~1.7k | Legacy simple management layer (:8081) | 1 |
| `services/reaper-sync` | ~2.5k | Replication client: control plane → agent | 1 |
| `services/reaper-bench` | small | Load/bench harness | — |
| `tools/reaper-cli` | ~3.5k | CLI: eval/test/bundle/policy ops | 0 |
| `tools/reaper-mcp` | small | Stdio MCP gate routing tool calls through the agent | some |
| `tests/e2e` | small | Cross-service e2e (`tests/e2e/tests`, `run-e2e-tests.sh`) | 2 |

> **Testing-guru lead:** the 62.9k-LOC control plane shows only **3**
> `tests/` files — most of its testing is inline `#[cfg(test)]` modules and
> `api_contract.rs`. Confirm whether integration/e2e coverage of the control
> plane matches its risk weight, or whether it leans on unit tests of handlers
> in isolation. `tools/reaper-cli` has **0** integration tests despite being the
> CI/CD entry point customers script against.

## Engine (`crates/policy-engine/src/`)

- **Evaluators** (`evaluators/`): `simple.rs` (wildcard, first-match),
  `cedar.rs`+`cedar_integration.rs` (AWS Cedar v4), `reaper_dsl/` (native DSL —
  the perf-critical path; `compiler.rs`, `expr_compiler.rs`, `expr_eval.rs`,
  typed `types/`). Second DSL surface: `reap/` (`parser/`, `compiler/`,
  `ast_evaluator/` with `builtin_functions/` incl. `jwt.rs`, `regex.rs`,
  `time.rs`; `limits.rs` resource caps).
- **Compilation/opt**: `policy_compilation.rs`, `compiled_evaluator.rs`,
  `partial_evaluation.rs`, `optimizer/decision_tree.rs`, `fast_parse.rs`,
  `regex_cache.rs` (thread-local), `batch.rs` (rayon bulk eval),
  `decision_cache.rs` (sharded), `decision_matrix.rs`, `clock.rs`.
- **Engine core** (`engine/`): `mod.rs`, `policy.rs`, `bundle.rs`, `package.rs`,
  `staging.rs`, `types.rs`, `tests.rs`.
- **DataStore** (`data/`): `store.rs`, `interning.rs` (refcounted interner),
  `relationships.rs` (ReBAC graph + traversal budget), `loader.rs` (JSON→store,
  delta upsert/delete), `indexes.rs`, `rbac.rs`, `views.rs`, `bundle.rs`,
  `join.rs`, `router.rs`, `streaming.rs`.
- **Audit**: `decision_log.rs` (`DecisionLogEntry`: policy_version,
  data_version/checksum, model_version, hash-chain checkpoints),
  `decision_buffer.rs` (lock-free ring), `decision_privacy.rs` (masking/
  pseudonymization/encryption), `decision_export.rs`.
- **Benches** (`benches/`, 12 across the tree): policy_evaluation, complex,
  rebac, data_scaling, caching, simd, builtins, optimization_phases, e2e.

## Agent (`services/reaper-agent/src/`)

- `main.rs`/`bootstrap.rs` (SSE subscribe, hot-swap), `panic_guard.rs`
  (CatchPanicLayer → fail-closed 500), `state.rs` (`DataSyncState`:
  version/checksum/model_version, cold-start gate, staleness budget +
  Enforce/Monitor, `deny_reason()` fail-closed), `auth.rs`, `tls.rs`, `uds.rs`.
- `handlers/`: `evaluate.rs` (hot path `/api/v1/messages`, `/api/v1/check`;
  decision capture → bounded buffer; stamps provenance), `data.rs`
  (`deploy-version`, `apply-deltas` contiguity-enforced, `confirm-version`),
  `check.rs`, `decisions.rs` (query/stats/SSE/export), `entities.rs`,
  `policies.rs`, `health.rs`.
- `management/`: `client.rs`, `sync.rs`, `sse.rs`, `verify.rs` (bundle sig),
  `anti_rollback.rs`, `revocation.rs`.

## Control plane (`services/reaper-management/src/`) — largest surface

- **API** (`api/`, ~97 utoipa-annotated route sites): `orgs`, `users/`+`auth/`
  (sessions, JWT, API keys, `oauth/` OIDC+GitHub, `scim/`), `policies`
  (ETag/If-Match), `bundles` (compile/stage/promote + change_requests +
  `idempotency`), `sources` (git/api/s3/bundle-url + `github_app`,
  `webhooks_git` HMAC), `deployments/` (`strategies`, `rollouts`, `pins`,
  `rollback_config`, `status`), `datastore` (ADM CRUD + migrations),
  `environments`, `change_requests`, `decisions`, `replay`, `audit`,
  `revocations`, `landscape`, `billing`, `teams`, `namespaces`,
  `webhook_subscriptions`, `events` (SSE), `connectors`, `capabilities`,
  `preconditions`, `pagination`, `openapi`, `error` (problem+json).
- **Auth** (`auth/`): `middleware.rs`, `scopes.rs`, `jwt.rs`, `jwks.rs`,
  `api_key.rs`, `mtls.rs`, `gateway.rs`, `sso/` (broker+store), `scim/`,
  `users/` (password).
- **DB** (`db/`): `connection.rs` (sqlx Any; SQLite dev migrations, PG
  versioned+checksummed advisory-locked migrations; failover-aware pool) +
  `repositories/` per aggregate (deployment split into pins/rollouts/waves/
  strategies).
- **Domain** (`domain/`): `datastore` (ADM + materialize), `migration` (typed
  ModelTransform + planner + rollback), `impact` (access-profile diff),
  `environment` (tiers, ApprovalPolicy), `change_request`, `promotion`, etc.
- **Sync** (`sync/`): `git.rs`, `github_app.rs`, `commit_verify.rs`, `drift.rs`,
  `bundle_url.rs`, `s3.rs`, `service.rs`, `url_guard.rs`/`api.rs` (SSRF guard).
- **Other**: `audit/` (mgmt-action audit, hash chain), `events_pg.rs` (PG
  NOTIFY bridge), `replay/`, `landscape/`, `billing/`, `quota/`, `rate_limit.rs`,
  `siem/`, `integrations/servicenow.rs`, `storage/` (s3/dynamodb/mongodb/fs),
  `graceful.rs`, `decisions/purge.rs`, `validation/`, `webhook/`.

## Distribution / promotion flow

Publish/rollout → SSE `ServerEvent` broadcast (+ pg_notify to siblings) →
agents/`reaper-sync` fetch → verified full deploy (`deploy-version`,
checksum-verified) or delta pull (`changes?since=seq`, transactional outbox
`adm_changes`, dedup, compaction floor → `snapshot_required` self-heal) →
atomic in-memory hot-swap. Rollouts complete on agent-confirmed convergence.
Bundles are **ed25519-signed**; agents verify before load. Promotion supports
change requests / dual control / approval gates / freeze windows / ServiceNow.

## CI/CD (`.github/workflows/`) — read these closely for persona 07

| Workflow | Purpose |
|---|---|
| `ci.yml` (1493 lines) | jobs: `lint-and-analyze`, `supply-chain`, `dependency-freshness`, `api-contract`, `unit-tests`, `management-tests-postgres`, `ebpf-build`, `wasm-build`, `volume-tests`, `memory-scale-tests`, `scale-tests`, `integration-tests`, `bdd-tests`, `eval-microbench`, `generate-report` (in-place PR comment) |
| `perf-gate.yml` | Blocking paired A/B perf gate on shared runner (removes cross-run variance) |
| `perf-tracking.yml` | Advisory trend dashboard |
| `benchmark.yml` (26k) | Reaper-vs-OPA comparison |
| `slo-harness.yml` | Nightly served-path SLO harness over a real release agent (p50/p99/p999) |
| `fuzz.yml` | cargo-fuzz over parser/compiler (`fuzz/fuzz_targets/parse_reap.rs`, `compile_reap.rs`) |
| `mutation.yml` | Nightly cargo-mutants over evaluators/compiler decision logic |
| `supply-chain-nightly.yml` | Scheduled cargo-audit (post-merge CVE disclosure) |
| `docker.yml` | Image build + Trivy scan (blocking CRITICAL/HIGH, ignore-unfixed) |
| `release.yml` | Release build + CycloneDX SBOM attach |

In-CI supply chain (in `ci.yml` `supply-chain` job): cargo-deny (advisories/
licenses/bans/sources), cargo-audit, Trivy, SBOM. Gates documented in
`CLAUDE.md` §Supply-Chain Gates and `docs/security/VULN_RESPONSE.md`.

## Deploy

`deploy/helm/reaper/` (management/platform/agent + HPA/PDB, anti-affinity,
decision-log pipeline, dev Bitnami PG + `externalDatabase.url`),
`deploy/kubernetes/` (`postgres-cnpg.yaml` 3-node HA + WAL archiving +
ScheduledBackup, `postgres-restore-check.yaml` nightly restore verify,
sidecar example), `deploy/doks/`, `deploy/decision-logs/` (Vector → ClickHouse,
SIEM sinks). Docs: `docs/deployment/CONTROL_PLANE_HA_DR.md`,
`FLEET_UPGRADE_RUNBOOK.md`.

## Known-deferred (do not re-flag as "missed"; challenge the deferral if wrong)

- Reaper dogfooding (authorizing its own control plane via its own engine).
- SAML (OIDC shipped; SAML deferred by decision).
- First DR game-day execution (procedures shipped, not yet run).
- Consistency tokens/zookies; multi-region active/active control plane.

## Priority order (shared ground rules)

eval hot path (`evaluators/reaper_dsl/`, `reap/`, `engine/`, agent
`evaluate.rs`) > API surface (`api/`) > distribution/promotion (`sync/`,
deployments, datastore changes feed) > audit (`decision_log`/`buffer`,
`audit/`) > data-plane persistence (`db/`) > everything else. State what you
did NOT cover.
