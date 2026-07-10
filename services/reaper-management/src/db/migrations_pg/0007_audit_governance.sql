-- Audit governance: tenant retention windows + legal holds (Plan 04, step 6).

CREATE TABLE IF NOT EXISTS audit_retention (
    org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    days INTEGER NOT NULL,
    updated_by TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_legal_holds (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    filter TEXT NOT NULL DEFAULT '{}',
    reason TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL,
    released_at TEXT,
    released_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_legal_holds_org ON audit_legal_holds(org_id);
CREATE INDEX IF NOT EXISTS idx_audit_legal_holds_active
    ON audit_legal_holds(org_id) WHERE released_at IS NULL;
