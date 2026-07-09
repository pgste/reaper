-- Enterprise Identity — SSO (Plan 03, Phase 1).

CREATE TABLE IF NOT EXISTS sso_configs (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    protocol TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    issuer TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret_encrypted TEXT,
    discovery_url TEXT,
    jwks_url TEXT,
    attr_map_json TEXT,
    allowed_domains_json TEXT,
    default_role TEXT NOT NULL DEFAULT 'viewer',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_sso_configs_org_protocol
    ON sso_configs(org_id, protocol);

-- Link a user to its external IdP identity; JIT lookups key on (issuer,
-- subject), never email. NULL for local password users (NULLs are distinct in
-- the unique index).
ALTER TABLE users ADD COLUMN IF NOT EXISTS external_idp_subject TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS external_idp_issuer TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_idp_identity
    ON users(external_idp_issuer, external_idp_subject);
