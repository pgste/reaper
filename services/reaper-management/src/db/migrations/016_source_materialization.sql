-- Plan 09 Step 2 (GitOps): link a compiled bundle to the source + commit that
-- produced it, so materialization is idempotent per SHA (a webhook and a poll
-- at the same commit never double-apply) and the sync trail is queryable.

ALTER TABLE bundles ADD COLUMN source_id TEXT REFERENCES policy_sources(id) ON DELETE SET NULL;
ALTER TABLE bundles ADD COLUMN source_commit TEXT;

CREATE INDEX IF NOT EXISTS idx_bundles_source_commit ON bundles(source_id, source_commit);
