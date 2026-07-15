-- SIEM export connectors (round-2 E1, slice 3). See the SQLite twin
-- (026_siem_connectors.sql) for the full rationale.

CREATE TABLE IF NOT EXISTS siem_connectors (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    connector_type TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    secret TEXT,
    format TEXT NOT NULL DEFAULT 'ocsf',
    enabled INTEGER NOT NULL DEFAULT 1,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_export_at TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_siem_connectors_org ON siem_connectors(org_id);
