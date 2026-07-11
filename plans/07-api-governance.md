# API Governance

> **STATUS: ✅ SHIPPED** — all 9 steps landed via PRs #27–#31 (2026-07-11), one PR
> per phase. A: generated OpenAPI 3.1 contracts served at `/openapi.json` on both
> planes with a blocking contract-parity CI gate (control plane single-sourced
> via utoipa-axum; agent dual-sourced so the enforcement hot path stayed
> byte-for-byte untouched). B: single `/api/v1` surface (bare root retired
> behind the default-off `serve_root_alias` deprecation lever) +
> `docs/api/VERSIONING.md`. C: ETag/If-Match optimistic concurrency with the
> SQL guard as atomic arbiter (`require_if_match` warn-only this release per
> ADR-3's rollout). D: `Idempotency-Key` claim-then-complete on promote/
> rollback/rollout/org-create (`idempotency_keys` table + sweeper). E: bounded
> keyset cursor pagination on every list endpoint + RFC 9457 problem+json with
> constraint-violation mapping (409/422). F: `docs/api/ROUTE_CONVENTIONS.md`.
> Suites verified on SQLite AND real PostgreSQL (now enforced by the
> `management-tests-postgres` CI job).

**Readiness gate:** Blocks CONDITIONAL → READY (control-plane API is not publishable to enterprise buyers without a contract, concurrency control, and standard error/pagination semantics).
**Priority:** P1 (elevate the lost-update and unversioned-root items — they cause silent data loss / permanent compatibility debt).
**Findings closed:** Synth #9; Code API-4, API-5, API-6, API-7, API-8, API-13, API-14; Product F9. (Auth findings API-1/API-2 are owned by the auth plan and are a hard prerequisite — see §5.)

---

## 1. Goal

Make the Reaper control-plane (`reaper-management`) and agent enforcement APIs governable by a team and consumable by enterprise customers:
1. A single machine-readable **OpenAPI 3.1 contract**, generated from the handlers, published at `/openapi.json`, with a CI gate that fails when a route exists that the spec does not describe (and vice-versa).
2. A **single versioned surface** (`/api/v1`) with a written versioning + deprecation policy; retire the permanent bare-root duplicate.
3. **Optimistic-concurrency control** (ETag / `If-Match`) on policy and bundle updates so concurrent edits cannot silently clobber each other.
4. **Idempotency keys** on the propagation-triggering POSTs (promote, rollout, org-create) so automation retries are safe.
5. **Bounded, cursor-based pagination** on every list endpoint.
6. **RFC 9457 `application/problem+json`** error bodies, with DB constraint violations mapped to 409/422 instead of an opaque 500.
7. A documented **route-modeling convention** (action sub-resources vs. verb endpoints) so the surface stays coherent as it grows.

Non-goal: rewriting business logic in handlers. This plan changes the API *contract and envelope*, not policy semantics.

## 2. Current state (evidence) — file:line

- **No API spec anywhere.** Searches for `openapi`/`utoipa`/`aide`/`swagger` across `*.rs`/`*.toml` return nothing (Code API-4; repo-map line 38). 19 control-plane route files + the agent API have no machine-readable contract.
- **API served at BOTH bare root AND `/api/v1`.** `services/reaper-management/src/main.rs:214-215`:
  ```rust
  let api_router =
      api::build_api_router().merge(Router::new().nest("/api/v1", api::build_api_router()));
  ```
  Every route is reachable at `/orgs/...` and `/api/v1/orgs/...`; `api::api_v1_routes` (`api/mod.rs:56`) is `#[allow(dead_code)]`. No versioning/deprecation policy documented (Code API-6).
- **No lost-update protection on policy updates.** `api/policies.rs:195` `update_policy` reads the body and writes unconditionally; the repo (`db/repositories/policy.rs:205-240`) reads `current_version`, computes `+1`, and does `UPDATE policies SET ... current_version = $4 WHERE id = $6` — **no `AND current_version = $expected`** guard. Two editors both read version N, both write N+1, later write wins silently. `content_hash` already computed (`policy.rs:208`) — an ETag source exists but is unused (Code API-5).
- **No lost-update protection on bundle updates.** `api/bundles.rs:119` `update_bundle` writes unconditionally; no `If-Match` (Code API-5).
- **No idempotency keys.** Search for `Idempotency-Key`/`idempotency` finds only a test comment. Promote (`api/bundles.rs` `promote_bundle`), rollout create (`api/deployments/rollouts.rs`), and org create lack idempotency — automation retry after a timeout double-applies (partly mitigated only for rollouts by `ActiveRolloutExists`→409). (Code API-14).
- **Unpaginated list endpoints.** `api/agents.rs:231` `list_agents`, `namespaces.rs:169`, `sources.rs:120`, `teams.rs:66` return all rows. Decisions is bounded (`decisions/mod.rs:486` caps `limit.min(1000)`) but uses **offset** pagination (`api/decisions.rs:486`), which drifts/degrades on the largest tables (Code API-8, API-13).
- **Non-standard error body; 409s hidden as 500.** `api/error.rs:45-56` emits custom `{error:{code,message,details}}`, not `application/problem+json`. `From<sqlx::Error>` (`api/error.rs:142-147`) collapses **every** sqlx error to `ApiError::Internal` → HTTP 500, so a unique-constraint violation (a 409) surfaces as a 500 (Code API-7).
- **Route-modeling is inconsistent.** Verb-style endpoints (e.g. bundle `promote`, and per synthesis `/init-all`, `/promotePolicy`) coexist with resource routes; no documented convention.

## 3. Definition of Done — testable checkboxes

- [ ] `GET /openapi.json` (mgmt) and `GET /openapi.json` (agent) return a valid OpenAPI 3.1 document that passes `openapi-spec-validator` / `redocly lint` with zero errors.
- [ ] CI job `api-contract` fails the build if any axum route registered in `build_api_router()` / agent router is absent from the generated spec, or if the spec lists a path with no live route. (Parity test asserts `router_paths == spec_paths`.)
- [ ] The bare-root duplicate is removed: `GET /orgs/...` (no `/api/v1`) returns 404; all internal callers (`reaper-sync`, tests, SDK) build URLs against `/api/v1`; `api::api_v1_routes` dead-code is deleted or made live.
- [ ] `docs/api/VERSIONING.md` exists: defines the stability contract, deprecation window (e.g. ≥180 days), `Sunset`/`Deprecation` headers, and what constitutes a breaking change.
- [ ] `GET` on a policy and a bundle returns a strong `ETag` header (derived from `content_hash` / bundle version). A `PUT` without `If-Match` returns **428 Precondition Required**; with a stale `If-Match` returns **412 Precondition Failed**; with a matching value succeeds and returns the new `ETag`.
- [ ] The policy `UPDATE` SQL includes `AND current_version = $expected`; a concurrent-update integration test (two writers, same base version) yields exactly one success and one **409/412**, never two silent successes.
- [ ] `POST` promote / rollout-create / org-create accept an `Idempotency-Key` header: a replayed key within the retention window returns the original result (same status + body) and does **not** re-trigger propagation. A test issues the same key twice and asserts one side effect.
- [ ] Every list endpoint (`agents`, `namespaces`, `sources`, `teams`, `decisions`, `policies`, `bundles`) enforces a default page size (e.g. 50) and a hard max (e.g. 200/1000), returns a `next_cursor`, and rejects `limit > max` with 400. Keyset/cursor pagination replaces offset on `policies` and `decisions`.
- [ ] Error responses use `content-type: application/problem+json` with `type`/`title`/`status`/`detail`/`instance` fields (RFC 9457). A unique-constraint violation returns **409**, a check/validation violation **422** — verified by an integration test that forces a duplicate insert.
- [ ] `docs/api/ROUTE_CONVENTIONS.md` documents the chosen convention and the migration status of existing verb endpoints.

## 4. Critical steps — ordered; per step what/where(files)/verify

1. **Adopt `utoipa` and generate the spec.**
   - What: Add `utoipa` + `utoipa-axum` (or `aide`) to `reaper-management` and `reaper-agent`. Annotate handlers with `#[utoipa::path(...)]` and request/response DTOs with `#[derive(ToSchema)]`. Assemble an `#[derive(OpenApi)]` root that references every handler; serve it at `/openapi.json`.
   - Where: `services/reaper-management/src/api/*.rs` (19 files), `services/reaper-management/src/main.rs` (mount route), `services/reaper-agent/src/main.rs:490` (agent router), new `api/openapi.rs` module.
   - Verify: `curl /openapi.json | redocly lint -` passes; count of documented operations == count of registered routes.

2. **Add the contract-parity CI gate.**
   - What: A test/binary that boots the router, walks its registered paths+methods (axum `Router` introspection or a maintained route list), and diffs against paths in the generated OpenAPI doc. Fail on any asymmetry. Wire into a new `.github/workflows/` job or extend `ci.yml`.
   - Where: `services/reaper-management/tests/api_contract.rs`, `.github/workflows/ci.yml`.
   - Verify: deliberately add a route without an annotation → CI red; annotate it → green.

3. **Collapse to a single `/api/v1` surface.**
   - What: Remove the root-merge at `main.rs:214-215`, keep only `nest("/api/v1", build_api_router())`. Update `reaper-sync`, SDK (`crates/reaper-sdk`), e2e/integration tests, and CLI to use `/api/v1`. Delete or activate `api::api_v1_routes` (`api/mod.rs:56`).
   - Where: `services/reaper-management/src/main.rs`, `services/reaper-sync/`, `crates/reaper-sdk/`, `tools/reaper-cli/`, `tests/e2e/`.
   - Verify: e2e suite green against `/api/v1`; a request to bare `/orgs/...` returns 404.

4. **Write the versioning + deprecation policy.**
   - What: `docs/api/VERSIONING.md` — stability guarantees, deprecation window, `Deprecation`/`Sunset` header usage, breaking-change definition. Add a `Deprecation` header emitter helper for future use.
   - Where: `docs/api/VERSIONING.md`, `services/reaper-management/src/app_middleware.rs` (optional header helper).
   - Verify: doc review; header helper unit-tested.

5. **Optimistic concurrency on policy + bundle updates.**
   - What: On `GET` policy/bundle, emit `ETag` from `content_hash` (policies) / version (bundles). On `PUT`, require `If-Match`; thread the expected version into the repo. Change `db/repositories/policy.rs:225-240` UPDATE to `... WHERE id = $6 AND current_version = $expected`; if `rows_affected == 0`, return `ApiError::Conflict` → 409/412. Same for the bundle repo used by `api/bundles.rs:119`.
   - Where: `api/policies.rs:180-219`, `api/bundles.rs:119`, `db/repositories/policy.rs:195-260`, bundle repo, `api/error.rs` (add `PreconditionRequired`/`PreconditionFailed` variants → 428/412).
   - Verify: integration test with two concurrent writers on the same base version → exactly one 2xx, one 4xx.

6. **Idempotency keys on propagation POSTs.**
   - What: Middleware/extractor that reads `Idempotency-Key`, looks up a persisted `(key, request-hash) → response` record (new table `idempotency_keys` with TTL), returns the stored response on replay, else executes and stores. Apply to promote (`api/bundles.rs`), rollout-create (`api/deployments/rollouts.rs`), org-create (`api/orgs.rs`).
   - Where: new `services/reaper-management/src/api/idempotency.rs`, `db/migrations/NNN_idempotency_keys.sql`, the three handlers.
   - Verify: replay the same key twice → one SSE `BundlePromoted` broadcast, identical response bodies.

7. **Bounded cursor pagination on all list endpoints.**
   - What: Introduce a shared `Page { limit (default 50, max 200), cursor }` query extractor and `Paginated<T> { items, next_cursor }` envelope. Convert offset→keyset (order by `(created_at, id)`) for `policies` and `decisions`; add limits to `agents`, `namespaces`, `sources`, `teams`.
   - Where: new `api/pagination.rs`; `api/agents.rs:231`, `namespaces.rs:169`, `sources.rs:120`, `teams.rs:66`, `policies.rs`, `decisions/mod.rs:486` / `api/decisions.rs:486`.
   - Verify: request `limit=10000` → 400; paging through a seeded 500-row table with cursors returns each row exactly once with no drift after an insert.

8. **RFC 9457 error envelope + DB error mapping.**
   - What: Change `IntoResponse for ApiError` (`api/error.rs:58-108`) to emit `application/problem+json` (`type`,`title`,`status`,`detail`,`instance`,`code`). Replace the blanket `From<sqlx::Error>` (`api/error.rs:142-147`): match `sqlx::Error::Database(db_err)` unique-violation SQLSTATE (`23505` pg / SQLite constraint) → `Conflict` (409); check-violation → `Validation` (422); keep others as 500. Add `#[non_exhaustive]` to `ApiError` (relates to API-10).
   - Where: `api/error.rs`.
   - Verify: force a duplicate org slug → 409 problem+json; malformed enum → 422; unit tests assert content-type and status per branch.

9. **Document the route-modeling convention.**
   - What: `docs/api/ROUTE_CONVENTIONS.md` — prefer resource + action sub-resource (`POST /bundles/{id}/promotions`) over ad-hoc verbs (`/promotePolicy`, `/init-all`); list existing verb endpoints with a deprecation/migration note (aliased under the deprecation policy from step 4).
   - Where: `docs/api/ROUTE_CONVENTIONS.md`.
   - Verify: doc review; new endpoints in steps 5-7 follow it.

## 5. Dependencies

- **Auth plane (hard prerequisite for exposure):** API-1/API-2 (default-deny auth gateway on control plane + agent) are covered by the auth remediation plan; the contract work here is safe to do in parallel but the surface must not be published as "enterprise-ready" until auth lands. ETag/idempotency add no value on an unauthenticated surface.
- **DB migration engine (Product F6):** the `idempotency_keys` table and any version-column additions need the migrations pipeline (`db/migrations/`).
- **SDK/CLI:** step 3 (single surface) requires coordinated updates to `reaper-sdk`, `reaper-cli`, `reaper-sync`, e2e tests.
- **utoipa/axum version compatibility** with the pinned axum 0.8.x in the workspace.

## 6. Testing & verification

- **Contract parity test** (step 2) — the primary regression gate; run in CI on every PR.
- **Concurrency test** — two-writer lost-update test for policies and bundles (step 5).
- **Idempotency test** — duplicate-key replay asserts single side effect (step 6).
- **Pagination tests** — limit-cap rejection, cursor no-drift-under-insert, per-endpoint default enforcement (step 7).
- **Error-mapping tests** — duplicate insert → 409 problem+json; validation → 422 (step 8).
- **Spec lint** — `redocly lint` / `openapi-spec-validator` in CI (step 1).
- **e2e** — existing `tests/e2e` suite must stay green after the single-surface migration (step 3).

## 7. Effort & phasing — S/M/L

- **Phase A (M):** OpenAPI generation + parity gate (steps 1-2). Highest governance leverage, no behavior change.
- **Phase B (S):** Single `/api/v1` surface + versioning doc (steps 3-4). Small code, moderate blast radius on callers.
- **Phase C (M):** Optimistic concurrency on policy + bundle (step 5). Correctness-critical; needs repo + error changes.
- **Phase D (M):** Idempotency keys (step 6) — new table + middleware.
- **Phase E (S-M):** Pagination (step 7) and RFC 9457 errors (step 8) — mechanical but broad.
- **Phase F (S):** Route-convention doc (step 9).

Overall: **L** in aggregate; each phase independently shippable.

## 8. Key decisions (ADR-style)

- **ADR-1: `utoipa` over `aide`.** Chosen: `utoipa` (annotation-driven, widely used with axum 0.8, generates 3.1). Rejected: hand-written spec (drifts immediately — the exact failure we're fixing); `aide` (viable alternative, but `utoipa` has broader axum-extractor coverage). Consequence: handlers carry `#[utoipa::path]` macros — accepted maintenance cost bought back by the parity gate.
- **ADR-2: ETag from existing `content_hash`.** Policies already compute `content_hash` (`policy.rs:208`); reuse it as the ETag rather than adding a column. Bundles use their version/UUID. Consequence: no schema change for policy ETags.
- **ADR-3: `If-Match` required (428 when absent), not optional.** For governed policy edits we fail closed on a missing precondition rather than allowing blind overwrite. Consequence: SDK/CLI must send `If-Match`; documented in the versioning policy.
- **ADR-4: Keyset (cursor) over offset pagination for large tables.** Offset drifts under concurrent inserts and degrades on deep pages (API-13). Small tables (teams, namespaces) keep a simple limit; large tables (policies, decisions) get keyset. Consequence: opaque cursors in the API, documented as non-decodable.
- **ADR-5: RFC 9457 problem+json.** Standard, tool-friendly. Consequence: existing consumers of `{error:{code,message}}` must migrate — done under the single-surface cutover (step 3) with a changelog entry.
- **ADR-6: Idempotency keys persisted with TTL, keyed by `(key, request-hash)`.** Guards against a client replaying a key with a *different* body (returns 422 mismatch). Consequence: a small write on every propagation POST.

## 9. Risks & rollback

- **Risk: removing the bare-root surface breaks an undiscovered consumer.** Mitigation: grep all in-repo callers first; emit `Deprecation`/`Sunset` headers on root for one release before deleting (feature-flag `serve_root_alias`, default off). Rollback: re-enable the flag.
- **Risk: `If-Match` requirement breaks existing automation that never sent it.** Mitigation: ship behind a per-org/config `require_if_match` flag defaulting to warn-only for one release, then enforce. Rollback: flip flag to warn-only.
- **Risk: keyset cursor migration changes list ordering.** Mitigation: keep ordering stable (`created_at, id`); contract test pins order. Rollback: revert to offset for the affected endpoint (envelope unchanged).
- **Risk: utoipa annotations drift from handlers over time.** Mitigation: the parity gate (step 2) makes drift a red build — this is the durable control.
- **Risk: idempotency table growth.** Mitigation: TTL + periodic cleanup job; bounded retention (e.g. 24-72h).
- **General rollback:** every phase is independently revertable; the OpenAPI spec and docs are additive and carry zero runtime risk if the parity gate is set to non-blocking first, then promoted to blocking.
