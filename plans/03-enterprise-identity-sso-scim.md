# Enterprise Identity: SSO (OIDC + SAML) + SCIM

**Readiness gate:** NOT READY → CONDITIONAL (hard gate on any regulated/bank security questionnaire).
**Priority:** P0 (Synthesis #4; Product Architecture F1; blocks design-partner evaluation).
**Findings closed:** Product **F1** (no SSO/SAML/OIDC, no SCIM — login is password + GitHub-OAuth-for-source only); Synthesis executive-summary item 4 ("no SSO/SAML/OIDC and no SCIM for the human admin console — strands the management audit log"). Partially unblocks the value of the already-good management-action audit log (`services/reaper-management/src/audit/mod.rs`) by tying its actor to a governed corporate identity.

---

## 1. Goal

Let a regulated enterprise log into the Reaper **admin control plane** with their own IdP (Okta, Entra ID, Ping, Google Workspace) via **OIDC Authorization Code + PKCE first, then SAML 2.0**, and manage the **user lifecycle** (provision / update role / deprovision-on-termination) automatically through **SCIM 2.0**. Every management action must resolve to a governed corporate identity, and a terminated employee must lose Reaper access without a Reaper admin doing anything.

Explicitly **in scope**: interactive human SSO to the management API; an IdP-agnostic **session broker** that maps external identities onto the existing `users` / `user_orgs` / `sessions` tables and mints the existing `rst_` session token (so `RequireAuth` is unchanged); SCIM Users + Groups endpoints; org-level RBAC for Reaper's **own** admin surface (group → `OrgRole` mapping); and surfacing the existing management-action audit log (which is distinct from decision/audit logs — see plan 04).

Explicitly **out of scope**: agent/machine auth (already covered by API keys, mTLS, shared-secret JWT, and external JWKS in `auth/middleware.rs`); the decision-log audit trail (plan 04); UI.

---

## 2. Current state (evidence) — file:line

- **No SSO, no SAML, no OIDC, no SCIM.** `grep -rin "scim|saml|oidc|openid|single sign"` over `services/`, `crates/` → 0 hits (confirmed by Security and Product reviewers). No `/auth/sso*` or `/scim/*` route.
- **Human login is local password + GitHub OAuth (for source connect only).**
  - Local password login lives in `services/reaper-management/src/api/users/auth.rs` (routes merged via `api/users/mod.rs`); passwords are Argon2 + `OsRng` salt (`auth/users/password.rs`).
  - GitHub OAuth (`api/oauth/github.rs:28-224`) connects a *repo source*, not a login: `github_authorize` requires an **existing** session (`get_user_id_from_session`, `github.rs:42`) and stores an `oauth_connections` row; it never creates a user or a session. `scope=repo` (`github.rs:70`), user token embedded in clone URL (`github.rs:302`).
- **Session model already exists and is the integration point.** `auth/users/mod.rs`:
  - `UserRepository` (create/find/update_status/verify_email), `UserOrgRepository` (add_membership/get_role/update_role/remove_membership), `SessionRepository` (create/find_by_token/delete/delete_all_for_user).
  - Session tokens are the `rst_`-prefixed opaque tokens; `SessionRepository::find_by_token` hashes with `hash_token` (`auth/users/password.rs`) and checks `is_expired()`.
- **`RequireAuth` already consumes `rst_` sessions.** `auth/middleware.rs:218-256` — a Bearer token starting `rst_` is validated via `SessionRepository::find_by_token`, then the user's first `user_orgs` membership role is mapped to scopes via `role_to_scopes` (`middleware.rs:465-515`). **SSO only needs to mint an `rst_` session; the rest of authZ is unchanged.**
- **Org RBAC roles exist.** `auth/users/types.rs:75-130`: `OrgRole::{Owner, Admin, Developer, Viewer}` with `can_manage_users` / `can_manage_policies` / `can_delete_org`. `role_to_scopes` (`middleware.rs:466`) already maps these to `Scope` values (`auth/scopes.rs`). Note the deliberate rule: an org `Owner` is **not** granted the global platform `admin` scope (`middleware.rs:475-489`, regression test `middleware.rs:521-541`) — SSO/SCIM group mapping MUST preserve this (never map an IdP group to `Scope::Admin`).
- **JWKS validator is a reusable OIDC building block.** `auth/jwks.rs`: `is_disallowed_ip` (`:17`), `validate_jwks_url` (SSRF guard, `:47`), `Validation::new(key.algorithm())` with **mandatory audience** (`:234-251`), issuer extraction (`extract_issuer_from_token`), RSA/EC-only decoding keys. This is exactly the ID-token signature/claims validation OIDC needs — reuse it, do not re-implement JWT verification.
- **CSRF-safe OAuth state primitive exists.** `api/oauth/types.rs` `OAuthState::{new,encode,decode,is_valid}` (HMAC-signed with `config.auth.jwt_secret`, used at `github.rs:64-95`) — reuse for the OIDC `state` and PKCE nonce binding.
- **Encryption-at-rest helper exists.** `api/oauth/helpers.rs` `encrypt_token` (authenticated encryption, fails closed without a key) — reuse for storing SCIM tokens and IdP client secrets.
- **Router wiring.** `api/mod.rs:31-52` `build_api_router()` merges per-file `routes()`; the control plane has no default-deny auth layer (`main.rs:215-236` — layer stack is security_headers/correlation_id/request_metrics/body_size_limit/access_log/TraceLayer). SSO/SCIM routes are added as new `.merge(...)` entries.
- **Audit action taxonomy is comprehensive but has no identity actions.** `audit/mod.rs:123-203` has `USER_LOGIN`, `OAUTH_CONNECT`, `JWKS_CONFIG_*`, etc. — but **no** `sso.login`, `scim.user_provision`, `scim.user_deprovision`. `AuditEntry::builder(action, ActorType, actor_id)` (`audit/mod.rs:221-255`) is the extension point.
- **Migrations are sequential SQL.** `db/migrations/001..009` — next is `010_sso_scim.sql`.

---

## 3. Definition of Done — testable checkboxes

- [ ] An org admin can register an OIDC IdP: `PUT /orgs/{org}/sso/config` stores issuer, client_id, encrypted client_secret, discovery/JWKS URL, attribute map, allowed-domains, and `default_role`.
- [ ] `GET /auth/sso/{org}/start` returns a 302 to the IdP authorize endpoint with `response_type=code`, `code_challenge` (PKCE S256), and an HMAC-signed `state` bound to org+nonce; the code verifier is stored server-side keyed by state.
- [ ] `GET /auth/sso/{org}/callback` exchanges the code, validates the **ID token** via the JWKS validator (signature, `iss`, `aud`, `exp`, nonce), JIT-provisions/updates `users` + `user_orgs`, mints an `rst_` session via `SessionRepository::create`, and redirects to the app — a subsequent `RequireAuth` request with that Bearer token succeeds unchanged.
- [ ] A forged/tampered `state`, a replayed `code`, a mismatched nonce, an ID token failing `aud`/`iss`/signature, or a user whose email domain is not in `allowed_domains` are all rejected (401/400) and never mint a session.
- [ ] SAML 2.0: `GET /auth/sso/{org}/saml/metadata` serves SP metadata; `GET /auth/sso/{org}/saml/start` issues a signed `AuthnRequest`; `POST /auth/sso/{org}/saml/acs` validates the signed assertion (XML signature over the assertion, `NotBefore`/`NotOnOrAfter`, `Audience`, `InResponseTo`) and JIT-provisions + mints the same `rst_` session.
- [ ] SCIM 2.0: `POST/GET/PATCH/PUT/DELETE /scim/v2/Users` and `/scim/v2/Groups` authenticate with a per-org bearer SCIM token (hashed at rest), and CRUD maps to `users` + `user_orgs` (group → `OrgRole`).
- [ ] SCIM **deprovision** (`DELETE /scim/v2/Users/{id}` or `PATCH active=false`) sets `UserStatus::Suspended` (or removes `user_orgs` membership) AND revokes all live sessions via `SessionRepository::delete_all_for_user` — a terminated user is denied within one request.
- [ ] Group → role mapping never yields `Scope::Admin`; an IdP-supplied `admin` group maps at most to `OrgRole::Owner` (which per `middleware.rs` is not platform-admin). Regression test asserts this.
- [ ] New audit actions `sso.login`, `sso.config_update`, `scim.user_provision`, `scim.user_update`, `scim.user_deprovision`, `scim.group_sync` are written via `AuditEntry::builder` with actor, IP, UA, and IdP subject.
- [ ] SCIM tokens and IdP client secrets are stored encrypted (`encrypt_token`); disabling/misconfiguring encryption fails closed (no plaintext secret persisted).
- [ ] Config is per-org and isolated: an SSO config or SCIM token for org A cannot authenticate into org B.
- [ ] ADR recorded (§8) on build-in-house vs integrate (WorkOS/Auth0), with the recommendation and the API-shape invariant that lets the decision be reversed without an API break.

---

## 4. Critical steps — ordered

Each step: **what / where (files) / verify.** Decomposed for a senior engineer; no code here.

### Step 1 — Data model: `sso_configs` + `scim_tokens` (+ identity linkage)
- **What:** Add `010_sso_scim.sql`. Tables:
  - `sso_configs(id, org_id, protocol[oidc|saml], enabled, issuer, client_id, client_secret_encrypted, discovery_url|jwks_url, saml_idp_entity_id, saml_idp_sso_url, saml_idp_cert_pem, attr_map_json, allowed_domains_json, default_role, created_at, updated_at)` — unique on `(org_id, protocol)`.
  - `scim_tokens(id, org_id, name, token_hash, created_by, created_at, last_used_at, revoked)`.
  - Extend user↔IdP linkage: add `external_idp_subject` + `external_idp_issuer` columns to `users` (nullable; unique `(external_idp_issuer, external_idp_subject)`), so JIT lookups are by IdP subject, not email (email can change/be reused).
- **Where:** `services/reaper-management/src/db/migrations/010_sso_scim.sql`; repositories beside `auth/users/mod.rs` (new `auth/sso/store.rs`, `auth/scim/store.rs`).
- **Verify:** migration applies on sqlite + postgres (both `AnyPool` backends); round-trip insert/select of an encrypted secret; unique constraints reject duplicate `(org_id, protocol)` and duplicate IdP subject.

### Step 2 — Session broker (IdP-agnostic core)
- **What:** A single `broker::establish_session(org_id, ExternalIdentity)` that: (a) looks up `users` by `(issuer, subject)` else by verified email else creates a user (`UserRepository::create`, status `Active`, `email_verified=true` since the IdP asserted it); (b) upserts `user_orgs` role from the mapped group/`default_role` (`UserOrgRepository::add_membership`/`update_role`); (c) mints an `rst_` session (`generate_session_token` + `SessionRepository::create`); (d) writes `sso.login` audit. Both OIDC and SAML callbacks call ONLY this — the broker is the one place that touches identity tables.
- **Where:** new `services/reaper-management/src/auth/sso/broker.rs`; reuses `auth/users/{mod,password,types}.rs`, `audit/mod.rs`.
- **Verify:** unit test drives the broker with a synthetic `ExternalIdentity` and asserts a valid `rst_` token comes back that `RequireAuth` accepts (integration test hitting a `RequireAuth`-protected route). Test that a second login for the same subject reuses the same user row (no duplicate).

### Step 3 — OIDC Authorization Code + PKCE (first protocol)
- **What:** `GET /auth/sso/{org}/start` → resolve org, load `sso_configs` (oidc), do OIDC discovery (cache JWKS), generate PKCE verifier+challenge and nonce, sign `state` with `OAuthState` primitive, persist verifier keyed by state (short-TTL row or signed cookie), 302 to authorize URL. `GET /auth/sso/{org}/callback` → verify `state`, exchange `code` at the token endpoint (SSRF-guard the token/discovery URLs with `validate_jwks_url`'s guard), validate the **ID token** with the existing `jwks` validator (mandatory `aud`, `iss`, `exp`, nonce match), map claims → `ExternalIdentity` via `attr_map_json`, enforce `allowed_domains`, call the broker.
- **Where:** new `services/reaper-management/src/api/auth/sso.rs` (add `pub mod sso;` and a `.merge(sso::routes())` in `api/auth/mod.rs` or `api/mod.rs`); reuse `auth/jwks.rs` (validator + SSRF guard), `api/oauth/types.rs` `OAuthState`, `api/oauth/helpers.rs` `encrypt_token`.
- **Verify:** end-to-end against a mock OIDC provider (e.g. a test container or a hand-rolled JWKS+token stub): happy path mints a session; tampered `state`, wrong `aud`, expired token, replayed code, and nonce mismatch each fail. Confirm the reused JWKS validator rejects `alg:none` and HMAC confusion (already covered by `jwks.rs` tests — assert the path routes through it).

### Step 4 — Org RBAC mapping for Reaper's own admin surface
- **What:** Translate IdP groups/roles → `OrgRole` using `attr_map_json` (e.g. `{"reaper-admins":"owner","reaper-devs":"developer"}`), default `default_role` when unmapped. Enforce the invariant: **no mapping may produce `Scope::Admin`**; the highest attainable is `OrgRole::Owner`, which `role_to_scopes` (`middleware.rs:475`) already keeps below platform-admin. Membership changes on each login (role drift from the IdP is reconciled).
- **Where:** `auth/sso/broker.rs` (mapping fn) + a small `group_to_role` helper; consumes `auth/users/types.rs::OrgRole`, `auth/scopes.rs`.
- **Verify:** table test of group→role; a regression test mirroring `middleware.rs:521-541` asserting an IdP `admin`/`owner` group never yields `perm.has(Scope::Admin)`. Cross-tenant test: a session minted for org A cannot read org B (existing `datastore.rs:100-119` pattern).

### Step 5 — SCIM 2.0 Users + Groups (provisioning/deprovisioning)
- **What:** RFC 7643/7644 subset. `/scim/v2/Users`: `POST` (create → `UserRepository::create` + `user_orgs`), `GET`/`GET {id}` (list with `filter=userName eq`), `PATCH {id}` (`active=false` → suspend + **`SessionRepository::delete_all_for_user`**), `PUT {id}`, `DELETE {id}` (deprovision). `/scim/v2/Groups`: map a group's members to `user_orgs` role. Auth via `scim_tokens` bearer (hash-compare, update `last_used_at`), org resolved from the token. Emit SCIM-shaped errors (`schemas`, `detail`, `status`) and `ListResponse` envelopes.
- **Where:** new `services/reaper-management/src/api/scim/{mod,users,groups}.rs`; `auth/scim/store.rs`; reuse `auth/users/*`, `audit/mod.rs`. Route group `/scim/v2/*` merged in `api/mod.rs`.
- **Verify:** run an SCIM conformance pass (e.g. Okta/Entra test-mode or the `scimmy`/`scim2` test vectors): create → appears in `user_orgs`; `active=false` → user suspended AND all sessions gone (assert a previously-valid `rst_` token now 401s); group change → role updated. Token for org A rejected on org B's implicit scope.

### Step 6 — SAML 2.0 (second protocol, same broker)
- **What:** SP metadata endpoint; `AuthnRequest` (signed, `InResponseTo` tracked); ACS endpoint validates the assertion's XML signature against `saml_idp_cert_pem`, `Conditions` (`NotBefore`/`NotOnOrAfter`, `AudienceRestriction`), `SubjectConfirmation` (`InResponseTo`, `Recipient`, `NotOnOrAfter`), replay-guards the assertion ID, maps attributes → `ExternalIdentity`, calls the broker. Use a vetted crate (`samael`) rather than hand-rolling XML-DSig.
- **Where:** `api/auth/sso.rs` SAML submodule; `auth/sso/broker.rs` (unchanged consumer).
- **Verify:** e2e against a SAML test IdP (samltest.id or `samael`'s fixtures): valid assertion mints a session; unsigned assertion, wrong audience, expired `Conditions`, and a replayed assertion ID are all rejected. XML-signature-wrapping test (moved/duplicated `Assertion`) rejected.

### Step 7 — Audit + operability
- **What:** Add the new action constants to `audit/mod.rs::actions`; write audit on every SSO login, config change, and SCIM lifecycle event with actor, IP, UA, IdP subject. Add metrics (sso_login_total by result, scim_ops_total). Document setup per IdP.
- **Where:** `audit/mod.rs`; handlers from steps 3/5/6; `docs/deployment/` new SSO/SCIM setup guide.
- **Verify:** each flow produces exactly one audit row of the right action; a deprovision produces `scim.user_deprovision` and the session-revocation is observable.

---

## 5. Dependencies

- **Reused primitives:** `auth/jwks.rs` (OIDC ID-token validation + SSRF guard), `auth/users/*` (users/sessions/memberships), `auth/scopes.rs` + `role_to_scopes` (`middleware.rs`), `api/oauth/types.rs::OAuthState` (CSRF/state), `api/oauth/helpers.rs::encrypt_token` (secret-at-rest), `audit/mod.rs` (`AuditEntry::builder`).
- **New crates (if build-in-house):** `openidconnect` (OIDC discovery + Auth-Code+PKCE client), `samael` (SAML SP + XML-DSig). If integrate: WorkOS/Auth0 Rust HTTP client (thin) — see ADR.
- **Migrations:** `010_sso_scim.sql` must land before any handler ships.
- **Config:** per-org config lives in DB (`sso_configs`), not env; no new global env beyond an optional `REAPER_SSO_BASE_URL` for redirect/ACS URL construction.
- **Cross-plan:** the new `sso.login` / `scim.*` audit actions feed the management-action audit log — this is the log referenced (but not itself made tamper-evident) here; tamper-evidence is plan 04's concern and could later be extended to this log too.

---

## 6. Testing & verification

- **Unit:** broker identity resolution (new/existing/email-collision), group→role mapping invariant (no `Scope::Admin`), state/nonce/PKCE verification, SCIM filter parsing, SCIM error envelopes.
- **Integration (mock IdP):** OIDC happy path + every rejection branch; SAML happy path + signature/condition/replay rejections; SCIM CRUD + deprovision-revokes-sessions; cross-tenant isolation for both SSO sessions and SCIM tokens.
- **Regression:** mirror `middleware.rs:521-541` — an IdP admin group must not confer platform-admin. Assert a minted `rst_` session is accepted by the unchanged `RequireAuth` on an existing protected route (e.g. `datastore.rs`).
- **Conformance:** run against at least one real IdP in test mode (Okta or Entra) for OIDC + SCIM before GA.
- **Security:** confirm secrets never appear in logs/audit details; confirm `alg:none`/HMAC-confusion rejected via the reused JWKS validator; SSRF guard applied to discovery/token/JWKS URLs.
- **Commands:** `cargo test -p reaper-management sso`, `... scim`, plus the mock-IdP e2e under `tests/e2e`.

---

## 7. Effort & phasing — S/M/L

- **Phase 1 (M) — OIDC + session broker + org RBAC mapping (Steps 1-4).** The minimum that passes "we support SSO." Highest value per unit effort because it reuses the JWKS validator and the session model wholesale.
- **Phase 2 (M) — SCIM Users/Groups + deprovision-revokes-sessions (Step 5).** The "deprovision on termination" questionnaire line.
- **Phase 3 (M) — SAML 2.0 (Step 6).** Same broker; effort is XML-DSig correctness, mitigated by using `samael`.
- **Phase 4 (S) — audit actions, metrics, per-IdP docs (Step 7).**
- Overall **M-L**. If the ADR chooses *integrate* (WorkOS), Phases 1-3 collapse toward **S-M** (one webhook/callback + directory-sync wiring) at the cost of a vendor in the trust path.

---

## 8. Key decisions (ADR-style)

**ADR-1: Build in-house (`openidconnect` + `samael`) vs integrate a broker (WorkOS / Auth0).**
- *Build:* no per-connection vendor cost, no third party in the identity trust path, full control; but you own IdP-quirk hell (Entra group overage claims, SAML clock-skew, per-IdP attribute shapes) and SCIM conformance across providers — genuinely L effort to get bank-grade.
- *Integrate (WorkOS):* SSO + Directory Sync (SCIM) across all major IdPs in days, passes questionnaires immediately, offloads conformance; but adds a vendor in the auth path and per-connection cost.
- **Recommendation: integrate WorkOS to unblock the P0 for design partners now, but keep every endpoint provider-agnostic** — the public shape (`/auth/sso/{org}/start|callback`, `/scim/v2/*`, `sso_configs`/`scim_tokens` tables, and the `broker::establish_session` seam) is identical whether the assertion came from WorkOS or from an in-house `openidconnect` client. That lets a later swap to in-house OIDC happen behind the broker with **no API break**. Rationale: speed-to-design-partner dominates now; the reused JWKS validator means the in-house OIDC path is cheap to add later if vendor cost/trust becomes an issue.

**ADR-2: OIDC before SAML.** OIDC reuses the existing JWKS validator (`auth/jwks.rs`) almost verbatim and covers Okta/Entra/Google; SAML needs XML-DSig and is only required by a subset of enterprises. Sequence OIDC first to close the gate fastest, add SAML as a fast-follow through the same broker.

**ADR-3: JIT provisioning keyed by IdP subject, reconciled each login, with SCIM as source of truth when present.** Login-time JIT keeps access current even without SCIM; when SCIM is configured it becomes authoritative for lifecycle (create/deprovision), and login reconciles role drift. Avoids the "email reused for a new person" identity-confusion class by keying on `(issuer, subject)`.

**ADR-4: Group→role mapping caps at `OrgRole::Owner`, never `Scope::Admin`.** Preserves the existing tenant-isolation invariant (`middleware.rs:475-489`); an external IdP must never be able to mint a platform super-admin.

---

## 9. Risks & rollback

- **Risk: IdP misconfiguration locks admins out.** *Mitigation:* keep local password login enabled alongside SSO (do not force SSO-only until an org opts in); provide a break-glass local Owner account per org. *Rollback:* disable `sso_configs.enabled` — local login is untouched.
- **Risk: JIT auto-provisioning creates unwanted accounts / privilege via a spoofed IdP.** *Mitigation:* `allowed_domains` allowlist, signature/audience/issuer validation via the reused JWKS validator, and the no-`Scope::Admin` cap. SCIM-authoritative mode disables JIT creation for orgs that want strict provisioning.
- **Risk: SCIM token leak = tenant user-management takeover.** *Mitigation:* tokens hashed at rest, per-org scoped, revocable, `last_used_at` tracked and auditable; rotate via `scim_tokens.revoked`.
- **Risk: SAML XML-signature-wrapping / assertion replay.** *Mitigation:* use `samael`, validate signature over the assertion (not just response), enforce `Conditions`/`SubjectConfirmation`, replay-guard assertion IDs. Ship SAML only after the wrapping/replay tests pass.
- **Risk: deprovision race (user acts between IdP disable and next reconcile).** *Mitigation:* SCIM `active=false` immediately calls `SessionRepository::delete_all_for_user`, so revocation is synchronous, not eventual.
- **General rollback:** all new surface is additive (new routes, new tables, new audit actions). Feature-flag SSO/SCIM per org; turning it off leaves password auth, `RequireAuth`, and all existing routes exactly as they are today.
