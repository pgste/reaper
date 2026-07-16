-- Round-3 Plan 03: decision-quality auto-rollback.
-- Extend the existing (deploy-apply) rollback trigger with a decision-quality
-- arm so a policy that installs cleanly on every agent but then errors /
-- denies wrongly self-reverts. All additive + defaulted so the trigger is
-- byte-for-byte today's behaviour until an operator sets a threshold.

-- Per-config decision-quality thresholds (NULL = arm disabled for that signal).
ALTER TABLE rollback_configs ADD COLUMN eval_error_rate_threshold REAL;
ALTER TABLE rollback_configs ADD COLUMN denial_delta_threshold REAL;
ALTER TABLE rollback_configs ADD COLUMN latency_p99_slo_us REAL;
ALTER TABLE rollback_configs ADD COLUMN min_decisions INTEGER NOT NULL DEFAULT 0;

-- Pre-rollout denial-rate baseline, captured at start_rollout, so a *legitimate*
-- policy change that moves the deny rate is not punished (only a shift BEYOND
-- the delta band trips). NULL until captured (absent prior traffic).
ALTER TABLE rollouts ADD COLUMN baseline_denial_rate REAL;
