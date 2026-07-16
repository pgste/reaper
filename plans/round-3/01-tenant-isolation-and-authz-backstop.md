# Tenant Isolation & Authorization Backstop

> **STATUS (2026-07-16): NOT STARTED** — round-3 remediation planning. This plan
> is the critical path for round 3. Nothing external ships until it lands.
>
> **Round-3 gate:** the security review (`reviews/round-3/02-security.md`) returns
> **NOT READY** on four independent P0 cross-tenant / account-takeover defects and
> two P1s. This plan closes all six **and** installs the structural backstop that
> makes the whole finding-class impossible to reintroduce. It is the "single most
> important next move" the synthesis names (`08-SYNTHESIS.md` §"The single most
> important next move").

**Readiness gate:** Moves **NOT READY → CONDITIONAL**. Rounds 1 and 2 hardened the
*anonymous-attacker* perimeter and the audit pipeline (both verified closed this
round — `02-security.md` §"Absence checks performed"). Round 3 hunts the layer the
round-2 gateway explicitly left to handlers — "does *this* caller belong to *this*
resource's tenant?" — and finds it present in ~20 handler files and **absent or
defeated in five**. For a product whose entire value proposition is *correct
authorization*, a tenant-isolation break is an authorization break in every
application that trusts Reaper. This is disqualifying on sight for a bank review
(SOC 2 CC6, ISO 27001 A.9, multi-tenancy assurance).

**Priority:** P0

**Findings closed:** Security **P0-1** (`R3-1`, OIDC email auto-linking →
account takeover), **P0-2** (`R3-2`, webhook-subscription handlers with no authz),
**P0-3** (`R3-3`, GitHub-App confused deputy), **P0-4** (`R3-4`, fail-open
bundle-update webhook), **P1-b** (`R3-6`, deployment resource-org recheck missing);
and the SSRF P1 (`R3-5`) is closed as an inseparable part of P0-4's fetch path.
Synthesis top-10 ranks #1, #2, #3, #4, #7. Adjacent P2/P3s (`R3-7`..`R3-17`) are
*out of scope here* and tracked separately (see §5) — this plan is the P0/P1
tenant-isolation lane only.

---

## 1. Goal

Make cross-tenant access impossible **by construction**, not by handler discipline.

Two deliverables, in priority order:

1. **A structural authorization backstop.** The round-2 gateway
   (`auth/gateway.rs`) authenticates every non-public route and stops there — "the
   gateway authenticates only … per-handler scope and tenant checks remain the
   handlers' responsibility" (`gateway.rs:10-14`). That delegation has **no
   backstop**: a handler that forgets the tenant check silently becomes a
   cross-tenant hole, and the existing contract test proves only that routes are
   *authenticated*, not *authorized-to-the-right-tenant* (`08-SYNTHESIS.md`
   §"single most important cross-cutting theme"). We add (a) a single
   **resource-ownership guard** (`authorize_resource`) + org-scoped repository
   reads that every mutating, resource-addressed route must pass, and (b) a
   **fitness function in `tests/api_contract.rs`** that fails CI when a mutating
   route reaches a by-id resource without a resource-tenant authorization — turning
   the whole finding-class off.

2. **The four P0 attack paths closed, each with a regression test.** OIDC issuer↔org
   trust binding (P0-1); `RequireAuth` + `authorize_org` on all six
   webhook-subscription handlers (P0-2); server-side installation-id binding (P0-3);
   fail-closed webhook signature + SSRF-guarded fetch (P0-4); resource-org recheck
   on every by-id deployment mutation (P1-b).

**The adjudication that drives the design** (`08-SYNTHESIS.md`): Code & API rated
this surface "no open P0/P1" having enumerated routes for *authentication*;
Security's token→query→resource trace found four P0s. Security wins, and *the
divergence itself* is the lesson — "'every route has an auth check' is the wrong
fitness function; it must be 'every route authorizes the resource's owning
tenant.'" The fitness function in this plan encodes exactly that property.

**Non-goals:** the CI/CD supply-chain P0 (separate lane); the auto-rollback signal
P1 (`PROD P1-new`, separate plan); the DSL-as-contract and CLI-verification P1s.
Those are finishing work on a sound system; this plan is the tenant-isolation lane
that must land first.

---

## 2. Current state (evidence)

### 2.1 The structural gap — authenticated, not authorized

- `auth/gateway.rs:78-111` `require_authentication` runs `RequireAuth`, stashes the
  `AuthenticatedUser` in extensions, and calls the handler. It never consults the
  resource's org. Its own doc comment concedes the boundary: *"The gateway
  authenticates only — per-handler scope and tenant checks remain the handlers'
  responsibility (a layer can't know which org a given route addresses)"*
  (`gateway.rs:10-14`).
- The correct per-resource pattern **exists and is reused in ~20 files**:
  `api/orgs.rs:323-347` `authorize_org(state, user, org_ref, &[Scope]) ->
  Organization` performs scope-check → `resolve_org` → `user.org_id !=
  organization.id && !Admin → Forbidden`, returning the *resolved* org so handlers
  stop trusting the path. Bundles route every id-addressed op through
  `authorize_org` + `bundle_service.get_scoped(org_id, bundle_id)`
  (`api/bundles.rs:139,182,212-215,251-255,301-307`). Deployment `strategies.rs`
  rechecks `strategy.org_id != organization.id → NotFound` after the by-id fetch
  (`deployments/strategies.rs:141-142,184-185`).
- The finding-class is precisely the handful of routes that skipped this pattern.
  `02-security.md` §10: *"The regression is structural, not knowledge … The P0s are
  the handful of routes that skipped the established pattern — which is exactly what
  an auditor flags as 'the isolation model is not enforced by construction.'"*

### 2.2 P0-1 — OIDC email auto-linking → cross-tenant account takeover

- `auth/sso/broker.rs:62-82` `establish_session` resolves the user in three
  branches: by `(issuer, subject)` (`:63`); **else by global email**
  (`find_by_email` at `:67`) — on a hit it links the attacker's IdP identity to the
  *pre-existing* account and returns it (`:68-72`); else provisions.
- `ExternalIdentity.email_verified` is captured (`broker.rs:26`) and the doc claims
  a *"pre-existing verified-email account"* (`broker.rs:60-61`) — but the middle
  branch **never checks `email_verified`**, and even checking it does not fix the
  break because the attacker controls the IdP.
- **Exploit chain:** `signup` makes you Owner of your own org; `PUT
  /orgs/{me}/sso/config` is gated only by `OrgAdmin` on *your own* org
  (`api/auth/sso.rs:126`), so you point `issuer`/`jwks_url`/`client_id` at an IdP
  you control; run your org's OIDC login asserting `email =
  victim@othercorp.com`, signed by your key. JWKS validation passes (your
  key/issuer/aud), nonce matches, and `establish_session` links your
  `(issuer,subject)` to the **victim's** account and mints a session for the
  victim's `user_id`. Sessions are user-bound, not org-bound — membership resolves
  per request-path org — so `/orgs/{victim_org}/…` now runs at the victim's role.
- The trust-model root cause: adoption crosses a trust boundary (a
  *tenant-self-served* IdP) by a global attribute (email). `SsoConfig` is per-org
  and carries no "platform-trusted issuer" flag (fields at `broker.rs:205-217`).

### 2.3 P0-2 — webhook-subscription handlers: zero authorization

- `api/webhook_subscriptions.rs:133,168,238,265,324,357`
  (`list/create/get/update/delete/test_webhook`) take `State` + `Path` + `Json`
  only. `grep RequireAuth` in the file = 0. Each resolves the org by slug for a
  UUID lookup, **not** an authorization: e.g. `list_webhooks`
  (`:133-153`) does `resolve_org(&org_repo, &org)` (`:139`) then
  `webhook_repo.list_by_org(organization.id, …)` (`:142-143`) — the caller is never
  bound to that org.
- Under the default **Enforcing** gateway the caller is *authenticated* but any
  authenticated principal of any org manages any org's subscriptions by slug (slugs
  are human-readable, enumerable). `create_webhook` (`:168-172+`) with an
  attacker-controlled `url`+`secret` subscribes the attacker to the victim org's
  event stream (decisions, bundle-promotions). Directly contradicts the gateway's
  own comment that *"webhook subscription management … stays authenticated"*
  (`gateway.rs:69-71`).

### 2.4 P0-3 — GitHub-App confused deputy (conditional on App configured)

- `sync/git.rs:107-119` `resolve_auth`: if the source config carries
  `installation_id` + `repo_full_name`, it mints
  `app.installation_token(installation_id)` for **whatever installation the stored
  config names** (`:116`) and clones `https://github.com/{repo_full_name}.git`
  (`:117`). The App client is a single shared `Arc<GitHubAppClient>` serving all
  tenants.
- `api/sources.rs:258` `validate_source_config(source_type, &request.config)` then
  `source_repo.create(organization.id, input)` (`:268`) passes `request.config` (a
  raw `serde_json::Value`) straight to persistence. `validate_source_config` for
  `Git` (`:577-585`) checks only that `url` is present — it **never rejects or
  scopes** `installation_id`/`repo_full_name`.
- **Exploit:** a tenant with `policy:write` in their own org creates a Git source
  whose config JSON names a *victim's* `installation_id`; at sync time the shared
  control plane mints the victim's installation token (a GitHub App can mint tokens
  for any installation of itself) and clones the victim's private repos into the
  attacker's org, materializing them as policies.
- **The correct pattern already exists next door:** `api/oauth/github.rs:476`
  `create_source_from_github` derives the installation id **server-side, bound to
  the caller's org**, via `get_github_installation_id(&state, organization.id)`
  (`api/oauth/helpers.rs:95`) and only then writes `installation_id` into config
  (`github.rs:498`). The generic source API bypasses this.

### 2.5 P0-4 — fail-open bundle-update webhook + unguarded SSRF fetch

- `/webhooks/bundle-update` is public-allowlisted (`gateway.rs:72-73`). In
  `process_bundle_webhook` (`api/webhooks.rs:119-220`) signature verification is
  gated `if config.webhook_secret.is_some()` (`:153`) — a `BundleUrl` source
  configured **without** a secret skips verification entirely.
- An unauthenticated caller then supplies `source_id` + `bundle_url`
  (`webhooks.rs:82-89`); the server fetches that attacker URL via
  `BundleUrlSyncer::fetch_bundle` (`:175-183`), stores it (`:186-189`), and
  broadcasts `BundlePromoted` to the org's agents (`:201-210`). The fetch path
  (`sync/bundle_url.rs:90-104`) has **no `url_guard`** and attaches the source's
  `auth_token` — so this is unauthenticated **SSRF + credential exfiltration +
  control-plane store poisoning**.
- Contrast the fail-closed git webhook (`api/webhooks_git.rs:141-148`, HMAC
  required, `subtle::ct_eq`). Enforcement-plane blast radius is contained by the
  agent's fail-closed signature verification (`require_signed_bundles=true`
  default), but the control-plane SSRF/exfil stands.
- **The SSRF P1 (`R3-5`) is the same fetch path:** `sync/url_guard.rs` is solid
  (https-only; rejects loopback/RFC1918/link-local/metadata/CGNAT/IPv6) and applied
  to git (`git.rs:144`) and JWKS, but **not** to the API source (`sync/api.rs:80`)
  or the bundle-URL fetch (`sync/bundle_url.rs:104`); and **no** sync
  `reqwest::Client` sets a redirect policy, so a public host that passes the guard
  can `302 → 169.254.169.254` and defeat even the guarded paths. Closing P0-4's
  fetch without closing this is illusory, so they land together.

### 2.6 P1-b — deployment by-id mutations skip the resource-org recheck

- `deployments/rollouts.rs:355-376` `cancel_rollout` calls
  `authorize_deploy(&state, &user, &org, …)` (checks caller ∈ **path** org +
  `DeploymentWrite`), then acts on `rollout_id` by **global UUID** —
  `get_rollout_by_id(id)` has no org argument (`deployment/service/mod.rs:507`),
  and there is **no `rollout.org_id == organization.id` recheck**.
- Same shape in `approve_wave` (`rollouts.rs:317`), `create_pin`/`delete_pin`
  (`pins.rs:34,121` — note `_organization` is discarded at `:40,126`),
  `acknowledge_deployment` (`status.rs:153`), and the by-id reads
  (`status.rs:35,74,113`). `strategies.rs:141-142,184-185` shows the correct
  recheck — so these are omissions, not a missing design. An attacker with
  `DeploymentWrite` in their own org mutates any rollout in any org by ID; only
  UUID opacity stands in the way (P0-adjacent — rated P1 because it needs a valid
  foreign UUID).

### 2.7 What the existing contract test does *not* catch

`tests/api_contract.rs` gates the OpenAPI surface: no undocumented raw routes
(`:78-111`), spec populated (`:113-166`), publishable error model (`:240-401`). It
verifies *documentation and authentication-shape*, never *per-tenant
authorization*. This is the blind spot the synthesis adjudication names — the
codebase's own fitness function checks the wrong property.

---

## 3. Definition of Done — testable checkboxes

**Structural backstop (the heart):**
- [ ] A single `authorize_resource(state, user, org_ref, resource_org_id, &[Scope])`
      guard exists and is the one place a by-id resource's owning tenant is verified
      against the caller; it composes `authorize_org` + the `resource_org_id ==
      organization.id → NotFound` recheck.
- [ ] Every id-addressed *repository read* used by a mutating handler is org-scoped
      (`get_scoped(org_id, id)` / a `WHERE … AND org_id = $n` predicate), mirroring
      `bundle_service.get_scoped`; a grep shows no mutating by-id handler calling an
      unscoped `get_by_id`.
- [ ] The extractor/guard is adopted **repo-wide** across the enumerated route list
      (§4.2) — not only the five P0 files.
- [ ] **Fitness function green and proven:** `tests/api_contract.rs` (or a new
      `tests/tenant_authz.rs`) fails CI when a mutating, resource-addressed route
      reaches its handler body without a resource-tenant authorization primitive; a
      deliberately-unguarded canary route makes the test **red** (proven, checked
      into the test as a negative fixture), and removing the guard from any real P0
      handler also makes it red.

**P0-1 (OIDC):**
- [ ] Email adoption of a pre-existing account happens **only** when the asserting
      issuer is a platform-trusted issuer bound to that account's own org; a
      tenant-self-served IdP asserting a foreign email provisions a **distinct** SSO
      user and never adopts. Regression test replays the exploit and asserts no
      cross-account link + session bound to the authenticating org.

**P0-2 (webhook subscriptions):**
- [ ] All six handlers take `RequireAuth` + `authorize_org(&[OrgAdmin])`; a
      cross-tenant CRUD/test by slug returns `403`; anonymous returns `401`.
      Regression test covers all six verbs.

**P0-3 (GitHub App):**
- [ ] `create_source`/`update_source` **reject** `installation_id`/`repo_full_name`
      in the generic config blob; installation identity is resolved server-side from
      a caller-org-bound connection record. Regression test: a source config naming a
      foreign `installation_id` is rejected at create, and sync never mints a token
      for an installation not bound to the source's org.

**P0-4 (webhook fail-open + SSRF):**
- [ ] Signature verification is **unconditional** — a `BundleUrl` source with no
      secret is rejected at create, and `process_bundle_webhook` fails closed (`401`)
      when a secret is absent or the signature is missing/invalid. The fetch applies
      `url_guard` and sets `redirect(Policy::none())`. Regression tests: secretless
      webhook → `401`; `302 → 169.254.169.254` → blocked.

**P1-b (deployment):**
- [ ] Every by-id deployment mutation (`cancel_rollout`, `approve_wave`,
      `create_pin`, `delete_pin`, `acknowledge_deployment`) and by-id read rechecks
      `resource.org_id == organization.id` (or scopes the repo query), returning
      `404` cross-tenant. Regression test drives a foreign rollout UUID under the
      attacker's org path → `404`.

Full negative suite (§6) green in CI.

---

## 4. Critical steps

### Phase A — The structural backstop (do first; everything else adopts it)

**A1. Add the `authorize_resource` guard.** *(S)*
- Build: one function next to `authorize_org` in `api/orgs.rs`, composing the
  existing org check with the resource-org recheck that `strategies.rs` does by
  hand. It returns the resolved org so handlers stop trusting the path.
  ```rust
  /// Authorize `user` for a resource fetched by GLOBAL id.
  ///
  /// Composes org authorization with the resource-org recheck. `resource_org_id`
  /// is the owning org of the already-fetched resource. Returns 404 (not 403) on
  /// a cross-tenant resource so a foreign id is not an existence oracle.
  pub async fn authorize_resource(
      state: &AppState,
      user: &AuthenticatedUser,
      org_ref: &str,
      resource_org_id: Uuid,
      required: &[Scope],
  ) -> ApiResult<Organization> {
      let org = authorize_org(state, user, org_ref, required).await?; // scope+org+resolve
      if resource_org_id != org.id {
          return Err(ApiError::NotFound("resource not found".into()));
      }
      Ok(org)
  }
  ```
- Prefer the **org-scoped repository read** as the primary defense (the data layer
  refuses cross-tenant reads even if a handler forgets the guard); `authorize_resource`
  is the handler-level recheck for repos not yet scoped. Where a `get_scoped(org_id,
  id)` exists or can be added cheaply, use it and return `404` on `None`.
- Touch: `api/orgs.rs` (new fn), `api/error.rs` (reuse `NotFound`).
- Verify: unit test — same-org resource passes, foreign-org resource → `404`,
  missing scope → `403`.

**A2. The fitness function — fail CI on an unguarded mutating route.** *(M)*
- This is the deliverable that makes the finding-class un-reintroducible. It encodes
  the property the synthesis names: *every mutating, resource-addressed route
  authorizes the resource's owning tenant.* Build it in the same static-analysis
  idiom as the existing `no_undocumented_raw_routes` (`api_contract.rs:57-111`),
  which already parses handler source textually — extend that machinery rather than
  invent a new one.
- Algorithm (static, source-scanning `src/api/**/*.rs`):
  1. Enumerate handler fns whose `#[utoipa::path(...)]` verb is a **mutation**
     (`post`/`put`/`patch`/`delete`) **and** whose `path = "..."` template contains
     a resource id segment after an `{org}` segment (i.e. addresses a specific
     resource by id: `…/{org}/…/{something_id}`), OR is a non-org-scoped mutation
     not on the explicit allowlist.
  2. For each, read the handler body and require it references at least one member
     of the **approved authorization set**: `authorize_resource`, `authorize_org`,
     `authorize_deploy`, `get_scoped`, `load_scoped`, or an explicit
     `.org_id == organization.id` recheck.
  3. Fail with the offending `verb path (fn)` list if any lack it. Maintain a
     small, justified `TENANT_AUTHZ_EXEMPT` allowlist for the *legitimately*
     non-tenant-scoped mutations — `POST /orgs` (create org: any authenticated
     user, sets self as owner), `/auth/*`, `/scim/*` (org derived from the bearer
     token, not the path — `02-security.md` "SCIM: SAFE"), the two signed webhooks
     (authenticate by signature). Every entry carries a one-line justification, and
     the list is asserted non-growing (a ratchet, like `MISSING_4XX_BASELINE` at
     `api_contract.rs:226`).
  4. **Prove it catches:** add a `#[cfg(test)]` negative fixture — a canary handler
     annotated as a mutating resource route with no authorization call — and a
     meta-test asserting the checker flags exactly that fixture. This is the
     "proven to catch a deliberately-unguarded route" DoD item; it guards the guard.
- Rationale for static over purely-dynamic: a dynamic cross-tenant probe (see §6)
  is the *behavioural* proof for the enumerated P0 routes, but it needs a valid
  request body per route and cannot cover a route nobody wrote a probe for. The
  static check is total over the surface and runs in milliseconds — it is the CI
  gate; the dynamic suite is the belt-and-braces for the known-hot routes.
- Touch: `services/reaper-management/tests/api_contract.rs` (or new
  `tests/tenant_authz.rs`), a canary fixture module.
- Verify: the meta-test (canary → red); the whole suite green after Phases B–E.

**A3. Adopt repo-wide.** *(M)*
- Sweep the enumerated route list (§4.2) so every mutating by-id handler goes
  through `authorize_resource`/`get_scoped`. This *is* the fix for P1-b and the
  hardening the fitness function then locks in. Order this so the fitness function
  goes green as the sweep completes.

#### 4.2 Route audit list (must pass the guard)

Mutating, resource-addressed routes to route through the backstop — the audit
surface the fitness function enforces:

| File | Handlers | Guard to apply |
|------|----------|----------------|
| `api/webhook_subscriptions.rs` | `create/get/update/delete/test_webhook` (`:168,238,265,324,357`) + `list` (`:133`) | `RequireAuth` + `authorize_org(&[OrgAdmin])` (P0-2) |
| `api/sources.rs` | `create_source` (`:258`), `update_source` (`:288`) + by-id ops | keep `authorize_org`; add config-blob rejection (P0-3) |
| `api/deployments/rollouts.rs` | `cancel_rollout` (`:355`), `approve_wave` (`:317`), rollback (`:400,457`) | `authorize_resource(rollout.org_id)` (P1-b) |
| `api/deployments/pins.rs` | `create_pin` (`:34`), `delete_pin` (`:121`) | `authorize_resource(pin.org_id)` (P1-b) |
| `api/deployments/status.rs` | `acknowledge_deployment` (`:153`) + by-id reads (`:35,74,113`) | `authorize_resource(...)` (P1-b) |
| `api/webhooks.rs` | `process_bundle_webhook` (`:119`) | unconditional signature (P0-4) |
| `auth/sso/broker.rs` | `establish_session` (`:50`) | issuer↔org trust binding (P0-1) |

Already-correct references the sweep must not regress: `api/bundles.rs`
(`authorize_org` + `get_scoped`), `deployments/strategies.rs:141` (recheck),
`change_requests.rs` (`load_scoped`), `api/scim/*` (token-derived org).

### Phase B — P0-2: webhook-subscription authorization *(S)*

**B1.** Add `RequireAuth(user): RequireAuth` to all six handlers and replace the
bare `resolve_org` with `authorize_org(&state, &user, &org, &[Scope::OrgAdmin])`,
using the returned `organization.id` for the repo call — the exact `bundles.rs`
shape (`bundles.rs:135-143`).
```rust
async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    RequireAuth(user): RequireAuth,          // + gateway already authenticated
    Json(request): Json<CreateWebhookRequest>,
) -> ApiResult<(StatusCode, Json<WebhookSummary>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    // … webhook_repo scoped to organization.id …
}
```
Read scope `OrgAdmin` for all six (subscription management is an admin action;
`test_webhook` can exfiltrate, so it is not a read-only exception).
- Touch: `api/webhook_subscriptions.rs` (six handlers).
- Verify: cross-tenant `403`; anonymous `401`; the fitness function now passes these
  six (was the primary offender).

### Phase C — P0-3: server-side installation-id binding *(M)*

**C1. Reject installation identity from the config blob.** In
`validate_source_config` for `Git` (`sources.rs:577-585`), reject a config that
carries `installation_id` or `repo_full_name` with a `422` — these are never
client-supplied.
```rust
SourceType::Git => {
    if config.get("url").and_then(|v| v.as_str()).is_none() {
        return Err(ApiError::Validation("Git source requires 'url'".into()));
    }
    if config.get("installation_id").is_some() || config.get("repo_full_name").is_some() {
        return Err(ApiError::Validation(
            "installation_id/repo_full_name are server-resolved; use the GitHub \
             connection flow (POST /orgs/{org}/sources/github)".into(),
        ));
    }
}
```

**C2. Resolve installation server-side, org-bound.** For App-backed git sources,
route creation through the caller-org-bound helper the OAuth path already uses —
`get_github_installation_id(&state, organization.id)`
(`api/oauth/helpers.rs:95`, as `create_source_from_github` does at
`github.rs:476-498`) — so the stored `installation_id` is always the caller org's.
Defense-in-depth at sync time: `resolve_auth` (`sync/git.rs:107`) should assert the
minted installation's org matches the source's `org_id` before use.
- Touch: `api/sources.rs` (validate + create path), `sync/git.rs` (sync-time
  assertion), reuse `api/oauth/helpers.rs`.
- Verify: foreign-`installation_id` config → `422` at create; a source whose stored
  installation does not belong to its org fails closed at sync.

### Phase D — P0-4: fail-closed webhook + SSRF guard *(M)*

**D1. Unconditional signature, fail-closed.** In `process_bundle_webhook`
(`webhooks.rs:152-170`) remove the `if config.webhook_secret.is_some()` gate; a
missing secret is a **misconfiguration that fails closed**, mirroring
`webhooks_git.rs:141-148`.
```rust
let secret = config.webhook_secret.as_deref().ok_or_else(|| {
    ApiError::Unauthorized("bundle-update webhook requires a configured secret".into())
})?;
let signature = headers
    .get("x-webhook-signature").or_else(|| headers.get("x-hub-signature-256"))
    .and_then(|v| v.to_str().ok())
    .ok_or_else(|| ApiError::Unauthorized("missing webhook signature".into()))?;
if !BundleUrlSyncer::default().validate_webhook_signature(config, body, signature)? {
    return Err(ApiError::Unauthorized("invalid webhook signature".into()));
}
```
Also reject at **create/update** a `BundleUrl` source without a `webhook_secret`
(in `validate_source_config`) so the fail-closed state is unreachable in normal
operation, not just caught at ingest.

**D2. SSRF-guard the fetch + kill redirect-following (closes R3-5).** Apply
`url_guard::validate_public_https_url` to the bundle-URL fetch
(`sync/bundle_url.rs:104`) and the API source fetch (`sync/api.rs:80`), and set
`.redirect(reqwest::redirect::Policy::none())` on **every** sync `ClientBuilder`
(`api.rs:35`, `bundle_url.rs:68`, `s3.rs:54`, `github_app.rs`, `jwks.rs:149`) so a
`302 → metadata` cannot bypass the pre-flight guard. Never attach `auth_token` to a
host that has not passed the guard.
- Touch: `api/webhooks.rs`, `api/sources.rs` (create-time secret requirement),
  `sync/bundle_url.rs`, `sync/api.rs`, all sync clients.
- Verify: secretless webhook → `401`; `302 → 169.254.169.254` blocked; `auth_token`
  never sent to an unguarded host.

### Phase E — P0-1: OIDC issuer↔org trust binding *(M)*

**E1. Model the trust boundary.** The break is that email adoption crosses a trust
boundary (a tenant-self-served IdP) using a global attribute (email). Add a
**platform-trusted-issuer** notion: an issuer is adoption-eligible only when it is
registered as platform-trusted (an operator-controlled allowlist, not a per-org
self-served config) **and** bound to the account's own org. The `SsoConfig` an org
admin can `PUT` is by definition *not* platform-trusted.

**E2. Rewrite the middle branch of `establish_session`** (`broker.rs:67-72`):
```rust
None => match users.find_by_email(&identity.email).await? {
    // Adopt a pre-existing account ONLY across a trusted boundary:
    // - the IdP is a platform-trusted issuer (not a tenant-self-served config), AND
    // - it is bound to THIS account's own org, AND
    // - the email is verified by that trusted IdP.
    Some(u)
        if identity.email_verified
            && is_platform_trusted_issuer(db, &identity.issuer).await?
            && issuer_bound_to_user_org(db, &identity.issuer, u.id).await? =>
    {
        users.link_idp_identity(u.id, &identity.issuer, &identity.subject).await?;
        u
    }
    // Otherwise never adopt: provision a DISTINCT SSO user for this (issuer,subject).
    _ => {
        let u = User::external(identity.email.clone(), identity.email_verified);
        users.create_external(&u, &identity.issuer, &identity.subject).await?;
        u
    }
},
```
Also **bind the session to the authenticating org**: carry `org_id` on the session
so a session minted via org A's IdP cannot be replayed against org B (defense in
depth — even a future adoption bug cannot grant multi-org reach). This addresses the
"sessions are user-bound not org-bound" amplifier the review flags
(`02-security.md` R3-1).
- Touch: `auth/sso/broker.rs`, `auth/sso/store.rs` (trusted-issuer registry +
  `issuer_bound_to_user_org`), session model (org binding), `api/auth/sso.rs`
  (thread the authenticating org).
- Verify: replay the full exploit (self-served IdP asserting a foreign verified
  email) → provisions a distinct user, no link to the victim, session scoped to the
  attacker's own org.

---

## 5. Dependencies

- **Internal ordering:** A1 (guard) → A2 (fitness function) → A3 (repo-wide sweep,
  which subsumes P1-b). B/C/D/E are independent of each other and can run in
  parallel once A1 lands; each makes its slice of A2 go green. Land A2's *checker*
  early (red is expected) so it drives the sweep; the suite turns green as B–E
  complete. Merge with A2 green as the gate.
- **Reuses existing primitives — no new auth model:** `authorize_org`
  (`orgs.rs:323`), `RequireAuth`/`AuthenticatedUser` (`auth/middleware.rs`),
  `Scope::{OrgAdmin,DeploymentWrite,…}` (`auth/scopes.rs`), `get_scoped`
  (`bundle/service.rs`), `url_guard` (`sync/url_guard.rs`),
  `get_github_installation_id` (`oauth/helpers.rs:95`), `webhooks_git.rs`
  fail-closed HMAC as the template for D1.
- **Builds on round-1 Plan 01:** that plan installed the default-deny *gateway*
  (`auth/gateway.rs`) and the `authorize_org` helper. This plan adds the *resource*
  layer the gateway explicitly delegates. The `AuthenticatedUser` contract is
  unchanged, so SSO/SCIM (Plan 03) flow through untouched.
- **Adjacent findings tracked separately (not this plan):** `R3-7` unbounded regex
  cache, `R3-8` non-verifying JWT builtin, `R3-9` Stripe webhook stub, `R3-10`
  anti-rollback floor persistence, `R3-11` plaintext source creds, `R3-12`
  `redirect_uri` from Host, `R3-13`..`R3-17` (P3s). The CI/CD supply-chain P0 and
  the auto-rollback-signal P1 are separate round-3 plans. Called out so seams line
  up: `R3-11` (encrypt source creds) touches the same `policy_sources.config` blob C
  hardens — sequence so the field-encryption and the installation-id rejection land
  without conflict.

---

## 6. Testing & verification strategy

**The fitness function is itself a test** (A2) — the primary CI artifact. It must:
be total over `src/api/**`, ratchet its exemption allowlist (non-growing), and ship
with the canary meta-test proving it goes red on an unguarded mutating route.
Removing `authorize_org` from any of the six webhook-subscription handlers, or the
recheck from a deployment handler, must independently turn it red.

**Behavioural cross-tenant suite (belt-and-braces for the hot routes):** seed two
orgs A and B with API keys/sessions.
- *P0-1:* stand up a mock tenant-self-served IdP asserting `email = A-user@…`;
  drive B's org SSO login; assert a **distinct** user is provisioned, no
  `idp_identity` link to A's user, and the minted session is scoped to B. Second
  case: a platform-trusted issuer bound to A's org *does* adopt (positive path).
- *P0-2:* B's token on `POST/GET/PUT/DELETE /orgs/{A}/webhooks[/…]` and
  `test_webhook` → `403`; anonymous → `401`. All six verbs.
- *P0-3:* `create_source` with a Git config JSON carrying a foreign
  `installation_id`/`repo_full_name` → `422`; a source whose stored installation is
  not org-bound → sync fails closed (no token minted).
- *P0-4:* secretless `BundleUrl` source rejected at create; `/webhooks/bundle-update`
  with no/invalid signature → `401`; a fetch target that `302`s to
  `169.254.169.254` (and to an RFC1918 host) → blocked; `auth_token` never leaves
  for an unguarded host.
- *P1-b:* B (a `DeploymentWrite` member of B) drives A's rollout/pin UUID under
  `/orgs/{B}/…` and under `/orgs/{A}/…` with B's token → `404` for
  `cancel_rollout`/`approve_wave`/`create_pin`/`delete_pin`/`acknowledge_deployment`
  and the by-id reads.

**Regression / parity:** the existing `api_contract.rs` gates
(`no_undocumented_raw_routes`, `contract_is_publishable`) stay green; the ~20
already-correct handler files (`bundles.rs`, `strategies.rs`, `change_requests.rs`,
`scim/*`) must not regress under the sweep — they are the parity oracle for the
guard's shape.

**Where:** `services/reaper-management/tests/` (extend `api_contract.rs` /
`integration_tests.rs`; the SSO exploit belongs in an integration test with a mock
IdP; SSRF cases can unit-test `url_guard` application + the redirect policy). Wire a
CI assertion that the tenant-authz fitness function's exemption allowlist is
non-empty *and* did not grow.

---

## 7. Effort & phasing

| Step | Effort |
|------|--------|
| A1 `authorize_resource` guard | S |
| A2 fitness function + canary meta-test | M |
| A3 repo-wide sweep (subsumes P1-b) | M |
| B1 webhook-subscription authz (P0-2) | S |
| C1/C2 installation-id server-binding (P0-3) | M |
| D1/D2 fail-closed webhook + SSRF guard (P0-4 + R3-5) | M |
| E1/E2 OIDC issuer↔org trust binding (P0-1) | M |

**Rough total:** ≈ **1.5–2.5 engineer-weeks**. Long poles: the OIDC trust-model
rework (E, because it also touches the session-org binding) and the fitness
function (A2, because getting the static analysis both sound *and* non-flaky — the
canary is what makes it trustworthy). B is a half-day mechanical sweep; the
disproportion between B's tiny fix and its P0 impact is the whole argument for A2.

---

## 8. Key decisions (ADR-style)

**D-1: A structural backstop, not a fourth patch. Static fitness function is the
gate; dynamic probes are secondary.**
The synthesis is explicit: *"this class of defect must be made impossible to
reintroduce, not merely patched four times"* (`08-SYNTHESIS.md`). Patching the five
handlers closes today's holes but repeats the failure mode on the next handler
someone adds without the check. Decision: the **primary** deliverable is the
fitness function encoding "every mutating, resource-addressed route authorizes the
resource's owning tenant," built as source-scanning static analysis (extending the
existing `no_undocumented_raw_routes` machinery) with a ratcheted exemption
allowlist and a canary meta-test. A purely-dynamic cross-tenant probe suite was
considered as the gate and rejected as *insufficient alone*: it needs a hand-written
request body per route and cannot cover a route nobody wrote a probe for — so a new
unguarded handler slips through exactly as today. The static check is total over the
surface; the dynamic suite is the behavioural proof for the enumerated hot routes.
Trade-off: static analysis can be fooled by an authorization call that is present
but wrong (e.g. `authorize_org` without the resource recheck). Mitigation: A3 makes
the *org-scoped repository read* (`get_scoped`) the canonical path so the data layer
refuses cross-tenant reads even when a handler-level call is present-but-weak, and
the dynamic suite covers the known-hot routes behaviourally. The residual — a novel
handler with a present-but-subtly-wrong check — is smaller than today's "no check at
all" class and is what code review + the dynamic suite backstop.

**D-2: `authorize_resource` composes `authorize_org` + recheck; org-scoped repo
reads are the deeper defense.**
Two layers, deliberately. The handler-level `authorize_resource` gives a uniform
call site the fitness function can detect; the repository-level `get_scoped(org_id,
id)` makes the *data layer itself* refuse a cross-tenant read, so a forgotten
handler check still fails closed (the `bundles.rs` precedent). Decision: prefer
`get_scoped` where a scoped read exists or is cheap to add; use `authorize_resource`
as the recheck for resources whose repos are not yet scoped (the deployment
by-id reads). Rejected: handler-check-only (a check can be forgotten — the exact
round-3 regression) and repo-scoping-only (some flows legitimately need the resolved
org object, and a uniform call site is what the fitness function keys on).

**D-3: `404` for cross-tenant by-id, `403` for cross-tenant org-slug.**
Matches the existing precedent — `authorize_org` returns `403` on `user.org_id !=
organization.id` (`orgs.rs:340-344`) for slug-addressed routes, while
`strategies.rs:142` returns `404` on a cross-tenant by-id resource. Decision: keep
that split. A by-id lookup returns `404` so a foreign UUID is not an existence
oracle; a slug route returns `403` because the org's existence is not secret. This
is the least-surprising behaviour and mirrors what auditors expect.

**D-4: OIDC — never adopt across a trust boundary by email; bind sessions to the
authenticating org.**
`email_verified` enforcement alone does **not** fix P0-1: the attacker controls the
IdP and can assert `email_verified=true` (`02-security.md` R3-1). Decision: adoption
of a pre-existing account requires the asserting issuer to be **platform-trusted**
(operator-registered, not a per-org self-served `SsoConfig`) **and** bound to that
account's own org; otherwise provision a distinct SSO user. Additionally bind the
session to the authenticating org so a session minted via org A's IdP cannot run as
the user in org B. Rejected: (a) `email_verified`-only (defeated by attacker-controlled
IdP); (b) dropping email adoption entirely (breaks the legitimate "corporate IdP
adopts my existing account" flow — the trusted-issuer gate preserves it); (c)
matching on verified domain (a self-served IdP can still assert any domain).

**D-5: Fail closed on a missing webhook secret, at create *and* at ingest.**
The bug is a fail-*open* conditional (`if secret.is_some()`). Decision: require the
secret at source create/update (so the fail-closed state is normally unreachable)
*and* make ingest fail closed when it is somehow absent — two independent gates,
mirroring `webhooks_git.rs`. Rejected: ingest-only enforcement (leaves a
misconfigured source that silently accepts unsigned webhooks until someone probes
it) and create-only (a pre-existing secretless source would still fail open at
ingest).

---

## 9. Risks & rollback

**Risks:**
- **Fitness-function false positives block unrelated PRs.** A legitimately
  non-tenant-scoped mutation (org create, SCIM, signed webhooks) trips the checker.
  Mitigation: the ratcheted, justified `TENANT_AUTHZ_EXEMPT` allowlist; the canary
  meta-test guarantees the checker's logic is exercised, so a false-positive shows up
  as an allowlist gap with a clear message, not a mystery failure.
- **Fitness-function false negatives (present-but-wrong check).** Covered by D-1's
  two-layer defense (org-scoped repo reads) + the dynamic suite on hot routes; the
  residual is smaller than the class it replaces.
- **OIDC trusted-issuer model breaks a legitimate adoption flow** for early SSO
  design partners who rely on email adoption today. Mitigation: seed the
  trusted-issuer registry with any genuinely operator-vetted issuers during
  migration; the provision-distinct-user fallback is safe (no data exposure), just
  less convenient — document it in release notes.
- **Session-org binding regresses multi-org users** who legitimately switch orgs in
  one session. Mitigation: bind at authentication time but resolve membership
  per-request as today for orgs the user *already* belongs to; the binding blocks
  *cross-account* replay, not legitimate multi-org membership. Validate against the
  existing multi-org session tests.
- **SSRF guard + `redirect(none)` breaks a legitimate redirecting bundle host.**
  Mitigation: legitimate bundle/artifact hosts serve `200` on a stable URL; a
  redirecting host is exactly the SSRF vector. If a real deployment needs one
  hop, add an explicit, guarded, single-hop follow later — not in this P0 fix.
- **Installation-id rejection breaks an existing App-backed source** created via the
  generic API. Mitigation: migrate such sources through the OAuth connection flow;
  the `422` message names the correct endpoint.

**Rollback:**
- Each phase is independently revertible. A: the guard and fitness function are
  additive (revert the test + the helper; no schema change). B/C/D: handler-level
  diffs revert cleanly; the create-time validations are additive. E: the
  trusted-issuer gate can be flipped to the old behaviour behind a config flag
  during migration (default **on** = safe), and the session-org binding column is
  `#[serde(default)]`/nullable so old sessions still resolve.
- No irreversible migrations: `authorize_resource` is pure; `get_scoped` additions
  are additive SQL; the trusted-issuer registry and session-org column are additive
  schema. The fitness-function allowlist is a source constant.
