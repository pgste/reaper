-- Audit governance: tenant retention windows + legal holds (Plan 04, step 6).
--
-- These are low-volume, transactional GOVERNANCE records, so they live in the
-- management database (the control plane's source of truth, audited like all
-- other governance state) — not in ClickHouse. The application-driven purge
-- reads them and executes against ClickHouse, skipping held rows. A legal hold
-- is never hard-deleted: releasing it stamps released_at/released_by, so the
-- hold's own lifecycle stays auditable.

CREATE TABLE IF NOT EXISTS audit_retention (
    org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    days INTEGER NOT NULL,          -- retention window in days (> 0)
    updated_by TEXT,                -- actor id that last set it
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_legal_holds (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    -- JSON filter selecting the held decisions (principal/action/resource/
    -- decision/policy_name/agent_id/from/to, all optional; {} = hold ALL of
    -- the org's decisions).
    filter TEXT NOT NULL DEFAULT '{}',
    reason TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL,
    released_at TEXT,               -- NULL = active (exempt from purge)
    released_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_legal_holds_org ON audit_legal_holds(org_id);
-- The purge path scans active holds only.
CREATE INDEX IF NOT EXISTS idx_audit_legal_holds_active
    ON audit_legal_holds(org_id) WHERE released_at IS NULL;
