-- Plan 10 follow-up: an optional external ITSM change-record reference (e.g.
-- a ServiceNow CHG number) attached to an env→env promotion. Environments can
-- require it (and require live validation) via their approval policy; by
-- default it is optional and stored opaquely when supplied.
ALTER TABLE change_requests ADD COLUMN external_change_ref TEXT;
