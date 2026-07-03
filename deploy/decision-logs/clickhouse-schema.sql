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

    ingested_at      DateTime               DEFAULT now()    CODEC(DoubleDelta, ZSTD(1))
)
ENGINE = ReplacingMergeTree(ingested_at)                     -- retries collapse on decision_id
PARTITION BY (tenant_id, toYYYYMM(timestamp))                -- monthly; drop = instant purge
ORDER BY (tenant_id, toStartOfHour(timestamp), action, resource, principal, decision_id)
TTL toDateTime(timestamp) + INTERVAL 90 DAY TO VOLUME 's3',  -- hot NVMe -> cold object store
    toDateTime(timestamp) + INTERVAL 6 YEAR DELETE           -- HIPAA-dominant; archive to WORM first
SETTINGS index_granularity = 8192;
-- NOTE: TTL ... TO VOLUME 's3' requires a storage_policy with an s3 disk; drop
-- that clause for a single-disk dev setup.

-- Deduped view (collapse at-least-once retries at query time).
CREATE VIEW IF NOT EXISTS reaper_audit.decisions_deduped AS
SELECT * FROM reaper_audit.decisions FINAL;

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
