-- Enterprise Identity — SCIM 2.0 provisioning tokens (Plan 03, Phase 2).
--
-- A per-org bearer token an IdP's directory-sync (SCIM) client presents to
-- provision/deprovision users. Only the SHA-256 hash is stored — the plaintext
-- is shown once at creation and never again.

CREATE TABLE IF NOT EXISTS scim_tokens (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL,       -- sha256(bearer token)
    created_by TEXT,                -- actor id that minted it
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_scim_tokens_org ON scim_tokens(org_id);
-- Auth looks a token up by its hash; the hash must be globally unique so it
-- resolves to exactly one org (tenant isolation).
CREATE UNIQUE INDEX IF NOT EXISTS idx_scim_tokens_hash ON scim_tokens(token_hash);
