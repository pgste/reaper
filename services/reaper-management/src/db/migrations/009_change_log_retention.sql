-- Change-log retention support
-- Retention sweeps delete by age (created_at < cutoff) so the change log
-- needs an index on created_at to avoid full-table scans on every sweep
CREATE INDEX IF NOT EXISTS idx_adm_changes_created_at ON adm_changes(created_at);
