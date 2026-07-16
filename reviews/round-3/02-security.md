# Reaper Security Review — Round 3 (Offensive Security Engineer / External Auditor)

**VERDICT: NOT READY** — four independent **P0** cross-tenant / account-takeover
defects. Round 1's *anonymous-attacker* P0s and Round 2's *audit-integrity* P1s
are genuinely closed and verified. But this round, hunting the **authorization**
layer that the Round-2 gateway explicitly left to handlers, surfaces a cluster of
**tenant-isolation breaks** on the newer surface (SSO identity linking, webhook
subscriptions, GitHub-App source sync, deployment rollouts). A bank cannot sign
"READY" while any user who can sign up can take over another tenant's account and
read another tenant's private git repos.

Counts: **P0: 4 · P1: 2 · P2: 6 · P3: 5**

The Round-2 gateway (`auth/gateway.rs`) does exactly what it claims — it
**authenticates** every non-public route (default-deny, verified below). It does
**not authorize**. Every P0 here lives in the layer the gateway delegates to the
handler: "does *this* caller belong to *this* resource's tenant?" That check is
present and correct in ~20 handler files and **absent or defeated** in five.

---

## Threat model (assets → actors → boundaries)

**Assets:** live policy set on each agent (allow/deny authority); ReBAC
entity/relationship graph; decision/audit logs + hash-chain checkpoints; ed25519
bundle signing keys; **SSO/OIDC trust config per org**; **GitHub-App installation
identity**; JWT/session/API-key/webhook secrets; git/S3/DB creds; tenant
boundaries.
**Actors:** anonymous network attacker; **self-service tenant admin** (anyone can
`signup` → Owner of their own org); malicious/curious tenant; least-privilege
insider; hostile operator; insider with ClickHouse/S3 write; compromised bundle
store/CDN/on-path proxy; compromised CI.
**Trust boundaries:** client→control plane; **IdP→control plane** (P0-1);
**tenant→tenant** (P0-1/2/3, P1-2); control plane→internal network (P1-1);
control plane→agent; bundle store/CDN→agent; git→control plane; agent→audit sink.

---

## Executive summary (≤10)

1. **P0-1 — Cross-tenant account takeover via OIDC email auto-linking.** A
   self-service org admin points *their own* org's SSO at an IdP they control,
   asserts `email = victim@othercorp.com`, and the broker links that identity to
   the victim's **global** account by email — no `email_verified` check, no
   trust-boundary scoping — then mints a session for the victim's user id. Sessions
   are user-bound not org-bound, so the attacker inherits the victim's role in
   every org the victim belongs to. (`auth/sso/broker.rs:67-70`)
2. **P0-2 — Webhook-subscription handlers have no authorization at all.** All six
   CRUD handlers resolve the org from the URL slug and do zero `RequireAuth`
   identity binding, zero scope, zero tenant check. Any authenticated principal of
   any org manages any org's webhook subscriptions by slug; creating one with an
   attacker URL exfiltrates the victim's event stream. (`webhook_subscriptions.rs:133-357`)
3. **P0-3 — Cross-tenant confused-deputy through GitHub-App source config.**
   `installation_id`/`repo_full_name` pass unvalidated through `create_source`;
   the single shared App client mints a token for *whatever* installation the
   stored config names. A tenant sets a victim's `installation_id` and the shared
   control plane clones the victim's private repos into the attacker's org.
   (`sync/git.rs:108-118`, `api/sources.rs:258,573`)
4. **P0-4 (conditional) — bundle-update webhook fails open on an optional
   secret.** `/webhooks/bundle-update` is public; signature verification runs only
   `if config.webhook_secret.is_some()`. A `BundleUrl` source with no secret ⇒
   unauthenticated caller drives a server-side fetch of an attacker URL (SSRF,
   with the source's `auth_token` attached) and stores/broadcasts the result.
   (`api/webhooks.rs:153`)
5. **P1-1 — SSRF: two of the four outbound source paths are unguarded, and every
   sync HTTP client follows redirects.** `url_guard` protects git + JWKS; the
   API-source and bundle-URL fetches call the raw tenant URL with no guard and no
   https requirement; and no client sets a redirect policy, so even the guarded
   paths are bypassable by a `302 → 169.254.169.254`. (`sync/api.rs:80`,
   `sync/bundle_url.rs:90-104`, `sync/url_guard.rs:10-13`)
6. **P1-2 — Deployment rollout/pin/status handlers authorize the path org but act
   on a global resource UUID with no resource-org recheck** (unlike
   `strategies.rs`, which does). Authenticated cross-tenant rollout cancel / wave
   approve / pin / rollback, gated only by UUID opacity. (`rollouts.rs:355`,
   `pins.rs:34,121`, `status.rs:153`)
7. **Round-1 anonymous P0s: CLOSED & verified.** Agent has a real auth verifier +
   fail-closed exposure guard; push-deploy verifies signature before parse; the
   gateway is default-deny Enforcing. **Round-2 audit P1s: CLOSED & verified** — a
   store-reading chain verifier ships (API `…/decisions/verify` + CLI `audit
   verify`, ordered by write order); WORM archive gated in Helm with cross-boot
   genesis linkage; mandatory-audit mode is durable+fail-closed (503); `resource`
   is now redactable.
8. **P2 cluster (new):** unbounded regex-pattern caches (memory DoS on the
   interpreted eval path); JWT builtin decodes without verifying (no `jwt::verify`
   exists); Stripe webhook verification is a TODO stub; anti-rollback floor resets
   on restart without `cache_dir`; source creds stored plaintext; OIDC
   `redirect_uri` from `Host` header when `REAPER_PUBLIC_URL` unset.
9. **Eval hot path is clean of panics.** Every `unwrap` on request-derived data is
   type-guarded (`evaluate.rs:811-812,1151-1157`); DSL nesting is bounded at
   source/parse/eval (`reap/limits.rs`); `CatchPanicLayer` fails a handler panic
   closed to 500. Residual: `math::abs(i64::MIN)` (P3).
10. **The regression is structural, not knowledge.** The same codebase authorizes
    ~20 handler files correctly (`bundles.rs` `get_scoped`, `sources.rs`
    resource-org recheck, `change_requests.rs` `load_scoped`, SCIM token-derived
    org). The P0s are the handful of routes that skipped the established pattern —
    which is exactly what an auditor flags as "the isolation model is not enforced
    by construction."

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R3-1 | **P0** | `auth/sso/broker.rs:67-70`; `api/auth/sso.rs:126,462-470` | OIDC login adopts a pre-existing account by **global email**, no `email_verified` enforced, org admin can self-configure the asserting IdP | Cross-tenant **account takeover**: attacker inherits victim's identity + role in every org the victim is in | Never link across trust boundaries by email; require IdP be a platform-trusted issuer (not tenant-self-served) before adoption, or drop email adoption; bind sessions to the authenticating org |
| R3-2 | **P0** | `api/webhook_subscriptions.rs:133,168,238,265,324,357` | Six webhook-subscription handlers: **no `RequireAuth`, no scope, no tenant check** — org taken from URL slug only | Any authenticated user (any org, any role) CRUDs another org's webhook subscriptions; create → exfiltrate victim's event stream | Add `RequireAuth` + `authorize_org(&[OrgAdmin/OrgWrite])` to all six (the `bundles.rs` pattern) |
| R3-3 | **P0** (cond.) | `sync/git.rs:108-118`; `api/sources.rs:258,573-585` | `installation_id`/`repo_full_name` pass unvalidated into stored source config; shared `GitHubAppClient` mints a token for any named installation | Confused-deputy: a tenant clones **another tenant's private repos** into their own org | Bind installation to the caller's org server-side (as `oauth/github.rs` does); reject `installation_id`/`repo_full_name` in the generic source-config blob |
| R3-4 | **P0** (cond.) | `api/webhooks.rs:153,172-210`; `sync/bundle_url.rs:90-104` | Public bundle-update webhook: signature check gated `if webhook_secret.is_some()`; secretless source ⇒ unauth. Fetched URL is attacker-supplied, unguarded, carries `auth_token` | Unauthenticated SSRF + credential exfil + control-plane bundle-store poisoning | Require a secret for `BundleUrl` sources (fail closed like `webhooks_git.rs`); SSRF-guard the fetch; never attach creds to an unvalidated host |
| R3-5 | **P1** | `sync/api.rs:80`; `sync/bundle_url.rs:104`; `sync/url_guard.rs:10-13`; all sync `ClientBuilder`s | API-source + bundle-URL fetches unguarded (no `url_guard`, no https req); no client sets a redirect policy → `302`→metadata bypasses even guarded git/JWKS; guard is pre-flight only (DNS-rebind TOCTOU) | SSRF to cloud metadata / internal services from the control plane | Apply `url_guard` to every outbound tenant URL; set `redirect(Policy::none())`; re-validate at connect time or pin resolved IP |
| R3-6 | **P1** | `api/deployments/rollouts.rs:317,355`; `pins.rs:34,121`; `status.rs:153`; `deployment/service/mod.rs:507` | `authorize_deploy` checks the **path** org; the resource is then fetched by **global UUID** with no `rollout.org_id == org` recheck | Authenticated **cross-tenant** rollout cancel / approve-wave / pin / rollback, gated only by UUID opacity (P0-adjacent) | Re-check `resource.org_id == organization.id` after fetch (as `strategies.rs:141` does), or thread `org_id` into the repo `WHERE` |
| R3-7 | P2 | `regex_cache.rs:69`; `reap/ast_evaluator/regex_methods.rs:36` | Both regex-pattern caches `insert()` with **no bound/eviction**; pattern arg can be request-derived in the interpreted path (`function_dispatch.rs:364`) | Memory-DoS on the eval fleet from unique-pattern-per-request; breaks the DSL's "bounded resource" guarantee | Bound/evict the caches (LRU), or reject regex patterns sourced from request input |
| R3-8 | P2 | `reap/ast_evaluator/builtin_functions/jwt.rs:27,32`; dispatch `function_dispatch.rs:247` | `jwt::decode`/`header` do **no signature/`exp`/`aud`/`iss` check**; `alg:none` decodes fine; **no `jwt::verify` exists** | A policy `jwt::decode(input.token).iss == …` trusts a fully forgeable claim | Ship a verifying `jwt::verify(token, jwks)`; rename/lint the non-verifying form so its nature can't be missed |
| R3-9 | P2 | `billing/service.rs:305-322` | Stripe webhook handler is a **TODO stub** — signature never verified; endpoint is public-allowlisted on the premise it self-authenticates | No state mutation today; becomes a billing/entitlement-spoof + SSRF surface the moment the handler is implemented | Implement `Webhook::construct_event` (constant-time HMAC + timestamp tolerance) before any state mutation; until then return 501 / don't expose |
| R3-10 | P2 | `management/verify.rs:63`; `agent/main.rs:379-385` | Anti-rollback floor persisted only when `policies.cache_dir` set; else `in_memory()` resets to 0 on restart | Post-restart downgrade window in cacheless deployments (bounded by envelope validity) | Default a persistent floor location; refuse mandatory-anti-rollback on a non-durable store |
| R3-11 | P2 | `sync/source.rs:111-114,215,259-262` (creds in `config` JSON) | Git userpass, S3 keys, API key, bundle `auth_token`, `webhook_secret` stored as plaintext `Option<String>` in `policy_sources.config` | DB read = all tenants' source creds in clear | Field-encrypt / `SecretString`; consider per-tenant KMS envelope |
| R3-12 | P2 | `api/auth/sso.rs:529-543` | OIDC `redirect_uri` built from `Host`/`x-forwarded-proto` when `REAPER_PUBLIC_URL` unset | Auth-code interception via Host-header injection given a permissive IdP allowlist | Require `REAPER_PUBLIC_URL`, or validate `Host` against an allowlist |
| R3-13 | P3 | `reap/ast_evaluator/builtin_functions/math.rs:16` | `math::abs(i64::MIN)` panics (debug) / wraps to negative (release), reachable from `input` | Debug crash / silent wrong numeric result in a policy comparison | `wrapping_abs`/`unsigned_abs` or explicit guard (neighboring `time.rs` uses saturating ops) |
| R3-14 | P3 | `api/users/auth.rs:73-75,448-503` | Signup returns `409 "Email already registered"`; reset-request timing differs by existence | User enumeration (signup is a direct oracle) | Return a neutral 202 on signup collision; equalize reset-request work. (Login path is already hardened with `verify_dummy_password`) |
| R3-15 | P3 | `auth/gateway.rs:72-74`; `api/webhooks_git.rs` | `/webhooks/git/{provider}` is **not** allowlisted → 401 under Enforcing; HMAC there is correct + fail-closed | Feature unreachable (fails *closed* — not a vuln), contradicts its design | Allowlist `/webhooks/git/*` (its HMAC is the auth) |
| R3-16 | P3 | `auth/gateway.rs:41-74` | Public matching mixes exact + broad prefix (`/health/`, `/metrics/`, `/webhooks/bundle-update/`); no `NormalizePath` layer | No active shadowing today; a future route under a public prefix is silently exposed | Segment-exact matching; add path normalization |
| R3-17 | P3 | `api/scim/users.rs:151-165` | SCIM `create_user` adopts a pre-existing **global** user by email into the caller's org as Viewer | Membership pollution (no data access to victim) | Provision fresh or require verified same-tenant identity |

---

## Detailed P0/P1

### R3-1 (P0) — Cross-tenant account takeover via OIDC email auto-linking
`establish_session` (`auth/sso/broker.rs:50`) resolves the user in three steps
(`:62-82`): by `(issuer, subject)`; **else by global email**; else provision. The
middle branch is the break:
```rust
None => match users.find_by_email(&identity.email).await? {   // :67 — GLOBAL
    Some(u) => { users.link_idp_identity(u.id, &identity.issuer, &identity.subject).await?; u }  // :70
```
`identity.email_verified` is captured (`api/auth/sso.rs:385`) and defaulted to
`true` when absent (`pick_email_verified`, `:462-470`) but **never checked here**,
despite the doc comment (`broker.rs:60-61`) claiming a "verified-email account."
**Exploit:** `signup` makes you Owner (`org:admin`) of your own org; `PUT
/orgs/{me}/sso/config` is gated only by `OrgAdmin` on your own org
(`api/auth/sso.rs:126` → `authorize_org(&[Scope::OrgAdmin])`), so you point
`issuer`/`jwks_url`/`client_id` at an IdP you control. Run your org's OIDC login
asserting `email = victim@othercorp.com`, token signed by your key: JWKS
validation passes (your key/issuer/aud), nonce matches, and `establish_session`
links your `(issuer,subject)` to the **victim's** account and returns a session
for `victim.user_id`. Sessions are user-bound, not org-bound
(`auth/middleware.rs` resolves membership from the request-path org), so
`/orgs/{victim_org}/…` now runs at the victim's role. `email_verified`
enforcement alone does **not** fix it (you control the IdP); the adoption must not
cross trust boundaries by email.
**Remediation:** only adopt an existing account when the asserting issuer is a
**platform-trusted** issuer bound to that account's own org — never a
tenant-self-served IdP; otherwise provision a distinct SSO user. Bind the session
to the authenticating org.

### R3-2 (P0) — Webhook-subscription handlers have no authorization
`list/create/get/update/delete/test_webhook` (`api/webhook_subscriptions.rs:133,
168,238,265,324,357`) take `State` + `Path` + `Json` only; `grep RequireAuth = 0`
in the file. Each calls `resolve_org(&org_repo, &org)` (e.g. `:139`) — a
slug→UUID lookup, **not** an authorization. Under the default **Enforcing**
gateway the caller is authenticated but never bound to the org in the path, so any
authenticated principal manages **any** org's webhook subscriptions by slug
(slugs are human-readable and enumerable). `create_webhook` with an
attacker-controlled `url`+`secret` subscribes the attacker to the victim org's
event stream (decisions, bundle-promotions). Under `Disabled`/`LogOnly` gateway
modes it is fully anonymous. Directly contradicts `gateway.rs:69-71` ("webhook
subscription management … stays authenticated").
**Remediation:** `RequireAuth(user)` + `authorize_org(&state,&user,&org,
&[Scope::OrgAdmin])` on all six, mirroring `bundles.rs`.

### R3-3 (P0, conditional on GitHub App configured) — cross-tenant confused-deputy
`resolve_auth` (`sync/git.rs:107-118`) mints `app.installation_token(installation_id)`
for whatever `installation_id` sits in the source's stored config and clones
`https://github.com/{repo_full_name}.git` with it. The App client is a **single
shared** `Arc<GitHubAppClient>` (one App key) attached to the one `GitSyncer`
serving all tenants (`sync/service.rs:139-141`). `create_source`
(`api/sources.rs:258`) passes `request.config` (raw `serde_json::Value`) straight
to persistence, and `validate_source_config` for `Git` (`:573-585`) checks only
that `url` is present — it never rejects or scopes `installation_id`/
`repo_full_name`. A tenant with `policy:write` in their **own** org therefore
creates a Git source whose config JSON names a **victim's** `installation_id`; at
sync time the shared control plane mints the victim's installation token (a GitHub
App can mint tokens for any installation of itself) and clones the victim's
private repos into the attacker's org, materializing them as policies. The
dedicated OAuth path (`api/oauth/github.rs:476-499`) derives `installation_id`
server-side bound to the caller — the generic source API does not.
**Remediation:** never accept `installation_id`/`repo_full_name` from the config
blob; resolve them server-side from a caller-bound GitHub connection record.

### R3-4 (P0, conditional on a secretless BundleUrl source) — fail-open webhook
`/webhooks/bundle-update` is public (`gateway.rs:72`). In `handle_bundle_webhook`
(`api/webhooks.rs:153`) the signature check is `if config.webhook_secret.is_some()`
— a `BundleUrl` source configured without a secret **skips verification entirely**.
An unauthenticated caller then supplies `source_id` + `bundle_url`; the server
fetches that URL (`bundle_url.rs:104`, no `url_guard`, and attaches the source's
`auth_token` if set, `:93-95`), stores the result, and broadcasts `BundlePromoted`
to the org's agents (`webhooks.rs:172-210`). Enforcement-plane impact is contained
by the agent's fail-closed signature verification (`require_signed_bundles=true`
default) — an unsigned injected bundle won't load on agents — but the
**unauthenticated SSRF + credential exfiltration + control-plane store poisoning**
stand.
**Remediation:** require a `webhook_secret` for `BundleUrl` (reject at create when
unset), fail closed like `webhooks_git.rs:141-148`; SSRF-guard the fetch.

### R3-5 (P1) — SSRF: unguarded paths + universal redirect-following
`url_guard::validate_public_https_url` (`sync/url_guard.rs:54-87`) is solid
(https-only; rejects loopback/RFC1918/link-local/metadata/CGNAT/IPv6-ULA/mapped)
and is applied to git (`git.rs:144`) and JWKS (`auth/jwks.rs:23`). It is **not**
applied to: the API source (`sync/api.rs:80` — raw `config.url`, no https
requirement) or the bundle-URL fetch (`sync/bundle_url.rs:104`). Worse, **no**
sync `reqwest::Client` sets a redirect policy (`api.rs:35`, `bundle_url.rs:68`,
`s3.rs:54`, `github_app.rs`, and `jwks.rs:149` all build with only a timeout), so
reqwest's default (follow ≤10 redirects) applies and a public host that passes the
guard can `302 → http://169.254.169.254/` — defeating even the guarded paths. The
guard is also pre-flight only (`url_guard.rs:10-13` self-acknowledges the
DNS-rebind TOCTOU).
**Remediation:** guard every outbound tenant URL; `.redirect(Policy::none())` on
all sync clients; validate at connect time or pin the resolved IP.

### R3-6 (P1) — Deployment resource-org check missing
`cancel_rollout` (`rollouts.rs:355-376`) calls `authorize_deploy(&state,&user,
&org,…)` (checks caller ∈ **path** org + `DeploymentWrite`), then
`service.cancel_rollout(rollout_id,…)` on the **global** UUID —
`get_rollout_by_id(id)` has no org argument (`deployment/service/mod.rs:507`). No
`rollout.org_id == organization.id` recheck. Same shape in `approve_wave`
(`:317`), `create_pin`/`delete_pin` (`pins.rs:34,121`), `acknowledge_deployment`
(`status.rs:153`), and the by-id reads (`status.rs:35,74,113`;
`rollback_config.rs:262,319`). An attacker who is a `DeploymentWrite` member of
their own org mutates any rollout in any org by ID; only UUID opacity — not an
authorization control — stands in the way. `strategies.rs:141` shows the correct
recheck, so this is an omission. (Round-2 R2-8 rated the underlying pattern P2 on
the assumption "the handler pre-check is always present"; it is **not** present
for these resources — hence P1 here.)
**Remediation:** re-check `resource.org_id == organization.id` after every by-id
fetch, or scope the repo query by `org_id`.

---

## Absence checks performed (falsifiable)

- **Round-1 P0-1 (agent unauth): CLOSED.** `AgentAuthVerifier` (Bearer HS256 JWT
  issuer/aud-pinned, or static token; digest compare) wired at
  `agent/main.rs:729-732`; exempts only health + (optionally) the 4 eval endpoints
  (`auth.rs:44-60`). Fail-closed exposure guard refuses a non-loopback bind
  without inbound auth (`main.rs:227-247`). `CatchPanicLayer` at `:745`.
- **Round-1 P0-2 (push-deploy no verify): CLOSED.** `deploy_bundle`
  (`handlers/policies.rs:384-416`) and `load_bundles_atomic` (`:502-546`) call
  `verify_push` **before** `from_bytes`, fail-closed 422.
- **Round-1 P0-3 (control-plane unauth): CLOSED.** Gateway default-deny Enforcing
  (`gateway.rs`; `GatewayMode::Enforcing` default). Public allowlist read in full:
  health/live/ready/metrics/openapi + genuine login/signup/refresh/reset/verify +
  github authorize/callback + the two signed webhooks. Dual-mount `/api/v1` strip
  handled; traversal not exploitable (no `NormalizePath`, matchit 404s literals).
- **Round-2 R2-2 (no shipped chain verifier): CLOSED.** Store-reading verifier
  ships as `GET /orgs/{org}/decisions/verify` (`api/decisions.rs:340-401` →
  `verify_range` → `decision_log::verify_records`) and CLI `audit verify`
  (`reaper-cli/src/main.rs`), both ordered by the exact per-boot write order
  `(chain_id, seq)` (`decisions/mod.rs:588-651`).
- **Round-2 R2-3 (WORM disabled by default): MITIGATED.** Helm gates an S3
  Object-Lock sink on `decisionLogs.worm.enabled` (`values.yaml:263`, default
  `false`, documented "REGULATED DEPLOYMENTS MUST ENABLE"); cross-boot genesis
  linkage present. Residual: default deploy still co-locates checkpoints with
  decisions — a documented deployment choice now, no longer a silent gap.
- **Round-2 R2-4 (served-before-persisted): CLOSED for mandatory mode.**
  `evaluate.rs:644-655,1045-1056` branch on `buffer.mandatory_durable()` and,
  when `log_durable` can't fsync, fail closed 503; `audit_gate` (`:117-124`) flips
  readiness not-ready.
- **Round-2 R2-1 (fleet-propagation least-privilege): CLOSED.** `authorize_deploy`
  (`deployments/mod.rs:75`) requires `DeploymentWrite`/`BundlePromote`/`OrgAdmin`.
  (The *resource-org* half is R3-6.)
- **Round-2 R2-5 (`resource` not redactable): CLOSED.** `hash_resource` with
  domain-separated HMAC (`decision_privacy.rs:53-55,149-153`). Still opt-in.
- **DSL DoS: CLOSED.** `reap/limits.rs` bounds nesting at source pre-scan, AST
  walk, default 64 / `REAPER_MAX_NESTING_DEPTH`.
- **Hot-path panics:** `evaluate.rs` unwraps on request data are type-guarded
  (`:811-812` behind `is_i64()`, `:1157` behind `is_empty()`). Remaining unwraps
  are `#[cfg(test)]`.
- **OIDC id_token validation: SAFE.** Signature vs provider JWKS, issuer set,
  **audience mandatory** (`auth/jwks.rs:189-194`), `exp` enforced, RSA/EC-only
  decoding keys → no `alg:none`/HMAC confusion; nonce checked (`sso.rs:371`).
- **OIDC CSRF/PKCE: SAFE.** PKCE-S256 + AEAD-sealed `state` (org+expiry) from
  `OsRng` (`sso.rs:244-268,417-448`).
- **SCIM: SAFE.** Bearer hashed + looked up by hash; org derived from the token
  (never the path); per-org token; handlers scope by `ctx.org_id`; mint/revoke
  require `OrgAdmin` (`api/scim/*`, `auth/scim/store.rs`).
- **Session hygiene: SAFE.** 256-bit CSPRNG `rst_` tokens stored as SHA-256;
  expiry enforced; fresh token per login; server-side logout; password change /
  reset / SCIM-deprovision revoke all sessions.
- **Constant-time compares: SAFE where it matters.** Git/GitLab webhook HMAC uses
  `subtle::ct_eq` (`webhooks_git.rs:59-63,71`); OAuth state HMAC `verify_slice`;
  tokens compared by hashed-lookup. Agent token compares digests (`==` on
  `[u8;32]`) after a length gate — acceptable (compares hashes, not secrets).
- **Git commit signatures + envelope v2 + revocation: REAL, fail-closed**
  (`commit_verify.rs` SSHSIG vs per-source keys; `verify.rs:194-222` envelope
  `not_before`/`expires_at`/version; revocation checked before anti-rollback,
  `force` can't bypass). Caveat: `require_signed_commits` defaults `false` per
  source (opt-in) — worth a hardened default.
- **`unsafe`:** 12 blocks, all in experimental `reaper-ebpf`; `services/` and
  `policy-engine` = 0. eBPF not audited for soundness — gate before GA.

## Coverage / what I did NOT cover
Priority order followed: eval hot path → API surface → distribution/promotion →
audit → data-plane. Not covered: eBPF `unsafe` soundness; the SAML-deferred path
(out of scope); exhaustive SQL-parameterization sweep across all repositories
(spot-checked — decisions/audit/api_key/jwks all bind params; the `decisions`
verify SQL binds every user value); rate-limiter bypass depth; the reaper-mcp gate;
Cedar evaluator internals (10-50µs path, off the sub-µs claim). The four P0s and
both P1s were each verified against source directly, not taken on a sub-agent's word.

## What's done well (≤5)
1. Round-1 and Round-2 blockers are **genuinely** closed at the structural layer
   (router-level default-deny auth, single signature chokepoint, durable
   fail-closed audit, shipped chain verifier) — verified, not cosmetic.
2. The audit pipeline is now defensible: write-ordered store queries, an
   independent WORM anchor with cross-boot genesis linkage, and a mandatory mode
   that returns 503 rather than serve an un-audited allow.
3. OIDC/SCIM/session **cryptographic** hygiene is careful — mandatory audience,
   RSA/EC-only keys, PKCE+AEAD state, CSPRNG hashed tokens, constant-time webhook
   HMAC — the P0s are trust-model/authorization gaps, not crypto gaps.
4. The correct tenant-isolation pattern is present and consistent across ~20
   handler files (`get_scoped`, `load_scoped`, resource-org rechecks), which is
   why the five that skip it read as omissions to be closed, not a missing design.
5. Bundle trust chain (verify-before-parse, envelope validity window, persistent
   anti-rollback floor, non-`force`-bypassable revocation) is complete and
   fail-closed.
