-- Round-3 Plan 03: decision-quality signal for auto-rollback.
-- Persist the agent-reported eval-error count alongside allow/deny so the
-- control plane can key auto-rollback on runtime decision quality, not only
-- bundle-apply success. Additive + defaulted: existing rows and older agents
-- (which omit the field) read 0.
ALTER TABLE agent_metrics_latest ADD COLUMN eval_errors INTEGER NOT NULL DEFAULT 0;
