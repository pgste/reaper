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
- **Data protection at capture** (`decision_privacy.rs`): HMAC-SHA-256 principal
  pseudonymization, context allowlist + key masking (context and explain `input_data`), and
  AES-256-GCM encryption of the explain snapshot — applied once in `log()` so every
  downstream view (ring, sinks, exports, central store) sees only protected data. Fail-closed
  on missing secrets; secrets are excluded from config serialization; `reaper-cli decisions
  keygen|decrypt` for operators. ✅

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
  cheap forensic scans (DuckDB/Athena/Trino). This is the **immutable audit anchor** (SEC
  R2-3): a ClickHouse insider can rewrite queryable rows, but not Object-Lock objects. It
  receives **both** decisions and checkpoints, and is the byte-identical source
  `reaper-cli audit verify --file` runs **ByteExact** against for the authoritative crypto
  proof (recompute every `entry_hash` + verify checkpoint signatures) — vs the store endpoint's
  sound-but-weaker **Linkage** pass over reprojected columns. **Cross-boot genesis linkage:**
  each writer boot's first signed checkpoint carries `prev_chain_id`/`prev_chain_head` (from the
  agent's continuity file, `REAPER_DECISION_LOG_CONTINUITY_FILE`) pinning the prior boot's
  terminal head, so deleting a whole boot from the archive surfaces as `missing_prior_boot` /
  `boot_linkage_broken`, not silence.
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

## Tamper-evidence: hash chain + signed checkpoints (Plan 04)

The durable stream is **tamper-evident**, so a regulator can prove the audit log wasn't edited
after the fact — even by an insider with write access to the store.

- **Per-record hash chain.** Each record carries `seq` (a monotonic per-agent counter), `prev_hash`,
  and `entry_hash = sha256(canonical(record without hashes) || prev_hash)`. The chain is stamped on
  the single-threaded **writer thread** (off the eval hot path) over the durable write order. Any
  insertion, deletion, reordering, or field mutation breaks re-verification and names the offending
  `seq`.
- **Signed checkpoints.** Every `N` records or `T` seconds the writer emits a `Checkpoint` record
  (`record_type = "checkpoint"`) over the covered `seq` range: `{chain_id, seq_start, seq_end, count,
  prev_hash, last_entry_hash, monotonic_start_ns, monotonic_end_ns, wallclock, key_id, algorithm,
  signature}`, signed with an agent signing key (reusing the bundle-signing Ed25519 / P-256
  primitive). Checkpoints let a verifier prove **completeness** of a range (no silently dropped
  record) from the checkpoints alone, and consecutive checkpoints chain end-to-end
  (`prev_hash == the prior checkpoint's last_entry_hash`). The monotonic bounds make wall-clock
  rollback between checkpoints detectable.

Checkpoints ship as their own NDJSON line; Vector routes `record_type == "checkpoint"` to
`reaper_audit.checkpoints` and decision records to `reaper_audit.decisions`.

**Agent config (checkpoints are opt-in; off unless a trigger is set):**

| Env var | Meaning |
|---|---|
| `REAPER_DECISION_LOG_CHECKPOINT_EVERY` | Emit a checkpoint every N durable records (0 = off by count) |
| `REAPER_DECISION_LOG_CHECKPOINT_INTERVAL_SECS` | Emit at least every T seconds when records are pending (0 = off by time) |
| `REAPER_DECISION_LOG_CHECKPOINT_SIGNING_KEY` | Hex private key (Ed25519 seed / P-256 scalar). Absent ⇒ **unsigned** checkpoints + a loud startup warning; **invalid ⇒ fail closed** at startup |
| `REAPER_DECISION_LOG_CHECKPOINT_KEY_ID` | Key id stamped into checkpoints (rotation / verifier pinning) |
| `REAPER_DECISION_LOG_CHECKPOINT_ALGORITHM` | `ed25519-sha256` (default) or `ecdsa-p256-sha256` |

A verifier pins the matching **public** key (same key-distribution story as bundle signing) and
runs `policy_engine::decision_log::verify_checkpoint(&checkpoint, &covered_entries, &vk,
Some(key_id))`, which checks the signature, the covered records' chain, the count, the range
coverage, and that they hash to `last_entry_hash`.

## Mandatory-audit (fail-closed) mode + drop alarms (Plan 04)

By default a dropped audit record is a best-effort loss. **Mandatory-audit mode**
(`REAPER_DECISION_LOG_MODE=mandatory`) makes the audit trail a hard requirement — for regulated
deployments where an un-audited decision must never be served:

- **Startup validation (fail closed).** Mandatory mode requires complete capture (no allow
  sampling, `log_allows` and `log_denies` both on), an **fsync-able file sink**
  (`REAPER_DECISION_LOG_FILE`), and **signed checkpoints** (a checkpoint trigger + a signing key).
  A file sink is required specifically because durable-before-serve fsyncs each entry before the
  allow is served, and **stdout cannot be fsynced** — a stdout-only mandatory config would degrade
  to a per-request `503`, so it is rejected at boot instead. Container deployments that ship logs
  via `stdout`→Vector must mount a file path (e.g. an `emptyDir`) and tail it. Any conflicting
  setting aborts startup rather than being silently overridden — so an operator's wrong mental model
  surfaces immediately. A buffer-creation failure in this mode is fatal (the agent refuses to run
  un-audited).
- **Durable-before-serve.** In mandatory mode the eval handler calls `log_durable(entry).await`,
  which writes + fsyncs the record to the file sink and only **then** acks — so a served `allow` is
  always already on disk. If persistence can't be guaranteed the handler returns **`503`** before
  the decision is served (never a serve-then-lose window). Best-effort mode is unchanged: a
  fire-and-forget `log()` with zero added latency.
- **Runtime fail-closed.** If the durable sink can't accept a record (writer queue saturated or a
  sink write error), the agent does **not** silently drop it. Per `REAPER_DECISION_LOG_ON_AUDIT_UNAVAILABLE`:
  - `fail_closed` (default): latch audit-compromised → `/ready` flips `not_ready`
    (`reason: audit_sink_unavailable`) and evaluation returns **`503`**, so callers and load
    balancers drain the instance instead of trusting un-audited decisions.
  - `block`: backpressure the writer hand-off so a record is never dropped (trades tail latency
    under sink pressure for zero loss).

**Drop counters & alarms (always on).** Two distinct losses are counted, alarmed once via
`tracing::error!`, and exported for alerting:

| Metric | Meaning |
|---|---|
| `reaper_decision_log_writer_dropped_total` | **Durable** loss (writer queue full / sink write error). Page on any increase; drives mandatory fail-closed |
| `reaper_decision_log_dropped_entries_total` | In-memory **query-ring** eviction (`buffer_capacity` too small) — not a durable loss |
| `reaper_decision_log_audit_compromised` | `1` when mandatory mode has latched fail-closed, else `0` — page on `1` |

Both counts (plus `audit_required` / `audit_compromised`) are also in the `/api/v1/decisions/stats`
payload.

## Retention & legal holds (Plan 04)

Retention is enforced by an **application-driven purge** on the management server, not by a static
ClickHouse `TTL ... DELETE` — a static TTL deletes legal-held rows regardless, the exact failure
legal holds exist to prevent. (Upgrading an existing store? `ALTER TABLE reaper_audit.decisions
REMOVE TTL;` — hot→cold `TO VOLUME` tiering TTLs remain safe: they move data, never delete it.)

**Governance state lives in the management DB** (transactional, tenant-scoped, audited); the purge
it governs executes against ClickHouse with parameter-bound `ALTER TABLE ... DELETE` mutations.

| Method | Path | Purpose |
|---|---|---|
| `GET`/`PUT` | `/orgs/{org}/audit/retention` | Read / set the tenant window `{days}` (1–3650) |
| `POST`/`GET` | `/orgs/{org}/audit/legal-holds` | Place a hold `{reason, filter?}` / list (active + released) |
| `GET`/`DELETE` | `/orgs/{org}/audit/legal-holds/{id}` | Inspect / **release** (the record survives, released) |
| `POST` | `/orgs/{org}/audit/purge` | Run the org's retention purge now |

All routes are **admin-only** (`org:admin`), tenant-isolated, and every mutation writes an audit
record (`audit.retention_update`, `audit.legal_hold_create`, `audit.legal_hold_release`,
`audit.purge`).

- A hold's `filter` selects rows on the decision dimensions (principal / action / resource /
  decision / policy_name / agent_id / time range). An **empty filter is a blanket hold**: it
  protects everything and suspends the org's purge entirely while active.
- Held rows are excluded from the purge with a `NOT (...)` clause per active hold; releasing a hold
  makes its rows purgeable on the next sweep. Releases are non-idempotent (a double release
  surfaces as 404, never masks a race).
- **Checkpoints** attest decision ranges, so a tenant's checkpoints are purged only when it has
  *no* active holds — while anything is held, the whole attestation chain is kept.
- The **background sweeper** (`REAPER_AUDIT_PURGE_INTERVAL_SECS`, default 6h, `0` = off) applies
  each org's explicit window, falling back to `REAPER_AUDIT_DEFAULT_RETENTION_DAYS` (default 90 —
  the window the old static TTL enforced; `0` = no default purging). Single-tenant stores
  (`REAPER_CLICKHOUSE_TENANT_FILTER=false`) run one global pass under the default window honoring
  every org's holds — per-org windows would race each other on unscoped deletes.

## Replayable capture tier (Plan 04)

Opt-in tier that stores the **full resolved request** per captured decision —
`replay_input = {"principal","action","resource","context"}` — the raw material for
**counterfactual replay** ("what would policy vX have decided on last month's traffic?").
Distinct from the display-oriented `context` (which may be allowlisted/dropped) and the explain
tier's `input_data` (resolved *entity attributes*, not the request). Combined with the
`data_version`/`data_checksum` provenance every row already carries, a replayable row pins exactly
what ran: request + policy version + data snapshot.

| Env var | Meaning |
|---|---|
| `REAPER_DECISION_LOG_REPLAY_INPUT` | Enable the tier (default off — zero hot-path cost when off) |
| `REAPER_DECISION_LOG_REPLAY_INPUT_DENIES_ONLY` | Capture denies only (default **false**, unlike the explain tier: flips happen in both directions) |

**Privacy is identical across sinks.** `mask_keys`, the context allowlist, and principal
pseudonymization apply to the replay blob exactly as everywhere else — a masked key is masked in
every view. That means masking/hashing *degrade replay fidelity* by design (the protected value is
what replays). Tenants that need **both** privacy and full-fidelity replay use `encrypt_input_data`:
the replay blob is sealed AES-256-GCM at capture and opened by the tenant key holder at replay
time. The blob is also inside the hash-chained record, so it is tamper-evident like every other
field.

## Counterfactual replay (Plan 04)

`POST /orgs/{org}/replay` answers **"what would bundle X have decided on real historical
traffic?"** — the safety net for policy changes: replay last month's decisions under the candidate
bundle before promoting it, and see exactly what flips.

```json
{ "bundle_id": "…", "from": "…", "to": "…", "filter": {…},
  "namespace": "prod", "data_version": 3, "max_rows": 10000 }
```

Returns `202 {job_id}`; poll `GET /orgs/{org}/replay/{job_id}` for progress and the diff:
scanned / replayed / skipped counts, `allow_to_deny` / `deny_to_allow` flip totals, and sample
flipped records with old and new rule attribution.

**Fidelity guarantees** (what makes the counterfactual trustworthy):
- **Decision semantics** come from `PolicyEngine::evaluate_set` — the *same function* the agent's
  serving path calls (default deny, first allow, deny overrides, errors deny). Replay and
  production cannot combine policies differently.
- **Requests** come from the replayable capture tier's `replay_input` blobs; encrypted blobs are
  opened with the tenant key passed per-job (never persisted). Rows captured without the tier are
  counted as `skipped_not_replayable`; a range with none fails with a clear "enable the tier" error.
- **Data** is pinned by `data_version` — the exact published snapshot document agents loaded (the
  same provenance every decision row records). A principal absent from the snapshot fails closed
  (deny + error), exactly as production would.
- **Policies** load from the bundle's compiled artifact through the same construction the agent's
  bundle apply uses.

Admin-only, tenant-isolated, audited (`audit.replay`, recording the range/bundle — never the key).
Jobs are ephemeral in-memory analyses (re-run after a control-plane restart). Scan cap 10k rows by
default, 100k max per job.

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

[sinks.s3_worm]                      # parallel, complete, immutable audit anchor (SEC R2-3)
type = "aws_s3"
inputs = ["decisions_fmt", "checkpoints_fmt"]   # BOTH decisions and checkpoints
bucket = "reaper-audit-worm"         # bucket has Object Lock (COMPLIANCE) + default retention
key_prefix = "decisions/date=%Y-%m-%d/hour=%H/" # NDJSON, gzip, time-partitioned keys
compression = "gzip"
encoding.codec = "ndjson"
buffer.type = "disk"
acknowledgements.enabled = true      # nothing lost before it lands in WORM
```

`reaper-cli audit verify --file` pulls these raw NDJSON objects and runs ByteExact against them
— the authoritative tamper-evidence proof the mutable ClickHouse copy cannot provide.

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
4. **Control-plane query API + UI** over ClickHouse — API ✅ done: `reaper-management` exposes
   `GET /api/v1/orgs/{org}/decisions` (filters: principal/action/resource/decision/
   policy_name/agent_id/from/to + pagination), `/decisions/stats`, and `/decisions/{id}`
   (explain view). ClickHouse HTTP interface via reqwest with server-side `param_*`
   binding (injection-safe), `FINAL` dedup reads, org-id tenant scoping enforced from the
   caller's auth (never from input). Configure `REAPER_CLICKHOUSE_URL` (+ `_DATABASE`,
   `_USER`, `_PASSWORD`; `REAPER_CLICKHOUSE_TENANT_FILTER=false` for single-tenant
   self-host). UI remains future work.
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
