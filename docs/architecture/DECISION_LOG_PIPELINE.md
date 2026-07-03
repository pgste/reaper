# Decision-Log Pipeline — Design

**Goal:** capture every authorization decision at Reaper's volume (100k+ decisions/sec/agent,
many agents) and land it in a central, queryable, compliance-grade store — **without the
logging path ever touching the sub-microsecond evaluation hot path**.

This doc is the plan. It's grounded in current (2026) practice (ClickHouse, Vector,
OpenTelemetry Logs) and in what Reaper already has (`DecisionBuffer`, NDJSON export, SSE).

## The one rule

Evaluation and log egress live in **different failure domains**. The eval path does exactly
one thing for logging: push a compact record into a bounded, lock-free, in-process ring and
return. It never blocks on a lock, socket, flush, or fsync. If the ring is full it **drops and
counts** (best-effort) or spills to disk (compliance) — it never back-pressures evaluation.

Every mature authz system converges on this shape (OPA `decision_logs`, Cerbos, Styra DAS,
Aserto/Topaz): structured event per decision → bounded local buffer flushed by size **or**
timer → batch+compress → ship to cold queryable storage → PDP hot path never blocked. We adopt
it and harden the two places the reference designs are weak: **lock-free sharded capture** and
**durable store-and-forward**.

## Architecture (four isolated layers)

```
 EVAL HOT PATH                         AGENT COLD PATH                 SIDECAR              CENTRAL
 (per-core, ~tens of ns)               (background thread)             (Vector)            (control plane)
 ┌───────────────┐   interned,   ┌──────────────────────┐   NDJSON   ┌───────────┐  CH   ┌──────────────┐
 │ record decision│ fixed-size   │ drain shards          │  file /   │ disk WAL  ├──────▶│ ClickHouse   │◀─ query API
 │  push to shard ├─────Copy────▶│ resolve interned ids  ├─socket/──▶│ batch     │       │ (hot, 90d)   │   + UI
 │  (drop+count)  │  no strings  │ format NDJSON          │  stdout   │ e2e acks  ├──────▶│ S3 WORM      │
 └───────────────┘  no serde     │ sink (file/stdout)    │           │ dedup key │  S3   │ (cold, 6yr)  │
   per-core sharded ArrayQueue    └──────────────────────┘           └───────────┘       └──────────────┘
   deny-priority sampling                 + bounded SSE "recent" tail for live debugging
```

### Layer 1 — Capture (agent, hot path): make it ~free — ✅ IMPLEMENTED (sharded ring)

The original `DecisionBuffer` was one `parking_lot::RwLock<VecDeque>` — every request thread
funneled through a single lock (collapses ~5x under 4-thread contention) and the writer thread
got a full deep clone of each entry. Implemented now:

- **Per-thread sharded ring** (`decision_buffer.rs`): the retention ring is split into N
  `CachePadded<RwLock<VecDeque>>` shards; each request thread maps to a stable shard, so
  concurrent producers take disjoint, uncontended locks. The push stays inline (same-thread
  alloc/free — no allocator arena ping-pong), entries are queryable immediately, and a global
  sequence number preserves exact cross-shard ordering (queries k-merge by seq; queries are
  rare, the eval path is what matters). Shard count: `REAPER_DECISION_LOG_SHARDS`
  (0 = auto-detect cores, clamped 1..=64).
- **Design note:** a lock-free `ArrayQueue` + background drain-thread variant was built and
  benchmarked first — 4x more concurrent throughput than the single ring, but the drain thread
  freed producer allocations cross-thread, contending the malloc arena against *every*
  allocation on the request path (+3.4µs/request single-thread in the handler bench). Sharding
  the ring keeps the multi-core win without that tax. Numbers in the module docs.
- **Writer hand-off is `Arc`-shared, not deep-cloned** — file/stdout serialization + I/O stay
  on the dedicated writer thread. ✅
- **Deny-priority sampling, decided inline (single-digit ns)** via `should_log()` — always keep
  denies; sample allows with a thread-local xorshift PRNG *before* the entry is built. ✅
- **Fast path captures too** (was unaudited). ✅
- **Never silently drop.** `dropped`, `sampled_out`, `writer_dropped` exported as Prometheus
  gauges. ✅
- *(remaining option, only if profiling demands)* **Compact `Copy` record**: push interned
  `u32` ids instead of a built entry. The shard push already removed the contention and the
  deep clone; the entry build (a few short `String`s) only happens for decisions that pass
  sampling, so this is a marginal follow-up, not a gap.

### Layer 2 — Local emit (agent, cold path): pluggable sink, minimal

A dedicated shipper thread (off the eval cores) drains the shards, resolves interned ids,
formats NDJSON, and writes to a **pluggable local sink**. The agent does **not** speak
ClickHouse/Kafka/S3 — it hands NDJSON to the local sink and lets the sidecar own the pipeline.
Sinks:

- **`stdout`** (default) — one JSON object per line; container log agents / Vector scrape it.
  Near-zero cost, instantly pluggable. (OPA `console`, Cerbos `file` do exactly this.)
- **`file`** — rotated NDJSON that Vector tails. **This file is the durable WAL**: a crash
  between "decided" and "shipped" loses only the in-ring tail, not the file.
- **`otlp`** *(optional)* — OpenTelemetry Logs over OTLP for shops standardized on OTel and
  wanting trace correlation (the OTel Logs data model is GA; OTLP→Collector→ClickHouse is
  production-viable). Trace/span ids already flow through Reaper.
- Keep the existing **bounded SSE "recent decisions" tail** (sampled) for live debugging —
  separate from the bulk path.

Batch/compress (size **or** timer trigger, adaptive backoff on pushback — OPA's model) happen
here, off the eval thread. Measure any byte limits **post-compression**.

### Layer 3 — Ship (Vector sidecar): durability we don't rebuild

**Default shipper: Vector** (Rust, Datadog). It tails the NDJSON file/socket and owns what we'd
otherwise reinvent: `buffer.type = disk` WAL that survives restarts, **end-to-end
acknowledgements** (a record isn't released until the sink confirms), at-least-once delivery,
client-side batching sized for ClickHouse, and native ClickHouse/S3/Kafka/OpenSearch sinks.
This is the "push the log off to the side" boundary: the agent captures; Vector ships.

We ship a reference `vector.toml` (below) and a ClickHouse schema. Alternatives: **OTel
Collector** if all-in on OTLP; **Fluent Bit** for RAM-starved edge nodes feeding an aggregation
tier.

### Layer 4 — Store + query (control plane): ClickHouse, not Postgres

The firehose must **not** flow through the control plane's transactional Postgres. Route it to
a columnar store:

- **Default: ClickHouse (OSS engine).** Best ingest/cost/query fit for flat append-only audit
  logs, and — critically for a "spin up cloud-hosted **or** self-hosted" product — it runs
  **identically** self-hosted, BYOC (Altinity), or managed (ClickHouse Cloud). At fleet scale
  put **Redpanda/Kafka in front** for replay + fan-out; at single-store scale a disk-buffered
  Vector inserts direct.
- **Compliance archive: S3 with Object Lock (WORM)**, written in parallel — complete and
  **unsampled**, retention = strictest applicable (HIPAA 6yr dominates). Parquet/Iceberg for
  cheap forensic scans (DuckDB/Athena/Trino).
- **Management plane** gets a decision **query/analytics API + UI** over ClickHouse (filter by
  principal/resource/decision/policy/time, aggregates from rollup MVs, export), tenant-scoped —
  NOT an ingest bottleneck in the firehose.

Deviate from ClickHouse only if: full-text search on resource/principal dominates (**Apache
Doris**), or regulatory WORM is the primary axis (make the **S3/Iceberg Object-Lock** tier
authoritative, ClickHouse the hot cache).

## Delivery-semantics tiers (configurable, like Cerbos `produceSync`)

| | Best-effort (observability) | At-least-once (compliance) |
|---|---|---|
| Ring on full | `drop_newest` + counter | spill to disk WAL / backpressure the shipper (never eval) |
| Sampling | sample allows under load | **none** — audit must be complete |
| Destination | ClickHouse (hot dashboards) | ClickHouse **+ S3 WORM** |
| Dedup | optional | `ReplacingMergeTree ORDER BY (decision_id)` |

**Exactly-once delivery is a myth** (any ack can be lost). We do **at-least-once + idempotent
dedup by `decision_id`** (which every decision already carries) → `ReplacingMergeTree` collapses
retry duplicates on merge. That yields exactly-once *results* without the coordination cost that
would poison a sub-µs path.

## ClickHouse schema (sketch)

```sql
CREATE TABLE decisions (
    tenant_id    LowCardinality(String),
    timestamp    DateTime64(3) CODEC(DoubleDelta, ZSTD(1)),
    decision_id  UUID          CODEC(ZSTD(1)),        -- high entropy: no delta
    principal    String        CODEC(ZSTD(3)),
    action       LowCardinality(String),
    resource     String        CODEC(ZSTD(3)),
    decision     LowCardinality(String),             -- allow/deny
    policy_id    LowCardinality(String),
    policy_name  LowCardinality(String),
    eval_time_ns UInt32        CODEC(T64, ZSTD(1)),
    agent_id     LowCardinality(String),
    trace_id     String        CODEC(ZSTD(1))
)
ENGINE = ReplacingMergeTree                          -- dedup retries by decision_id
PARTITION BY (tenant_id, toYYYYMM(timestamp))        -- monthly; drop partition = instant purge
ORDER BY (tenant_id, toStartOfHour(timestamp), action, resource, principal, decision_id)
TTL toDateTime(timestamp) + INTERVAL 90 DAY TO VOLUME 's3',   -- hot NVMe -> cold S3
    toDateTime(timestamp) + INTERVAL 6 YEAR DELETE            -- to WORM archive before delete
SETTINGS storage_policy = 'hot_to_s3';

-- Dashboard rollup kept off the raw table
CREATE MATERIALIZED VIEW decisions_rollup_1m_mv TO decisions_rollup_1m AS
SELECT tenant_id, toStartOfMinute(timestamp) AS minute, policy_id, decision,
       countState() AS cnt, quantileState(0.99)(eval_time_ns) AS p99
FROM decisions GROUP BY tenant_id, minute, policy_id, decision;
```

## Vector reference (agent NDJSON → disk-buffered → ClickHouse + S3 WORM)

```toml
[sources.decisions]
type = "file"                       # tail the agent's rotated NDJSON WAL
include = ["/var/log/reaper/decisions.ndjson"]

[sinks.clickhouse]
type = "clickhouse"
inputs = ["decisions"]
endpoint = "http://clickhouse:8123"
database = "audit"
table = "decisions"
batch.max_bytes = 10_000_000        # few, large inserts (CH prefers this)
batch.timeout_secs = 1
buffer.type = "disk"                # WAL: a CH outage never blocks the agent
buffer.max_size = 5_000_000_000
buffer.when_full = "block"          # at-least-once; "drop_newest" for best-effort
acknowledgements.enabled = true     # release only after CH confirms

[sinks.s3_worm]                      # parallel, complete, immutable compliance copy
type = "aws_s3"
inputs = ["decisions"]
bucket = "reaper-audit-worm"         # bucket has Object Lock (COMPLIANCE) enabled
key_prefix = "decisions/date=%Y-%m-%d/"
compression = "zstd"
encoding.codec = "ndjson"
buffer.type = "disk"
```

## Reaper-specific decisions

- **Agent stays lean.** It captures + emits NDJSON; Vector owns shipping/batching/durability/
  dedup. This is the lowest-risk high-volume path and keeps the agent out of the
  firehose-management business. *Optional* embedded shipper (native ClickHouse `Inserter` via
  the official `clickhouse` crate, RowBinary, LZ4) is a Tier-2 mode for sidecar-less
  high-density deployments — not the default.
- **Product fit.** ClickHouse OSS + Vector bundle into the managed stack (Helm), and
  self-hosters point Vector at their own ClickHouse/S3 — the same artifacts either way, matching
  "spin up cloud-hosted or self-hosted reapers."
- **Control plane** integrates with (queries) ClickHouse; it does not ingest the firehose into
  Postgres. `CLAUDE.md` currently advertises an agent `/api/v1/decisions/stream` SSE route —
  reconcile docs with the real endpoints as part of this work.

## Phased implementation

1. **Harden capture (pure win, store-agnostic):** ✅ done — per-thread sharded ring (disjoint
   uncontended locks, inline push, exact seq ordering) + deny-priority sampling + fast-path
   logging + `dropped`/`sampled` metrics. (Compact interned `Copy` record remains an optional
   micro-follow-up; see Layer 1.)
2. **Pluggable local sink:** `stdout` + rotated-`file` NDJSON WAL behind a `DecisionSink` trait
   (mirrors the pluggable-evaluator pattern); keep SSE tail.
3. **Reference pipeline:** commit `vector.toml` + ClickHouse schema/migrations + Helm wiring
   (Vector sidecar + optional bundled ClickHouse) so managed and self-hosted get the same setup.
4. **Control-plane query API + UI** over ClickHouse (tenant-scoped filters, rollups, export).
5. **Compliance tier:** S3 Object-Lock archive branch + retention config; document SOC2/HIPAA/PCI
   posture (immutability, retention, no sampling of the audit copy).
6. *(scale)* **Optional Redpanda/Kafka** front + optional embedded native-ClickHouse shipper.

## Sources

ClickHouse async inserts / compression / multi-tenancy / rollup MVs / observability cost
playbook; Vector buffering model + end-to-end acknowledgements + guarantees; OpenTelemetry Logs
data model (GA) + collector-contrib ClickHouse exporter; OPA `decision_logs`, Styra DAS (SLP
`/scratch` store-and-forward), Cerbos audit backends, Aserto/Topaz exactly-once batch shipping;
crossbeam `ArrayQueue`, `thingbuf`, `disruptor-rs`; the `clickhouse` Rust crate `Inserter`,
`object_store`, `rdkafka`. (URLs captured in the research task outputs for this change.)
