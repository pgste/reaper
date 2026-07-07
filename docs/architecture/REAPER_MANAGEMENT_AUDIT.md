# Reaper Management Control-Plane Audit

**Date:** 2026-07-03
**Scope:** `services/reaper-management/` (plus cross-referenced `services/reaper-agent/src/management/`, `services/reaper-sync/`, `services/reaper-platform/`)
**Vision under audit:** A multi-tenant SaaS control plane where users spin up **cloud-hosted OR self-hosted** Reaper agents, **publish policy** to them, and **capture decision logs centrally**.

---

## Executive Summary

The control plane is a substantial, genuinely-built multi-tenant management skeleton — **not** a set of stubs. Persistence is real (SQLite/Postgres via `sqlx` with 5 migrations and a runner), tenant isolation is real (every query is org-scoped, and there is a deliberate fix ensuring an org Owner is *not* a platform super-admin), and five auth mechanisms are wired (API key, mTLS, shared-secret JWT, external JWKS, user sessions). Critically, the **plane→agent policy-publish path actually works end-to-end for the rollout flow**: the deployment service broadcasts a `BundlePromoted` SSE event and the fully-implemented agent-side sync client (`reaper-agent/src/management/sync.rs`) receives it, pulls the bundle, and hot-swaps it. The two dominant gaps versus the vision are (1) **central decision-log ingestion is essentially absent** — the plane stores only aggregate allow/deny *counts* piggybacked on heartbeats, never individual decision records — and (2) **there is zero provisioning/orchestration** to "spin up" a cloud agent (only registration of agents that already exist). Billing is a pure Stripe placeholder. **Overall: ~45% of the vision.** The management/publish half is ~65-70% done; the decision-log-capture and cloud-provisioning halves are ~5-10% done.

---

## Capability Matrix

| Vision capability | Status | Evidence | Gap |
|---|---|---|---|
| Multi-tenant orgs/teams/users | ✅ done | `db/migrations/001_initial.sql` (organizations, teams), `004_users_and_audit.sql` (users, user_orgs, sessions); `api/orgs.rs`, `api/teams.rs`, `api/users/` | Namespaces exist but are a sub-org grouping, not hard tenant boundary |
| Tenant isolation (org-scoped) | ✅ done | Every handler resolves org then guards `user.org_id != organization.id` e.g. `api/agents.rs:130,223,269`; `auth/middleware.rs:465-514` deliberately withholds global `admin` from Owners | Isolation enforced in handlers, not at DB/row-security layer |
| AuthN (multiple methods) | ✅ done | `auth/middleware.rs:166-330`: API key, mTLS (`:192`), shared-secret JWT (`:259`), JWKS external IdP (`:280`), user session `rst_` tokens (`:220`) | mTLS off by default (needs trusted-proxy header configured, `:156`) |
| AuthZ / scopes on endpoints | ✅ done | `auth/scopes.rs`; per-handler checks e.g. `api/agents.rs:120,215,283` | Scope checks are hand-rolled per handler (no central policy) |
| Durable persistence | ✅ done | `db/mod.rs:15-19` runs migrations; `db/connection.rs`; repositories in `db/repositories/*` | — |
| Self-hosted agent register + heartbeat | ✅ done | `api/agents.rs:113-361`; agent side `reaper-agent/src/management/sync.rs:331-363` | Works only for an agent that is already running and configured |
| **Cloud-hosted agent spin-up (provisioning)** | 🔴 missing | No terraform/k8s/docker/cloud-SDK anywhere in `src/` (grep returned nothing) | Entire provisioning/orchestration layer absent |
| Policy CRUD + versioning | ✅ done | `api/policies.rs`; `db/migrations/001_initial.sql` (policies, policy_versions) | — |
| Bundle build/compile | ✅ done | `bundle/service.rs:159-228` compiles + stores `.rbb` | — |
| Bundle promote lifecycle | 🟡 partial | `bundle/service.rs:256-310` transitions status; **`:306` `TODO: trigger deployment to agents`** | `promote()` alone does **not** notify agents; you must start a rollout |
| Deployment strategies / rollouts | ✅ done | `deployment/service/mod.rs:206-321`; strategies immediate/canary/percentage/label; `api/deployments/` (many routes) | — |
| **Plane→agent convergence (publish)** | 🟡 partial | `deployment/service/helpers.rs:179-297` broadcasts `BundlePromoted`; agent pulls via `reaper-agent/src/management/sync.rs:209-280` + `client.rs:232` (`/bundles/promoted`) | **No confirmation loop**: wave marked complete optimistically (`helpers.rs:221-227`); no agent→plane deploy-status callback |
| Policy-source sync (Git/S3/API→plane) | 🟡 partial | `sync/service.rs`, `sync/git.rs|s3.rs|api.rs|bundle_url.rs` implemented; **but `api/sources.rs:381` `TODO: Actually trigger the sync`** | Manual "sync now" endpoint is a stubbed no-op; background scheduler exists in service but is not wired from the API |
| **Central decision-log ingestion** | 🔴 missing | Only aggregate counts: `domain/agent.rs:131-133` (`decisions_allow/deny`), stored via heartbeat `api/agents.rs:347-355` → `agent_metrics_latest`; surfaced by `api/landscape.rs`. Agent exposes per-decision logs **locally only** (`reaper-agent/src/main.rs:506-508`) and never pushes | No ingestion endpoint, no per-decision table, no query API |
| Fleet visibility / landscape | ✅ done | `api/landscape.rs` (`/landscape`, `/metrics`, `/dashboard`); `landscape/service.rs` | Metrics limited to counts; `service.rs:269 TODO` pins not counted |
| Billing / quotas | 🔴 missing | `billing/service.rs:157,197,228,320` all `TODO`, return `cus_placeholder_…` (`:165`); `api/billing.rs:179` | No Stripe integration, no quota enforcement |
| Audit log | ✅ done | `db/migrations/004_users_and_audit.sql` (audit_log); `audit/mod.rs`; tested `tests/integration_tests.rs:1172-1255` | — |
| Webhooks | 🟡 partial | `webhook/service.rs`, `api/webhooks.rs`, `api/webhook_subscriptions.rs`; table in `004` | Delivery path present; not exercised by tests |

---

## Per-Subsystem Findings

### 1. Service map (`services/reaper-management/src/`)

- **`api/`** — REST surface, ~20 modules, all mounted in `api/mod.rs:29-48`. Real handlers for orgs, teams, users (signup/login/sessions/members), oauth (github), agents, events (SSE), sources, bundles, deployments (rollouts/pins/strategies), namespaces, landscape, webhooks, billing.
- **`auth/`** — Real. `middleware.rs` (multi-method extractor), `api_key.rs`, `jwt.rs`, `jwks.rs`, `mtls.rs`, `scopes.rs`, `users/` (password hashing, sessions).
- **`db/`** — Real. `connection.rs` (sqlx pool, SQLite+Postgres), `migrations/*.sql` (5), `repositories/*` (organization, team, namespace, source, agent, deployment/*, bundle, policy).
- **`bundle/`** — Real. `service.rs` (compile/stage/promote/deprecate/download), `compiler.rs`.
- **`deployment/`** — Real. `service/mod.rs` + `service/helpers.rs` orchestrate rollouts/waves/pins. **Convergence is broadcast-only** (see §5).
- **`sync/`** — Real but not API-triggered. Source-of-truth pull from Git/S3/API/bundle-URL **into** the plane (`sync/service.rs`, `git.rs`, `s3.rs`, `api.rs`, `bundle_url.rs`). Note: this is *inbound* source sync, orthogonal to *outbound* agent delivery.
- **`billing/`** — Stub. Stripe placeholders only (`service.rs:157…`).
- **`webhook/`** — Real-ish delivery service (`service.rs`, 306 lines).
- **`landscape/`** — Real. Fleet aggregation over agent metrics (`service.rs`, 451 lines).
- **`storage/`** — Real, pluggable bundle blob storage: `filesystem.rs`, `s3.rs`, `dynamodb.rs`, `mongodb.rs` behind `traits.rs`.
- **`validation/`** — Real policy validation service (`service.rs`, syntax checks).
- **`audit/`** — Real audit-log writer.
- **`config/`, `metrics.rs`, `middleware.rs`, `rate_limit.rs`, `graceful.rs`, `state.rs`** — Real production-hardening (config validation, Prometheus, correlation IDs, security headers, body-size limits, rate limiting, graceful shutdown).

### 2. Persistence — **real and durable**

`db/mod.rs:15-19` opens the pool and runs migrations on startup. Tables (from `migrations/`):
- `001_initial`: organizations, teams, policy_sources, policies, policy_versions, bundles, bundle_policies, bundle_promotions, agents, agent_bundles, api_keys, jwks_configs, data_sources.
- `002_namespaces`: namespaces, agent_subscriptions, deployment_strategies, rollouts, rollout_waves, version_pins, agent_metrics_latest, org_metrics_hourly, client_certificates.
- `003_security`: jwks_configs, jwks_sessions.
- `004_users_and_audit`: users, user_orgs, sessions, audit_log, oauth_connections, webhook_subscriptions, password_reset_tokens, email_verification_tokens.
- `005_phase2_operations`: agent_deployments, rollback_configs, bundle_diffs.

Migrations runner is real (`db/mod.rs:17` `db.run_migrations()`); every test spins up a real SQLite DB and runs them (e.g. `bundle/service.rs:379-380`). **No state is in-memory in reaper-management** (contrast reaper-platform, §8).
Note: `agent_deployments` (005) exists to track per-agent deploy status but is **not populated by real agent confirmations** because no confirmation callback exists (§5).

### 3. Multi-tenancy & auth — **real isolation, 5 auth methods**

- Orgs are the tenant boundary; namespaces (`002`) are intra-org grouping with agent subscriptions; teams for user grouping.
- Isolation is enforced in every handler by resolving the org and comparing `user.org_id` (e.g. `api/agents.rs:130,223,269,305,339`). Cross-org access requires the global `Scope::Admin`.
- Key hardening: `auth/middleware.rs:465-514` + regression test `:521-541` ensure an org **Owner** gets full control of its *own* org but **not** the global `admin` scope (which is the cross-org escape hatch) — a real tenant-isolation fix.
- Auth methods wired in `RequireAuth` (`auth/middleware.rs:140-341`): API key (`X-API-Key`, `:167`), mTLS via trusted-proxy fingerprint header (`:192`, disabled unless `mtls_fingerprint_header` configured, `:156`), user session `rst_` token (`:220`), shared-secret JWT for agents (`:259`), external JWKS/IdP (`:280`). `OptionalAuth` mirrors this (`:347-444`).
- Agents authenticate with a shared-secret JWT minted at registration (`api/agents.rs:158-205`) granting `agent:read`, `policy:read`, `bundle:read`.

### 4. Agent lifecycle — **register + heartbeat real; provisioning absent**

- Register: `api/agents.rs:113-206` — creates a row, mints agent JWT, broadcasts `AgentRegistered`. Heartbeat: `:315-361` — updates `last_heartbeat_at` and stores metrics (`agent_repo.update_metrics`, `db/repositories/agent.rs:323+`). List/get/delete present.
- Agent side is fully built: `reaper-agent/src/management/sync.rs:331-355` registers with exponential-backoff retry; `:358-363` heartbeats with real metrics (`collect_metrics:366-419`, including `decisions_allow/deny` from atomics).
- **Provisioning/orchestration to *create* an agent (cloud or self-hosted) does not exist.** Grep for `provision|terraform|kubernetes|docker|spin.?up|ec2|fargate|launch` over `src/` finds nothing. "Self-hosted" works only in the sense that an operator runs an agent binary configured with `ManagementSettings`, and it self-registers. There is no plane-driven lifecycle (create/start/stop/destroy) of agent compute.

### 5. Policy publish (plane→agents) — **works via rollout; gaps at promote() and confirmation**

Trace of the *working* path:
1. Policy CRUD → `api/policies.rs` (broadcasts `PolicyUpdated`).
2. Bundle create → add policies → compile → store `.rbb` (`bundle/service.rs:54-228`).
3. **Start a rollout** (`deployment/service/mod.rs:206-321`) selects active agents by strategy, creates waves, and for immediate strategy calls `execute_rollout_wave`.
4. `execute_rollout_wave` (`deployment/service/helpers.rs:179-297`) broadcasts `ServerEvent::BundlePromoted` with a `download_url` (`:207-218`).
5. SSE fan-out: `api/events.rs:115-170` streams org/namespace-filtered events to the agent's `/orgs/{org}/agents/{agent_id}/events`.
6. Agent receives it: `reaper-agent/src/management/sync.rs:209-221` → `sync_bundle_by_id` → `client.download_bundle` → hot-swap via `update_tx`. Agent also polls `/orgs/{org}/bundles/promoted` as a fallback (`client.rs:232`). Endpoints align with the plane (`api/bundles.rs:70,74`).

**Gaps / TODOs:**
- `bundle/service.rs:306` — `promote()` on its own is a DB status change and **does not broadcast** any agent-facing event. Publishing requires `start_rollout`. This dual path is a footgun.
- `deployment/service/helpers.rs:221-227` — the wave is marked `Completed` and deployed-count incremented **immediately**, without waiting for agent acknowledgement ("in a real system, this would wait for agent confirmations"). There is **no agent→plane deploy-status callback endpoint**, so `agent_deployments` (migration 005) is never confirmed from reality; rollout success is optimistic.
- `api/sources.rs:381` — the manual "trigger sync" endpoint is a stubbed no-op that just flips status to `Syncing`; it does not invoke `SyncService`.
- Two overlapping agent-delivery clients exist: the in-process `reaper-agent/src/management/` client (SSE+poll, the real one) and the standalone `reaper-sync` sidecar (§8 / see below).

### 6. Decision-log capture (agents→plane) — **the biggest vision gap**

- The plane captures **only aggregate counts**: `AgentMetrics { decisions_allow, decisions_deny, … }` (`domain/agent.rs:131-133`) sent on heartbeat (`api/agents.rs:347-355`), persisted to `agent_metrics_latest` / `org_metrics_hourly` (`db/repositories/agent.rs:323-346`), and rolled up in `landscape/service.rs:70-72,295-296` / `api/landscape.rs:112-135,261-315`.
- The agent produces **rich per-decision NDJSON logs locally** (ring buffer + export) and exposes them for *pull* only: `reaper-agent/src/main.rs:506-508` (`GET /api/v1/decisions`, `/decisions/stats`, `POST /decisions/export`, backed by `reaper-agent/src/handlers/decisions.rs`). **The agent never pushes individual decisions to the plane**, and the plane has **no ingestion endpoint, no per-decision table, and no query API**.
- Note: `CLAUDE.md` advertises `GET /api/v1/decisions/stream` (SSE) on the agent, but **no such route or handler exists** — the only decision routes are `decisions`, `decisions/stats`, `decisions/export`. (The agent's SSE code in `management/sse.rs` is the agent acting as an SSE *client* to the plane, inbound only.)
- To meet the vision, a full pipeline is missing: an agent-side shipper (batch POST or streaming), a plane ingestion endpoint (org-scoped, authenticated), durable storage (time-series/append table or object store + index), and a query/export API. This is greenfield.

### 7. Billing / quotas — **Stripe placeholder only**

`billing/service.rs`: `get_or_create_customer` returns `cus_placeholder_{org}` (`:165`); `create_checkout_session` (`:197`), `billing_portal` (`:228`), and `webhook` verification (`:320`) are all `TODO`. `api/billing.rs:179` mirrors the placeholder. No quota tables and no enforcement anywhere.

### 8. Two management surfaces — **consolidate onto reaper-management**

- `reaper-platform` (`services/reaper-platform/`, ~341 LOC total) is a simpler single-node surface with **in-memory** state: `state.rs:12-20` holds an embedded `PolicyEngine`, a `HashMap` bundle store, and an `agents: HashMap` field that is `#[allow(dead_code)]` (`:18`). Agent handlers are explicit placeholders (`handlers/agents.rs:1,9,25`), and bundles are `TODO` (`handlers/bundles.rs:144`). It is effectively a demo/embedded evaluator + management shim.
- `reaper-management` is the real multi-tenant, DB-backed, auth-enforced control plane.
- **Recommendation:** treat `reaper-platform` as deprecated. Its only unique value is the embedded in-process `PolicyEngine` (useful for a single-binary/dev mode). Migrate that "embedded engine" convenience into a dev profile of the agent (or a mode of management) and retire the service to remove the confusing second surface. Nothing durable needs migrating (state is in-memory).
- Also note the third overlapping component: **`reaper-sync`** (`services/reaper-sync/`, ~1700 LOC: `server_client.rs`, `agent_client.rs`, `sync_engine.rs`) is a standalone **poll-and-push** sidecar that reads a server's `GET /api/v1/policies` and POSTs to the agent's `POST /api/v1/policies/deploy`. **Critically, it is written against the flat `/api/v1/...` route shape of `reaper-platform`, not the org-scoped `/orgs/{org}/...` routes of `reaper-management`** (`reaper-sync/src/server_client.rs:126-165` vs the real plane's `api/bundles.rs`). It also relies on server-provided `version`/`checksum` fields for change detection (`reaper-sync/src/sync_engine.rs:120`) that `reaper-platform` does not expose — so it is currently mis-wired for the real plane and only partially wired for the demo one. It duplicates the in-agent `management/` client (which *is* correctly wired to `reaper-management` via SSE). Pick one delivery mechanism (recommend the in-agent client) and either delete `reaper-sync` or repurpose it as the sidecar packaging of that same client against the real routes.

### 9. Tests & CI

- `tests/integration_tests.rs` (~1400 lines, 26 `#[tokio::test]`) spins up the real router + SQLite and covers: health, org CRUD, policy lifecycle, bundle workflow (create→compile→promote), agent registration, **heartbeat metric storage** (`:664-794`), source CRUD, JWKS config lifecycle, user signup/login/session/logout, audit-log on signup/login, org member list, password change/reset, session-token auth against API endpoints.
- Unit tests are embedded across services (deployment strategies `deployment/service/mod.rs:636-708`, bundle workflow `bundle/service.rs:444-546`, auth `auth/middleware.rs:521-586`, etc.).
- **Not tested:** end-to-end plane→agent convergence (no test drives an agent SSE client against the plane), agent→plane decision ingestion (doesn't exist), rollout confirmation, webhook delivery, billing. There is no cross-service integration test wiring `reaper-management` + `reaper-agent` together.

---

## Sequenced Build Plan

Ordered by leverage toward the vision. **Correction to the proposed order:** plane→agent convergence is *more* complete than assumed (the agent-side pull client is fully built and endpoints align), so Phase 1 is a small "close the loop" effort rather than a build-out. The largest *unbuilt* vision pillar is **central decision-log ingestion**, so it moves up to Phase 2. Consolidation is cheap and de-risks everything, so it slots early. Cloud provisioning is the biggest lift and comes after the data plane is coherent. Billing is last.

### Phase 1 — Close the plane→agent publish loop (unlocks reliable "publish policy")  · effort **M**
**Goal:** Make "publish to agents" a single coherent, confirmable action.
- Wire `bundle/service.rs:306` so `promote()` (when `!notify_only`) starts an immediate rollout (or directly broadcasts `BundlePromoted`). Decide the canonical publish verb (recommend: promote → rollout).
- Add an **agent→plane deploy-status callback**: new endpoint `POST /orgs/{org}/agents/{agent_id}/deployments/{bundle_id}/status`; have the agent report success/failure/active-checksum after hot-swap; populate/confirm `agent_deployments` (already in `005`). Replace optimistic completion in `deployment/service/helpers.rs:221-227` with confirmation-driven wave advancement (with timeout).
- Wire `api/sources.rs:381` to actually invoke `SyncService`.
- **Files:** `bundle/service.rs`, `deployment/service/helpers.rs`, `deployment/service/mod.rs`, `api/deployments/*`, `api/agents.rs`, `api/sources.rs`; agent side `reaper-agent/src/management/sync.rs`, `client.rs`.
- **Deps:** none. **Test:** add a cross-service integration test (agent client vs plane).

### Phase 2 — Central decision-log ingestion + storage + query (the core missing pillar)  · effort **L**
**Goal:** Capture individual decision records centrally, multi-tenant, queryable.
- **Agent shipper:** batch/stream the existing NDJSON ring buffer to the plane (reuse the export format; add backpressure + at-least-once). Files: `reaper-agent/src/` decision buffer + a new shipper task alongside `management/sync.rs`.
- **Plane ingestion endpoint:** `POST /orgs/{org}/agents/{agent_id}/decisions` (auth = agent JWT/mTLS, org-scoped, batched).
- **Storage:** new migration `006_decisions.sql` (append-optimized `decisions` table partitioned by org/time) or pluggable sink to object storage + index; reuse `storage/` trait pattern for cold storage. Consider retention/rollup into existing `org_metrics_hourly`.
- **Query API:** `GET /orgs/{org}/decisions` (filter by agent/principal/action/resource/decision/time, paginated), `/decisions/stats`, `/decisions/export`, and an SSE tail — mirroring the agent's local API but fleet-wide.
- **Files:** new `api/decisions.rs`, `db/migrations/006_*.sql`, `db/repositories/decision.rs`, `domain/decision.rs`, `state.rs` (event type), `storage/`.
- **Deps:** Phase 1 auth patterns. **Highest vision leverage** — takes decision-capture from ~5% to functional.

### Phase 3 — Consolidate management surfaces  · effort **S**
**Goal:** One control plane, one agent-delivery client.
- Deprecate `reaper-platform`; fold its embedded-`PolicyEngine` dev convenience into an agent dev-mode or a `--embedded` flag on management. Remove the placeholder agents/bundles handlers.
- Choose one agent-delivery path: keep the in-agent `management/` SSE client; delete or re-scope `reaper-sync` to be *that* client packaged as a sidecar.
- **Files:** remove/retire `services/reaper-platform/`, `services/reaper-sync/` (or re-scope); update `docker-compose` profiles and docs.
- **Deps:** Phase 1 (so the surviving path is the complete one).

### Phase 4 — Cloud agent provisioning / orchestration (unlocks "spin up cloud-hosted")  · effort **L (largest)**
**Goal:** Plane can create/start/stop/destroy agent compute.
- New `provisioning/` subsystem with pluggable drivers (Kubernetes Job/Deployment, Fargate/ECS, plain Docker) behind a trait, mirroring the `storage/` pattern.
- Agent bootstrap tokens: pre-mint an org-scoped registration credential injected into the provisioned agent so it auto-registers (ties into `api/agents.rs` register).
- Lifecycle tables + API: `agent_instances`, `POST /orgs/{org}/agents/provision`, status reconciliation loop.
- Self-hosted path: formalize the bootstrap-token + install-command UX (agent already self-registers, so this is mostly credential issuance + docs).
- **Files:** new `provisioning/`, `api/provisioning.rs`, migration `007_*`, `domain/agent.rs`.
- **Deps:** Phases 1-3 (needs a coherent register + convergence + single surface).

### Phase 5 — Billing & quotas  · effort **M**
**Goal:** Monetize + enforce plan limits.
- Implement `billing/service.rs` against the `stripe` crate (customer, checkout, portal, webhook verification at `:157,197,228,320`).
- Add quota tables + enforcement middleware (agents/policies/decisions-per-month per plan), checked in relevant handlers and the ingestion path.
- **Files:** `billing/`, `api/billing.rs`, new `quota` middleware, migration `008_*`.
- **Deps:** Phase 2 (decision volume is the natural metered dimension).

---

## Risks & Unknowns

- **Optimistic rollout success is misleading today.** Until Phase 1's confirmation loop lands, the plane reports deployments as succeeded even if an agent is offline or the hot-swap failed (`deployment/service/helpers.rs:221-227`). Operators cannot trust convergence status.
- **SSE-only delivery + broadcast channel bound (1024, `state.rs:210`).** Under many agents/high event rate, `BroadcastStream` lag drops events (`api/events.rs:102,160` ignore lag). The poll fallback mitigates but the buffer size and lag semantics are unvalidated at fleet scale.
- **Decision-log volume is the scaling unknown.** Sub-microsecond agents can emit enormous decision streams; naive per-record ingestion into SQLite will not hold. Phase 2 storage choice (partitioning, object-store offload, sampling) is the critical design decision and is currently unspecified.
- **Two/three overlapping delivery components** (`reaper-agent/management/`, `reaper-sync`, `reaper-platform`) create ambiguity about the supported path; consolidating (Phase 3) before building more reduces divergence risk.
- **mTLS is off by default and proxy-header based** (`auth/middleware.rs:156`), i.e. it trusts a front proxy to terminate TLS and set the fingerprint header. For a SaaS trust boundary this needs an explicit, hardened deployment story.
- **Isolation is handler-enforced, not DB-enforced.** A single missing `org_id` guard in a new handler silently breaks tenant isolation; consider a repository-level org scoping or Postgres row-level security as the plane grows.
- **Namespaces vs orgs** as the tenancy unit is slightly muddled (agents are org-wide, `api/agents.rs:195`; subscriptions are namespace-scoped). Clarify before provisioning multiplies agents.
- Unverified at runtime: I did not compile/run the services; all findings are from source. The claimed endpoint alignment (agent `client.rs:232` `/bundles/promoted` ↔ plane `api/bundles.rs:74`) is by inspection.
