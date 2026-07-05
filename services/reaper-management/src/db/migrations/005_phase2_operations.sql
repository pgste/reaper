-- Phase 2: Operational Excellence
-- Agent deployment tracking, auto-rollback configuration

-- Per-agent deployment status tracking
CREATE TABLE IF NOT EXISTS agent_deployments (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    rollout_id TEXT REFERENCES rollouts(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, deploying, deployed, failed
    error_message TEXT,
    deployed_at TEXT,
    acknowledged_at TEXT,  -- When agent confirmed receipt
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_deployments_agent ON agent_deployments(agent_id);
CREATE INDEX IF NOT EXISTS idx_agent_deployments_bundle ON agent_deployments(bundle_id);
CREATE INDEX IF NOT EXISTS idx_agent_deployments_rollout ON agent_deployments(rollout_id);
CREATE INDEX IF NOT EXISTS idx_agent_deployments_status ON agent_deployments(status);

-- Auto-rollback configuration
CREATE TABLE IF NOT EXISTS rollback_configs (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    namespace_id TEXT REFERENCES namespaces(id) ON DELETE CASCADE,
    is_enabled INTEGER DEFAULT 0,
    error_rate_threshold DOUBLE PRECISION DEFAULT 5.0,  -- Percentage (e.g., 5.0 = 5%)
    window_seconds INTEGER DEFAULT 300,     -- Time window to evaluate errors
    min_requests INTEGER DEFAULT 100,       -- Minimum requests before triggering
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, namespace_id)
);
CREATE INDEX IF NOT EXISTS idx_rollback_configs_org ON rollback_configs(org_id);

-- Bundle diff cache (for preview/comparison)
CREATE TABLE IF NOT EXISTS bundle_diffs (
    id TEXT PRIMARY KEY,
    base_bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    target_bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    diff_json TEXT NOT NULL,  -- Cached diff result
    created_at TEXT NOT NULL,
    UNIQUE(base_bundle_id, target_bundle_id)
);
CREATE INDEX IF NOT EXISTS idx_bundle_diffs_base ON bundle_diffs(base_bundle_id);
CREATE INDEX IF NOT EXISTS idx_bundle_diffs_target ON bundle_diffs(target_bundle_id);
