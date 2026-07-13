-- Plan 12 Phase 3: model-version provenance on published data versions +
-- full model snapshots on the migration history (rollback inputs).
ALTER TABLE adm_versions ADD COLUMN IF NOT EXISTS model_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE adm_model_versions ADD COLUMN IF NOT EXISTS model_before TEXT;
ALTER TABLE adm_model_versions ADD COLUMN IF NOT EXISTS model_after TEXT;
