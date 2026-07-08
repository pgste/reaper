# AuthN/AuthZ Foundation

**Readiness gate:** Unblocks the "identity & tenant-isolation" gate on the enterprise security questionnaire — the hard stop that keeps Reaper at NOT READY. Moves both the enforcement plane (agent) and the control plane (management) from "auth is opt-in and fails open" to "default-deny, fail-closed." This is the #1 prerequisite the synthesis names before any other remediation (Synthesis §"good news", sequence step 1).

**Priority:** P0

**Findings closed:** Synthesis #1, Synthesis #2; Security P0-1, P0-3, P0-3b; Code API-1, API-2. (Adjacent but explicitly *out of scope* here: Security P0-2 signature-bypass — separate "distribution hardening" workstream; API-3 `panic=abort` — availability workstream. Called out under Dependencies so the seams line up.)

---

## 1. Goal

Make every non-health route on both services reject unauthenticated callers by construction, and make every tenant-addressed operation reject cross-tenant callers.

Concretely:
1. **Agent enforcement plane (`reaper-agent`)** — today serves policy-deploy, entity/relationship writes, decision dumps, and `/debug/datastore` with zero inbound auth on `0.0.0.0`. Add a default-deny inbound auth gateway (mTLS client-cert and/or a registration-minted / shared bearer token), bind loopback/UDS by default, and remove `/debug/datastore` from production builds.
2. **Control plane (`reaper-management`)** — today the `bundles`/`policies`/`orgs`/`teams`/`billing` route groups take no `RequireAuth`, no `RequireScope`, and no tenant check, and id-addressed bundle ops act on a global UUID (IDOR). Add a default-deny auth **layer** so a handler that forgets the extractor still fails closed, apply `RequireAuth` + `RequireScope` + `user.org_id == org.id` on every mutation, and org-scope every id-addressed repository call.
3. Reuse the primitives the review praised — the `RequireAuth` extractor and the `authorize()` pattern at `api/datastore.rs:92-134` (scope check + `resolve_org` + `user.org_id != org.id && !Admin → Forbidden`), and the already-modelled `AuthMethod::Mtls` — rather than inventing new machinery.
4. **Stretch:** "eat your own dogfood" — authorize the control-plane's own admin mutations through a Reaper policy evaluated by the engine, proving the product on its own surface.

Non-goal: SSO/SAML/OIDC/SCIM (Product F1) — that is a separate identity-federation workstream; this plan hardens the *authorization gateway* those identities will flow through.

---

## 2. Current state (evidence)

**Agent enforcement plane — zero inbound auth:**
- `services/reaper-agent/src/main.rs:490-539` builds the entire router and applies exactly one layer — `DefaultBodyLimit::max(256 MB)` at `main.rs:538` — before `.with_state(state)` at `main.rs:539`. No auth middleware, no extractor.
- Unauthenticated mutating/reading routes registered in that block: `POST /api/v1/policies/deploy` (`main.rs:511`), `/api/v1/bundles/deploy` (`:520`), `/api/v1/bundles/load` (`:521`), `POST /api/v1/entities` + `entities/batch` + `DELETE entities/{type}/{id}` (`:523-530`), `POST /api/v1/data` + `data/sync` + `data/apply-deltas` (`:504-509`), `GET /api/v1/decisions` + `POST /api/v1/decisions/export` (`:534,536`), and `GET /debug/datastore` (`:532`).
- The two bundle-load handlers take only `State` + `Json` and immediately hot-swap: `handlers/policies.rs:308` `deploy_bundle` → `deploy_bundle_with_store(...)` at `:339-341`; `handlers/policies.rs:395` `load_bundles_atomic`. No identity is read.
- Defaults make this internet-facing: `crates/reaper-core/src/config/settings.rs:51-53` `default_bind_address()` returns `"0.0.0.0"`; `TlsSettings` (`settings.rs:503-522`) defaults `enabled=false` and `require_client_cert=false` (test asserts both false at `settings.rs:565-569`). The same router is also served on UDS (`main.rs:542-568`), so the TCP listener carries the full unauthenticated surface.
- The agent has outbound auth to management (`services/reaper-agent/src/management/client.rs:22-25` holds a registration-minted JWT `token`; `X-API-Key` at `:131`; `Authorization: Bearer` at `:196`) but **no inbound** verification of any kind.

**Control plane — opt-in per-handler auth, whole groups omitted:**
- `services/reaper-management/src/main.rs:206-243` `build_router`: the layer stack is `security_headers → correlation_id → request_metrics → body_size_limit → access_log → TraceLayer` (+ optional rate-limit) — **no auth layer**. Auth is therefore per-handler only.
- `api/mod.rs:31-52` `build_api_router()` merges all route groups flat; the same router is mounted at both root and `/api/v1` (`main.rs:214-215`).
- **Omitted groups (no `RequireAuth`):** `api/bundles.rs` (every handler; grep for `RequireAuth` in orgs.rs returns nothing either — confirmed), `api/policies.rs`, `api/orgs.rs`, `api/teams.rs`, `api/billing.rs`. Example: `api/bundles.rs:98` `create_bundle`, `:119` `update_bundle`, `:141` `delete_bundle`, `:199` `promote_bundle` take only `State`/`Path`/`Json`.
- **IDOR (P0-3b / API-1):** `api/bundles.rs:113` `get_bundle` does `let _org_id = parse_org_id(&org, &state)…` then `state.bundle_service.get(bundle_id)` — org discarded. Same pattern at `:124,145,156,170,183,193,204,215,224,379`. `bundle/service.rs:113` `get(bundle_id)` calls the repo; `db/repositories/bundle.rs:67` `get_by_id(id)` runs `SELECT … FROM bundles WHERE id = $1` — the table **has an `org_id` column** (`bundle.rs:74`) but it is not in the predicate. Any bundle in any org is reachable by UUID.
- `promote_bundle` (`api/bundles.rs:199-207`) broadcasts `BundlePromoted` over SSE and drives agents to pull — so the unauthenticated path is a cross-tenant policy-injection primitive.

**The correct pattern already exists (reuse target):**
- `api/datastore.rs:92-134` `authorize()`: takes `RequireAuth`-extracted `AuthenticatedUser`, checks `user.has_permission(Scope::…)` (`:99-107`), resolves the org via `resolve_org` (`:117`), then enforces `if user.org_id != organization.id && !user.has_permission(Scope::Admin) → Forbidden` (`:118-122`). All datastore routes (`api/datastore.rs:33-79`) go through it. This is the template.
- Primitives available: `auth/middleware.rs:140` `RequireAuth` extractor (API key → mTLS fingerprint → session/JWT/JWKS chain); `auth/middleware.rs:29` `AuthenticatedUser`; `auth/middleware.rs:45,64` `AuthMethod::Mtls` + `from_certificate`; `auth/middleware.rs:449-462` `RequireScope::check`; `auth/scopes.rs` (`BundleWrite`, `BundlePromote`, `PolicyWrite`, `OrgAdmin`, `Admin`, etc.). `resolve_org` is public at `api/orgs.rs:190`.
- mTLS is already wired end-to-end in management: `RequireAuth` reads a trusted-proxy fingerprint header (`auth/middleware.rs:156-206`) and validates it via `auth/mtls.rs:validate_certificate` (registration/revocation/validity/agent-binding). The agent has no equivalent.

---

## 3. Definition of Done — testable checkboxes

Gateway / default-deny:
- [ ] A control-plane integration test hits **every** mutation route in `bundles`/`policies`/`orgs`/`teams`/`billing` with **no** credentials and gets `401` (not `200`/`500`).
- [ ] A "forgotten extractor" test: a handler deliberately without `RequireAuth`, mounted behind the gateway, still returns `401` unauthenticated (proves the layer, not the extractor, is the guarantee).
- [ ] An agent integration test hits every non-health `/api/v1/*` route and `/debug/*` with no credentials and gets `401`; `/health`, `/ready`, `/live`, `/metrics` still return `200` unauthenticated.

Tenant isolation / IDOR:
- [ ] Org B, authenticated, requesting org A's bundle by UUID (`GET /orgs/{B}/bundles/{A-bundle}` and `GET /orgs/{A}/bundles/{A-bundle}` with B's token) gets `404` (not `200`).
- [ ] Cross-tenant `promote`/`update`/`delete`/`download` of a foreign bundle UUID gets `404`/`403`.
- [ ] `BundleRepository::get_by_id_scoped(org_id, id)` (new) is the only path used by id-addressed bundle handlers; a grep shows no id-addressed handler calling the unscoped `get`/`get_by_id`.
- [ ] Every id-addressed handler in `policies.rs`/`bundles.rs` enforces `user.org_id == resolved.org_id || Admin`.

Scope enforcement:
- [ ] Read routes require a read scope (`BundleRead`/`PolicyRead`/`OrgRead`); mutations require the matching write/promote scope (`BundleWrite`/`BundlePromote`/`PolicyWrite`/`OrgAdmin`). A `Viewer`-scoped token gets `403` on any mutation.

Agent inbound auth:
- [ ] With the hardened default profile, the agent binds `127.0.0.1` (or UDS) unless an operator sets an explicit network bind **and** an auth method; startup **refuses** to bind a non-loopback address with auth disabled (fail-closed config validation).
- [ ] A request with a valid mTLS client cert (or valid bearer token) succeeds; an invalid/absent one gets `401`.
- [ ] `/debug/datastore` is absent from the release router (compiles out under `#[cfg(debug_assertions)]` or an explicit default-off flag), verified by a test asserting `404` in a release-profile build.

Regression / negative suite (see §6) is green in CI.

---

## 4. Critical steps

### Phase A — Control-plane default-deny gateway (closes API-1 gateway half, P0-3 structurally)

**A1. Add a default-deny authentication middleware layer.** *(M)*
- Build: a `middleware::from_fn_with_state` layer `require_authentication` in a new `services/reaper-management/src/auth/gateway.rs`. It runs the same resolution logic as `RequireAuth` (`auth/middleware.rs:140-341`) — factor the body of `RequireAuth::from_request_parts` into a shared `authenticate(parts, state) -> Result<AuthenticatedUser, Response>` so the layer and the extractor share one code path (avoids drift). On success it inserts `AuthenticatedUser` into request extensions and calls `next.run`; on failure it returns `401`.
- Allowlist (unauthenticated) by path prefix: `/health*`, `/metrics*`, `/auth/*` (login/signup/refresh/password/email-verify), `/auth/github/*` and `/oauth/*` (callback), and any public webhook-ingest route (`api/webhooks.rs`) that authenticates by signature. Everything else is deny-by-default.
- Touch: `services/reaper-management/src/main.rs:219-232` — add the layer to `build_router` **after** `security_headers`/`correlation_id` so it runs on every API request; `auth/middleware.rs` (extract `authenticate`); new `auth/gateway.rs`; `auth/mod.rs` (export).
- Verify: the "forgotten extractor" DoD test; existing authenticated tests still pass (the extractor can now read the user from extensions as a fast path, but keeping the extractor working standalone is fine).

**A2. Make `RequireAuth` read the gateway result.** *(S)*
- Build: have `RequireAuth::from_request_parts` first check `parts.extensions.get::<AuthenticatedUser>()` (populated by A1) and only fall back to full resolution if absent. Keeps every existing handler compiling unchanged while guaranteeing the layer already ran.
- Touch: `auth/middleware.rs:140-341`.
- Verify: unit test that a handler behind the layer never triggers a second DB validation (assert via a call-count or that it works with the same request).

### Phase B — Control-plane authZ + tenant scoping on the omitted groups (closes API-1, P0-3, P0-3b)

**B1. Extract the reusable org-authorization helper.** *(S)*
- Build: lift `api/datastore.rs:92-134` `authorize()` into a shared `auth::authorize_org(state, user, org_ref, required: Scope) -> ApiResult<Resolved{org_id}>` (new function in `auth/gateway.rs` or `api/orgs.rs`). It performs: `RequireScope`-style scope check, `resolve_org`, and the `user.org_id != org.id && !Admin → Forbidden` guard. Have `datastore.rs::authorize` call it (proves parity, no behavior change there).
- Touch: `api/datastore.rs`, new shared helper, `api/orgs.rs` (`resolve_org` already public at `:190`).
- Verify: existing datastore tests still green.

**B2. Bundles: add `RequireAuth` + scope + org-scope every handler.** *(M)*
- Build: every handler in `api/bundles.rs` gains `RequireAuth(user): RequireAuth` and calls `authorize_org(&state, &user.0, &org, scope)` where scope is `BundleRead` for `get`/`list`/`download`/`diff`/`get_promoted`, `BundleWrite` for `create`/`update`/`delete`/`add_policies`/`remove_policies`/`compile`/`stage`/`deprecate`, `BundlePromote` for `promote`. Replace `let _org_id = parse_org_id(...)` with the returned `resolved.org_id`.
- Kill IDOR: add `BundleRepository::get_by_id_scoped(org_id, id)` (`db/repositories/bundle.rs`, `WHERE id = $1 AND org_id = $2`, returns `None` → `404`), and add `BundleService::get_scoped(org_id, id)` (`bundle/service.rs:113`). Route every id-addressed handler through it (`bundles.rs:113,124,145,156,170,183,193,204,215,224,379,385-405` incl. `get_bundle_diff`'s `base` bundle lookup at `:385`).
- Touch: `api/bundles.rs` (all handlers), `bundle/service.rs`, `db/repositories/bundle.rs`.
- Verify: the cross-tenant `404` DoD tests; a `Viewer` token → `403` on promote.

**B3. Policies: same treatment.** *(M)*
- Build: `api/policies.rs` handlers (`list_policies`, `create_policy` at `:132`, `get_policy`, `update_policy` at `:195`, `delete_policy`, `list_versions`, `get_version`, `validate_*`) gain `RequireAuth` + `authorize_org(..., PolicyRead|PolicyWrite)`. Scope the policy repo lookups by org: `db/repositories/policy.rs:76` `get_by_id` → add `get_by_id_scoped(org_id, id)` and use it in every id-addressed handler.
- Touch: `api/policies.rs`, `db/repositories/policy.rs`.
- Verify: cross-tenant policy read/write → `404`/`403`.

**B4. Orgs / teams / billing.** *(M)*
- Build: `api/orgs.rs`, `api/teams.rs`, `api/billing.rs` mutation handlers gain `RequireAuth` + appropriate scope (`OrgAdmin`/`OrgWrite` for org & team management; `OrgAdmin` for billing). Org *creation* (`POST /orgs`) is the one legitimately un-org-scoped mutation — it must still require an authenticated user (any logged-in user may create an org) and set them as owner; it is covered by the A1 gateway (401 if anonymous) even before per-handler scope work.
- Touch: `api/orgs.rs`, `api/teams.rs`, `api/billing.rs`.
- Verify: anonymous → `401` (gateway); cross-tenant team/billing mutation → `403`.

> After B, the gateway (A1) is the safety net and B is defense-in-depth: even a future handler added without the extractor fails closed.

### Phase C — Agent inbound auth + hardened defaults (closes API-2, P0-1)

**C1. Add an agent auth config + fail-closed bind validation.** *(M)*
- Build: new `AgentAuthSettings` in `crates/reaper-core/src/config/settings.rs` with `enabled: bool`, `mode: {Mtls, BearerToken, Both}`, an optional `mtls_fingerprint_header` (trusted-proxy pattern, mirroring management), and `bearer_token`/`jwt_secret` (shared with management so the registration-minted JWT the agent already holds — `management/client.rs:22` — validates). Add `auth: AgentAuthSettings` to `ReaperAgentConfig` (`config/mod.rs:41-77`).
- Change the insecure default: `default_bind_address()` (`settings.rs:51-53`) → `"127.0.0.1"`. At startup (`services/reaper-agent/src/main.rs`, before bind at `:548`), **refuse to start** if `bind_address` is non-loopback and `auth.enabled == false` and `tls.require_client_cert == false` — print a hard error. This makes "internet-facing + unauthenticated" impossible without an explicit, auditable opt-out.
- Touch: `crates/reaper-core/src/config/settings.rs`, `crates/reaper-core/src/config/mod.rs`, `services/reaper-agent/src/main.rs`.
- Verify: unit test on the config validator (non-loopback + no auth → error); default-config test asserts loopback bind.

**C2. Build the agent inbound auth extractor/layer.** *(L)*
- Build: a `require_agent_auth` middleware layer in a new `services/reaper-agent/src/auth.rs`, applied in `main.rs:490-539` to the router **before** `.with_state`, exempting only `/health`, `/ready`, `/live`, `/metrics`. Two accepted credentials:
  - **mTLS client cert** — primary. When TLS terminates at the agent (`tls.rs`, `config.tls.require_client_cert`), require and validate the peer cert; when terminated by a trusted proxy, read the configured fingerprint header (mirror `auth/middleware.rs:156-206`). Reuse `AuthMethod::Mtls` semantics.
  - **Bearer token** — secondary/simple. Validate `Authorization: Bearer` against the shared `jwt_secret` (same `JwtManager` shape management uses in `auth/middleware.rs:259-277`) or a static `bearer_token` for the localhost-sidecar case.
- Default-deny: any non-exempt route with neither valid credential → `401`.
- Touch: `services/reaper-agent/src/main.rs` (layer wiring at `:490-539`), new `services/reaper-agent/src/auth.rs`, `services/reaper-agent/src/tls.rs` (surface peer cert), `Cargo.toml` for `reaper-agent` (add `jsonwebtoken` if not already available via `reaper-core`).
- Verify: agent negative-auth DoD test; valid-cert / valid-token happy paths.

**C3. Remove `/debug/datastore` from production.** *(S)*
- Build: gate the route registration at `main.rs:531-532` behind `#[cfg(debug_assertions)]`, or an explicit `config.debug.datastore_endpoint` flag defaulting `false`. Prefer compile-out for release.
- Touch: `services/reaper-agent/src/main.rs:531-532`, `handlers/*` (`debug_datastore`).
- Verify: release-profile test asserts `/debug/datastore` → `404`.

### Phase D — Stretch: dogfood the engine (Synthesis "eat your own dogfood")

**D1. Author a Reaper policy that expresses the control-plane authZ rules** (principal = authenticated user, action = route verb, resource = `org/{id}/bundles` etc.), load it into an embedded `policy-engine` instance in `reaper-management`, and have `authorize_org` (B1) additionally consult it via `PolicyEngine::evaluate`. *(L)* Keep the hard-coded scope checks as the fail-closed backstop; the engine decision is advisory-then-enforcing behind a flag until parity is proven by differential test (compare hard-coded vs engine decision on a matrix of user×route). Touch: `reaper-management` state (add engine), new policy in `test-data/`, `authorize_org`. Verify: differential test shows 100% agreement before flipping to enforce.

---

## 5. Dependencies

- **Ordering:** A (gateway) → B (per-handler authZ) can proceed in parallel once A2's extractor fast-path lands; C (agent) is independent of A/B and can run concurrently. D depends on B1.
- **Shared code:** B reuses the `authorize_org` helper (B1) and the `AuthenticatedUser`/`Scope`/`resolve_org` primitives — no new auth model.
- **Config plumbing:** C1 must land before C2 (extractor reads the new settings).
- **Adjacent workstreams (seams, not blockers):**
  - Security **P0-2** (unverified push-path signature bypass) lives in the same agent handlers (`handlers/policies.rs:308,395`) this plan touches — coordinate so the signature-verification insertion and the auth layer land without conflict; auth alone does **not** close P0-2.
  - API-3 (`panic=abort` + `CatchPanicLayer`) shares the router-layer wiring in both `main.rs` files — sequence the layer additions together.
  - Product **F1** (SSO/SAML/OIDC/SCIM) plugs identities into the gateway this plan builds; the `AuthenticatedUser` shape is the contract.
- **Helm/k8s:** `deploy/helm/reaper/` and `deploy/kubernetes/` agent manifests currently expose `0.0.0.0:8080`; the loopback-default change (C1) requires updating the sidecar/DaemonSet manifests to either use UDS or set an explicit authenticated network bind. Coordinate with deployment owners.

---

## 6. Testing & verification strategy

**Negative tests (the core of this plan):**
- *Unauthenticated → 401:* parametrized test over every non-exempt control-plane route and every non-health agent route asserting `401` with no credentials (DoD gate). Include a `#[cfg(test)]` handler with no `RequireAuth` behind the gateway to prove the layer, not the extractor, is the guarantee.
- *Cross-tenant → 403/404:* seed two orgs A and B with API keys; assert B's key on A's bundle/policy UUID → `404` (scoped repo returns `None`); assert B's key on A's org-slug routes → `403` (`user.org_id != org.id`). Cover `get`/`update`/`delete`/`promote`/`download`/`diff` for bundles and `get`/`update`/`delete` for policies.
- *IDOR probe fails:* enumerate a known-valid foreign `bundle_id` under B's own org path (`/orgs/{B}/bundles/{A-uuid}`) → `404`; assert the SQL predicate includes `org_id` (unit test on `get_by_id_scoped`).
- *Scope insufficiency → 403:* a `Viewer`-scoped token (`auth/middleware.rs:509-513`) → `403` on any mutation; a `BundleWrite`-but-not-`BundlePromote` token → `403` on promote.
- *Agent credential matrix:* valid mTLS cert → `200`; expired/absent cert → `401`; valid bearer → `200`; tampered bearer → `401`.
- *Config fail-closed:* `AgentAuthSettings` validator rejects non-loopback bind with auth disabled (unit test).
- *Debug endpoint gone:* release-profile build → `/debug/datastore` returns `404`.

**Positive/regression:** the existing control-plane and datastore tests (which already pass through `RequireAuth`) must stay green — proves A2's extensions fast-path didn't regress the authenticated path. The datastore route suite is the parity oracle for B1.

**Where:** control-plane tests alongside `services/reaper-management/tests/` and the existing platform BDD; agent tests in `services/reaper-agent/tests/` (add an auth integration test); reuse the process-level data-plane E2E harness referenced at `main.rs:190-198`. Wire a CI job asserting the negative suite is non-empty and green (a "no route without an auth assertion" guard).

**Manual/dogfood check (D):** the differential test comparing hard-coded scope decisions to engine decisions over a user×route matrix must show 100% agreement before enabling enforce mode.

---

## 7. Effort & phasing

| Step | Effort |
|------|--------|
| A1 gateway layer + factor `authenticate` | M |
| A2 extractor fast-path | S |
| B1 extract `authorize_org` | S |
| B2 bundles authZ + `get_by_id_scoped` (IDOR) | M |
| B3 policies authZ + scoping | M |
| B4 orgs/teams/billing authZ | M |
| C1 agent auth config + fail-closed bind | M |
| C2 agent inbound auth layer (mTLS + bearer) | L |
| C3 remove `/debug/datastore` from prod | S |
| D dogfood engine (stretch) | L |

**Rough total:** core P0 close (A+B+C, excluding D) ≈ **2–3 engineer-weeks**; the agent inbound-auth layer (C2) and the bundle/policy scoping sweep (B2/B3) are the long poles. D (stretch) adds ~1 week and is decoupled.

---

## 8. Key decisions (ADR-style)

**D-1: Gateway middleware layer vs per-handler extractor — do BOTH, layer is the guarantee.**
The root cause of P0-3/API-1 is that auth is *opt-in per handler* and five files simply forgot the extractor. A per-handler extractor sweep alone repeats that failure mode the next time someone adds a route. Decision: add a **default-deny middleware layer** (A1) with an explicit unauthenticated allowlist as the structural guarantee, **and** keep `RequireAuth` + `RequireScope` + `authorize_org` in handlers for scope/tenant granularity (a layer can authenticate but not know which org a given route addresses). The extractor reads the layer's result from request extensions (A2) so there's one authentication code path and no double DB hit. Rationale: fail-closed by construction beats fail-closed by discipline; the review explicitly frames the problem as "no default-deny gateway."

**D-2: Agent inbound auth — mTLS primary, bearer secondary, loopback default.**
The agent is the enforcement point; its trust boundary is control-plane→agent and app→agent. Decision: support **both** mTLS client-cert (primary — reuses the already-modelled `AuthMethod::Mtls` and the trusted-proxy fingerprint pattern from management) and a shared/registration-minted **bearer token** (secondary — simplest for the localhost-sidecar deployment, and the agent already holds a management-minted JWT at `management/client.rs:22`). Default bind becomes **loopback/UDS**, and a non-loopback bind without auth is a **startup error**. Rationale: mTLS is the enterprise-grade answer for the network-bind case; bearer covers the common sidecar case without cert-provisioning friction; the fail-closed bind validation removes the "internet-facing by default" footgun (`settings.rs:51-53`) at its source. Rejected: bearer-only (weak for the network boundary the synthesis flags); mTLS-only (too much provisioning friction for the sidecar case, would slow adoption of the fix).

**D-3: Kill IDOR via scoped repository methods, not just handler checks.**
Decision: add `get_by_id_scoped(org_id, id)` to the bundle and policy repositories (`WHERE id = $1 AND org_id = $2`) and route all id-addressed handlers through them, returning `404` on miss — in addition to the `user.org_id == org.id` handler check. Rationale: defense-in-depth — a handler check can be forgotten, but if the *only* id-lookup method requires an org_id, the data layer itself refuses cross-tenant reads. `404` (not `403`) avoids confirming existence of foreign resources.

**D-4: `404` for cross-tenant id-addressed, `403` for cross-tenant org-slug.**
Matches the existing `datastore.rs` precedent (`403` when `user.org_id != org.id`) for slug-addressed routes, while id-addressed lookups return `404` to avoid an existence oracle. Consistent, and mirrors the reused pattern.

---

## 9. Risks & rollback

**Risks:**
- **Breaking legitimate unauthenticated callers.** Internal tooling/tests may currently rely on the open surface (the root-mount comment at `main.rs:211-215` says "existing consumers/tests"). Mitigation: land the allowlist precisely; run the full existing test suite; stage the gateway behind a config flag (`auth.gateway_enforcing`) that defaults **on** but can be flipped for a migration window, with logging of would-be-401s before enforcing.
- **Agent loopback default breaks existing deployments** that expect `0.0.0.0:8080`. Mitigation: the fail-closed validator emits an explicit, actionable error naming the env var to set; update Helm/k8s manifests (see §5) in the same change; document the migration in the release notes.
- **mTLS provisioning friction** could stall rollout of C2. Mitigation: bearer-token mode is the low-friction fallback so C2 is shippable without a cert pipeline; mTLS can be enabled per-deployment.
- **Performance:** the gateway adds a DB-backed API-key/JWT validation per request. Mitigation: reuse the existing `RequireAuth` validation (already on every authenticated route today), share one code path (A2), and rely on the existing short-circuit ordering (API key → mTLS → JWT).
- **Dogfood (D) latency/complexity.** Mitigation: engine decision is behind a flag with the hard-coded check as fail-closed backstop; enforce only after differential parity.

**Rollback:**
- Each phase is independently revertible. A/B: the gateway layer and per-handler extractors can be removed by reverting the `build_router` layer addition and the handler diffs — no schema change (the scoped repo methods are additive; the old unscoped methods remain until callers are migrated, then removed).
- C: config-flag `auth.enabled=false` + reverting the default bind restores prior behavior (explicitly an insecure emergency escape, gated behind the loopback validator so it can't silently re-expose the network surface).
- D: single flag disables engine consultation, reverting to hard-coded scope checks.
- No irreversible migrations: `get_by_id_scoped` is additive SQL; `AgentAuthSettings` is `#[serde(default)]` so old config files still parse.
