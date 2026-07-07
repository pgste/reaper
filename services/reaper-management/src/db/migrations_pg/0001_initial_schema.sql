-- Reaper Management — PostgreSQL initial schema (v1)
-- Concatenation of SQLite migrations 001-008, which are written in
-- portable SQL (TEXT PKs, INTEGER flags, TEXT timestamps). Append-only:
-- future schema changes go in 0002_*.sql, never edits to this file.

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
    description TEXT,
    source_type TEXT NOT NULL,              -- 'git' or 'api'
    config TEXT NOT NULL,                   -- JSON configuration
    sync_interval_secs INTEGER DEFAULT 300, -- Sync interval in seconds (0 = manual only)
    sync_status TEXT DEFAULT 'pending',     -- 'pending', 'syncing', 'success', 'failed', 'disabled'
    is_enabled INTEGER DEFAULT 1,
    last_sync_at TEXT,
    last_sync_commit TEXT,                  -- Git commit hash or API version
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
    issuer TEXT NOT NULL,                    -- Expected issuer claim
    audience TEXT,                           -- Expected audience claim
    is_active INTEGER DEFAULT 1,
    cache_ttl_secs INTEGER DEFAULT 3600,     -- How long to cache JWKS keys
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_jwks_configs_org ON jwks_configs(org_id);
CREATE INDEX IF NOT EXISTS idx_jwks_configs_issuer ON jwks_configs(issuer);

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
-- Reaper Management Server - Security Enhancements
-- Phase 5: JWKS Validation and mTLS Support

-- JWKS configurations for external identity providers
CREATE TABLE IF NOT EXISTS jwks_configs (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    jwks_url TEXT NOT NULL,              -- JWKS endpoint URL
    issuer TEXT NOT NULL,                -- Expected issuer claim
    audience TEXT,                        -- Expected audience claim (optional)
    is_active INTEGER DEFAULT 1,
    cache_ttl_secs INTEGER DEFAULT 3600, -- How long to cache JWKS keys
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);

CREATE INDEX IF NOT EXISTS idx_jwks_configs_org ON jwks_configs(org_id);
CREATE INDEX IF NOT EXISTS idx_jwks_configs_issuer ON jwks_configs(issuer);

-- Session tokens for JWKS-authenticated users (optional tracking)
CREATE TABLE IF NOT EXISTS jwks_sessions (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    config_id TEXT NOT NULL REFERENCES jwks_configs(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,               -- sub claim from token
    email TEXT,                           -- email claim if present
    groups TEXT DEFAULT '[]',            -- JSON array of groups
    token_jti TEXT,                      -- JWT ID for revocation tracking
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_jwks_sessions_org ON jwks_sessions(org_id);
CREATE INDEX IF NOT EXISTS idx_jwks_sessions_subject ON jwks_sessions(subject);
CREATE INDEX IF NOT EXISTS idx_jwks_sessions_expires ON jwks_sessions(expires_at);
-- Reaper Management Server - Users and Audit Logging
-- Phase 1: SaaS Foundation - User accounts, sessions, audit trails

-- Users table (email/password auth)
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    email_verified INTEGER DEFAULT 0,
    password_hash TEXT NOT NULL,        -- Argon2id
    status TEXT DEFAULT 'pending',      -- pending, active, suspended
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_login_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_status ON users(status);

-- User-Org memberships with roles
CREATE TABLE IF NOT EXISTS user_orgs (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'viewer', -- owner, admin, developer, viewer
    invited_by TEXT,                     -- user_id who invited
    joined_at TEXT NOT NULL,
    UNIQUE(user_id, org_id)
);
CREATE INDEX IF NOT EXISTS idx_user_orgs_user ON user_orgs(user_id);
CREATE INDEX IF NOT EXISTS idx_user_orgs_org ON user_orgs(org_id);

-- Sessions for user auth (token-based sessions)
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,    -- SHA-256 of session token
    ip_address TEXT,
    user_agent TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token_hash);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

-- Audit log for compliance and debugging
CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY,
    org_id TEXT,                        -- NULL for platform-level actions
    actor_type TEXT NOT NULL,           -- 'user', 'api_key', 'agent', 'system'
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,               -- 'org.create', 'bundle.promote', etc.
    resource_type TEXT,                 -- 'org', 'agent', 'bundle', 'policy', 'user'
    resource_id TEXT,
    details TEXT,                       -- JSON: before/after, metadata
    ip_address TEXT,
    user_agent TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_log_org ON audit_log(org_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON audit_log(actor_type, actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_action ON audit_log(action);
CREATE INDEX IF NOT EXISTS idx_audit_log_resource ON audit_log(resource_type, resource_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON audit_log(created_at);

-- OAuth connections for Git providers
CREATE TABLE IF NOT EXISTS oauth_connections (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,             -- 'github', 'gitlab', 'bitbucket'
    provider_user_id TEXT,
    provider_username TEXT,
    access_token_encrypted TEXT NOT NULL,
    refresh_token_encrypted TEXT,
    token_expires_at TEXT,
    scopes TEXT,                        -- JSON array
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, provider)
);
CREATE INDEX IF NOT EXISTS idx_oauth_connections_org ON oauth_connections(org_id);
CREATE INDEX IF NOT EXISTS idx_oauth_connections_user ON oauth_connections(user_id);

-- Outbound webhook subscriptions (customer-defined webhooks)
CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT,                        -- For HMAC signing outbound requests
    events TEXT NOT NULL,               -- JSON array of event types
    is_active INTEGER DEFAULT 1,
    last_triggered_at TEXT,
    failure_count INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, name)
);
CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_org ON webhook_subscriptions(org_id);
CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_active ON webhook_subscriptions(is_active);

-- Password reset tokens
CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,    -- SHA-256 of reset token
    expires_at TEXT NOT NULL,
    used_at TEXT,                       -- When token was used (NULL if not used)
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_user ON password_reset_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_token ON password_reset_tokens(token_hash);

-- Email verification tokens
CREATE TABLE IF NOT EXISTS email_verification_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_user ON email_verification_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_token ON email_verification_tokens(token_hash);
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
-- Data Plane: managed Authorization Data Model (ADM) per namespace.
-- See docs/development/DATA_PLANE_PLAN.md (Phase D1).

-- One datastore per (org, namespace): holds the model definition (schema
-- layer) and version counters. Records live in the adm_* tables below.
CREATE TABLE IF NOT EXISTS datastores (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    namespace_id TEXT NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    template TEXT NOT NULL,                  -- 'rbac' | 'abac' | 'rebac' | 'combined'
    model TEXT NOT NULL,                     -- JSON ModelDefinition (typed vocabulary)
    current_version INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id, namespace_id)
);

CREATE INDEX IF NOT EXISTS idx_datastores_org ON datastores(org_id);

-- Entities: subjects/resources with TYPED attributes (validated against the
-- model at write time — the type-strict comparison contract enforced at the
-- source).
CREATE TABLE IF NOT EXISTS adm_entities (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    entity_id TEXT NOT NULL,                 -- the id policies see (e.g. "alice")
    entity_type TEXT NOT NULL,               -- must exist in model.entity_types
    attributes TEXT NOT NULL DEFAULT '{}',   -- JSON object, schema-validated
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(datastore_id, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_adm_entities_store ON adm_entities(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_entities_type ON adm_entities(datastore_id, entity_type);

-- Role bindings: subject -> role (RBAC records — roles are model vocabulary).
-- scope '' means org/namespace-wide.
CREATE TABLE IF NOT EXISTS adm_role_bindings (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,                   -- entity_id of a user/group
    role TEXT NOT NULL,                      -- must exist in model.roles
    scope TEXT NOT NULL DEFAULT '',          -- optional resource/namespace scope
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, subject, role, scope)
);

CREATE INDEX IF NOT EXISTS idx_adm_bindings_store ON adm_role_bindings(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_bindings_role ON adm_role_bindings(datastore_id, role);
CREATE INDEX IF NOT EXISTS idx_adm_bindings_subject ON adm_role_bindings(datastore_id, subject);

-- Relationship tuples: (object, relation, subject) — Zanzibar shape,
-- materialized verbatim into entity relationships{} for the ReBAC graph.
CREATE TABLE IF NOT EXISTS adm_tuples (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    object TEXT NOT NULL,                    -- entity_id the relation is ON
    relation TEXT NOT NULL,                  -- must exist in model.relations
    subject TEXT NOT NULL,                   -- entity_id that HOLDS the relation
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, object, relation, subject)
);

CREATE INDEX IF NOT EXISTS idx_adm_tuples_store ON adm_tuples(datastore_id);
CREATE INDEX IF NOT EXISTS idx_adm_tuples_object ON adm_tuples(datastore_id, object, relation);
CREATE INDEX IF NOT EXISTS idx_adm_tuples_subject ON adm_tuples(datastore_id, subject);

-- Published versions: immutable materialized data bundles. `document` is the
-- exact policy-engine DataLoader format — agents/sync fetch it by version.
CREATE TABLE IF NOT EXISTS adm_versions (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    checksum TEXT NOT NULL,                  -- sha256 of the document
    document TEXT NOT NULL,                  -- materialized entities JSON
    entity_count INTEGER NOT NULL,
    tuple_count INTEGER NOT NULL,
    binding_count INTEGER NOT NULL,
    published_by TEXT NOT NULL,
    published_at TEXT NOT NULL,
    UNIQUE(datastore_id, version)
);

CREATE INDEX IF NOT EXISTS idx_adm_versions_store ON adm_versions(datastore_id, version);
-- Data Plane D2: durable change log (transactional outbox pattern).
-- Every ADM mutation appends a dirty-entity marker with a monotonic
-- per-datastore sequence. Sync clients PULL deltas by sequence — a lost
-- event can never lose data, because the log IS the source, not the
-- notification. (Postgres LISTEN/NOTIFY becomes the wake-up signal when
-- the pg driver lands. Polling stays the always-correct fallback.)

ALTER TABLE datastores ADD COLUMN change_seq INTEGER NOT NULL DEFAULT 0;

ALTER TABLE adm_versions ADD COLUMN change_seq INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS adm_changes (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    entity_id TEXT NOT NULL,                 -- whose materialized doc changed
    tombstone INTEGER NOT NULL DEFAULT 0,    -- 1 = entity deleted
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_adm_changes_store_seq ON adm_changes(datastore_id, seq);
-- Two-way data-plane visibility: agents report their replica state with
-- every heartbeat so the control plane (landscape/UI) can show exactly
-- which data version each reaper serves and whether it is stale.

ALTER TABLE agent_metrics_latest ADD COLUMN data_version INTEGER;

ALTER TABLE agent_metrics_latest ADD COLUMN data_applied_seq INTEGER;

ALTER TABLE agent_metrics_latest ADD COLUMN data_stale INTEGER;

-- Auth hot path: the per-request API-key lookup is by key_hash, which had
-- no index — a table scan on every authenticated request.
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
