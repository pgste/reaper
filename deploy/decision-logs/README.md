# Decision-log pipeline — reference deployment

Gets Reaper decision logs from agents into a central, queryable, compliance-grade
store, **without touching the eval hot path**. Full design:
[`docs/architecture/DECISION_LOG_PIPELINE.md`](../../docs/architecture/DECISION_LOG_PIPELINE.md).

```
agent (NDJSON: stdout or file)  ──▶  Vector (disk WAL, acks, batch)  ──▶  ClickHouse (hot, query)
                                                       └──────────────────▶  S3/MinIO WORM (compliance)
```

The agent only **captures** and emits NDJSON; **Vector owns shipping** (durability,
retry, dedup, batching). ClickHouse is the store; the control plane queries it —
the firehose never goes through the control plane's Postgres.

## Everything here is open source / self-hostable

ClickHouse (Apache-2.0), Vector (MPL), MinIO/S3, optional Redpanda — no per-GB
ingest tax, no lock-in. The same artifacts run in the managed stack and
self-hosted. Cost is just the bytes you choose to keep; control it with retention
(90d hot → cheap cold), **deny-priority sampling**, and rollups.

## Quick start (local)

```bash
docker compose -f deploy/decision-logs/docker-compose.yml up -d   # ClickHouse + Vector

REAPER_DECISION_LOG_ENABLED=true \
REAPER_DECISION_LOG_STDOUT=true \
REAPER_DECISION_LOG_SAMPLE_ALLOW_RATE=0.1 \
./reaper-agent            # emit NDJSON to stdout; a collector/Vector scrapes it

# query
docker compose -f deploy/decision-logs/docker-compose.yml exec clickhouse \
  clickhouse-client -q "SELECT decision, count() FROM reaper_audit.decisions GROUP BY decision"
```

## Agent config (what to emit)

| Env | Meaning |
|-----|---------|
| `REAPER_DECISION_LOG_ENABLED=true` | turn logging on |
| `REAPER_DECISION_LOG_STDOUT=true` | emit NDJSON to stdout (container-native) |
| `REAPER_DECISION_LOG_FILE=/var/log/reaper/decisions.ndjson` | or/also a file for Vector to tail (durable WAL) |
| `REAPER_DECISION_LOG_SAMPLE_ALLOW_RATE=0.01` | keep 1% of allows + **100% of denies** (deny-priority sampling) — the cheapest volume lever |
| `REAPER_DECISION_LOG_ALLOWS=false` | drop allows entirely (denies only) |
| `REAPER_DECISION_LOG_CONTEXT=false` | strip request context (privacy) |
| `REAPER_DECISION_LOG_INPUT_DATA=true` | "explain" tier: snapshot resolved principal/resource attributes into `input_data` |
| `REAPER_DECISION_LOG_INPUT_DATA_DENIES_ONLY=false` | capture the snapshot on allows too (default: denies only) |
| `REAPER_DECISION_LOG_SHARDS=0` | lock-free capture shards; 0 = auto (one per detected core, max 64) |

Sampling and disable decisions are made **before** the log entry is built, so
they cost nothing on the eval path. The retention ring is sharded per thread
(each request thread pushes to its own shard under a disjoint, uncontended
lock), so concurrent workers never queue on a shared lock, and JSON/file I/O
happen on a dedicated writer thread. `dropped`/`sampled_out`/`writer_dropped`
counters are exposed on `/api/v1/decisions/stats` and as Prometheus metrics so
audit gaps are never silent.

## What's in each record

Every decision logs: `timestamp`, `decision_id`, `trace_id`, `principal`,
`action`, `resource`, request `context`, `decision`, `policy_id`/`policy_name`/
`policy_version` (pins the logic that ran), `evaluation_time_ns`, `cache_hit`,
`agent_id`, `matched_rule`.

**Reproducibility note:** the resolved entity attributes the evaluation branched
on (e.g. `user.clearance_level`, `resource.department`) are captured only in the
optional **explain** tier (heavier; ideal for denies/compliance) into the
`input_data` column — see the design doc. `policy_version` is always captured so
you know exactly which policy logic ran.

## Delivery tiers

- **Best-effort** (dashboards): sample allows, `buffer.when_full = "drop_newest"`,
  ClickHouse only.
- **At-least-once / compliance**: all events, Vector disk buffer + acks,
  ClickHouse **+ S3/MinIO WORM** (Object Lock, retention = strictest applicable —
  HIPAA's 6y dominates). Dedup by `decision_id` via `ReplacingMergeTree` (exactly-
  once *results* without exactly-once delivery, which is a myth).

## Scale

At fleet scale put **Redpanda/Kafka** in front (replay + fan-out) and run
ClickHouse self-hosted or BYOC — managed per-GB ingest fees dominate the bill at
10M/s. A single agent + single-node ClickHouse + MinIO handles typical volumes on
existing hardware.

## Files

- `clickhouse-schema.sql` — the `decisions` table (ReplacingMergeTree dedup,
  codecs, monthly partitions, TTL-to-S3) + per-minute rollup MV.
- `vector.toml` — agent NDJSON → ClickHouse (+ commented S3 WORM branch).
- `docker-compose.yml` — runnable ClickHouse + Vector (+ optional MinIO).
