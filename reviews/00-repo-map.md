# Reaper — Repo Map (shared context for all reviewers)

**Scale:** ~157k lines of Rust across 11 workspace members. UI is out of scope.
**Edition:** Rust 2021. Async runtime: tokio (full). Web: axum. Lockfile committed (`Cargo.lock`).

## Workspace members (Cargo.toml)
| Member | LOC (src) | Role |
|---|---|---|
| `crates/policy-engine` | 47,530 | **Eval engine**: DSL parse/compile/eval, DataStore, ReBAC graph, decision log/buffer/cache, bundle format |
| `services/reaper-management` | 37,584 | **Control plane**: orgs/teams/users, policy CRUD+versions, bundles, deployments/rollouts, decisions API, billing, github sync, auth |
| `services/reaper-agent` | 8,565 | **Enforcement node (reaper)**: eval HTTP/UDS handlers, management sync client, decision endpoints |
| `crates/reaper-ebpf` | 7,513 | Experimental eBPF kernel integration (JWT parse in-kernel); contains most `unsafe` |
| `services/reaper-bench` | 7,117 | Benchmark harness (vs OPA/EOPA) |
| `tools/reaper-cli` | 3,527 | CLI: eval/test/compile/bundle/validate/policy |
| `services/reaper-sync` | 2,055 | Policy sync client/engine (server↔agent) |
| `crates/reaper-core` | 1,893 | Shared types, config, **`bundle_signing.rs`** |
| `services/reaper-platform` | 1,027 | Lightweight policy management (older/simpler than management) |
| `crates/reaper-sdk` | 984 | Client SDK (HTTP) |
| `tests/e2e` | — | End-to-end journey tests (management→agent) |

## The four product pillars → where they live
1. **Eval engine (sidecar/service)** — `crates/policy-engine/`, served by `services/reaper-agent/`.
2. **Policy distribution** — `services/reaper-management/src/{bundle,sync,api/deployments}/` → `services/reaper-agent/src/management/{sync,client}.rs` (+ `services/reaper-sync/`).
3. **Data fork / RBAC-ABAC-ReBAC data** — `crates/policy-engine/src/data/` (DataStore, interning, relationships graph); data-plane write APIs on agent (`/api/v1/entities`, `/api/v1/data/*`).
4. **Decision audit** — `crates/policy-engine/src/{decision_log,decision_buffer,decision_privacy}.rs` (capture) → `services/reaper-agent/src/handlers/decisions.rs` → `services/reaper-management/src/{decisions,audit,api/decisions.rs}` → `deploy/decision-logs/` (Vector→ClickHouse).

## Eval hot path (trace target for perf + security)
- Agent entry: `services/reaper-agent/src/handlers/evaluate.rs` (+ `fast_evaluate_policy`, `/api/v1/messages`, `/api/v1/fast-messages`, `/api/v1/check`, `/api/v1/batch-messages`).
- Engine: `crates/policy-engine/src/engine/mod.rs` (lock-free `DashMap<PolicyId, Arc<EnhancedPolicy>>`, `arc-swap`), `indexed_engine.rs`, `optimized_engine.rs`, `batch.rs`.
- Evaluators: `crates/policy-engine/src/evaluators/` (Simple, Cedar, ReaperDsl) + `crates/policy-engine/src/reap/` (parser, compiler, ast_evaluator, **compiled DSL v2**).
- Data: `crates/policy-engine/src/data/{store,interning,relationships}.rs` (interner is refcounted/evictable as of recent work; ReBAC graph is doubly-indexed, sorted SmallVec adjacency, bounded BFS with `TRAVERSAL_NODE_BUDGET=4096`).
- Decision capture: `decision_buffer.rs` (lock-free sharded ring), `decision_log.rs`, `decision_privacy.rs` (redaction).

## API surface (control plane = `reaper-management`, 19 route files)
Auth: `/auth/{login,logout,signup,me,token/refresh,password/*,email/verify,github/authorize,github/callback}`.
Tenanted: `/orgs`, `/orgs/{org}/{agents,agents/register,agents/{id}/heartbeat,agents/{id}/pin,bundles,bundles/promoted,bundles/{id}/stage,bundles/{id}/diff,decisions,decisions/{stats,facets,timeseries},events,dashboard,landscape,metrics,namespaces/tree,github/repos,billing,billing/{checkout,plans,portal}}`.
Agent (enforcement): `/api/v1/{messages,fast-messages,check,batch-messages,policies,policies/deploy,policies/compile,bundles,bundles/deploy,bundles/load,entities,entities/{type},entities/{type}/{id},entities/batch,data,data/{sync,stream,deploy-version,apply-deltas,confirm-version},decisions,decisions/{stats,export},decisions/{id}}`; health `/health{,/deep,/live,/ready}`, `/metrics`.
**No OpenAPI/Swagger spec found anywhere in repo** (searched `*openapi*`, `*swagger*`).

## Distribution / promotion flow
- `services/reaper-management/src/bundle/{compiler,service}.rs` compiles + stores `.rbb`.
- `services/reaper-management/src/api/deployments/{rollouts,pins,strategies,rollback_config,status}.rs` — strategies exist (immediate/canary/percentage/label per prior audit).
- Promotion broadcasts `BundlePromoted` via SSE `/orgs/{org}/events`; agent pulls via `services/reaper-agent/src/management/sync.rs` + `client.rs` (`/orgs/{org}/bundles/promoted`).
- Git source integration present: `services/reaper-management/src/sync/{git,s3,bundle_url}.rs`, `/orgs/{org}/github/repos`.
- **Bundle signing:** `crates/reaper-core/src/bundle_signing.rs` exists; `reaper-cli keygen` referenced (docs/security/BUNDLE_SIGNING.md). Verify whether agents *enforce* verification before load.

## Persistence (data plane / control plane)
- Control-plane DB: `services/reaper-management/src/db/` — `sqlx::AnyPool`, backend by `REAPER_DATABASE_TYPE` (sqlite default / postgres). Migrations in `db/migrations/` (incl. `004_users_and_audit.sql`). Tenant scoping enforced in handlers (per prior audit `auth/middleware.rs`).
- Data plane (entities/relationships): in-memory `DataStore` on the agent, hydrated from bundles/deltas.

## CI / supply chain (`.github/workflows/`)
- `ci.yml` (lint+test+volume+scale+bdd+integration+e2e+eval-microbench; clippy `--all-targets -D warnings` **now with pipefail**), `docker.yml` (build/scan/e2e), `benchmark.yml` (vs OPA), `perf-tracking.yml` (criterion vs main baseline, **comment-only** as of recent change), `mutation.yml` (nightly cargo-mutants), `release.yml`.
- **No `cargo audit`, `cargo deny`, or `cargo fuzz`/`fuzz/` targets found** (searched `.github`, repo root). Trivy image scan exists in `docker.yml` (continue-on-error).
- Docker E2E: builds images, `docker compose --profile management up`, runs `reaper-e2e-tests`.

## Deploy (`deploy/`)
- Helm chart `deploy/helm/reaper/` (profiles: engine, platform, managed-stack, full, engine-uds-sharded; HPA/PDB/PVC/configmap templates).
- Raw k8s `deploy/kubernetes/` (agent, platform, management, postgres, ingress, sidecar-example).
- DOKS manifests `deploy/doks/`. Decision-log pipeline `deploy/decision-logs/` (Vector configs + ClickHouse schema).

## Risk hotspots (counts — starting points, not verdicts)
- **`unsafe` blocks: 12** (concentrated in `reaper-ebpf`; enumerate + soundness per Security persona).
- **`unwrap()/expect()/panic!/unreachable!` in non-test engine+service src: ~957** — triage which are reachable from network input (panic in sidecar = host availability incident).
- DSL spec: `docs/reference/reap-language.md`, `docs/development/DSL_V2_DESIGN.md` (grammar/semantics exist in docs, not only in parser).

## Known context from recent work (already fixed — don't re-report as new)
- String interner is refcounted/evictable (bounded memory under churn); ReBAC subjects reclaimed.
- Eval-path interner bounding (principal/resource via `lookup`, transient result reclamation).
- Clippy gate now actually blocks (pipefail fix); perf gate is comment-only (shared-runner variance).
- `docker compose --profile management` boot fixed (`REAPER_DATABASE_TYPE=postgres`).

## Coverage priority (if you can't read everything)
eval hot path > API surface > distribution/promotion > audit pipeline > data-plane persistence > everything else. State what you did not cover.
