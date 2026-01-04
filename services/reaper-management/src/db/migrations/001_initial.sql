-- Reaper Management Server - Initial Schema
-- Supports SQLite and PostgreSQL

-- Organizations: Top-level multi-tenancy unit
CREATE TABLE IF NOT EXISTS organizations (
    id TEXT PRIMARY KEY,                    -- UUID as text for SQLite compatibility
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,              -- URL-safe identifier (lowercase, hyphens)
    display_name TEXT,
    description TEXT,
    settings TEXT DEFAULT '{}',             -- JSON settings
    created_at TEXT NOT NULL,               -- ISO 8601 timestamp
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_organizations_slug ON organizations(slug);

-- Teams: Belong to organizations
CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    description TEXT,
    settings TEXT DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_teams_org ON teams(org_id);

-- Policy Sources: Git repositories or external APIs
CREATE TABLE IF NOT EXISTS policy_sources (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    source_type TEXT NOT NULL,              -- 'git' or 'api'
    config TEXT NOT NULL,                   -- JSON configuration
    is_active INTEGER DEFAULT 1,
    last_sync_at TEXT,
    last_sync_commit TEXT,                  -- Git commit hash or API version
    last_sync_status TEXT,                  -- 'success', 'failed', 'pending'
    last_sync_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_policy_sources_org ON policy_sources(org_id);

-- Policies: Managed per organization/team
CREATE TABLE IF NOT EXISTS policies (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    team_id TEXT REFERENCES teams(id) ON DELETE SET NULL,
    source_id TEXT REFERENCES policy_sources(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    description TEXT,
    language TEXT NOT NULL DEFAULT 'reaper', -- 'reaper', 'cedar', 'simple'
    source_path TEXT,                        -- Path in source (git path or API id)
    current_version INTEGER NOT NULL DEFAULT 1,
    is_active INTEGER DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_policies_org ON policies(org_id);
CREATE INDEX IF NOT EXISTS idx_policies_team ON policies(team_id);
CREATE INDEX IF NOT EXISTS idx_policies_source ON policies(source_id);

-- Policy Versions: Immutable history
CREATE TABLE IF NOT EXISTS policy_versions (
    id TEXT PRIMARY KEY,
    policy_id TEXT NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,                   -- Policy source code
    content_hash TEXT NOT NULL,              -- SHA-256 of content
    source_commit TEXT,                      -- Git commit or API version
    created_at TEXT NOT NULL,
    created_by TEXT,
    change_summary TEXT,
    UNIQUE(policy_id, version)
);

CREATE INDEX IF NOT EXISTS idx_policy_versions_policy ON policy_versions(policy_id);

-- Bundles: Compiled policy packages with promotion workflow
CREATE TABLE IF NOT EXISTS bundles (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    version TEXT NOT NULL DEFAULT '1.0.0',
    status TEXT NOT NULL DEFAULT 'draft',   -- 'draft', 'compiled', 'staged', 'promoted', 'deprecated'
    storage_key TEXT,                        -- Key in storage backend
    size_bytes INTEGER,
    checksum TEXT,                           -- SHA-256 of bundle
    policy_count INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    compiled_at TEXT,
    promoted_at TEXT,
    UNIQUE(org_id, name, version)
);

CREATE INDEX IF NOT EXISTS idx_bundles_org ON bundles(org_id);
CREATE INDEX IF NOT EXISTS idx_bundles_status ON bundles(status);

-- Bundle Policies: Policies included in a bundle
CREATE TABLE IF NOT EXISTS bundle_policies (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    policy_id TEXT NOT NULL REFERENCES policies(id) ON DELETE CASCADE,
    policy_version INTEGER NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,     -- Lower = higher priority
    created_at TEXT NOT NULL,
    UNIQUE(bundle_id, policy_id)
);

CREATE INDEX IF NOT EXISTS idx_bundle_policies_bundle ON bundle_policies(bundle_id);

-- Bundle Promotions: Audit log for status changes
CREATE TABLE IF NOT EXISTS bundle_promotions (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    promoted_by TEXT,
    notes TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bundle_promotions_bundle ON bundle_promotions(bundle_id);

-- Agents: Registered agents per organization
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    hostname TEXT,
    ip_address TEXT,
    port INTEGER DEFAULT 8080,
    version TEXT,
    capabilities TEXT DEFAULT '[]',          -- JSON array
    labels TEXT DEFAULT '{}',                -- JSON object
    status TEXT DEFAULT 'pending',           -- 'pending', 'healthy', 'unhealthy', 'offline'
    last_heartbeat_at TEXT,
    registered_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    metadata TEXT DEFAULT '{}',
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_agents_org ON agents(org_id);
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);

-- Agent Bundles: Track deployments per agent
CREATE TABLE IF NOT EXISTS agent_bundles (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    deployed_at TEXT NOT NULL,
    deployment_status TEXT DEFAULT 'pending', -- 'pending', 'deployed', 'failed'
    error_message TEXT,
    UNIQUE(agent_id, bundle_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_bundles_agent ON agent_bundles(agent_id);

-- API Keys: Per-organization authentication
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    key_prefix TEXT NOT NULL,                -- First 8 chars for identification
    key_hash TEXT NOT NULL,                  -- SHA-256 of full key
    scopes TEXT DEFAULT '["agent:read", "agent:register"]', -- JSON array
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL,
    created_by TEXT,
    is_revoked INTEGER DEFAULT 0,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_api_keys_org ON api_keys(org_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(key_prefix);

-- JWKS Configurations: Per-organization JWT validation
CREATE TABLE IF NOT EXISTS jwks_configs (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    jwks_url TEXT NOT NULL,                  -- URL to fetch JWKS
    issuer TEXT,                             -- Expected issuer claim
    audience TEXT,                           -- Expected audience claim
    cache_ttl_seconds INTEGER DEFAULT 3600,
    last_fetched_at TEXT,
    cached_keys TEXT,                        -- Cached JWKS JSON
    is_active INTEGER DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_jwks_configs_org ON jwks_configs(org_id);

-- Data Sources: Configuration for entity data notifications (not data itself)
CREATE TABLE IF NOT EXISTS data_sources (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    source_type TEXT NOT NULL,               -- 'http', 'kafka', 's3', etc.
    config TEXT NOT NULL,                    -- JSON configuration
    refresh_interval_seconds INTEGER DEFAULT 300,
    last_updated_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_data_sources_org ON data_sources(org_id);
