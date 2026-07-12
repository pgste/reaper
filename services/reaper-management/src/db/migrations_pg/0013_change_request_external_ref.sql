-- Plan 10 follow-up: optional external ITSM change-record reference (e.g. a
-- ServiceNow CHG number) on env→env promotions.
ALTER TABLE change_requests ADD COLUMN IF NOT EXISTS external_change_ref TEXT;
