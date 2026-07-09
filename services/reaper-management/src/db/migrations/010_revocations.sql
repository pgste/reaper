-- Bundle revocation list (Plan 02, Phase B, step 4).
-- Per-org distrust of already-signed bundles: by bundle-bytes SHA-256
-- ('hash') or by signing key id ('key_id', a leaked key).

CREATE TABLE IF NOT EXISTS bundle_revocations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,          -- 'hash' | 'key_id'
    value TEXT NOT NULL,         -- lowercase-hex sha256, or the key id
    reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(org_id, kind, value)
);

CREATE INDEX IF NOT EXISTS idx_bundle_revocations_org ON bundle_revocations(org_id);

-- Per-org monotonic serial for the served list, so an agent can reject a
-- stale/replayed list. Bumped on every add.
CREATE TABLE IF NOT EXISTS revocation_state (
    org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    serial INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
