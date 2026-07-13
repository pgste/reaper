-- Plan 12 step 4: append-only model-version history + current model version.

CREATE TABLE IF NOT EXISTS adm_model_versions (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    model_version BIGINT NOT NULL,
    transforms TEXT NOT NULL,
    author TEXT NOT NULL,
    model_before_hash TEXT NOT NULL,
    model_after_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, model_version)
);

CREATE INDEX IF NOT EXISTS idx_adm_model_versions_store
    ON adm_model_versions(datastore_id, model_version);

ALTER TABLE datastores ADD COLUMN IF NOT EXISTS model_version BIGINT NOT NULL DEFAULT 0;
