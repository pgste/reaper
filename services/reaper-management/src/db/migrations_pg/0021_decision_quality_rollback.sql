-- Round-3 Plan 03: decision-quality auto-rollback (see the SQLite twin 028).
-- Additive + defaulted so the trigger is unchanged until a threshold is set.

ALTER TABLE rollback_configs ADD COLUMN IF NOT EXISTS eval_error_rate_threshold DOUBLE PRECISION;
ALTER TABLE rollback_configs ADD COLUMN IF NOT EXISTS denial_delta_threshold DOUBLE PRECISION;
ALTER TABLE rollback_configs ADD COLUMN IF NOT EXISTS latency_p99_slo_us DOUBLE PRECISION;
ALTER TABLE rollback_configs ADD COLUMN IF NOT EXISTS min_decisions INTEGER NOT NULL DEFAULT 0;

ALTER TABLE rollouts ADD COLUMN IF NOT EXISTS baseline_denial_rate DOUBLE PRECISION;
