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

## Protecting sensitive data (masking / pseudonymization / encryption)

Decision logs carry identity and — with the explain tier — resolved entity
attributes. Three independent, composable protection layers are applied **once
at capture**, so the query API, file/stdout sinks, exports, and the central
pipeline only ever see protected values:

| Env | Effect |
|-----|--------|
| `REAPER_DECISION_LOG_HASH_PRINCIPAL=true` + `REAPER_DECISION_LOG_HASH_SALT=<secret>` | `principal` becomes `sha256:<hex>` (HMAC-SHA-256): stable → joinable for investigations, irreversible, dictionary-attack-proof without the salt |
| `REAPER_DECISION_LOG_CONTEXT_ALLOWLIST=request_id,ip` | drop all context keys not listed |
| `REAPER_DECISION_LOG_MASK_KEYS=ssn,password,token` | replace those values with `"***"` in context **and** in explain `input_data` (case-insensitive) |
| `REAPER_DECISION_LOG_ENCRYPT_INPUT_DATA=true` + `REAPER_DECISION_LOG_ENCRYPTION_KEY=<64-hex>` | seal the explain snapshot with AES-256-GCM; the log store sees only ciphertext, the key holder (control plane, per tenant) decrypts |

Properties worth knowing:

- **Fail-closed.** Hashing without a salt, or encryption without a valid
  32-byte key, is a startup error — the agent never silently logs raw data.
  If encryption fails at runtime the entry is discarded, not logged plaintext.
- **Masking happens before encryption**, so even the key holder never sees
  masked fields.
- **Secrets never echo.** `hash_salt`/`encryption_key` are excluded from
  serialization, so `/api/v1/decisions/stats` config output can't leak them.

Tooling:

```bash
reaper-cli decisions keygen        # generate salt + AES key as ready-to-paste env vars
reaper-cli decisions decrypt --key <hex> '<input_data envelope or full entry JSON>'
# or pipe an NDJSON line:  cat decisions.ndjson | tail -1 | reaper-cli decisions decrypt --key <hex> -
```

In ClickHouse, encrypted `input_data` is just an opaque JSON column — the
control plane decrypts per tenant at query time.

## Deploying the pipeline

**Kubernetes (Helm)** — one switch deploys capture + shipper + store and wires
the management query API:

```bash
helm install reaper ./deploy/helm/reaper -f profiles/managed-stack.yaml \
    --set decisionLogs.clickhouse.password=$(openssl rand -hex 24)
```

Every agent pod gets a Vector sidecar tailing the shared decision file with an
on-disk buffer (a ClickHouse outage never blocks the agent); the bundled
single-node ClickHouse initializes the schema on first start (90-day retention
by default via `decisionLogs.clickhouse.retentionDays`). Bring your own store
with `decisionLogs.clickhouse.enabled=false` + `url` + `existingSecret`.

**Docker Compose** — the main compose file has an `audit` profile:

```bash
DECISION_LOG_ENABLED=true \
REAPER_CLICKHOUSE_URL=http://clickhouse:8123 \
docker compose --profile engine --profile audit up -d
```

The agent writes to the shared `agent-logs` volume, Vector
(`deploy/decision-logs/vector-file.toml`) tails it into ClickHouse, and the
management service (if running) serves the query API from the same store.

## Querying from the control plane

`reaper-management` serves the full history (cross-agent, tenant-scoped)
straight from ClickHouse once pointed at it:

```bash
REAPER_CLICKHOUSE_URL=http://clickhouse:8123 \
REAPER_CLICKHOUSE_DATABASE=reaper_audit \
REAPER_CLICKHOUSE_USER=reaper REAPER_CLICKHOUSE_PASSWORD=... \
./reaper-management
```

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/orgs/{org}/decisions` | filtered history (`principal`, `action`, `resource`, `decision`, `policy_name`, `agent_id`, `from`, `to`, `limit`, `offset`) |
| `GET /api/v1/orgs/{org}/decisions/stats` | totals, allow/deny counts, active agents, avg eval time, top denied policies |
| `GET /api/v1/orgs/{org}/decisions/{decision_id}` | one decision incl. `input_data` (decrypt with the tenant key) |

Queries are pinned to the caller's organization (the `tenant_id` Vector
injects) and use ClickHouse server-side parameter binding — filters never
touch the SQL text. Requires `agent:read` or `org:admin` scope. For
single-tenant self-host stores (empty `tenant_id`) set
`REAPER_CLICKHOUSE_TENANT_FILTER=false`.

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
