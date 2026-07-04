-- Data Plane D2: durable change log (transactional outbox pattern).
-- Every ADM mutation appends a dirty-entity marker with a monotonic
-- per-datastore sequence. Sync clients PULL deltas by sequence — a lost
-- event can never lose data, because the log IS the source, not the
-- notification. (Postgres LISTEN/NOTIFY becomes the wake-up signal when
-- the pg driver lands. Polling stays the always-correct fallback.)

ALTER TABLE datastores ADD COLUMN change_seq INTEGER NOT NULL DEFAULT 0;

ALTER TABLE adm_versions ADD COLUMN change_seq INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS adm_changes (
    id TEXT PRIMARY KEY,
    datastore_id TEXT NOT NULL REFERENCES datastores(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    entity_id TEXT NOT NULL,                 -- whose materialized doc changed
    tombstone INTEGER NOT NULL DEFAULT 0,    -- 1 = entity deleted
    created_at TEXT NOT NULL,
    UNIQUE(datastore_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_adm_changes_store_seq ON adm_changes(datastore_id, seq);
