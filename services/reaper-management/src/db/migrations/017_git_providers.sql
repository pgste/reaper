-- Plan 09 Step 6/7 (GitOps Phase B): reshape the git link from
-- personal-OAuth-token to GitHub App install + webhook, and add an auditable
-- sync trail.

-- Provider identity on a source. `provider` distinguishes github/gitlab so a
-- webhook can resolve the right handler; `installation_id` is the GitHub App
-- installation whose short-lived token clones this repo (no PAT in the URL);
-- `repo_full_name` is the "owner/repo" a webhook payload carries, so a push
-- event can find its source without parsing the (token-free) clone URL.
ALTER TABLE policy_sources ADD COLUMN provider TEXT;
ALTER TABLE policy_sources ADD COLUMN installation_id TEXT;
ALTER TABLE policy_sources ADD COLUMN repo_full_name TEXT;

CREATE INDEX IF NOT EXISTS idx_policy_sources_repo ON policy_sources(provider, repo_full_name);

-- Auditable per-sync history (mini-design D-B). One row per sync attempt:
-- what commit, what came of it, and which bundle it produced.
CREATE TABLE IF NOT EXISTS source_syncs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES policy_sources(id) ON DELETE CASCADE,
    commit_sha TEXT,
    status TEXT NOT NULL,                    -- 'success' | 'failed'
    policies_found INTEGER NOT NULL DEFAULT 0,
    bundle_id TEXT REFERENCES bundles(id) ON DELETE SET NULL,
    trigger TEXT NOT NULL DEFAULT 'manual',  -- 'manual' | 'schedule' | 'webhook'
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_source_syncs_source ON source_syncs(source_id, started_at);

-- GitHub App installations per org (Plan 09 Step 6). Stores the installation
-- id (NOT a user PAT) recorded when an org admin installs the App; clone auth
-- mints a short-lived token from it at sync time. One installation per org.
CREATE TABLE IF NOT EXISTS github_app_installations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    installation_id TEXT NOT NULL,
    account_login TEXT,                       -- the GitHub org/user the App was installed on
    installed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(org_id)
);
