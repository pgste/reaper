-- Idempotency keys for propagation-triggering POSTs (Plan 07, Phase D).
--
-- A row is claimed (INSERT, unique on scope+scope_id+idem_key) before the
-- operation runs and completed with the stored response afterwards; replays
-- within the retention window return the stored response instead of
-- re-triggering the side effect. request_hash pins the key to the exact
-- request (ADR-6): the same key with a different body is a 422, not a replay.

CREATE TABLE IF NOT EXISTS idempotency_keys (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,            -- operation, e.g. 'bundles.promote'
    scope_id TEXT NOT NULL,         -- tenant scope (org id) or '-' for global
    idem_key TEXT NOT NULL,         -- the client-supplied Idempotency-Key
    request_hash TEXT NOT NULL,     -- fingerprint of the request identity
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'completed'
    response_status INTEGER,        -- stored HTTP status (completed only)
    response_body TEXT,             -- stored JSON body (completed only)
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    UNIQUE (scope, scope_id, idem_key)
);

CREATE INDEX IF NOT EXISTS idx_idempotency_expires ON idempotency_keys(expires_at);
