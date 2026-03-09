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
