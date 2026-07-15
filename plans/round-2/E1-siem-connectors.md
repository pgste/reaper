# Workstream E1 — Native SIEM Export Connectors

Round-2 remediation (`reviews/round-2/`, backlog `plans/round-2/00-NEXT-BACKLOG.md`).
*Closes PROD R2-2.* Decision logs are exportable in exactly one format today —
NDJSON (plus a pretty-printed JSON variant) — fanned out by **Vector** to
ClickHouse + optional S3/WORM. A bank SOC onboarding wants first-class
Kafka / Splunk-HEC / CEF / OCSF delivery and a push-export API. This is a
**greenfield feature on a mature capture/durability base** — no format or
transport code for any of those exists yet (every `ocsf`/`cef`/`splunk`/`kafka`
hit in the tree is in `docs/`/`plans/`, never `.rs`), and there is no
`Sink`/`Exporter`/`Connector` trait to extend.

This doc is updated as each slice lands.

---

## STATUS (2026-07-15) — all four slices landed; E1 complete

**Decisions locked (with the maintainer):**
1. **OCSF class = Authorize Session** (`class_uid` 3003, IAM category `category_uid`
   3), schema **version 1.1.0** — chosen for cleanest SIEM integration (native
   ingest by Amazon Security Lake / Splunk / Snowflake, no custom parsers).
   Allow/deny rides the universal `status_id` axis (allow→1 Success, deny→2
   Failure, log/other→99 Other); the decision isn't one of the class's
   Assign-Privileges/Groups activities, so `activity_id` is honestly `99` (Other)
   with `activity_name = "Authorization Decision"`. The action reads as a
   requested `privileges` entry, the resource as a `resources` entry, and every
   Reaper-specific field with no OCSF home is preserved under the schema's blessed
   `unmapped` object (nothing dropped, still validates).
2. **Push-export authority = a dedicated `audit:export` scope** (slice 3), mirroring
   the `audit:erase` separation-of-duties precedent — a read-only/admin token
   can't wire up an exfiltration sink without it.

**Landed (slice 1):**
- `crates/policy-engine/src/decision_export.rs`: `ExportFormat` enum
  (`ndjson`/`ocsf`/`cef`, parse + `as_str`) and `DecisionLogEntry::{to_ocsf,
  to_ocsf_ndjson, to_cef, export}`. OCSF per the mapping above; CEF (ArcSight)
  with correct header (`\`, `|`) and extension (`\`, `=`, newlines) escaping,
  standard keys (`rt`/`suser`/`act`/`outcome`/`request`/`externalId`) plus
  labelled `cs*`/`cn*` customs for policy/rule/agent/trace/eval-time/data-version.
  Both formats omit the large, possibly-encrypted `input_data`/`replay_input`
  blobs (they stay in NDJSON for the replay path); redaction is inherited from
  capture-time `decision_privacy`.
- Golden fixture `src/testdata/decision_ocsf.json` (the reviewable sign-off
  artifact) + 9 unit tests incl. `ocsf_matches_golden_fixture` and CEF
  metacharacter-escaping.
- Re-exported `policy_engine::ExportFormat`. No transport yet (slices 2–3).

**Landed (slice 2):**
- `deploy/decision-logs/vector-siem-sinks.toml`: a Vector overlay loaded next to
  `vector.toml` (`vector --config vector.toml --config vector-siem-sinks.toml`).
  An active `decisions_ocsf` remap transform reshapes `route._unmatched` into the
  same OCSF Authorize Session shape as the Rust `to_ocsf` (verified by eye against
  the golden fixture), plus commented copy-paste **Kafka**, **Splunk-HEC**, and
  **S3 data-lake** sink blocks (uncomment + set env, like the WORM block). Sinks
  point `inputs` at `decisions_ocsf` (OCSF) or `route._unmatched` (raw NDJSON).
  CEF is deferred to the native connector (slice 3) — Vector's encoders don't
  emit CEF. README gained a "SIEM export" section + the file listing. TOML
  syntax validated (no Vector binary in CI; operators run `vector validate`).

**Landed (slice 3):**
- New **`audit:export`** scope (`auth/scopes.rs`, all 4 sites + separation-of-duties
  test) — a connector is a standing exfiltration path, so it's not conferred by
  `org:admin`.
- Migration `026_siem_connectors.sql` / `0019_siem_connectors.sql` (`siem_connectors`
  table) + `AuditConnectorRepository` (`db/repositories/audit_connector.rs`):
  `ConnectorType {splunk_hec, http}`, format reuses `policy_engine::ExportFormat`,
  CRUD + `record_export` accounting, tenant-scoped.
- `ConnectorDeliveryService` (`src/siem/mod.rs`) generalised from
  `WebhookDeliveryService`: async reqwest, exponential-backoff retries,
  5xx/timeout classification. Splunk-HEC token auth + `{"event":…}` wrapping;
  generic-HTTP NDJSON body + optional HMAC-SHA-256 signature. Transport only —
  takes shaped lines.
- `api/connectors.rs` (`audit:export`-gated, audited): CRUD
  (`/orgs/{org}/audit/connectors[/{id}]`), `POST …/{id}/test` (synthetic record),
  and the **push-export** `POST …/{id}/export` — reads a decision range from
  `DecisionStore`, reconstructs `DecisionLogEntry`, shapes via
  `entry.export(format)`, delivers. Fully typed DTOs + ProblemDetails → the
  api_contract publishability gate passes (the `/orgs/{org}/audit/` prefix is a
  typed group, so every endpoint is hard-checked). Secrets never returned
  (`ConnectorSummary.has_secret`). New audit actions
  `audit.connector_{create,update,delete,export}` + `ResourceType::Connector`.
- Tests: 4 siem unit tests (HEC/HTTP body shaping, empty-batch no-op), scope
  separation test, integration `siem_connector_repo_crud_round_trips` (CRUD +
  export accounting + tenant isolation).

**Landed (slice 4):**
- policy-engine: the decision writer gained an optional streaming mirror — a
  non-blocking `SyncSender<Arc<DecisionLogEntry>>` on `WriterSinks` (E1 slice 4).
  `DecisionBuffer::new_with_stream` / `create_shared_buffer_with_stream` /
  `decision_stream_channel` wire it; the writer `try_send`s each captured entry
  after the durable write, dropping (counted as the new `stream_dropped` stat)
  rather than ever blocking durability. Best-effort telemetry, never the audit
  artifact. 2 unit tests (`streaming_mirror_receives_captured_entries`,
  `…_drops_when_saturated_without_blocking`).
- reaper-agent: `decision_stream.rs` consumer — `StreamSinkConfig::from_env`
  (`REAPER_DECISION_STREAM_URL`/`_FORMAT`/`_TOKEN`/`_BATCH`/`_FLUSH_MS`/`_QUEUE`)
  and a dedicated OS thread that batches, shapes via `entry.export(format)`, and
  POSTs to the SIEM (async reqwest on a private current-thread runtime; bearer
  auth; 5xx/transport retries). Wired in `main.rs` (uses `_with_stream` when the
  URL is set). README "SIEM export" gained the agent-streaming option.

---

## What already exists (reuse, don't rebuild)

- **Decision record + capture-time redaction**: `DecisionLogEntry`
  (`crates/policy-engine/src/decision_log.rs:11`) with `to_ndjson()` (SIMD
  `sonic-rs`); PII protection (`decision_privacy.rs`) is applied **once at
  capture** inside `DecisionBuffer::prepare_entry` (`decision_buffer.rs:934`), so
  *every* downstream sink — ring, file, stdout, export, Vector, any new connector
  — inherits redaction for free. Encrypted `input_data` arrives as an
  `{"enc":"aes256gcm",…}` envelope; connectors must not expect plaintext there.
- **The one fan-out point** in the agent is the private `WriterSinks { file,
  stdout }` on the background writer thread (`decision_buffer.rs:376`) — the
  natural in-agent hook for a native streaming sink, but it runs on a `std::thread`,
  not the async reactor.
- **Durable central pipeline**: agent NDJSON → Vector (`deploy/decision-logs/vector.toml`)
  → ClickHouse `reaper_audit.{decisions,checkpoints}` (+ commented S3 WORM sink).
  Control-plane query layer: `DecisionStore` (`services/reaper-management/src/decisions/mod.rs`).
- **Outbound-delivery plumbing to model on**: `WebhookDeliveryService`
  (`services/reaper-management/src/webhook/service.rs:47`) already has async
  `reqwest` delivery, exponential-backoff retries, HMAC-SHA-256 request signing,
  5xx/timeout retry classification, and per-org DB-backed subscriptions with
  secrets. A Splunk-HEC / generic-HTTP connector is ~90 % this service
  generalised. `integrations/servicenow.rs` is a second outbound-HTTP precedent.
- **Router + scope patterns**: `api/mod.rs` composes routers via `.merge(...)`;
  `api/webhook_subscriptions.rs` is the CRUD-for-outbound-config template. Scopes
  live in `auth/scopes.rs` (enum + `as_str`/`parse`/`all`, with a round-trip test
  — a new variant must be added in all four places).

## Two altitudes — the design decision

| Altitude | Fits | Cost | Notes |
|----------|------|------|-------|
| **Vector sinks (config-only)** | Kafka, Splunk-HEC, S3, Elasticsearch | Lowest — Vector already ships these sinks natively | No Rust; add `sinks.*` to `vector.toml`. Best for pure *transport* fan-out where the record shape is fine as-is. Cannot do rich OCSF/CEF *shaping* cleanly. |
| **Native connectors (reaper-management)** | Splunk-HEC, generic-HTTP push, OCSF/CEF-shaped delivery, a push-export API | Higher — new Rust module + config model | Needed for a first-class, per-tenant, authenticated **push-export API** and for format *shaping* (OCSF/CEF) that Vector can't express well. Model on `WebhookDeliveryService`. Reads the complete history from `DecisionStore`, not the agent's bounded ring. |

**Recommendation:** split by concern —
1. **Format shaping in Rust** regardless of transport: an OCSF (and CEF) mapper
   over `DecisionLogEntry`, mirroring `to_ndjson()`, in `policy-engine` so both
   the agent path and the control-plane path can emit shaped records.
2. **Transport**: ship the cheap **Vector sink configs** (Kafka/Splunk/S3) for
   operators who already run Vector, *and* a **native control-plane push
   connector** (modelled on `WebhookDeliveryService`) for Splunk-HEC / generic
   HTTP with per-org subscriptions — because a "push export API" is explicitly
   asked for and Vector config isn't an API.

Home for the push API: **reaper-management**, not the agent — the central store
has full history and the authenticated, OpenAPI-contracted, multi-tenant surface.

## Work breakdown (sliced by PR)
**Slice 1 — OCSF field mapping.** *(LANDED — see STATUS.)* A `to_ocsf()` (and
`to_cef()`) over `DecisionLogEntry` in `policy-engine`, with a fixed OCSF class
mapping (Authorize Session 3003), unit-tested against golden fixtures. No
transport yet.

**Slice 2 — Vector sink configs.** *(LANDED — see STATUS.)* Documented Kafka /
Splunk-HEC / S3 sink blocks in `deploy/decision-logs/vector-siem-sinks.toml`,
wired to the existing routes; a `decisions_ocsf` (`--format ocsf`) transform.
Config-only, no service change.

**Slice 3 — native push connector + API.** *(LANDED — see STATUS.)*
`api/connectors.rs` (CRUD for per-org connector subscriptions: type = splunk_hec |
http, endpoint, secret, format), a `ConnectorDeliveryService` generalised from
`WebhookDeliveryService`, a new `audit:export` scope (4 sites in `scopes.rs`),
reading shaped records from `DecisionStore`. Retry/signing/delivery-result
tracking reused.

**Slice 4 (optional) — agent streaming sink.** *(LANDED — see STATUS.)* Extend
`WriterSinks` fan-out for low-latency in-agent push where the central-store hop is
too slow.

## Open questions
1. **OCSF version/class** to target (Authorization Activity 3.x) and the exact
   field mapping — needs a golden-fixture sign-off.
2. **Kafka**: Vector-sink-only (no Rust dep) vs a native `rdkafka` producer
   (adds a C-lib dependency to the tree). Recommend Vector-only unless a native
   producer is specifically required.
3. **Scope**: new `audit:export` vs reuse `org:admin`.
