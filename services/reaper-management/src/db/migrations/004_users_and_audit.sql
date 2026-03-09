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
