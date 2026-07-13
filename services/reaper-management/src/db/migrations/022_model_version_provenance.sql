-- Plan 12 Phase 3 (steps 7-8):
--  * every published data version records the MODEL version it was
--    materialized under, so decision audit can pin exactly which model
--    shape a decision saw (alongside data_version/data_checksum);
--  * the migration history stores the full before/after models (not just
--    hashes) so rollback can mechanically compose inverse transforms.
ALTER TABLE adm_versions ADD COLUMN model_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE adm_model_versions ADD COLUMN model_before TEXT;
ALTER TABLE adm_model_versions ADD COLUMN model_after TEXT;
