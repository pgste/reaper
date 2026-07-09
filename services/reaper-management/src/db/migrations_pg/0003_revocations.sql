-- Bundle revocation list (Plan 02, Phase B, step 4).

CREATE TABLE IF NOT EXISTS bundle_revocations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(org_id, kind, value)
);

CREATE INDEX IF NOT EXISTS idx_bundle_revocations_org ON bundle_revocations(org_id);

CREATE TABLE IF NOT EXISTS revocation_state (
    org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    serial BIGINT NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
