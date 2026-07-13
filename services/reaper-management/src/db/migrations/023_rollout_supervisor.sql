-- B2 (PROD R2-1): autonomous auto-rollback control loop.
--  * rollback_configs.mode: 'monitor' (evaluate + audit + alert, take NO
--    action — the safe dry-run default per the T-3 design ADR) vs 'enforce'
--    (the supervisor cancels the bad rollout and rolls back).
--  * rollouts.triggered_by: provenance marker. Rollback rollouts started by
--    the supervisor are stamped 'auto_rollback' so the supervisor never
--    evaluates (and re-rolls-back) its own remediation — the loop guard.
--  * idx_rollouts_status: the supervisor enumerates ACTIVE rollouts across
--    all orgs every tick; keep that a single indexed scan.
ALTER TABLE rollback_configs ADD COLUMN mode TEXT NOT NULL DEFAULT 'monitor';
ALTER TABLE rollouts ADD COLUMN triggered_by TEXT;
CREATE INDEX IF NOT EXISTS idx_rollouts_status ON rollouts(status);
