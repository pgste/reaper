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
**Slice 1 — OCSF field mapping.** A `to_ocsf()` (and `to_cef()`) over
`DecisionLogEntry` in `policy-engine`, with a fixed OCSF class mapping
(Authorization Activity), unit-tested against golden fixtures. No transport yet.

**Slice 2 — Vector sink configs.** Documented Kafka / Splunk-HEC / S3 sink blocks
in `deploy/decision-logs/`, wired to the existing routes; a `--format ocsf`
transform option. Config-only, no service change.

**Slice 3 — native push connector + API.** `api/connectors.rs` (CRUD for per-org
connector subscriptions: type = splunk_hec | http, endpoint, secret, format),
a `ConnectorDeliveryService` generalised from `WebhookDeliveryService`, a new
`audit:export` scope (4 sites in `scopes.rs`), reading shaped records from
`DecisionStore`. Retry/signing/delivery-result tracking reused.

**Slice 4 (optional) — agent streaming sink.** Extend `WriterSinks` fan-out for
low-latency in-agent push where the central-store hop is too slow.

## Open questions
1. **OCSF version/class** to target (Authorization Activity 3.x) and the exact
   field mapping — needs a golden-fixture sign-off.
2. **Kafka**: Vector-sink-only (no Rust dep) vs a native `rdkafka` producer
   (adds a C-lib dependency to the tree). Recommend Vector-only unless a native
   producer is specifically required.
3. **Scope**: new `audit:export` vs reuse `org:admin`.
