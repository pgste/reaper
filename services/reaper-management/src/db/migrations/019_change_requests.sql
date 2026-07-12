-- Plan 10 Phase B (Environments & Promotion): a change_request captures a
-- pending env→env promotion, pinned to a bundle + data version; approvals
-- record each approver decision. On reaching the target env's approval
-- threshold the request is Approved and the existing rollout machinery is
-- invoked, then the request is marked Applied.

CREATE TABLE IF NOT EXISTS change_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    from_env_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    to_env_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    data_version INTEGER,                     -- source env data-plane version pinned at request time
    strategy_id TEXT,                          -- rollout strategy (NULL = target env / org default)
    status TEXT NOT NULL DEFAULT 'pending',    -- pending|approved|rejected|applied|cancelled
    requested_by TEXT NOT NULL,
    rollout_id TEXT,                           -- set when the promotion is applied
    reason TEXT,                               -- rejection/cancel reason
    created_at TEXT NOT NULL,
    decided_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_change_requests_org_status ON change_requests(org_id, status);

CREATE TABLE IF NOT EXISTS change_request_approvals (
    id TEXT PRIMARY KEY,
    change_request_id TEXT NOT NULL REFERENCES change_requests(id) ON DELETE CASCADE,
    approver_id TEXT NOT NULL,
    decision TEXT NOT NULL,                    -- approve|reject
    reason TEXT,
    created_at TEXT NOT NULL,
    -- one decision per approver per request; a re-vote updates in place
    UNIQUE(change_request_id, approver_id)
);

CREATE INDEX IF NOT EXISTS idx_cr_approvals_request ON change_request_approvals(change_request_id);
