-- Plan 06 Phase E (R3-07): honest bundle ETag. The old ETag was
-- updated_at.to_rfc3339() — two writes inside one timestamp resolution share
-- a tag, defeating the If-Match guard (the same R2-10 trap policies already
-- fixed in 0017/024). row_version is a dedicated integer bumped by EVERY
-- bundle UPDATE; the ETag derives from it and the repository guards updates
-- with AND row_version = $expected — a monotonic counter, not a clock.
ALTER TABLE bundles ADD COLUMN IF NOT EXISTS row_version BIGINT NOT NULL DEFAULT 1;
