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
TTL toDateTime(timestamp) + INTERVAL 90 DAY DELETE           -- retention (archive to WORM first for compliance)
SETTINGS index_granularity = 8192;
-- PRODUCTION TIERING: with a storage_policy that has an s3 disk, replace the
-- TTL above with hot->cold tiering + long retention, e.g.
--   TTL toDateTime(timestamp) + INTERVAL 90 DAY TO VOLUME 's3',
--       toDateTime(timestamp) + INTERVAL 6 YEAR DELETE
-- (the plain DELETE default keeps this schema runnable on single-disk setups).

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
TTL toDateTime(wallclock) + INTERVAL 90 DAY DELETE           -- keep >= the decisions retention
SETTINGS index_granularity = 8192;

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
