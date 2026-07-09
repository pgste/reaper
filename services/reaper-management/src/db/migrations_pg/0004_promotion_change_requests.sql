-- Governed promotion (Plan 02, Phase B, step 5).

CREATE TABLE IF NOT EXISTS promotion_change_requests (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL,
    bundle_version TEXT,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    requester_id TEXT NOT NULL,
    approver_id TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_promotion_cr_org ON promotion_change_requests(org_id);
CREATE INDEX IF NOT EXISTS idx_promotion_cr_bundle ON promotion_change_requests(bundle_id);
