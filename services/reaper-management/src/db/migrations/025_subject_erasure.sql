-- Subject-erasure receipts (E2 follow-up #3): a queryable history of GDPR
-- Art. 17 erasures, beyond the append-only audit-trail entry.
--
-- The audit trail (audit.subject_erasure) is the primary durable proof; this
-- table is the optional convenience the DSAR workflow queries — "show me every
-- erasure we ran for org X / subject Y, with its outcome". One row per completed
-- POST /orgs/{org}/audit/erasure. The full ErasureReceipt JSON (incl. the
-- immutable-surface exemptions[]) is stored verbatim in `receipt`; the columns
-- beside it are decomposed only so the history is filterable/queryable.
--
-- Recorded best-effort after the (irreversible) erasure completes: a write hiccup
-- here never fails an erasure that already ran, and never double-inserts on an
-- Idempotency-Key replay (the recording sits inside the idempotency-guarded op).

CREATE TABLE IF NOT EXISTS audit_erasure_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    requested_by TEXT,
    requested_at TEXT NOT NULL,
    -- decision-log step outcome: submitted | deferred_blanket_hold | store_not_configured
    decision_log_status TEXT NOT NULL,
    holds_honored INTEGER,               -- non-NULL only when submitted
    matched_pseudonyms INTEGER NOT NULL DEFAULT 0,
    -- datastore step outcome: erased | skipped
    datastore_status TEXT NOT NULL,
    datastores_scanned INTEGER NOT NULL DEFAULT 0,
    entities_deleted INTEGER NOT NULL DEFAULT 0,
    -- post-erasure store verification posture (always "linkage" today)
    verification_posture TEXT NOT NULL,
    -- the full ErasureReceipt JSON as shipped to the caller and audit trail
    receipt TEXT NOT NULL,
    completed_at TEXT NOT NULL
);

-- History reads are org-scoped and newest-first.
CREATE INDEX IF NOT EXISTS idx_audit_erasure_requests_org
    ON audit_erasure_requests(org_id, requested_at);
-- "every erasure for this subject" lookups.
CREATE INDEX IF NOT EXISTS idx_audit_erasure_requests_subject
    ON audit_erasure_requests(org_id, subject);
