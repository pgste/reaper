-- Plan 10 (Environments & Promotion): first-class environment objects layered
-- over namespaces. The namespace stays the deployment/data scope unit; the
-- environment adds lifecycle ordering, an approval policy, and change windows
-- so env→env promotion is a governed transition.

CREATE TABLE IF NOT EXISTS environments (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    tier_order INTEGER NOT NULL DEFAULT 0,   -- dev < staging < prod ordering
    namespace_id TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    data_plane_ref TEXT,                       -- datastore id carried on promotion
    approval_policy TEXT NOT NULL DEFAULT '{}',-- JSON ApprovalPolicy
    change_windows TEXT NOT NULL DEFAULT '{}', -- JSON ChangeWindows (freeze periods)
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name),
    UNIQUE(namespace_id)                        -- one environment per namespace
);

CREATE INDEX IF NOT EXISTS idx_environments_org ON environments(org_id);
