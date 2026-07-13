-- Reaper decision-log store (ClickHouse).
-- Columns match the agent's DecisionLogEntry NDJSON fields 1:1 so Vector's
-- clickhouse sink (JSONEachRow) maps them directly. See DECISION_LOG_PIPELINE.md.
--
--   clickhouse-client --multiquery < clickhouse-schema.sql
--
-- Volume/cost levers live here: LowCardinality dictionaries, codecs, monthly
-- partitions (instant per-tenant/retention drops), ReplacingMergeTree dedup by
-- decision_id (idempotent at-least-once), and TTL tiering hot NVMe -> cold S3.

CREATE DATABASE IF NOT EXISTS reaper_audit;

CREATE TABLE IF NOT EXISTS reaper_audit.decisions
(
    -- Injected by Vector from the agent->org mapping (agents don't self-assert
    -- tenant). Defaults empty for single-tenant / self-host.
    tenant_id        LowCardinality(String) DEFAULT '',

    timestamp        DateTime64(3)          CODEC(DoubleDelta, ZSTD(1)),
    -- Monotonic per-agent capture sequence: the exact ordering key and the
    -- position in the tamper-evident hash chain (Plan 04). A gap signals a
    -- dropped/deleted record.
    seq              UInt64                 DEFAULT 0 CODEC(DoubleDelta, ZSTD(1)),
    -- Per-writer-boot chain identity (round-2 A2): matches the checkpoints
    -- table's chain_id for the same boot. seq restarts at 0 each boot, so a
    -- verifier reconstructs the hash chain by (chain_id, seq), never table
    -- order. Empty on pre-A2 records / single-boot NDJSON.
    chain_id         String                 DEFAULT '' CODEC(ZSTD(1)),
    decision_id      UUID                   CODEC(ZSTD(1)),   -- high entropy: no delta
    trace_id         String                 CODEC(ZSTD(1)),

    principal        String                 CODEC(ZSTD(3)),
    action           LowCardinality(String),
    resource         String                 CODEC(ZSTD(3)),
    decision         LowCardinality(String),                 -- allow / deny / log

    policy_id        LowCardinality(String),
    policy_name      LowCardinality(String),
    policy_version   LowCardinality(String) DEFAULT '',      -- pins the logic that ran
    matched_rule     LowCardinality(String) DEFAULT '',

    evaluation_time_ns UInt64               CODEC(T64, ZSTD(1)),
    cache_hit        UInt8                  DEFAULT 0,
    agent_id         LowCardinality(String) DEFAULT '',

    -- Request-supplied context (raw JSON object).
    context          String                 CODEC(ZSTD(3)) DEFAULT '{}',

    -- OPTIONAL "explain" snapshot: the resolved principal/resource entity
    -- attributes the evaluation branched on. Populated only in explain/denies-
    -- only/sampled mode (heavier). Empty otherwise. Makes a decision reproducible.
    input_data       String                 CODEC(ZSTD(3)) DEFAULT '',

    -- OPTIONAL replayable-capture tier (Plan 04 step 7): the full resolved
    -- request as a self-contained JSON blob ({"principal","action","resource",
    -- "context"}), enough to re-evaluate the decision under a different
    -- policy/data version (counterfactual replay). May be an encryption
    -- envelope when the tenant encrypts at capture. Empty when the tier is off.
    replay_input     String                 CODEC(ZSTD(3)) DEFAULT '',

    -- Tamper-evident hash chain over the durable stream (Plan 04): entry_hash =
    -- sha256(canonical(record without hashes) || prev_hash). A verifier
    -- recomputes the chain to detect any insertion/deletion/reorder/mutation.
    prev_hash        String                 DEFAULT '' CODEC(ZSTD(1)),
    entry_hash       String                 DEFAULT '' CODEC(ZSTD(1)),

    ingested_at      DateTime               DEFAULT now()    CODEC(DoubleDelta, ZSTD(1))
)
ENGINE = ReplacingMergeTree(ingested_at)                     -- retries collapse on decision_id
PARTITION BY (tenant_id, toYYYYMM(timestamp))                -- monthly; drop = instant purge
ORDER BY (tenant_id, toStartOfHour(timestamp), action, resource, principal, decision_id)
SETTINGS index_granularity = 8192;
-- RETENTION (Plan 04 step 6): deliberately NO static `TTL ... DELETE` here.
-- A static TTL deletes legal-held rows regardless — the exact failure legal
-- holds exist to prevent. Retention is enforced by the management server's
-- application-driven purge (per-tenant window, skips rows matched by active
-- holds): the background sweeper + `POST /orgs/{org}/audit/purge`.
-- Upgrading an existing deployment? Drop the old TTL:
--   ALTER TABLE reaper_audit.decisions REMOVE TTL;
-- PRODUCTION TIERING: hot->cold `TO VOLUME` TTLs remain safe with holds (they
-- move data, never delete it), e.g. with an s3 storage_policy:
--   ALTER TABLE reaper_audit.decisions
--     MODIFY TTL toDateTime(timestamp) + INTERVAL 90 DAY TO VOLUME 's3';

-- Deduped view (collapse at-least-once retries at query time).
CREATE VIEW IF NOT EXISTS reaper_audit.decisions_deduped AS
SELECT * FROM reaper_audit.decisions FINAL;

-- Signed checkpoints (Plan 04, step 3). Each row attests a contiguous run of the
-- decision stream: the covered seq range, the entry count, and the chain head
-- (last_entry_hash), signed with an agent key. A verifier proves completeness of
-- a range (no silently dropped record) from checkpoints alone, and consecutive
-- checkpoints chain via prev_hash == the prior checkpoint's last_entry_hash.
CREATE TABLE IF NOT EXISTS reaper_audit.checkpoints
(
    -- Injected by Vector from the agent->org mapping; empty for single-tenant.
    tenant_id          LowCardinality(String) DEFAULT '',

    -- Per-writer-boot chain identity (fresh UUID each agent/writer start).
    chain_id           String                 CODEC(ZSTD(1)),
    seq_start          UInt64                 CODEC(DoubleDelta, ZSTD(1)),
    seq_end            UInt64                 CODEC(DoubleDelta, ZSTD(1)),
    count              UInt64                 CODEC(T64, ZSTD(1)),

    -- Chain head before this range (prior checkpoint's last_entry_hash) and at
    -- its end: consecutive checkpoints link prev_hash -> last_entry_hash.
    prev_hash          String                 DEFAULT '' CODEC(ZSTD(1)),
    last_entry_hash    String                 CODEC(ZSTD(1)),

    -- Monotonic clock bounds (ns since writer boot): only ever increase, so
    -- wall-clock rollback between checkpoints is detectable.
    monotonic_start_ns UInt64                 CODEC(DoubleDelta, ZSTD(1)),
    monotonic_end_ns   UInt64                 CODEC(DoubleDelta, ZSTD(1)),
    wallclock          DateTime64(3)          CODEC(DoubleDelta, ZSTD(1)),

    -- Signature (empty ⇒ unsigned checkpoint: completeness provable, not authenticity).
    key_id             LowCardinality(String) DEFAULT '',
    algorithm          LowCardinality(String) DEFAULT '',
    signature          String                 DEFAULT '' CODEC(ZSTD(1)),

    ingested_at        DateTime               DEFAULT now()    CODEC(DoubleDelta, ZSTD(1))
)
ENGINE = ReplacingMergeTree(ingested_at)                     -- retries collapse per (chain, seq_start)
PARTITION BY (tenant_id, toYYYYMM(wallclock))
ORDER BY (tenant_id, chain_id, seq_start)
SETTINGS index_granularity = 8192;
-- No static TTL (same rationale as `decisions`): checkpoints attest decision
-- ranges, so they must outlive any held decisions they cover. The application
-- purge deletes a tenant's checkpoints alongside its decisions — and only
-- when the tenant has NO active legal holds.
-- Upgrading? ALTER TABLE reaper_audit.checkpoints REMOVE TTL;

-- Per-minute rollup so dashboards read aggregates, not raw rows.
CREATE TABLE IF NOT EXISTS reaper_audit.decisions_rollup_1m
(
    tenant_id  LowCardinality(String),
    minute     DateTime,
    policy_id  LowCardinality(String),
    decision   LowCardinality(String),
    cnt        AggregateFunction(count),
    p99_eval   AggregateFunction(quantile(0.99), UInt64)
)
ENGINE = AggregatingMergeTree
PARTITION BY toYYYYMM(minute)
ORDER BY (tenant_id, minute, policy_id, decision);

CREATE MATERIALIZED VIEW IF NOT EXISTS reaper_audit.decisions_rollup_1m_mv
TO reaper_audit.decisions_rollup_1m AS
SELECT tenant_id, toStartOfMinute(timestamp) AS minute, policy_id, decision,
       countState() AS cnt, quantileState(0.99)(evaluation_time_ns) AS p99_eval
FROM reaper_audit.decisions
GROUP BY tenant_id, minute, policy_id, decision;
