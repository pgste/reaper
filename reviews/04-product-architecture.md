# Reaper — Product Architecture & Enterprise-Readiness Review (Persona 4)

**Reviewer:** Product Architect (dev-infra → enterprise GA)
**Scope:** Feature set & system shape vs the *intended* product; control-plane journey; the Git link; distribution architecture; environments/promotion; multi-tenancy; data-fork lifecycle; audit-as-a-product; enterprise table stakes; competitive frame. UI out of scope. Review-only; no code modified.
**Method:** Read `reviews/00-repo-map.md`, the planning docs (`REAPER_MANAGEMENT_AUDIT.md`, `DATA_PLANE_PLAN.md`, `DECISION_LOG_PIPELINE.md`, `DATA_PROTECTION_PEP_DESIGN.md`, roadmap docs), then traced the real code: `services/reaper-management/src/{api,sync,deployment,oauth,decisions,audit}`, `services/reaper-agent/src/management/`, `crates/policy-engine/src/decision_log.rs`, `tools/reaper-cli`. Note: `REAPER_MANAGEMENT_AUDIT.md` is dated 2026-07-03 and several of its "missing" items have since shipped (confirmation loop, ClickHouse decision query, data plane D1/D2); I verified current code rather than trusting it.

---

## VERDICT: NOT READY

One P0 that fails a regulated/bank review outright, plus multiple P1s. The engine, distribution, and data-fork pillars are further along than the internal audit suggests — genuinely impressive fleet-grade rollout machinery — but the **control-plane identity story (SSO/SAML/OIDC + SCIM) does not exist**, and the advertised **"manage policy as code" (BYO Git) pillar is non-functional in production** (the sync engine is never wired into the running service). Both are adoption gates for the target buyer.

---

## Executive summary (≤10 lines)

1. **Distribution is the strongest pillar** — strategies (immediate/canary/percentage/label), waves, approval gates, agent-confirmed convergence, version pins, dry-run, auto-rollback on error-rate — all real and wired (`api/deployments/*`, `deployment/service/helpers.rs:224-256`). The July audit's "optimistic completion" gap is **fixed** (`helpers.rs:240 require_agent_confirmation`).
2. **Data fork (RBAC/ABAC/ReBAC) shipped to D1+D2** — typed model, materializer, snapshot + gap-proof delta sync, staleness budgets, decision provenance (`DATA_PLANE_PLAN.md §7-8`, `api/datastore.rs`, `decision_log.rs:69-82`). This is a real differentiator vs OPA/OpenFGA/Cerbos.
3. **Central decision audit query shipped** over ClickHouse, tenant-scoped (`api/decisions.rs`), fed off-path by a Vector sidecar (`DECISION_LOG_PIPELINE.md`). Capture is off the hot path. Solid.
4. **P0 — no SSO/SAML/OIDC and no SCIM** for the human control-plane. `grep scim|saml` → nothing. Login is password + GitHub OAuth only. Bank/regulated buyers gate on this.
5. **P1 — "policy as code" is a demo.** `GitSyncer` works in isolation but `SyncService` is instantiated only in a test (`sync/service.rs:333`); `main.rs` never spawns it; the manual trigger is a `TODO` no-op (`api/sources.rs:381`). BYO-Git ingest does not run.
6. **P1 — GitHub link is user-OAuth-token, broad `repo` scope, no GitHub App, no webhook, poll-only, no GitLab handler** (config exists, handler absent). No PR-based promotion, no drift detection, no UI↔git conflict model.
7. **P1 — environments are namespaces-by-convention**, not a first-class env object with per-env data planes and promotion gates between envs.
8. **P1 — no control-plane DR/HA/backup story**; single Postgres, no documented RPO/RTO, no fleet-upgrade runbook.
9. **P2 — no OpenAPI/spec** for a product whose thesis is "API-driven"; no cloud provisioning ("spin up a reaper"); decision **replay is reproduction-only**, not counterfactual ("decide under policy vX").
10. Net: the enforcement + distribution + data + audit *machinery* is ~70% of a GA product; the *enterprise control-plane wrapper* (identity, GitOps, DR, envs) is ~30%.

---

## Findings table

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| F1 | **P0** | No SSO (SAML/OIDC) and no SCIM for the human control plane; admin identity is password + GitHub OAuth only | `grep -rin scim\|saml\|"single sign"` over `services/`,`crates/` → 0 hits; `auth/users/`, `api/oauth/github.rs` only |
| F2 | **P1** | Policy-as-code (BYO Git) non-functional in production: sync engine never wired | `sync/service.rs:333` (SyncService only in test), `main.rs:144-173` (spawns only change-log sweeper), `api/sources.rs:381` `TODO: Actually trigger the sync` |
| F3 | **P1** | Git link is user-token OAuth (broad `repo` scope), no GitHub App, no webhook, poll-only; no GitLab/Bitbucket handler | `api/oauth/github.rs:70` `scope=repo`, `:302` token embedded in clone URL; `config/oauth.rs:26` GitLab config but `api/oauth/` has only `github.rs` |
| F4 | **P1** | No first-class environment object; dev→staging→prod is namespace slugs by convention; no cross-env promotion or per-env gate | `db/repositories/namespace.rs`, `api/namespaces.rs`; no `Environment` type; promotion is bundle-status + rollout, not env→env |
| F5 | **P1** | No control-plane HA/DR/backup posture; single Postgres, no RPO/RTO, no fleet-upgrade-without-downtime runbook | No DR doc in `docs/deployment/`; `db/connection.rs` single pool; Helm ships one PG |
| F6 | **P1** | Data-fork has no model-migration engine: renaming a role / adding a relation / changing an attribute type has no defined path for existing records | `domain/datastore.rs` (model versioned, no ALTER/rename/migrate); `DATA_PLANE_PLAN.md §6` lists "temporal versioning & point-in-time restore" as "Later" |
| F7 | **P2** | Decision replay is reproduction-only (needs explain tier on), not counterfactual ("what would vX decide?") | `decision_log.rs:11-83`: `input_data` is opt-in, principal/resource attrs only; no full-request capture, no replay engine |
| F8 | **P2** | No cloud provisioning ("spin up a reaper"); only registration of already-running agents | `api/agents.rs` register/heartbeat; no `provisioning/`, terraform, k8s-Job driver (audit §4 confirmed, still true) |
| F9 | **P2** | No OpenAPI/Swagger for an "API-driven" product; no published SDK for the management API | `grep openapi\|utoipa\|swagger` → 0; repo map §API confirms |
| F10 | **P2** | Audit export connectors are single-path (Vector→ClickHouse/S3); no native Kafka/Splunk-HEC/SIEM-CEF/OCSF emitter, no retention/legal-hold API on the query plane | `DECISION_LOG_PIPELINE.md` (Vector-only shipper); `api/decisions.rs` has no export/retention route |
| F11 | **P2** | Three overlapping surfaces still shipped (`reaper-platform`, `reaper-sync`, in-agent client); `reaper-platform` still in `docker-compose.yml:106` and Helm | audit §8; `docker-compose.yml:106`, `deploy/helm/reaper/values.yaml:105` |
| F12 | **P3** | CLI is developer/CI-only (eval/compile/validate/check/keygen); no GitOps/fleet/env/promote verbs; CLAUDE.md advertises `policy`/`status` commands not in the enum | `tools/reaper-cli/src/main.rs:64-180` |
| F13 | **P3** | Multi-tenancy has orgs→namespaces but no project/workspace tier, and isolation is handler-enforced not DB-enforced (no row-level security); no quotas/noisy-neighbor controls | audit §3, §7; `billing/service.rs` all TODO |

---

## Detailed findings

### F1 (P0) — No SSO/SCIM for the control plane
The target buyer (regulated enterprise) will send a security questionnaire whose first page requires SAML 2.0 or OIDC SSO for the admin console and SCIM 2.0 for user lifecycle (deprovision-on-termination). Reaper has neither. Human auth is local password (`auth/users/`) plus GitHub OAuth *for source connection* (`api/oauth/github.rs`), and the multi-method auth in `auth/middleware.rs` targets **agents/machines** (API key, mTLS-via-proxy-header, shared-secret JWT, external JWKS), not interactive human SSO. External JWKS validates agent tokens, not a login flow. There is a real management-action audit log (`audit/actions` covers user/org/agent/bundle/rollout/apikey/namespace/team/webhook/jwks — good, F-positive), but an audit log without SSO/SCIM means you can't tie management actions to a corporate identity or guarantee deprovisioning. **This alone fails a bank review.**

### F2 (P1) — "Manage policy as code" is not actually running
This is the most important product finding because it undercuts a named pillar ("manageable as code AND via UI"). The code exists and is well-factored: `GitSyncer::sync` clones/pulls, checks out a branch, globs policy files, detects language (`sync/git.rs:39-215`); `SyncService` fans Git/S3/API/BundleUrl syncers (`sync/service.rs:202-205`). **But nothing invokes it in production.** `SyncService::new` appears only inside `#[cfg(test)]` (`sync/service.rs:333`). `main.rs` (`:144-173`) spawns exactly one background task — a change-log retention sweeper — not a source-sync scheduler. The one API entry point, `POST /orgs/{org}/sources/{id}/sync`, flips status to `Syncing` and returns a placeholder: `// TODO: Actually trigger the sync via SyncService` (`api/sources.rs:377-388`). So a customer can create a Git source, connect GitHub, and it will never pull. GitOps is a compiled-but-dead capability.

### F3 (P1) — The Git link is the wrong shape for enterprise
Even once wired, the integration is built as **personal OAuth**, not an app:
- `scope=repo` (`github.rs:70`) is broad read/write to *all* the connecting user's repos; enterprises require fine-grained, per-repo, org-approved **GitHub App** installations.
- The token is the *individual user's* — it dies when they leave the company, and it's embedded directly in the clone URL (`github.rs:302 https://x-access-token:{token}@...`), so revocation orphans every source.
- **Poll-only** (`config.poll_interval_seconds: 300`, `github.rs:320`); no webhook, so no push-on-commit and (given F2) no polling either.
- **No GitLab/Bitbucket handler** despite `GitLabOAuthConfig`/`BitbucketOAuthConfig` in `config/oauth.rs:26` — config surface without an implementation.
- **One-way ingest only.** Git → plane. There is no UI-write-becomes-a-commit path, so the moment someone edits a policy in the (future) UI, git and deployed state diverge with no reconciliation. No drift detection, no PR-based promotion, no branch-per-env vs dir-per-env decision anywhere in code or docs.

### F4 (P1) — Environments are a naming convention, not a concept
Namespaces carry slugs like `production`/`staging` (`db/repositories/namespace.rs:547`, `deployment/service/mod.rs:770 {"env":"production"}`) and strategies/rollouts/data planes are namespace-scoped — so you *can* approximate environments. But there is no `Environment` domain type, no notion that a bundle is "promoted from staging to prod," no per-environment approval policy, and no change-window/freeze concept. Promotion today is: compile bundle → set status Promoted → start a rollout to agents in a namespace. That's binary publish, not staged environment promotion. Regulated change management (dev→stage→prod with distinct approvers and a change record per transition) is not modeled.

### F5 (P1) — No DR/HA/backup story for the control plane
The control plane is a single service over a single Postgres (or SQLite dev). There is no documented HA topology, no failover, no backup/restore runbook, no stated RPO/RTO, and no story for upgrading the fleet's control plane without an availability gap. The *agents* fail safe if the plane is down (they keep serving the last bundle, poll fallback exists), which is good — but the plane itself holding policy source-of-truth, audit query, and data-plane change log is a single point of failure with no described recovery. For a compliance question like "restore last Tuesday's policy set," there is no point-in-time restore.

### F6 (P1) — Data-fork model migration is unspecified
`DATA_PLANE_PLAN.md` is honest that the model is versioned and materialization is deterministic, but it has **no migration engine**. Real questions with no answer in code: rename role `editor`→`author` — what happens to existing role-bindings? Change `clearance` from `string` to `int` — are existing entity values coerced, rejected, or silently broken? Add a required relation — is existing data backfilled? The plan defers "temporal versioning & point-in-time restore" to "Later" (§6). For an authorization data plane this is load-bearing: a botched model change is a mass allow/deny incident. Backup/restore and test-data forks of the datastore are also absent.

### F7 (P2) — Replay is reproduction, not counterfactual
The decision record (`decision_log.rs:11-83`) is rich — principal/action/resource/context/decision/policy_id/policy_version/matched_rule, plus `data_version`+`data_checksum` provenance (excellent for "what data did this see"). With the opt-in explain tier, `input_data` snapshots the resolved principal/resource attributes, making a decision *reproducible in place*. But: (a) explain is off by default and denies-only, so most records can't be replayed; (b) `input_data` is only the two entities the rule branched on, not the full request/data graph; (c) there is no replay engine and no "evaluate this historical request under a *different* policy/data version" API. So the compelling enterprise question — "if we ship policy v7, how many of last month's allows flip to deny?" — is not answerable. The provenance fields make this *buildable*, but it isn't built.

### F8–F13
- **F8 (P2)** No provisioning; "spin up a cloud-hosted reaper" is register-only.
- **F9 (P2)** No OpenAPI. For an API-first product this blocks SDK generation, partner integration, and API governance review.
- **F10 (P2)** Audit egress is Vector→ClickHouse+S3 only. No native connectors customers ask for by name (Kafka topic, Splunk HEC, SIEM CEF/OCSF normalization), and the query plane has no retention/legal-hold/purge-by-subject (GDPR) API.
- **F11 (P2)** Consolidation debt: `reaper-platform` (in-memory, placeholder handlers) and `reaper-sync` (mis-wired to the flat routes) still ship in compose/Helm, creating a "which one is supported?" hazard.
- **F12 (P3)** CLI is CI/dev-focused; no `reaper policy pull/push`, `reaper env promote`, `reaper fleet status`. CLAUDE.md documents `policy`/`status`/`benchmark` verbs the CLI enum (`main.rs:64`) does not contain.
- **F13 (P3)** Tenancy has no project/workspace layer; isolation is handler-enforced (a single missing `org_id` guard leaks cross-tenant — audit §3); no quotas (`billing/*` all TODO).

---

## Absence checks (where I looked and found nothing)

- **SSO/SCIM:** `grep -rin "scim|saml|single sign"` over `services/`,`crates/` → 0. No SAML/OIDC login route in `api/auth`/`api/oauth`.
- **OpenAPI/spec:** `grep openapi|utoipa|swagger` → 0 (matches repo map §API).
- **Cloud provisioning:** no `provisioning/`, no terraform/k8s-Job/Fargate driver in `services/reaper-management/src/`.
- **Env object:** no `Environment` enum/type; only namespace slugs.
- **Data-plane migration:** no `migrate`/`alter`/`rename` handling in `domain/datastore.rs`; no PITR.
- **Wired source sync:** `SyncService` referenced only in test + module re-export; not in `main.rs`.
- **GitLab handler:** config type exists (`config/oauth.rs`), no handler file in `api/oauth/`.
- **Replay engine / counterfactual eval API:** none in `api/decisions.rs` or agent handlers.

---

## What's done well (≤5)

1. **Fleet-grade distribution is real and confirmation-driven.** Strategies + waves + approval gates + agent-acknowledged convergence + version pins + dry-run + auto-rollback (`api/deployments/*`, `deployment/service/helpers.rs:224-256`, `types.rs:299-408`). The July audit's optimistic-completion footgun is fixed.
2. **Data plane (D1+D2) genuinely closes the OPA "data is your problem" gap** — one typed model spanning RBAC+ABAC+ReBAC, deterministic materialize, snapshot+gap-proof delta sync, staleness budgets with fail-closed option, decision provenance (`DATA_PLANE_PLAN.md §7-8`).
3. **Decision capture is correctly off the eval path** (sharded lock-free ring, deny-priority sampling, redaction/encryption at capture) and the central query API over ClickHouse is tenant-scoped and injection-safe (`DECISION_LOG_PIPELINE.md`, `api/decisions.rs`).
4. **Management-action audit log is comprehensive** — 40+ typed actions across every mutation surface (`audit/actions`), with actor/IP/UA — the compliance substrate is present even if SSO to feed it is not.
5. **Honest, high-quality internal design docs** that name their own gaps (staleness modes, consistency posture, "exactly-once is a myth") — rare and valuable.

---

# Required Addition 1 — Gap Register

| Gap | Why it blocks enterprise adoption | Proposed solution | Build/Buy/Integrate | Effort | Priority |
|-----|-----------------------------------|-------------------|---------------------|--------|----------|
| No SSO (SAML/OIDC) | Security questionnaire hard-gate; can't map actions to corp identity | Add OIDC (Authorization Code+PKCE) then SAML; front with an IdP-agnostic session broker | Integrate (`openidconnect` crate / WorkOS/Auth0 for speed) | M | **P0** |
| No SCIM | Can't guarantee deprovision-on-termination | SCIM 2.0 Users/Groups endpoints mapping to `users`/`user_orgs` | Build (or WorkOS Directory Sync) | M | **P0** |
| GitOps not wired | Named "as code" pillar doesn't run | Spawn `SyncService` scheduler in `main.rs`; implement `trigger_sync`; add reconciliation loop | Build (code mostly exists) | S | **P1** |
| Git link shape | User-token/broad-scope/poll-only won't pass repo-access review | GitHub App install + webhook push + fine-grained perms; add GitLab handler | Build | M | **P1** |
| No environment model | Regulated promotion needs env→env gates + change records | First-class `Environment` over namespaces with promotion API + approvals | Build | M | **P1** |
| No control-plane DR/HA | Single point of failure over source-of-truth | Postgres HA (managed/replica), documented RPO/RTO, backup/PITR, upgrade runbook | Integrate (managed PG) + Build (runbook) | M | **P1** |
| No data-model migration | Model change = mass authz incident | Migration engine: typed transforms, dry-run diff, backfill, versioned model with rollback | Build | L | **P1** |
| No decision replay | Can't answer "impact of policy vX" | Replay service over the decision store + engine, using stored provenance | Build | M | **P2** |
| No cloud provisioning | "Spin up a reaper" is manual | Pluggable provisioner (k8s Job/Fargate/Docker) + bootstrap tokens | Build | L | **P2** |
| No OpenAPI/SDK | API governance + partner integration blocked | Annotate with `utoipa`, generate spec + typed SDKs | Build | S | **P2** |
| Audit connectors | SIEM teams want native Kafka/Splunk/CEF/OCSF + retention API | Vector sink configs + OCSF mapping + retention/legal-hold API | Integrate (Vector) + Build (API) | M | **P2** |
| Surface sprawl | "Which service is supported?" risk | Retire `reaper-platform`/`reaper-sync`; fold dev-mode into agent/management | Build (deletion) | S | **P3** |

---

# Required Addition 2 — Proposed tooling mini-designs

### D-A. Control-plane identity: OIDC/SAML SSO + SCIM (closes F1 / P0)
**Goal:** Corporate SSO login to the admin console and automated user lifecycle, so every management action ties to a governed identity and terminated employees lose access automatically.
**API sketch:**
- `GET /auth/sso/{org}/start` → redirect to IdP (OIDC Auth Code + PKCE; SAML AuthnRequest variant).
- `GET|POST /auth/sso/{org}/callback` → validate assertion, JIT-provision `users`+`user_orgs`, mint existing `rst_` session.
- `GET /orgs/{org}/sso/config`, `PUT ...` → per-org IdP metadata (issuer, JWKS/cert, entity-id, attribute map).
- SCIM: `/scim/v2/Users`, `/scim/v2/Groups` (bearer = per-org SCIM token) → CRUD into `users`/`user_orgs`, group→role mapping.
**Data-model touchpoints:** reuse `users`, `user_orgs`, `sessions`, `audit_log`; add `sso_configs`, `scim_tokens`. New audit actions `sso.login`, `scim.user_provision`, `scim.user_deprovision`.
**Composes with:** slots beside existing `auth/middleware.rs` methods; `RequireAuth` unchanged (still validates `rst_` sessions). Org-scoped like everything else.
**ADR trade-off:** *Build in-house `openidconnect`+`samlassertion`* (no per-seat vendor cost, full control, ~M-L effort, you own IdP-quirk hell) **vs integrate WorkOS/Auth0** (SSO+SCIM in days, passes questionnaires immediately, adds a vendor in the identity trust path + per-connection cost). **Recommend:** integrate WorkOS to unblock the P0 for design partners now; keep the endpoint shape provider-agnostic so in-house OIDC can replace it later without an API break.

### D-B. GitOps sync engine + drift/conflict model (closes F2/F3 — P1)
**Goal:** Make "policy as code" real: continuous, secure, bi-directional-aware sync between a customer Git repo and deployed bundles, with drift detection and a defined git↔UI conflict rule.
**API sketch:**
- Wire `SyncService` scheduler in `main.rs`; implement `POST /orgs/{org}/sources/{id}/sync` to call it.
- `POST /webhooks/git/{provider}` → verify signature, enqueue sync for the matching source (push-based; poll as fallback).
- `GET /orgs/{org}/sources/{id}/drift` → diff(git HEAD ↔ last-materialized bundle) → `{in_sync|drift, added/changed/removed}`.
- GitHub App: `GET /orgs/{org}/git/install` (App install URL), store installation-id not user token.
**Sync engine spec:** reconciliation loop (default 60s) *and* webhook push; both converge on the same idempotent "materialize repo@sha → bundle" step keyed by commit SHA (no double-apply). Branch-per-env (`main`→prod, `staging`→staging) as the default mapping, dir-per-env as an option in source config.
**Conflict rule (the ADR):** **Git is source of truth; UI writes become commits.** A UI policy edit opens a branch + commit + (optionally) a PR via the App, rather than mutating deployed state directly — so there is exactly one lineage and drift is structurally impossible. *Alternative:* last-writer-wins with drift alerts (simpler, but guarantees eventual divergence and is unauditable). **Recommend the commit-back model**; it also gives regulated buyers PR-based approval for free.
**Data-model touchpoints:** `sources` (add `provider`, `installation_id`, `env_mapping`, `last_synced_sha`), new `source_syncs` history table, reuse `audit_log`.
**Composes with:** feeds the existing bundle compile → rollout pipeline unchanged; drift surfaces on the existing SSE stream and landscape.

### D-C. Environment & promotion model with approval gates (closes F4 — P1)
**Goal:** First-class dev→staging→prod with per-env data planes, promotion between envs, and approver/change-window gates — the regulated change-management spine.
**API sketch:**
- `CRUD /orgs/{org}/environments` (name, tier, namespace binding, approval policy, change windows).
- `POST /orgs/{org}/environments/{env}/promote` `{bundle_id, from_env}` → creates a **change request** (bundle+data version pinned), evaluates approval policy, then triggers the existing rollout on success.
- `POST /orgs/{org}/change-requests/{id}/approve|reject`.
**Data-model touchpoints:** new `environments`, `change_requests`, `approvals`; environments wrap the existing `namespaces` (thin — namespaces already scope agents/strategies/data). New audit actions `env.promote`, `change_request.approve`.
**Composes with:** promotion terminates in the *existing* rollout+strategy machinery — this is a governance layer on top, not a rewrite. Reuses strategy `require_approval` (`helpers.rs:288-296`) at the env boundary.
**ADR:** model environments as a new object *vs* keep overloading namespaces. **Recommend new object** — namespaces stay the deployment/data-scope primitive; environments add lifecycle/approval semantics without muddying the tenancy unit (addresses audit's "namespaces vs orgs muddle").

### D-D. Fleet inventory & version reporting (hardening the strong pillar)
**Goal:** Answer "what version is every reaper running *right now*, and is it converged?" — a compliance question — as a first-class inventory, not an inference from metrics.
**API sketch:**
- `GET /orgs/{org}/fleet` → per-agent `{agent_id, env, active_bundle_id, active_data_version, target_bundle_id, drift: bool, last_seen, health}`.
- `GET /orgs/{org}/fleet/versions` → histogram of active bundle/data versions across the fleet (converged? stragglers?).
- Offline/air-gapped: signed bundle export + `reaper bundle import` for sneakernet, with the agent reporting active checksum on next reconnect.
**Data-model touchpoints:** mostly a read model over existing `agent_deployments` (already confirmed via `deployment/acknowledge`), `agents`, version pins, and data-plane `applied_seq`. Add `active_bundle_id`/`active_data_version` columns denormalized on `agents` from the ack callback.
**Composes with:** the confirmation loop already populates truth; this exposes it as inventory. Incremental/delta bundle updates already exist (data plane); extend the same delta idea to policy bundles for large fleets.

### D-E. Decision replay & audit export connectors (closes F7/F10 — P2)
**Goal:** "What would policy vX / data vY decide for last month's traffic?" plus native SIEM egress.
**API sketch:**
- `POST /orgs/{org}/replay` `{time_range, filter, policy_version|bundle_id, data_version}` → streams historical decisions re-evaluated against the specified policy/data, returns a diff summary (flips allow↔deny, counts, sample records).
- `GET /orgs/{org}/decisions/export` `{format: ndjson|cef|ocsf, sink: s3|kafka|splunk}`; retention/legal-hold: `PUT /orgs/{org}/audit/retention`, `POST /orgs/{org}/audit/legal-hold`.
**Data-model touchpoints:** replay needs the *full* input, so extend capture: an opt-in "replayable" tier that stores the complete request + data-version reference (not just the 2-entity explain snapshot). Reuse `data_version`/`data_checksum` already on `DecisionLogEntry` to fetch the exact datastore snapshot. Export = Vector sink configs + an OCSF field-mapping table.
**Composes with:** replay = ClickHouse (source rows) + a headless `PolicyEngine` loaded with the historical bundle+data snapshot (both already versioned/checksummed and stored). The provenance fields make this *buildable today*; the only new capture cost is full-input storage for the replayable tier.
**ADR:** full-input capture cost vs replay fidelity — offer it as a per-namespace tier (sampled or denies-only) so the sub-µs hot path and storage aren't taxed org-wide.

---

# Required Addition 3 — Sequenced roadmap to "deployable in a regulated enterprise"

**#1 most important next move: ship control-plane SSO + SCIM (D-A).** It is the single hard gate that turns "interesting" into "evaluable" for the target buyer, and it activates the management audit log you already have. Integrate WorkOS to get there in weeks; keep the API provider-agnostic.

1. **D-A — SSO + SCIM (P0).** Unblocks the security review. Prereq to everything sold to a bank.
2. **D-B — Wire + reshape GitOps (P1).** Two parts: (a) *this week* — spawn `SyncService` and implement `trigger_sync` so the existing engine runs (S, unblocks the "as code" claim); (b) then GitHub App + webhook + commit-back conflict model + GitLab (M). Restores a named pillar.
3. **D-C — Environments + promotion + approvals (P1).** Layers regulated change management on the already-strong rollout machinery. Cheap because rollout/approval primitives exist.
4. **Control-plane DR/HA/backup (F5, P1).** Managed-Postgres HA topology, documented RPO/RTO, PITR, fleet-upgrade runbook. Mostly ops + docs, but a compliance blocker.
5. **D-D — Fleet inventory (P1-hardening).** Expose the convergence truth you already capture as an auditable "which version is everything running" inventory.
6. **F6 — Data-model migration engine (P1).** Before customers put production authz data in the data plane at scale, they need a safe way to evolve the model.
7. **D-E — Replay + audit export connectors (P2).** High-value differentiators once the table stakes are in.
8. **Consolidation + OpenAPI + provisioning (F8/F9/F11, P2-P3).** Retire `reaper-platform`/`reaper-sync`, publish the API spec/SDK, add cloud provisioning for the fully-managed offering.

**Competitive frame (blunt):** Reaper's *combination* — a fast DSL engine + a managed multi-model (RBAC/ABAC/ReBAC) data plane + off-path decision audit + confirmation-driven fleet distribution — is a genuinely differentiated position that **no single competitor holds**: OPA+OPAL make data your problem and bolt sync on; OpenFGA/SpiceDB give tuples but no attributes/roles and need a second system for ABAC; Cerbos is stateless (your services become the data plane); Cedar/AVP pass entities per-request with no fleet sync. Reaper *wins on the loop* (managed data → nanosecond enforcement → central audit). It is **weaker than all of them on the enterprise wrapper**: OPA/Styra DAS, SpiceDB, and AVP all ship SSO, mature Git/CI integration, and provisioning that Reaper lacks. The engine and data story are ahead of GA; the control-plane wrapper (identity, GitOps, environments, DR) is what's between here and a regulated deployment — and it's the smaller build.

---

## Coverage statement — what I did and did not cover

**Covered:** the full control-plane journey (org→source→data/policy model→deploy→propagate) traced through real routes; the Git link (OAuth/sync/webhook/conflict); distribution architecture (SSE + poll + strategies + confirmation + rollback + pins); environments/promotion; multi-tenancy shape; data-fork lifecycle & migration; audit-as-a-product (capture→ship→query→replay→export); enterprise table stakes (SSO/SCIM/DR/management-audit/OpenAPI); competitive positioning.

**Not covered (other personas / out of scope):** eval-engine internals, DSL semantics, and hot-path performance (Persona perf/eng); deep security soundness of auth/mTLS/`unsafe`/injection (Persona security) — I relied on the repo map and audit for those and only touched auth where it intersects product identity; UI (explicitly out of scope); I did not compile or run the services — all findings are from source inspection at current HEAD, cross-checked against the dated internal audit.
