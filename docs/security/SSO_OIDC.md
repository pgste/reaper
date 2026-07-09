# Enterprise SSO (OIDC)

Reaper's admin control plane supports **OIDC (OpenID Connect) single sign-on**
so a regulated enterprise logs in with its own IdP (Okta, Entra ID, Ping, Google
Workspace) instead of a local password. Login uses **Authorization Code + PKCE**,
every management action resolves to a governed corporate identity, and IdP
**groups map to Reaper org roles**.

This is built natively — no third-party identity broker in the auth path. SAML
and SCIM (directory-sync deprovisioning) are separate, later phases.

## How it works

```
user → GET /auth/sso/{org}/start
         → 302 to IdP authorize  (PKCE S256 challenge + nonce + sealed state)
user ← IdP login ← ← ←
     → GET /auth/sso/{org}/callback?code&state
         → validate state · exchange code (PKCE verifier + client secret)
         → validate ID token (signature via JWKS, iss, aud=client_id, exp, nonce)
         → JIT provision/reconcile user + role → mint rst_ session
```

The returned `session_token` (`rst_…`) is an ordinary Reaper session — send it as
`Authorization: Bearer <token>` and every existing `RequireAuth` route accepts it
unchanged.

## 1. Register Reaper with your IdP

Create an **OIDC web application** in your IdP with:

- **Redirect URI:** `https://<reaper-host>/auth/sso/<org-slug>/callback`
- **Grant type:** Authorization Code (PKCE enabled)
- **Scopes:** `openid email profile` (plus a **groups** claim — see provider notes)

Note the **issuer**, **client ID**, and **client secret**.

## 2. Configure the org in Reaper

An org admin registers the IdP (requires `org:admin`):

```bash
curl -X PUT https://<reaper-host>/orgs/<org>/sso/config \
  -H "Authorization: Bearer <admin-session-or-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "protocol": "oidc",
    "enabled": true,
    "issuer": "https://example.okta.com",
    "client_id": "0oaXXXX",
    "client_secret": "••••",
    "allowed_domains": ["example.com"],
    "default_role": "viewer",
    "attr_map": {
      "groups_claim": "groups",
      "group_map": { "reaper-admins": "owner", "reaper-devs": "developer" }
    }
  }'
```

- `client_secret` is stored **encrypted** (XChaCha20-Poly1305) and never returned;
  `GET .../sso/config` shows the config with the secret redacted.
- `discovery_url` is derived from `issuer` (`{issuer}/.well-known/openid-configuration`)
  unless you set it explicitly; `jwks_url` likewise comes from discovery.
- `allowed_domains` (optional) restricts login to those verified email domains.
- `default_role` applies when a user's groups match nothing in `group_map`.

## 3. Log in

Point users at `GET /auth/sso/<org>/start`. On success the callback returns the
session token.

## Group → role mapping

`group_map` maps IdP group names to Reaper roles (`owner`, `admin`, `developer`,
`viewer`); the **highest-privilege** matched group wins, else `default_role`. Role
drift is reconciled **on every login**, so removing a user from an IdP group
downgrades them next time they authenticate.

**Hard invariant:** an IdP group can never confer platform super-admin. The
ceiling is `OrgRole::Owner`, which is full control of *its own* org but is
deliberately **not** the platform `admin` scope — so a compromised or misconfigured
IdP cannot mint a cross-tenant super-admin. This is structural (mapping only
produces an `OrgRole`) and covered by a regression test.

## Identity linkage

Users are keyed on `(issuer, subject)`, not email — email can change or be reused
for a new person. First login by a known verified email **adopts** an existing
local account (linking its IdP identity); otherwise a new SSO user is provisioned
with no usable local password (IdP-only). Every login writes an `sso.login` audit
record (actor = corporate subject, IP, user-agent).

## Security properties

- **PKCE (S256)** on every flow; **`nonce`** binds the ID token to the request.
- **State** is sealed with authenticated encryption (carrying org + nonce + PKCE
  verifier), so it can't be forged or read and the flow needs no server-side
  storage. Tampered/expired/cross-org state is rejected.
- **ID-token validation** reuses the hardened JWKS validator: mandatory
  `audience` (= client_id), issuer check, expiry, and rejection of `alg:none` /
  HMAC-confusion.
- **SSRF guard** on the discovery, token, and JWKS URLs — they must resolve to
  public HTTPS endpoints, never internal/metadata addresses.

## Behind a proxy

The callback URL must match between the authorize request and the token
exchange. Set `REAPER_PUBLIC_URL` (e.g. `https://reaper.example.com`) so Reaper
builds the exact redirect URI regardless of internal `Host` headers.

## Provider notes (groups claim)

- **Okta:** add a "groups" claim to the ID token (filter to the groups you map);
  `groups_claim: "groups"`.
- **Entra ID (Azure AD):** emits `roles` for app roles or `groups` (object IDs);
  set `groups_claim` accordingly. For large directories enable group claims or use
  app roles to avoid the overage indirection.
- **Google Workspace:** no groups in the ID token by default — map by
  `allowed_domains` + `default_role`, or provision via SCIM (below).

---

# SCIM 2.0 Provisioning

SCIM lets your IdP's **directory sync** create and — critically —
**deprovision** Reaper users automatically. When a user is removed or
deactivated in the IdP, Reaper drops their org membership **and revokes all their
live sessions within the same request**, so a terminated employee loses access
without an admin touching Reaper.

## 1. Mint a SCIM token

An org admin creates a per-org bearer token (`org:admin`):

```bash
curl -X POST https://<reaper-host>/orgs/<org>/scim/tokens \
  -H "Authorization: Bearer <admin>" -H "Content-Type: application/json" \
  -d '{"name":"okta-directory-sync"}'
# → { "id": "...", "token": "scim_…" }   ← shown once; store it now
```

The token is stored only as a SHA-256 hash. `GET`/`DELETE
/orgs/<org>/scim/tokens[/{id}]` list and revoke them. The org a token belongs to
is the **only** tenant it can act on.

## 2. Point your IdP at the SCIM base URL

- **Base URL:** `https://<reaper-host>/scim/v2`
- **Auth:** HTTP Header, `Authorization: Bearer scim_…`

Supported: `Users` (`POST`, `GET` with `filter=userName eq "…"`, `GET/{id}`,
`PUT`, `PATCH`, `DELETE`) and read-only `Groups` (the org's four roles, for
discovery). Responses use the standard SCIM `ListResponse` / `Error` envelopes.

## Lifecycle

- **Create** (`POST /Users`) provisions the user (or adopts an existing
  verified-email account) and adds them to the org at the default role.
- **Deprovision** (`DELETE /Users/{id}` or `PATCH active=false`) removes the org
  membership, **revokes every session** for that user, and suspends the account
  if it has no remaining orgs.
- Users are tenant-scoped: a token only ever sees/acts on members of its own org.

Every SCIM operation writes an audit record (`scim.user_provision`,
`scim.user_deprovision`, `scim.token_create`, …).

> Role assignment via SCIM **Group** membership push (directory-driven roles) is
> a later phase; today roles come from the OIDC group→role mapping at login or the
> Reaper API. SCIM Group endpoints are read-only for discovery.
