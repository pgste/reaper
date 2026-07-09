-- Enterprise Identity — SSO (Plan 03, Phase 1).
--
-- Per-org OIDC identity-provider configuration and the linkage from a Reaper
-- user to its external IdP identity. SCIM (scim_tokens) and SAML columns land
-- in later phases; this migration carries only what native OIDC needs.

CREATE TABLE IF NOT EXISTS sso_configs (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    protocol TEXT NOT NULL,                 -- 'oidc' (saml in a later phase)
    enabled INTEGER NOT NULL DEFAULT 1,
    issuer TEXT NOT NULL,                   -- IdP issuer (iss claim, discovery base)
    client_id TEXT NOT NULL,
    client_secret_encrypted TEXT,           -- encrypt_token() output; NULL for public/PKCE-only clients
    discovery_url TEXT,                      -- {issuer}/.well-known/openid-configuration (derived if NULL)
    jwks_url TEXT,                           -- explicit override; else taken from discovery
    attr_map_json TEXT,                      -- {"groups_claim":"groups","group_map":{"reaper-admins":"owner"}}
    allowed_domains_json TEXT,               -- ["example.com"] — empty/absent = any verified email
    default_role TEXT NOT NULL DEFAULT 'viewer',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- One config per protocol per org.
CREATE UNIQUE INDEX IF NOT EXISTS idx_sso_configs_org_protocol
    ON sso_configs(org_id, protocol);

-- Link a user to its external IdP identity. JIT lookups key on (issuer,
-- subject), never email — email can change or be reused for a new person.
-- Both columns are NULL for local password users; SQLite treats NULLs as
-- distinct, so the unique index does not collide across password accounts.
ALTER TABLE users ADD COLUMN external_idp_subject TEXT;
ALTER TABLE users ADD COLUMN external_idp_issuer TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_idp_identity
    ON users(external_idp_issuer, external_idp_subject);
