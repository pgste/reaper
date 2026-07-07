-- Reaper Management Server - Namespaces & Deployment Features
-- Phase 2-4: Namespaces, Controlled Deployments, Fleet Metrics

-- Namespaces: Hierarchical isolation within organizations
CREATE TABLE IF NOT EXISTS namespaces (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,                   -- URL-safe slug (e.g., "production", "production/us-east")
    display_name TEXT,
    parent_id TEXT REFERENCES namespaces(id) ON DELETE CASCADE,
    description TEXT,
    settings TEXT DEFAULT '{}',           -- JSON settings
    is_active INTEGER DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_namespaces_org ON namespaces(org_id);
CREATE INDEX IF NOT EXISTS idx_namespaces_parent ON namespaces(parent_id);

-- Agent subscriptions to namespaces
CREATE TABLE IF NOT EXISTS agent_subscriptions (
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    namespace_id TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    include_children INTEGER DEFAULT 1,   -- Whether to receive events from child namespaces
    created_at TEXT NOT NULL,
    PRIMARY KEY (agent_id, namespace_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_subscriptions_namespace ON agent_subscriptions(namespace_id);

-- Deployment strategies
CREATE TABLE IF NOT EXISTS deployment_strategies (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    namespace_id TEXT REFERENCES namespaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    strategy_type TEXT NOT NULL,          -- 'immediate', 'canary', 'percentage', 'label_selector'
    config TEXT NOT NULL DEFAULT '{}',    -- JSON configuration
    is_default INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, namespace_id, name)
);

CREATE INDEX IF NOT EXISTS idx_deployment_strategies_org ON deployment_strategies(org_id);
CREATE INDEX IF NOT EXISTS idx_deployment_strategies_namespace ON deployment_strategies(namespace_id);

-- Rollouts: Track deployment progress
CREATE TABLE IF NOT EXISTS rollouts (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    strategy_id TEXT REFERENCES deployment_strategies(id),
    namespace_id TEXT REFERENCES namespaces(id),
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending', 'in_progress', 'completed', 'failed', 'rolled_back', 'cancelled'
    current_wave INTEGER DEFAULT 0,
    target_agent_count INTEGER DEFAULT 0,
    deployed_agent_count INTEGER DEFAULT 0,
    started_at TEXT,
    completed_at TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rollouts_bundle ON rollouts(bundle_id);
CREATE INDEX IF NOT EXISTS idx_rollouts_status ON rollouts(status);

-- Rollout waves: Track individual deployment waves
CREATE TABLE IF NOT EXISTS rollout_waves (
    id TEXT PRIMARY KEY,
    rollout_id TEXT NOT NULL REFERENCES rollouts(id) ON DELETE CASCADE,
    wave_number INTEGER NOT NULL,
    target_agents TEXT NOT NULL DEFAULT '[]',  -- JSON array of agent IDs
    status TEXT NOT NULL DEFAULT 'pending',     -- 'pending', 'deploying', 'completed', 'failed'
    deployed_count INTEGER DEFAULT 0,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(rollout_id, wave_number)
);

CREATE INDEX IF NOT EXISTS idx_rollout_waves_rollout ON rollout_waves(rollout_id);

-- Version pins: Pin agents to specific bundle versions
CREATE TABLE IF NOT EXISTS version_pins (
    agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    pinned_by TEXT,
    reason TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_version_pins_bundle ON version_pins(bundle_id);

-- Agent metrics: Latest metrics per agent
CREATE TABLE IF NOT EXISTS agent_metrics_latest (
    agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    requests_total INTEGER DEFAULT 0,
    requests_per_second DOUBLE PRECISION DEFAULT 0.0,
    latency_p50_us DOUBLE PRECISION DEFAULT 0.0,
    latency_p99_us DOUBLE PRECISION DEFAULT 0.0,
    decisions_allow INTEGER DEFAULT 0,
    decisions_deny INTEGER DEFAULT 0,
    memory_bytes INTEGER DEFAULT 0,
    uptime_seconds INTEGER DEFAULT 0,
    current_bundle_id TEXT,
    current_bundle_version TEXT,
    updated_at TEXT NOT NULL
);

-- Org metrics: Hourly aggregated metrics per org
CREATE TABLE IF NOT EXISTS org_metrics_hourly (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    hour TEXT NOT NULL,                    -- ISO 8601 hour (e.g., "2024-01-15T10:00:00Z")
    total_requests INTEGER DEFAULT 0,
    avg_latency_p99_us DOUBLE PRECISION DEFAULT 0.0,
    total_agents INTEGER DEFAULT 0,
    healthy_agents INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE(org_id, hour)
);

CREATE INDEX IF NOT EXISTS idx_org_metrics_hourly_org ON org_metrics_hourly(org_id);
CREATE INDEX IF NOT EXISTS idx_org_metrics_hourly_hour ON org_metrics_hourly(hour);

-- Add namespace_id to policies and bundles
-- Note: Using ALTER TABLE for SQLite which supports adding columns
-- ALTER TABLE policies ADD COLUMN namespace_id TEXT REFERENCES namespaces(id);
-- ALTER TABLE bundles ADD COLUMN namespace_id TEXT REFERENCES namespaces(id);

-- Client certificates for mTLS (Phase 5)
CREATE TABLE IF NOT EXISTS client_certificates (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
    fingerprint TEXT UNIQUE NOT NULL,
    subject TEXT,
    issuer TEXT,
    not_before TEXT,
    not_after TEXT,
    is_revoked INTEGER DEFAULT 0,
    revoked_at TEXT,
    revocation_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_client_certificates_org ON client_certificates(org_id);
CREATE INDEX IF NOT EXISTS idx_client_certificates_fingerprint ON client_certificates(fingerprint);
CREATE INDEX IF NOT EXISTS idx_client_certificates_agent ON client_certificates(agent_id);
