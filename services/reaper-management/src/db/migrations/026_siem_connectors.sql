-- SIEM export connectors (round-2 E1, slice 3): per-org outbound push targets
-- for decision-log export (Splunk HEC / generic HTTP), shaped as NDJSON / OCSF /
-- CEF. Low-volume config records — a standing exfiltration path — so they live
-- in the management DB (transactional, audited), gated on the dedicated
-- `audit:export` scope. Delivery reads the full history from the ClickHouse
-- decision store; only the connector *config* lives here.

CREATE TABLE IF NOT EXISTS siem_connectors (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- transport: splunk_hec | http
    connector_type TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    -- HMAC secret (http) or HEC token (splunk_hec); NULL = unauthenticated http.
    secret TEXT,
    -- record shape: ndjson | ocsf | cef
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
