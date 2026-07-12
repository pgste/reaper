-- Plan 10 Phase B (Environments & Promotion): change requests + approvals.

CREATE TABLE IF NOT EXISTS change_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    from_env_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    to_env_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    data_version INTEGER,
    strategy_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    requested_by TEXT NOT NULL,
    rollout_id TEXT,
    reason TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_change_requests_org_status ON change_requests(org_id, status);

CREATE TABLE IF NOT EXISTS change_request_approvals (
    id TEXT PRIMARY KEY,
    change_request_id TEXT NOT NULL REFERENCES change_requests(id) ON DELETE CASCADE,
    approver_id TEXT NOT NULL,
    decision TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(change_request_id, approver_id)
);

CREATE INDEX IF NOT EXISTS idx_cr_approvals_request ON change_request_approvals(change_request_id);
