# Audit Integrity & Counterfactual Replay

**Readiness gate:** NOT READY → CONDITIONAL (the audit trail must be defensible to a regulator: tamper-evident, complete, and time-attested).
**Priority:** P1 (Security **P1-2**; Product **F7**; Synthesis executive-summary item 5 and cross-cutting theme 4).
**Findings closed:** Security **P1-2** (decision logs not tamper-evident, lossy ring, sampling/disable can drop records, wall-clock only); Product **F7** (replay is reproduction-only, not counterfactual); Product **F10** (no retention/legal-hold API on the query plane). Preserves the good attribution already present (`policy_version`, `data_version`, `data_checksum`).

---

## 1. Goal

Make Reaper's **decision audit trail** answer the regulator's two core questions under DORA / SOC 2:
1. *"Prove which policy version (and which data version) made decision X at time T, and prove the record wasn't tampered with or silently dropped."*
2. *"If we ship policy vX (or data vY), how many of last month's `allow`s would flip to `deny`?"* — a **counterfactual replay**, not just a reproduction.

To do that: hash-chain decision entries with periodic **signed checkpoints** exported to the sink (reusing the bundle-signing primitive); add a **mandatory-audit (fail-closed)** mode that is incompatible with sampling/disable; make buffer drops **counted and alarmed**; pair a **monotonic counter** with the wall-clock timestamp; add a **retention / legal-hold** API on the ClickHouse query plane; and build a **counterfactual replay engine** over stored provenance. Keep the excellent existing attribution (`policy_version`, `data_version`, `data_checksum`) intact.

Out of scope: management-action audit log integrity (that log lives in `audit/mod.rs`; the same hash-chain technique could be applied later but is not required here); the SSO/SCIM identity work (plan 03).

---

## 2. Current state (evidence) — file:line

**Good attribution already present (preserve):** `DecisionLogEntry` (`crates/policy-engine/src/decision_log.rs:11-83`) carries `decision_id`, `policy_id`, `policy_name`, `policy_version` (`:46`), `evaluation_time_ns`, `matched_rule`, `data_version` (`:72`), `data_checksum` (`:77`, "sha256:… verified by the agent on load"), `data_stale` (`:82`), and an opt-in explain snapshot `input_data` (`:67`). ClickHouse schema mirrors these 1:1 (`deploy/decision-logs/clickhouse-schema.sql:13-51`), including `policy_version` ("pins the logic that ran", `:30`) and `input_data` (`:43`).

**Gap 1 — not tamper-evident.** `DecisionLogEntry` has **no `prev_hash`, no `entry_hash`, no signature, no chain sequence**. The buffer (`decision_buffer.rs`) is an in-memory sharded ring; the file sink writes plain NDJSON (`decision_buffer.rs:306-320`), and ClickHouse uses `ReplacingMergeTree` (`clickhouse-schema.sql:47`) — nothing detects deletion or mutation of a row on disk or in ClickHouse. There is no per-entry HMAC/signature and no write-once/WORM guarantee.

**Gap 2 — lossy ring, drops not alarmed.** `decision_buffer.rs:335-393` `log()` evicts the oldest entry when a shard is full (`:388-391`, `dropped_entries.fetch_add`), and drops on writer-queue saturation (`:380-384`, `writer_dropped`). These counters exist in `DecisionBufferStats` (`:124-137`) and are surfaced by `/api/v1/decisions/stats` (agent `handlers/decisions.rs:137`), but **nothing treats a nonzero drop as an alarm/SLO violation**, and in mandatory-audit terms an eviction is an undetectable audit hole.

**Gap 3 — completeness is defeatable by config.** `DecisionLogConfig` (`decision_log.rs:196-289`): `enabled` (default `false`, `:298`), `log_allows`/`log_denies`, and `sample_allow_rate` (`:222-226`) can each legitimately produce **no record** for a decision. `should_log` (`decision_buffer.rs:276-293`) samples allows via a thread-local PRNG. There is a `REAPER_DECISION_LOG_MODE=full` preset (`decision_log.rs:388-420`) that forces `sample_allow_rate=1.0` and both allow+deny on — but it is (a) opt-in, (b) still bypassable by other env vars set after it? no — mode wins — but (c) it does **not** make the *ring/writer* lossless or fail-closed if the sink is unavailable. There is no "audit unavailable ⇒ fail closed" behavior anywhere.

**Gap 4 — wall-clock only.** `DecisionLogEntry::new` stamps `chrono::Utc::now().to_rfc3339()` (`decision_log.rs:96`) — unattested wall clock, no monotonic pairing. Clock skew/rollback is undetectable in-record, and ordering across a skew event is ambiguous (the buffer has a global `seq` at `decision_buffer.rs:367`, but it is **not persisted into the entry** and is lost past the in-memory ring).

**Gap 5 — replay is reproduction-only.** `input_data` (`decision_log.rs:67`) is opt-in (`include_input_data` default false, denies-only by default, `:236-243`) and snapshots only the two branched-on entities — not the full request/data graph. `api/decisions.rs` (management) has query/stats/timeseries/facets/get-by-id but **no replay route and no counterfactual eval**. The provenance to build it exists (`data_version`, `data_checksum` pin the exact datastore snapshot; `policy_version` pins the logic) — but nothing re-evaluates historical rows against a *different* policy/data version.

**Gap 6 — no retention/legal-hold API.** ClickHouse has a TTL `DELETE` at 90 days (`clickhouse-schema.sql:50`) with a commented WORM/tiering note (`:52-56`), but `api/decisions.rs:29-36` exposes no `PUT retention` / `POST legal-hold` route — retention is schema-baked, not tenant-controllable, and a legal hold cannot suspend TTL for a subset.

**Reusable primitives:**
- **Signing** (`crates/reaper-core/src/bundle_signing.rs`): `SigningKey`/`VerifyingKey` (Ed25519 + P-256), `sign_bundle`/`verify_bundle`, `sha256`, `key_id` pinning, constant-time compare. This same primitive can **sign audit checkpoints** — sign the checkpoint bytes exactly as it signs bundle bytes.
- **Sharded ring + background writer** (`decision_buffer.rs`) — the capture path to extend, carefully (hash-chaining needs a serialization point; see Step 2).
- **Global `seq`** (`decision_buffer.rs:367`) — already a per-entry monotonic counter; needs to be persisted into the entry.
- **ClickHouse `DecisionStore`** (`services/reaper-management/src/decisions/mod.rs`, surfaced by `api/decisions.rs`) — the query plane to extend with retention/legal-hold and to source replay rows.

---

## 3. Definition of Done — testable checkboxes

- [ ] `DecisionLogEntry` gains `seq: u64` (persisted monotonic counter), `prev_hash: String`, and `entry_hash: String`; NDJSON + ClickHouse schema carry them. Existing attribution fields are unchanged.
- [ ] Each entry's `entry_hash = sha256(canonical(entry_without_hash) || prev_hash)`; the chain is verifiable: given a run of NDJSON lines, a verifier recomputes every hash and detects any insertion, deletion, reordering, or mutation.
- [ ] The agent periodically emits a **signed checkpoint** record `{chain_id, seq_start, seq_end, last_entry_hash, count, monotonic_start, monotonic_end, wallclock, signature}` signed with a `SigningKey` (reusing `bundle_signing::sign_bundle`/`sha256`), exported to the same sink (file/stdout → Vector → ClickHouse). Checkpoints let a verifier prove completeness of a range without every intervening entry being online.
- [ ] A verifier tool/endpoint validates a checkpoint's signature with a pinned `VerifyingKey` and confirms the covered entries hash-chain to `last_entry_hash`; a tampered or missing entry in the range fails verification with the offending `seq`.
- [ ] **Mandatory-audit mode** (`REAPER_DECISION_LOG_MODE=mandatory` or `audit_required=true`): sampling is forced off (`sample_allow_rate=1.0`, `log_allows=log_denies=true`), and if the durable sink is unavailable (writer queue saturated / file unwritable) the agent **fails closed** — the eval either blocks or the agent reports unhealthy per a configured policy, rather than silently dropping an audit record. This mode is mutually exclusive with any sampling config (startup error if combined).
- [ ] Ring eviction and writer-queue drops increment counters that are (a) exported as metrics, (b) raise an alarm/log-error, and (c) in mandatory mode are treated as a fail-closed trigger, not a silent loss. `dropped_entries` / `writer_dropped` are already counted (`decision_buffer.rs:390,382`) — this wires them to alarms and to mandatory-mode enforcement.
- [ ] Every entry records a **monotonic counter** (persisted `seq`) alongside the wall-clock `timestamp`; a monotonic *time* source (e.g. steady-clock offset captured at start) is included in checkpoints so wall-clock rollback between checkpoints is detectable.
- [ ] Retention/legal-hold API on the query plane: `PUT /orgs/{org}/audit/retention {days}` and `POST /orgs/{org}/audit/legal-hold {filter, reason}` / `DELETE .../legal-hold/{id}` — a legal hold suspends TTL deletion for matching rows; tenant-scoped and audited.
- [ ] **Counterfactual replay** engine: `POST /orgs/{org}/replay {time_range, filter, policy_version|bundle_id, data_version}` streams historical decisions re-evaluated under the specified policy/data snapshot and returns a diff summary (counts of allow→deny / deny→allow flips + sample flipped records). Requires the replayable capture tier (below) for the rows in range.
- [ ] A **replayable capture tier** (opt-in, per-namespace, sampled/denies-only allowed) stores enough input (full resolved request + `data_version` reference) to re-evaluate — distinct from today's 2-entity explain snapshot. Documented cost trade-off; hot path untaxed when off.
- [ ] All new behavior is covered by tests (chain tamper-detection, checkpoint signature verify, mandatory-mode fail-closed, drop-alarm, replay flip-counting) and none of it regresses the sub-µs eval hot path (capture stays off-path; hashing happens on the writer/serialization side — see Step 2 rationale).

---

## 4. Critical steps — ordered

Each step: **what / where (files) / verify.**

### Step 1 — Persist the monotonic counter into the entry
- **What:** Add `seq: u64` to `DecisionLogEntry` (`decision_log.rs:11`). Populate it from the buffer's existing global `seq` (`decision_buffer.rs:367`) at `log()` time (it's currently computed but only used for in-ring ordering, `:392`). Add `seq` to NDJSON and to `clickhouse-schema.sql` (a `UInt64`, part of ordering). This is the cheapest, most independent win and the foundation for chain ordering.
- **Where:** `crates/policy-engine/src/decision_log.rs`, `decision_buffer.rs:335-393`, `deploy/decision-logs/clickhouse-schema.sql`.
- **Verify:** unit test asserts entries returned from the buffer carry strictly increasing `seq`; NDJSON round-trips it; existing `test_multi_shard_ordering_is_global` still passes.

### Step 2 — Hash-chain entries (serialization point)
- **What:** Add `prev_hash` + `entry_hash` to `DecisionLogEntry`. Compute `entry_hash = bundle_signing::sha256(canonical_bytes(entry_sans_hashes) ++ prev_hash_bytes)`. **Design constraint:** the ring is *sharded* for concurrency (`decision_buffer.rs:145-182`), so there is no single serialization order on the capture path — hash-chaining requires one. Do the chaining on the **background writer thread** (already the single-threaded serialization point, `decision_buffer.rs:218-231`, `handle_writer_msg`), ordering by the persisted `seq` from Step 1. The chain is thus over the *durable* stream (the audit artifact), not the in-memory query ring — which is exactly what a regulator inspects. This keeps the eval/capture hot path untouched (no hashing inline).
- **Where:** `decision_buffer.rs` (writer thread: maintain `last_hash`, stamp `prev_hash`/`entry_hash` before `to_ndjson`), `decision_log.rs` (fields + `canonical_bytes` helper using a stable field order), `clickhouse-schema.sql` (two `String` columns).
- **Verify:** new test writes N entries to a file sink, then a verifier recomputes the chain and passes; mutating/deleting/reordering one NDJSON line makes the verifier fail at the right `seq`. Bench confirms writer throughput still drains the queue at target volume (hashing is ~one SHA-256 per entry on the writer thread, not the request thread).

### Step 3 — Signed checkpoints exported to the sink
- **What:** Every `N` entries or `T` seconds, the writer emits a `Checkpoint` record over the covered `seq` range: `{chain_id (agent boot uuid), seq_start, seq_end, count, last_entry_hash, monotonic_start_ns, monotonic_end_ns, wallclock, key_id, signature}`. Sign `canonical(checkpoint_sans_sig)` with a `SigningKey` loaded from config — **reuse `bundle_signing::sign_bundle` / `sha256`** (the plan explicitly reuses the signing primitive here). Emit the checkpoint as its own NDJSON line (typed by a `record_type` discriminator) so Vector ships it to ClickHouse into a `reaper_audit.checkpoints` table.
- **Where:** `decision_buffer.rs` (writer thread checkpoint emitter), `crates/reaper-core/src/bundle_signing.rs` (reused as-is), new `reaper_audit.checkpoints` table in `clickhouse-schema.sql`, Vector routing in `deploy/decision-logs/vector.toml`.
- **Verify:** test signs a checkpoint and verifies it with the matching `VerifyingKey` (mirrors `bundle_signing` tests); a checkpoint whose `last_entry_hash` doesn't match the recomputed chain fails; `key_id` pinning enforced. Config with signing disabled emits unsigned checkpoints with a loud warning (or, in mandatory mode, refuses to start — Step 4).

### Step 4 — Mandatory-audit (fail-closed) mode, incompatible with sampling
- **What:** Add `audit_required: bool` (`REAPER_DECISION_LOG_MODE=mandatory`). When set: force `enabled=true`, `sample_allow_rate=1.0`, `log_allows=log_denies=true`; **reject at startup** if any sampling/disable var conflicts (fail-closed config validation, mirroring `DecisionBuffer::new`'s existing fail-closed-on-bad-protection behavior at `decision_buffer.rs:190-192`). At runtime, if the durable sink cannot accept an entry (writer queue full or file error), do **not** silently drop: either (a) block/backpressure the log call, or (b) flip the agent to unhealthy/`503` on eval per a configured `on_audit_unavailable` policy. Require a signing key in this mode (checkpoints must be signed).
- **Where:** `decision_log.rs` (config + `apply_mode` extension at `:396-420`), `decision_buffer.rs` (writer-unavailable handling; today `try_send` failure just counts `writer_dropped` at `:380-384`), agent health wiring in `services/reaper-agent/src/handlers/` + `main.rs`.
- **Verify:** startup test — `mandatory` + `sample_allow_rate=0.5` errors. Runtime test — saturate the writer queue and assert the configured fail-closed behavior (block or unhealthy), never a silent drop. Assert `mode=mandatory` implies a signed checkpoint stream.

### Step 5 — Counted + alarmed buffer drops
- **What:** Surface `dropped_entries` and `writer_dropped` as Prometheus counters and emit a `tracing::error!` + increment an SLO/alarm signal when either is nonzero over a window. In non-mandatory mode this is an alert; in mandatory mode it is the Step-4 fail-closed trigger. Add both to the `/decisions/stats` payload if not already fully surfaced.
- **Where:** `decision_buffer.rs` (`stats()` at `:462-475` already exposes them), agent metrics registration (`services/reaper-agent/src/` metrics), `handlers/decisions.rs:137` stats handler.
- **Verify:** force evictions (small `buffer_capacity`, as `test_buffer_capacity_limit` does at `decision_buffer.rs:769`) and assert the counter/metric/alarm fires; assert zero drops under capacity leaves it silent.

### Step 6 — Retention / legal-hold API on the query plane
- **What:** `PUT /orgs/{org}/audit/retention {days}` sets a tenant retention window; `POST /orgs/{org}/audit/legal-hold {filter, reason}` marks matching rows exempt from TTL deletion; `GET`/`DELETE` to list/release. Implement via a `reaper_audit.legal_holds` table + a ClickHouse retention strategy that respects holds (e.g. move TTL enforcement to an application-driven purge that skips held partitions/rows, replacing the static `TTL ... DELETE` for held data). Tenant-scoped + audited (reuse `authorize()` pattern in `api/decisions.rs:54-76`, add a new audit action `audit.retention_update` / `audit.legal_hold`).
- **Where:** new routes in `services/reaper-management/src/api/decisions.rs` (or a sibling `api/audit.rs`), `services/reaper-management/src/decisions/mod.rs` (`DecisionStore` methods), `deploy/decision-logs/clickhouse-schema.sql` (holds table + revised TTL/purge), `audit/mod.rs` (new actions).
- **Verify:** integration test against a ClickHouse test instance: set retention → old rows purge; place a legal hold → matching rows survive past TTL; release → they become purgeable. Cross-tenant isolation enforced (org A cannot hold/read org B).

### Step 7 — Replayable capture tier
- **What:** Add an opt-in per-namespace tier that stores the **full resolved request** (principal, action, resource, full context) plus the `data_version` reference needed to reload the exact datastore snapshot — beyond today's 2-entity `input_data` (`decision_log.rs:67`). Reuse the existing privacy pipeline (`decision_privacy.rs`, masking/encryption) so replay data is protected at capture. Gate it exactly like the explain tier (`include_input_data`/`input_data_denies_only`) so the hot path is untaxed when off and cost is sampled/denies-only when on.
- **Where:** `decision_log.rs` (new `replay_input` field or extend `input_data` semantics + config flag), `decision_buffer.rs` (`should_capture_input` sibling), `clickhouse-schema.sql` (column), agent `handlers/evaluate.rs` (populate from the resolved request).
- **Verify:** with the tier on, a stored row contains enough to reconstruct the request; with it off, no extra work and no field. Privacy: masked/encrypted fields never stored raw (mirror `decision_buffer.rs` protection tests).

### Step 8 — Counterfactual replay engine
- **What:** `POST /orgs/{org}/replay {time_range, filter, policy_version|bundle_id, data_version}` → stream historical rows (replayable tier) from ClickHouse, load a **headless `PolicyEngine`** with the specified historical/target bundle (`data_version`/`data_checksum` pin the exact datastore snapshot; both are already stored per row) and re-evaluate each request. Return a diff summary: total, allow→deny flips, deny→allow flips, and a sample of flipped decisions with old/new `matched_rule`. Run as an async job (large ranges) with progress + result streaming.
- **Where:** new `services/reaper-management/src/replay/` (engine + job runner), route in `api/decisions.rs`/`api/audit.rs`, consumes `crates/policy-engine` (`PolicyEngine`, evaluators) and the bundle/datastore snapshots (`data_version` → stored snapshot). Reuse `DecisionStore` (`decisions/mod.rs`) as the row source.
- **Verify:** seed a known set of decisions, replay under a policy that inverts one rule, assert the flip counts and sample records are exactly right; replay under the *same* policy/data version yields zero flips (reproduction sanity check); a range lacking the replayable tier returns a clear "not replayable — enable the tier" error.

---

## 5. Dependencies

- **Reused primitives:** `crates/reaper-core/src/bundle_signing.rs` (`SigningKey`/`VerifyingKey`/`sign_bundle`/`verify_bundle`/`sha256`/`key_id` pinning) for checkpoint signing; the sharded ring + background writer (`decision_buffer.rs`) as the serialization point; the global `seq` (`decision_buffer.rs:367`); `decision_privacy.rs` for replayable-tier protection; `DecisionStore` (`decisions/mod.rs`) + `api/decisions.rs::authorize` for the query-plane routes; `PolicyEngine` (`crates/policy-engine/src/engine/mod.rs`) for replay.
- **Schema:** `deploy/decision-logs/clickhouse-schema.sql` gains `seq`, `prev_hash`, `entry_hash`, a `checkpoints` table, a `legal_holds` table, and a replay-input column; `deploy/decision-logs/vector.toml` routes the new record types.
- **Config:** new `audit_required` / `mandatory` mode and a checkpoint signing key in `DecisionLogConfig` (`decision_log.rs`) + agent config; a `VerifyingKey` pinned in the control plane for checkpoint verification (same key-distribution story as bundle signing).
- **Cross-plan:** independent of plan 03, but the same hash-chain + signed-checkpoint technique is directly transferable to the management-action audit log (`audit/mod.rs`) fed by SSO/SCIM — note as a follow-on, not required here.

---

## 6. Testing & verification

- **Unit:** `seq` monotonicity; canonical serialization stability; `entry_hash` computation; checkpoint sign/verify (mirrors `bundle_signing` tests); mandatory-mode config validation; drop-counter/alarm; replay flip-counting.
- **Tamper tests (the core deliverable):** given an NDJSON run + signed checkpoints, prove detection of (a) a mutated field, (b) a deleted line, (c) an inserted line, (d) reordered lines, (e) a forged checkpoint (bad signature), (f) a checkpoint whose range hides a dropped entry (count/last_hash mismatch). Each must fail verification and name the offending `seq`.
- **Completeness/fail-closed:** mandatory mode + saturated writer ⇒ configured fail-closed behavior, never silent loss; mandatory + sampling config ⇒ startup error.
- **Time:** simulate wall-clock rollback between checkpoints ⇒ detectable via monotonic bounds; entries always carry both clocks.
- **Retention/legal-hold:** TTL purge, hold survival, release-then-purge, cross-tenant isolation (ClickHouse integration test).
- **Replay:** flip-count correctness, zero-flip reproduction sanity, "tier not enabled" error path.
- **Non-regression:** eval hot-path benchmark unchanged (capture stays off-path; hashing on the writer thread) — run `cargo bench -p policy-engine` and the agent full-handler bench referenced in `decision_buffer.rs` module docs.
- **Commands:** `cargo test -p policy-engine decision`, `cargo test -p reaper-management replay retention`, plus the ClickHouse-backed integration tests.

---

## 7. Effort & phasing — S/M/L

- **Phase 1 (S-M) — integrity core (Steps 1-3): `seq`, hash chain, signed checkpoints.** The single highest-value slice; reuses the signing primitive and the existing writer thread. Delivers "tamper-evident + provably complete over a signed range."
- **Phase 2 (S) — mandatory mode + drop alarms (Steps 4-5).** Closes "completeness defeatable by config" and "lossy + silent." Small because the counters already exist; the work is enforcement + fail-closed wiring.
- **Phase 3 (M) — retention/legal-hold (Step 6).** Query-plane + ClickHouse work; needed for GDPR/legal-hold questionnaire lines.
- **Phase 4 (M-L) — replay (Steps 7-8).** The differentiator ("impact of policy vX"); larger because of the replayable-capture tier and the headless-engine job runner. Sequence last — it depends on nothing in Phases 1-3 but is lower on the compliance-gate priority.
- Overall **M-L**, with a compliance-unblocking **S-M** first slice (Phase 1+2).

---

## 8. Key decisions (ADR-style)

**ADR-1: Chain on the durable stream (writer thread), not the in-memory sharded ring.** The ring is deliberately sharded for lock-free concurrency (`decision_buffer.rs` module docs) — imposing a single chain order on the capture path would reintroduce the contention that design removed and tax the sub-µs eval path. The audit artifact a regulator inspects is the durable NDJSON→ClickHouse stream, so chain there, ordered by the persisted `seq`. *Trade-off:* the in-memory query ring itself isn't individually chained, but it is transient and not the compliance record. **Recommend chain-on-write.**

**ADR-2: Reuse `bundle_signing` for checkpoints rather than a new signing stack.** The primitive is already fail-closed, crypto-agile (Ed25519 + P-256 for FIPS), constant-time, and `key_id`-pinned with good tests. One signing story for bundles and audit checkpoints means one key-management and rotation model. **Recommend reuse.**

**ADR-3: Per-entry signature vs periodic signed checkpoint.** Signing every entry is simplest to reason about but expensive at >100K decisions/s and bloats storage. A hash chain (cheap per entry) + periodic signed checkpoint gives the same tamper-evidence and completeness guarantee over a range at a fraction of the cost. **Recommend hash-chain + signed checkpoints.**

**ADR-4: Mandatory-audit is a distinct fail-closed mode, not a default.** Blocking eval on audit-sink unavailability is the correct posture for a regulated tenant but an availability risk for a latency-sensitive one. Make it explicit opt-in (`mandatory`), mutually exclusive with sampling, and let non-regulated deployments keep today's fast, lossy-but-alarmed behavior. **Recommend opt-in mandatory mode with configurable `on_audit_unavailable` (block vs unhealthy).**

**ADR-5: Replay fidelity vs capture cost — a per-namespace replayable tier.** Full-input capture is what makes counterfactual replay possible but taxes storage (and, if done wrong, the hot path). Offer it as an opt-in, sampled/denies-only tier per namespace, reusing the explain-tier gating so org-wide sub-µs performance is never taxed by default. **Recommend tiered opt-in capture.**

---

## 9. Risks & rollback

- **Risk: chaining/serialization becomes a writer-thread bottleneck at high volume.** *Mitigation:* one SHA-256 per entry on a thread that already serializes to NDJSON; bench before/after. If the writer can't keep up in mandatory mode, that is *correctly* a fail-closed signal, not a silent drop. *Rollback:* disable chaining (fields become empty) — capture reverts to today's behavior.
- **Risk: mandatory fail-closed mode turns an audit-sink outage into an eval outage.** *Mitigation:* it is opt-in and only for tenants who explicitly choose integrity over availability; `on_audit_unavailable` lets them pick block vs unhealthy; non-mandatory deployments are unaffected. *Rollback:* clear the mode; alarms remain.
- **Risk: checkpoint signing key compromise forges "valid" completeness proofs.** *Mitigation:* same key-management rigor as bundle signing (`key_id` pinning, rotation, revocation list — cross-references Security P1-1 anti-rollback work); keep the signing key off the agents that only need to *emit* if a control-plane-side signer is preferred. 
- **Risk: legal-hold implementation drifts from ClickHouse TTL semantics and either over-deletes (destroys held data) or under-deletes (misses retention).** *Mitigation:* replace static `TTL DELETE` for held data with application-driven purge that checks `legal_holds` first; integration tests assert both directions; default (no hold) path keeps the existing simple TTL.
- **Risk: replay uses a data/policy snapshot that no longer exists.** *Mitigation:* replay validates `data_checksum`/`policy_version` availability up front and errors clearly if the pinned snapshot was purged (respecting retention) rather than silently replaying against wrong data.
- **General rollback:** every change is additive (new fields default-empty, new modes off by default, new routes, new tables). Turning all flags off yields exactly today's behavior with the good attribution (`policy_version`, `data_version`, `data_checksum`) intact.
