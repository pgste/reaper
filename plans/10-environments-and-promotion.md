# Environments & Promotion

**Readiness gate:** NOT READY → CONDITIONAL (adds the regulated change-management spine on top of the already-strong rollout machinery)
**Priority:** P1 (cheap relative to value — rollout/approval primitives already exist; this is a governance layer, not a rewrite)
**Findings closed:** Product **F4** (no first-class environment object; dev→staging→prod is namespace slugs by convention; promotion is bundle-status + rollout, not env→env; no approval gates / change records / windows)

---

## 1. Goal

Introduce a first-class **`Environment`** object layered over the existing `namespaces` primitive, plus an **env→env promotion API** with regulated change-management gates:

1. **Environments** carry lifecycle/ordering semantics (dev < staging < prod), a namespace binding, an approval policy, and change windows/freezes — namespaces stay the deployment/data-scope unit.
2. **Promotion** is a governed transition: `promote(bundle, from_env → to_env)` creates a **change request** (bundle + data version pinned), evaluates the approval policy, and only on approval triggers the **existing** rollout+strategy machinery into the target env's namespace.
3. **Approval gates:** two-person (N-of-M distinct approvers), mandatory change record, and scheduled change windows / freeze periods.
4. **Per-env data planes** so a promotion carries the right entity/relationship data version, not just policy.
5. **Compose, don't replace:** promotion terminates in the current rollout strategies (immediate/canary/percentage/label) and the current SSE distribution flow.

This reuses mini-design **D-C** in `reviews/04-product-architecture.md:155-163`.

---

## 2. Current state (evidence) — file:line

- **No `Environment` type.** `grep` for an environment domain object finds nothing; the only lifecycle signal is a namespace **slug** by convention (`production`, `staging`) — see `domain/namespace.rs:15-33` (`Namespace` has `slug`, `settings: serde_json::Value`, `parent_id`, `is_active` — no tier/order/approval/env fields) and the slug-validation tests using `production`/`staging` (`api/namespaces.rs:610-626`, `domain/namespace.rs:248-254`).
- **Namespaces are the scope unit for everything.** Strategies, rollouts, subscriptions, and data planes are namespace-scoped: `DeploymentStrategy.namespace_id` (`domain/deployment.rs:57`), `Rollout.namespace_id` (`api/deployments/types.rs:111`), agent subscriptions per namespace (`api/namespaces.rs:481-539`), rollback per namespace (`api/deployments/rollouts.rs:227-275`). So a namespace *approximates* an environment but carries no promotion semantics.
- **Promotion today is binary publish + rollout, not env→env.** A bundle is compiled, its status set to Promoted, then a rollout is started to agents in a namespace via `StartRollout { bundle_id, strategy_id, namespace_id }` (`api/deployments/rollouts.rs:23-97`). There is no notion of "bundle X promoted **from staging to prod**"; nothing records a from→to transition.
- **Approval exists only *within* a rollout's waves, not at an env boundary.** `StrategyConfig::Canary`/`Percentage` carry `require_approval` (`domain/deployment.rs:79,88`); a rollout can enter `RolloutStatus::AwaitingApproval` (`domain/deployment.rs:114`) and be advanced by `approve_wave` (`api/deployments/rollouts.rs:162-191`), gated in `deployment/service/helpers.rs:284-308`. This is *intra-rollout wave* approval — single approver, no distinct-approver/two-person rule, no change record, no window. There is no gate on *entering* an environment.
- **No change record / change window / freeze.** No `change_requests`, `approvals`, or freeze-window tables in migrations (`db/migrations/` runs to `009_change_log_retention.sql`); no scheduled-window logic anywhere in `deployment/` or `api/deployments/`.
- **Namespaces vs orgs muddle (context).** The July audit flagged namespace/org overloading; F4 recommends *not* overloading namespaces further with lifecycle semantics — hence a new object.

---

## 3. Definition of Done — testable checkboxes

- [ ] `CRUD /orgs/{org}/environments` exists (name, tier/order, `namespace_id` binding, `approval_policy`, `change_windows`), org-scoped with the same `RequireAuth` + `user.org_id == org.id` pattern used in `api/namespaces.rs`.
- [ ] An environment binds exactly one namespace; two environments cannot bind the same namespace (unique constraint), and an env cannot promote to a lower-or-equal tier (ordering enforced).
- [ ] `POST /orgs/{org}/environments/{env}/promote {bundle_id, from_env}` creates a **change request** with the bundle **and** data version pinned, and does **not** start a rollout until the approval policy is satisfied.
- [ ] `POST /orgs/{org}/change-requests/{id}/approve|reject` records approver identity; a two-person policy requires **N distinct** approvers (the requester cannot self-approve to meet the minimum) before the change request becomes `Approved`.
- [ ] A promotion attempted **inside a freeze window** is rejected (or queued to window end, per policy) with an auditable reason; outside the window it proceeds.
- [ ] On approval, promotion invokes the **existing** `DeploymentService::start_rollout` (`api/deployments/rollouts.rs:66-87`) into the target env's namespace using the env's configured strategy — no new rollout engine.
- [ ] The pinned **data version** is carried into the target env's data plane so the promoted bundle enforces against the intended entity/relationship snapshot.
- [ ] Every transition emits audit actions `env.promote`, `change_request.create`, `change_request.approve`, `change_request.reject` (via `audit/actions`), each tying to a governed identity.
- [ ] Canary/percentage/label strategies still function unchanged when reached through a promotion (regression: an existing rollout test passes when driven via the promote path).
- [ ] `GET /orgs/{org}/change-requests` lists change records with from_env, to_env, bundle, data version, approvers, status, timestamps — a defensible change-management trail.

---

## 4. Critical steps — ordered

Each step: **what / where(files) / verify / schema**.

### Step 1 — `Environment` domain object over namespaces (M)
- **What:** New domain type `Environment { id, org_id, name, tier_order: i32, namespace_id, data_plane_ref, approval_policy: serde_json, is_active }`. Environments wrap namespaces (thin): the namespace stays the scope for agents/strategies/data; the environment adds lifecycle + approval semantics. `tier_order` gives dev<staging<prod ordering for "can only promote upward".
- **Where:** new `domain/environment.rs` (mirror the shape/patterns of `domain/namespace.rs:15-93`); repository `db/repositories/environment.rs` (mirror `NamespaceRepository`).
- **Verify:** unit tests for ordering (`can_promote_to(other)` iff `other.tier_order > self.tier_order`) and namespace-binding uniqueness.
- **Schema:** migration **012_environments.sql** — `environments (id UUID PK, org_id UUID FK, name TEXT, tier_order INT, namespace_id UUID FK UNIQUE, data_plane_ref TEXT NULL, approval_policy JSONB, change_windows JSONB, is_active BOOL, created_at, updated_at)`; unique `(org_id, name)` and unique `namespace_id`.

### Step 2 — Environment CRUD API (S)
- **What:** `GET/POST /orgs/{org}/environments`, `GET/PUT/DELETE /orgs/{org}/environments/{env}`. Reuse the exact auth+scope+org-resolution pattern from `api/namespaces.rs` (`RequireAuth`, `resolve_org`, `user.org_id != org.id && !Admin → Forbidden`, `Scope::PolicyRead`/`PolicyWrite`).
- **Where:** new `api/environments.rs`; register in `api/mod.rs` router build alongside `namespaces::routes()`.
- **Verify:** create dev/staging/prod bound to three namespaces; cross-org access returns 403/404; duplicate namespace binding returns 409.
- **Schema:** uses 012.

### Step 3 — Approval policy + change-window model (M)
- **What:** Define `ApprovalPolicy { min_approvers: u8, distinct_from_requester: bool, required_scopes: Vec<Scope>, allow_self_approve: bool }` and `ChangeWindow { cron/rrule or weekly windows, freeze_periods: Vec<{start,end,reason}> }`, both stored as JSON on `environments` (Step 1) so policy is per-target-env. Add a `windows` evaluator (`is_change_allowed(now) -> Allowed | InFreeze | OutsideWindow`).
- **Where:** `domain/environment.rs` (policy structs + evaluator); reuse `auth/scopes.rs` `Scope` for `required_scopes`.
- **Verify:** unit tests: 2-of-N with distinct-requester; freeze period rejects; weekly window allows/blocks by timestamp.
- **Schema:** JSON fields on 012 (no new table needed for policy; change records are Step 5).

### Step 4 — Change request + approvals data model (M)
- **What:** A `change_request` captures a pending env→env promotion: `{id, org_id, from_env_id, to_env_id, bundle_id, data_version, requested_by, status: pending|approved|rejected|applied|cancelled, strategy_id, created_at, decided_at}`. `approval` rows record each approver decision `{id, change_request_id, approver_id, decision, reason, created_at}`.
- **Where:** new `domain/change_request.rs`, `db/repositories/change_request.rs`.
- **Verify:** repository tests: create → 1 approval (insufficient) → 2nd distinct approval → status flips to `approved`; requester self-approval blocked when `distinct_from_requester`.
- **Schema:** migration **013_change_requests.sql** — `change_requests` and `approvals` tables with FKs to `environments`, `bundles`; index `(org_id, status)`.

### Step 5 — Promotion API that creates a change request (M)
- **What:** `POST /orgs/{org}/environments/{env}/promote {bundle_id, from_env, strategy_id?}`: validate `from_env.tier_order < to_env.tier_order` (upward-only), validate the change window (Step 3), pin the bundle **and** the source env's current data version, create a `change_request` (Step 4) in `pending`. Return the change request — **no rollout yet**.
- **Where:** new handler in `api/environments.rs`; reuse `resolve_org`; look up target env's `namespace_id` and default strategy.
- **Verify:** promote staging→prod returns a pending change request with `data_version` populated; promote prod→dev (downward) → 400; promote during freeze → 409/queued.
- **Schema:** uses 012/013.

### Step 6 — Approve/reject → trigger existing rollout (M)
- **What:** `POST /orgs/{org}/change-requests/{id}/approve|reject`. On reaching the approval threshold, transition to `approved` and invoke the **existing** rollout: build `StartRollout { bundle_id, strategy_id: env.default_strategy, namespace_id: to_env.namespace_id }` and call `DeploymentService::start_rollout(org, &input, &state)` (`api/deployments/rollouts.rs:66-87`, service in `deployment/`). Mark the change request `applied` when the rollout starts.
- **Where:** new handlers in `api/environments.rs`; reuse `DeploymentService` (`deployment/service/`), the wave/approval and confirmation loop stay as-is (`deployment/service/helpers.rs:240,284-308`).
- **Verify:** two distinct approvals → a real rollout appears in `GET /orgs/{org}/rollouts` targeting prod's namespace with the env's strategy; the promoted canary strategy still enters `AwaitingApproval` per wave (intra-rollout gate unchanged).
- **Schema:** uses 013 (`status` transitions).

### Step 7 — Per-env data plane binding (M)
- **What:** Ensure the pinned `data_version` from `from_env` is applied to `to_env`'s data plane as part of promotion, so policy + data move together. Bind via `environments.data_plane_ref`; on `applied`, coordinate the target namespace's data-plane version alongside the bundle rollout (data plane is already versioned/delta-synced per `DATA_PLANE_PLAN.md`).
- **Where:** `domain/environment.rs` (`data_plane_ref`); promotion apply step in `api/environments.rs` wiring to the existing data-plane version/deploy path (agent `/api/v1/data/deploy-version`, mgmt datastore repo).
- **Verify:** promoting a bundle that reads relationship data enforces against the source env's data snapshot, not prod's stale data; E2E decision reflects the promoted data version.
- **Schema:** `data_plane_ref` on 012; reuse existing data-plane version tables.

### Step 8 — Audit + change-record listing (S)
- **What:** Emit `env.promote`, `change_request.create/approve/reject` audit actions; expose `GET /orgs/{org}/change-requests` (+ `/{id}`) for the auditable trail (from_env, to_env, bundle, data_version, approvers, timestamps, resulting rollout id).
- **Where:** `audit/actions` (add action constants); `api/environments.rs` list/get handlers.
- **Verify:** every promotion path produces audit rows tied to identity; change-request list is paginated and org-scoped.
- **Schema:** reuses `audit_log`; no new table.

---

## 5. Dependencies

- **Auth P0s (Synthesis P0-3):** promotion/approval mutate live authorization for a tenant — these routes must sit behind the default-deny auth gateway and org-scope guard. Follow the `api/namespaces.rs` pattern exactly.
- **Existing rollout machinery** (`deployment/service/`, `api/deployments/rollouts.rs`) is the required termination point — promotion is a governance layer, not a new rollout engine. The confirmation loop (`helpers.rs:240`) and wave approval (`helpers.rs:284-308`) are reused as-is.
- **Data plane versioning** (`DATA_PLANE_PLAN.md`, agent `/api/v1/data/*`) must expose "current data version for a namespace" and "apply data version to a namespace" for Step 7.
- **GitOps (plan 09) composes but is not a hard prerequisite:** if git is source of truth, a promotion is a git operation (promote = merge staging→main); envs can also promote UI-created bundles. Land the `Environment` object independent of GitOps; integrate commit-back promotion later.
- **Migrations 012/013** must precede the environment/change-request code paths.

---

## 6. Testing & verification

- **Unit:** tier ordering (`can_promote_to`); approval-policy evaluation (2-of-N, distinct-from-requester, self-approve blocked); change-window/freeze evaluation by timestamp.
- **Integration (management, real DB):** create dev/staging/prod bound to namespaces; promote → pending change request with pinned bundle + data version; approve twice → rollout row created targeting the correct namespace.
- **BDD (`tests/features`):** "regulated promotion" feature — staging→prod requires two distinct approvers, a change record, and respects a freeze window; downward promotion rejected.
- **Regression:** existing canary/percentage/label rollout tests pass when the rollout is initiated via the promote path (prove composition, not replacement — `api/deployments/rollouts.rs`, `deployment/service/helpers.rs`).
- **E2E (`tests/e2e`):** promote a bundle staging→prod → agents in prod's namespace pull the new bundle (via `services/reaper-agent/src/management/sync.rs`) and a decision reflects both the promoted policy and the promoted data version.
- **Negative:** self-approval to meet threshold → rejected; promote during freeze → 409; promote to same/lower tier → 400; duplicate namespace binding → 409.

---

## 7. Effort & phasing — S/M/L

- **Phase A — Env object + CRUD (P1).** Step 1 (M), Step 2 (S). First-class environments over namespaces; unblocks "dev/staging/prod are real objects." **~M.**
- **Phase B — Governed promotion (P1).** Step 3 (M), Step 4 (M), Step 5 (M), Step 6 (M). The regulated change-management spine: change requests + two-person/window gates → existing rollout. **~L.**
- **Phase C — Data plane + audit trail (P1→P2).** Step 7 (M), Step 8 (S). Per-env data version on promotion + the defensible change-record listing. **~M.**

Cheapest slice that closes F4's core: **Phase A + Steps 5-6** (env objects + env→env promotion with approval) — the change-window/freeze and per-env data-plane refinements can follow.

---

## 8. Key decisions (ADR-style)

### ADR-1 — Environment model shape: new object vs overloaded namespace
- **Decision:** **New first-class `Environment` object that *wraps* a namespace** (one-to-one binding), per `reviews/04:163`.
- **Why:** Namespaces already carry deployment/data scope and agent subscriptions (`domain/namespace.rs`, `api/namespaces.rs:481-539`); piling tier/approval/window semantics onto `settings: serde_json::Value` (`domain/namespace.rs:28`) would deepen the existing namespaces-vs-orgs muddle the audit flagged. A thin `Environment` adds lifecycle + approval without muddying the tenancy/scope unit.
- **Rejected:** (a) encode env in the namespace slug (today's convention — carries no promotion semantics, F4); (b) put env state in namespace `settings` JSON (untyped, unqueryable, no FK integrity for change records).

### ADR-2 — Promotion = governed transition that *reuses* rollout, not a new engine
- **Decision:** `promote(from_env → to_env)` creates a **change request**, evaluates the approval policy + window, then calls the **existing** `DeploymentService::start_rollout` into the target namespace with the env's strategy.
- **Why:** The rollout machinery (strategies, waves, agent-confirmed convergence, auto-rollback) is the review's strongest pillar (`reviews/04:17,103`); rebuilding it for promotion would be wasteful and risky. Env promotion is a governance envelope around a proven core.
- **Composition with strategies:** the env's bound strategy can be immediate/canary/percentage/label (`domain/deployment.rs:15-25`); the **env-boundary** approval (two-person change request) is *distinct from and additional to* the **intra-rollout** wave `require_approval` (`domain/deployment.rs:79,88`). Both can apply: N-of-M to enter prod, then per-wave approval during the canary.

### ADR-3 — Approval gate placement: change request vs strategy config
- **Decision:** **Approval policy lives on the target `Environment`** (change-request gate), not only on the strategy.
- **Why:** Regulated change management gates on *entering an environment* (dev→stage→prod distinct approvers + change record), which is an env property, not a rollout-strategy property. The existing strategy `require_approval` remains as the finer intra-rollout wave control. Two independent gates, clearly separated.

---

## 9. Risks & rollback

- **Risk: promotion becomes a second, competing deploy path.** Confusion between "start rollout directly" and "promote." **Mitigation:** for env-bound namespaces, make promotion the sanctioned path and (optionally) restrict direct `start_rollout` into a prod-tier namespace to Admin/break-glass, audited. Direct rollout stays available for non-env namespaces.
- **Risk: data/policy version skew on promotion.** Promoting a bundle without its data version could flip decisions unexpectedly (the F6 migration hazard nearby). **Mitigation:** Step 7 pins and carries the data version; a promotion that cannot resolve a compatible data version fails closed rather than deploying policy against stale data.
- **Risk: approval bypass / self-approval.** A single actor meeting the threshold defeats two-person control. **Mitigation:** enforce `distinct_from_requester` and N distinct approver identities in the repository transition (Step 4), tied to governed identities (depends on SSO/SCIM, Synthesis P0 F1) for real deprovisioning.
- **Risk: freeze-window lockout during an incident.** A freeze could block an emergency fix. **Mitigation:** an audited break-glass override (Admin scope) that records an explicit reason and bypasses the window, surfaced in the change record.
- **Rollback plan:** additive migrations 012/013 (new tables + nullable columns) — down-migrations drop them without touching `namespaces`/`bundles`/rollouts. Feature-flag the promotion routes; disabling them reverts to today's direct-rollout behavior with zero data loss. Because promotion terminates in the existing rollout, existing rollback (`api/deployments/rollouts.rs:227` namespace rollback, `:278` org rollback) still recovers any promoted bundle.
