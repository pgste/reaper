-- Plan 12 (data-model migration engine) step 4: append-only model-version
-- history. One row per applied migration: the typed transform list, the
-- author, and before/after model hashes — rollback is a NEW forward row
-- (ADR-3), never a rewrite of this table.

CREATE TABLE IF NOT EXISTS adm_model_versions (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    model_version INTEGER NOT NULL,
    transforms TEXT NOT NULL,            -- JSON array of ModelTransform
    author TEXT NOT NULL,
    model_before_hash TEXT NOT NULL,
    model_after_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, model_version)
);

CREATE INDEX IF NOT EXISTS idx_adm_model_versions_store
    ON adm_model_versions(datastore_id, model_version);

-- The datastore's CURRENT model version (0 = never migrated). Bumped inside
-- the same transaction as the record transforms and the history row.
ALTER TABLE datastores ADD COLUMN model_version INTEGER NOT NULL DEFAULT 0;
