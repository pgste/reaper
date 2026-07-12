-- Plan 09 Step 6/7 (GitOps Phase B): reshape the git link from
-- personal-OAuth-token to GitHub App install + webhook, and add an auditable
-- sync trail.

ALTER TABLE policy_sources ADD COLUMN IF NOT EXISTS provider TEXT;
ALTER TABLE policy_sources ADD COLUMN IF NOT EXISTS installation_id TEXT;
ALTER TABLE policy_sources ADD COLUMN IF NOT EXISTS repo_full_name TEXT;

CREATE INDEX IF NOT EXISTS idx_policy_sources_repo ON policy_sources(provider, repo_full_name);

CREATE TABLE IF NOT EXISTS source_syncs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES policy_sources(id) ON DELETE CASCADE,
    commit_sha TEXT,
    status TEXT NOT NULL,
    policies_found INTEGER NOT NULL DEFAULT 0,
    bundle_id TEXT REFERENCES bundles(id) ON DELETE SET NULL,
    trigger TEXT NOT NULL DEFAULT 'manual',
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_source_syncs_source ON source_syncs(source_id, started_at);

CREATE TABLE IF NOT EXISTS github_app_installations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    installation_id TEXT NOT NULL,
    account_login TEXT,
    installed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id)
);
