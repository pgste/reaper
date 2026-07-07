-- Two-way data-plane visibility: agents report their replica state with
-- every heartbeat so the control plane (landscape/UI) can show exactly
-- which data version each reaper serves and whether it is stale.

ALTER TABLE agent_metrics_latest ADD COLUMN data_version INTEGER;

ALTER TABLE agent_metrics_latest ADD COLUMN data_applied_seq INTEGER;

ALTER TABLE agent_metrics_latest ADD COLUMN data_stale INTEGER;

-- Auth hot path: the per-request API-key lookup is by key_hash, which had
-- no index — a table scan on every authenticated request.
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
