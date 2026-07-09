-- Governed promotion (Plan 02, Phase B, step 5).
-- A promotion is no longer a single privileged call: it becomes a change
-- request that a SECOND distinct principal must approve before the bundle is
-- promoted. Every transition is recorded here (and in the audit log) so
-- "who promoted what, approved by whom, when" is always answerable.

CREATE TABLE IF NOT EXISTS promotion_change_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL,
    bundle_version TEXT,             -- pinned bundle version at request time
    kind TEXT NOT NULL,             -- 'promote' | 'rollback'
    status TEXT NOT NULL,           -- 'pending' | 'executed' | 'rejected'
    requester_id TEXT NOT NULL,     -- actor id that opened the request
    approver_id TEXT,               -- distinct actor id that approved+executed
    notes TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT                 -- when approved+executed or rejected
);

CREATE INDEX IF NOT EXISTS idx_promotion_cr_org ON promotion_change_requests(org_id);
CREATE INDEX IF NOT EXISTS idx_promotion_cr_bundle ON promotion_change_requests(bundle_id);
