# Reaper Repo Map — Review Round 2 (2026-07-13)

Orientation document for all round-2 review subagents. **Context:** this is a
RE-REVIEW. Round 1 (`reviews/00`–`05`, same personas) returned NOT READY and
spawned a 12-plan remediation roadmap (`plans/00-ROADMAP.md`) — all 12 plans
have since shipped (PRs #10–#48). Your job is to review the CURRENT tree:
verify round-1 closures are real (not cosmetic), and find what is NEWLY wrong
or was missed. The original findings live in `reviews/*.md`; the shipped
scope per plan is summarized in each `plans/NN-*.md` STATUS banner.

## Workspace

Rust 2021 workspace (`Cargo.toml`), ~185k LOC of Rust. `workspace.lints`
includes `clippy::unwrap_used = "deny"` (with allowlisted exceptions).

| Member | LOC | Role |
|---|---|---|
| `crates/policy-engine` | ~80.6k | Evaluation engine: DSL, evaluators, DataStore, decision log |
| `crates/reaper-core` | ~2.9k | Shared types/traits |
| `crates/reaper-sdk` | ~1.2k | Client SDK |
| `crates/reaper-ebpf` | small | Experimental eBPF (Linux only) |
| `services/reaper-management` | ~62.9k | Multi-tenant control plane (axum, sqlx Any: SQLite dev / Postgres prod) |
| `services/reaper-agent` | ~13.1k | Enforcement sidecar/service (port 8080) |
| `services/reaper-platform` | ~1.7k | Legacy simple management layer (port 8081) |
| `services/reaper-sync` | ~2.5k | Replication client: control plane → agent (full deploy + delta pull) |
| `services/reaper-bench` | small | Load/bench harness |
| `tools/reaper-cli` | ~3.5k | CLI: eval/test/bundle/policy ops |

## Engine (`crates/policy-engine/src/`)

- **Evaluators** (`evaluators/`): `simple.rs` (wildcard, first-match),
  `cedar.rs`+`cedar_integration.rs` (AWS Cedar v4), `reaper_dsl/` (native DSL,
  pest grammar `reap.pest`, the perf-critical one).
- **Compilation tiers**: `policy_compilation.rs`, `compiled_evaluator.rs`,
  `optimized_engine.rs`, `indexed_engine.rs`, `partial_evaluation.rs`,
  `optimizer/` — AST → compiled/indexed forms; `fast_parse.rs` (request
  parsing), `arena.rs` (bump alloc), `regex_cache.rs` (thread-local),
  `batch.rs` (rayon-parallel bulk eval), `decision_cache.rs` (sharded),
  `decision_matrix.rs`.
- **DataStore** (`data/`): `store.rs` (DashMap store, multi-index),
  `interning.rs` (refcounted interner: entity-owned strings counted/evicted,
  schema vocabulary pinned), `relationships.rs` (graph, traversal budget),
  `loader.rs` (JSON → store, `upsert_entity_doc`/`delete_entity` delta ops),
  `indexes.rs`, `rbac.rs`, `views.rs`, `bundle.rs` (binary snapshot),
  `join.rs`, `router.rs`, `streaming.rs`.
- **Audit**: `decision_log.rs` (`DecisionLogEntry` — policy_version,
  data_version/checksum, **model_version** (new, Plan 12), hash-chain
  checkpoints), `decision_buffer.rs` (lock-free ring), `decision_privacy.rs`
  (masking/pseudonymization/encryption).
- **Benches** (`benches/`): policy_evaluation, complex_policy, rebac,
  data_scaling, caching, simd, builtins, optimization_phases, e2e.
- **Tests**: 36 integration test files incl. `delta_sync_differential_tests`
  (proptest delta≡rebuild), `eval_interner_bounding_tests`,
  `rebac_interner_bounding_tests`, `context_interner_leak_tests`,
  `migration_rename_interner_tests`, `check_mode_differential_tests`,
  `compiled_ast_equivalence_tests`, `concurrent_hotswap_tests`, fuzz targets
  under `fuzz/` (parser).

## Agent (`services/reaper-agent/src/`)

- `main.rs` (bootstrap, SSE subscribe, hot-swap), `panic_guard.rs`
  (CatchPanicLayer → 500, process survives), `state.rs` (`DataSyncState`:
  version/checksum/**model_version**, cold-start gate `REAPER_DATA_REQUIRE_SYNC`,
  staleness budget + Enforce/Monitor modes, `deny_reason()` fail-closed).
- `handlers/`: `evaluate.rs` (hot path: `/api/v1/messages`, `/api/v1/check`;
  decision capture → bounded buffer; stamps policy/data/model provenance),
  `data.rs` (`deploy-version` verified full snapshot, `apply-deltas`
  contiguity-enforced 409 self-correcting, `confirm-version`), `check.rs`,
  `decisions.rs` (query/stats/SSE/export NDJSON), `entities.rs`, `policies.rs`.
- UDS + sharded thread-per-core option (helm values `agent.uds.*`).
- Decision-log shipping: NDJSON file → Vector sidecar → ClickHouse
  (`deploy/decision-logs/`, helm `decisionLogs.*`).

## Control plane (`services/reaper-management/src/`)

- **API** (`api/`, 97 `routes!` sites, all utoipa-annotated): orgs, users/auth
  (sessions, JWT, API keys, OIDC `oauth/`, SCIM `scim/`), policies (ETag/
  If-Match), bundles (compile/stage/promote with optional dual-control change
  requests + idempotency keys), sources (git/api/s3/bundle-url + GitHub App,
  webhooks_git HMAC), deployments (strategies, rollouts+waves, pins,
  rollbacks, auto-rollback config, **promotion-path guard** for env-bound
  namespaces), datastore (ADM CRUD, publish, versions, changes feed,
  **migrations plan/apply/history/rollback** — Plan 12), environments +
  promotions (change requests, approvals, freeze windows, ServiceNow
  integration `integrations/servicenow.rs`), decisions (ClickHouse query),
  replay (counterfactual engine `replay/`), audit (hash-chain + checkpoints,
  legal holds, retention), revocations, landscape, billing, teams,
  namespaces, webhook_subscriptions, events (SSE), health.
  Contract gate: `tests/api_contract.rs` (no raw routes, unique operationIds).
- **Auth** (`auth/`): middleware (RequireAuth), scopes, JWKS agent auth,
  sessions (DB-backed), rate limiting on signup/login.
- **DB** (`db/`): `connection.rs` (sqlx Any; SQLite idempotent migrations
  001–022; PG versioned checksummed migrations 0001–0015, **advisory-locked**;
  failover-aware pool: test_before_acquire, lifetimes, bounded connect
  retry, optional replica URL) + `repositories/` per aggregate.
- **Domain** (`domain/`): incl. `datastore.rs` (ADM model + materialize),
  `migration.rs` (typed ModelTransform set + planner + compose_rollback,
  Plan 12), `impact.rs` (headless engine access-profile diff),
  `environment.rs` (tiers, ApprovalPolicy incl. require_change_record +
  external_change_record off/reference/validated), `change_request.rs`.
- **Sync engine** (`sync/`): git (GitHub App tokens, signed-commit verify,
  SSRF-guarded), api, s3, bundle_url, drift detection, commit-back.
- **Other**: `audit/` (mgmt audit, hash chain), `events_pg.rs` (pg NOTIFY
  cross-instance bridge), `replay/`, `landscape.rs`, `billing/`,
  `integrations/servicenow.rs`, `graceful.rs`, `url_guard.rs`.

## Distribution flow

Publish/rollout → SSE `ServerEvent` broadcast (+ pg_notify to sibling
instances) → agents/`reaper-sync` fetch → verified full deploy
(`deploy-version`, checksum-verified) or delta pull (`changes?since=seq`,
transactional outbox `adm_changes`, dedup, compaction floor →
`snapshot_required` self-heal) → atomic in-memory hot-swap. Rollouts complete
on **agent-confirmed convergence**. Bundles are ed25519-signed; agents verify
before load (Plan 02).

## CI (`.github/workflows/`)

`ci.yml` (fmt, clippy -D warnings, full test matrix SQLite+Postgres, volume
10k, memory/scale 100k, eBPF, BDD, micro-bench; in-place PR comment),
`perf-gate.yml` (blocking paired A/B perf gate on crates/services changes),
`perf-tracking.yml`, `benchmark.yml` (Reaper-vs-OPA), `fuzz.yml`,
`mutation.yml`, `supply-chain-nightly.yml` + in-ci cargo-deny/audit/Trivy/
SBOM (Plan 06), `docker.yml`, `release.yml`.

## Deploy

`deploy/helm/reaper/` (management/platform/agent + HPA/PDB, zero-gap
updateStrategy, soft anti-affinity, RWO-PVC NOTES warning, decision-log
pipeline, dev-grade Bitnami PG + `externalDatabase.url`),
`deploy/kubernetes/` (dev postgres.yaml DEV-ONLY-labeled, `postgres-cnpg.yaml`
3-node HA + WAL archiving + ScheduledBackup, `postgres-restore-check.yaml`
nightly restore verification), `docs/deployment/CONTROL_PLANE_HA_DR.md` +
`FLEET_UPGRADE_RUNBOOK.md` (RPO ≤5min / RTO ≤30min targets, game-day script).

## Known-deferred (do not re-flag as "missed" — flag if you think deferral is wrong)

- Plan 01 Phase D dogfooding (Reaper authorizing its own control plane).
- SAML (OIDC shipped; SAML deferred by explicit decision).
- First DR game-day execution (procedures shipped; not yet run in k8s).
- Consistency tokens/zookies; scoped role-binding materialization (rejected
  loudly at API); multi-region active/active control plane.

## Priorities per the shared ground rules

eval hot path (`evaluators/reaper_dsl/`, `engine/`, agent `evaluate.rs`) >
API surface (`api/`) > distribution/promotion (`sync/`, deployments,
datastore changes feed) > audit (`decision_log/buffer`, `audit/`) >
data-plane persistence (`db/`) > everything else.
