-- Subject-erasure receipts (E2 follow-up #3): a queryable history of GDPR
-- Art. 17 erasures, beyond the append-only audit-trail entry. See the SQLite
-- twin (025_subject_erasure.sql) for the full rationale.

CREATE TABLE IF NOT EXISTS audit_erasure_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    requested_by TEXT,
    requested_at TEXT NOT NULL,
    decision_log_status TEXT NOT NULL,
    holds_honored INTEGER,
    matched_pseudonyms INTEGER NOT NULL DEFAULT 0,
    datastore_status TEXT NOT NULL,
    datastores_scanned INTEGER NOT NULL DEFAULT 0,
    entities_deleted INTEGER NOT NULL DEFAULT 0,
    verification_posture TEXT NOT NULL,
    receipt TEXT NOT NULL,
    completed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_erasure_requests_org
    ON audit_erasure_requests(org_id, requested_at);
CREATE INDEX IF NOT EXISTS idx_audit_erasure_requests_subject
    ON audit_erasure_requests(org_id, subject);
