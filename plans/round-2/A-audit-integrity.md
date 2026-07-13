# Workstream A — Audit Integrity Operationalization

Round-2 remediation (`reviews/round-2/`, backlog `plans/round-2/00-NEXT-BACKLOG.md`).
The hash-chain + signed-checkpoint primitives are real and unit-tested; this
workstream makes them **operational end-to-end** so a regulator can actually run
"prove the audit trail is complete and unaltered" against the live store.

Items A1–A5; implemented in order. This doc is updated as each lands.

---

## A1 + A2 — store-backed verifier + chain verifiable from the store *(combined, landed)*

**Why combined:** A1 (ship a verifier) cannot be *correct* across writer boots
without A2 (make the chain reconstructable from the queryable store). The
ClickHouse `decisions` table stores `seq`/`prev_hash`/`entry_hash` but **not
`chain_id`**, and `seq` resets to 0 each writer boot — so two boots' `seq=5` rows
are indistinguishable, and the table's `ORDER BY (tenant, hour, action, …)` +
ReplacingMergeTree dedup does not preserve chain (write) order. A verifier built
on A1 alone would work on a single-boot NDJSON file but not against production.

### A2 — bind `chain_id` into every decision record
- `DecisionLogEntry` gains `chain_id: String` (`#[serde(default,
  skip_serializing_if = "String::is_empty")]`). Because it serializes when
  present, it is part of `canonical_bytes` and therefore **bound into the entry
  hash** — a record cannot be moved to another chain undetected. Old records
  (empty `chain_id`) hash exactly as before → backward compatible.
- One `chain_id` per writer boot, shared by the `HashChain` and the
  `Checkpointer` (previously only the checkpointer had it). `HashChain::stamp`
  sets `record.chain_id` *before* computing the hash.
- ClickHouse `decisions` table + Vector mapping carry `chain_id`. Verification
  queries `ORDER BY chain_id, seq` explicitly (never relying on table order).

### A1 — the verifier
- **Reusable core** in `policy-engine::decision_log`: `verify_records(entries,
  checkpoints, keys, mode) -> VerificationReport` — groups entries by `chain_id`,
  sorts by `seq`, verifies each chain and every checkpoint over its covered
  range. Reports the mode, chains checked, checkpoints verified, records covered,
  and every violation (with `seq`).
- **Two `VerifyMode`s — the soundness decision.** A *queryable* store projects
  each record into typed columns (timestamps re-rendered; A2-absent fields like
  `data_version`/`model_version` have no columns), so a record reconstructed from
  store rows does **not** reproduce the exact bytes that were hashed.
  Recomputing `entry_hash` from a reconstruction therefore false-positives on
  clean data — unacceptable for a "prove the audit is intact" tool. So:
  - **`ByteExact`** recomputes every `entry_hash` from content (full crypto
    guarantee: catches content mutation too). Requires the byte-identical raw
    NDJSON — used by `--file` over the write-ordered stream / immutable WORM
    archive. **Authoritative.**
  - **`Linkage`** verifies chain linkage using the **stored**
    `prev_hash`/`entry_hash` plus checkpoint signatures, without recomputing
    content hashes. Sound over the store (no false positives); catches deletion,
    insertion, reordering, truncation, and checkpoint tampering. A pure in-place
    content edit that preserves the stored hashes is left to a `ByteExact` pass
    over the immutable archive (A3). **Operational-monitoring guarantee.**
  The report carries `mode` so the caller always knows which guarantee it holds.
- **`reaper-cli audit verify`** — `--file <ndjson>` (offline / air-gapped →
  `ByteExact`, authoritative) or ClickHouse `--url/--tenant/--chain/--from/--to`
  (→ `Linkage`). Prints the report; exits non-zero on a violation. Checkpoint
  signatures verified with `--verifying-key key_id:hex` (repeatable).
- **`GET /orgs/{org}/decisions/verify`** — admin-scoped management endpoint
  running `Linkage` over the store, returning the structured report (the surface
  a scheduled verifier or a regulator's read-only credential hits). Verifying
  keys from `REAPER_DECISION_LOG_CHECKPOINT_VERIFYING_KEY`.

**Closes:** SEC R2-2, PROD R2-10 (audit tamper-evidence becomes operable, not a
library-only property), with the honest guarantee split: byte-exact crypto proof
via `--file`/WORM, sound structural monitoring via the store endpoint.

---

## A3 — immutable checkpoint anchor + cross-boot linkage  *(in progress)*

Closes SEC R2-3: today checkpoints land in the *same mutable* ClickHouse as
decisions (WORM sink commented out) and each writer boot mints a fresh random
`chain_id` with **no linkage to the prior boot** — so an insider with store write
access can delete a whole boot's decisions *and* its checkpoints and leave no
evidence the boot ever existed. Two fixes:

### A3.1 — immutable WORM sink (deploy + docs)
Enable the S3 **Object-Lock (WORM)** sink for the checkpoint stream (and,
recommended, decisions) in `deploy/decision-logs/vector.toml`; add
`decisionLogs.worm.*` helm values and document the compliance-mode bucket setup.
The WORM copy is the independent anchor a ClickHouse insider cannot rewrite; the
`reaper-cli audit verify --file` ByteExact path runs against it for the
authoritative proof.

### A3.2 — cross-boot genesis linkage (code)
- `Checkpoint` gains optional `prev_chain_id` + `prev_chain_head` (serde default,
  skip-if-empty; bound into the signature), set only on a boot's **first**
  checkpoint — a signed genesis anchor pointing at the previous boot's terminal
  chain head.
- The agent keeps a small **continuity file** (`{chain_id, last_head, last_seq}`)
  updated as checkpoints emit; on restart it reads the prior boot's values and
  threads them into the new `Checkpointer` so its first checkpoint carries the
  linkage.
- `verify_records` gains a **cross-boot check** (both modes): the genesis
  `prev_chain_id`/`prev_chain_head` must match a prior chain's terminal
  checkpoint. A boot whose genesis names an absent prior chain
  (`missing_prior_boot`) or a mismatched head (`boot_linkage_broken`) is a
  violation — so deleting an entire boot from the archive is detectable via the
  next boot's dangling reference.

This upgrades A1+A2's residual (a whole-boot deletion, previously invisible) to
detectable, and — with the WORM anchor — the in-place-content-edit residual too
(caught by a ByteExact pass over the immutable copy).

*(Control-plane chain registry — an even-more-independent anchor where the agent
registers each `chain_id`+`prev_chain_id` with the management plane at startup —
is a possible future refinement; the WORM archive + continuity linkage close
R2-3's core without coupling agent startup to the control plane.)*

## A4 — durable-before-serve for mandatory-audit mode  *(landed)*
Move the durable hand-off off the reactor; no served-allow loss window and no
`Block`-mode reactor stall. **Closes SEC R2-4, PERF R2-P2-2.**

**Design (landed).**
- New async path `DecisionBuffer::log_durable(entry).await -> bool`: hands the
  `Arc<DecisionLogEntry>` to the writer over `WriterMsg::EntryAck(entry, oneshot)`;
  the writer thread writes the NDJSON line, `flush()`es, and `sync_data()`
  fsyncs the file sink, then acks `true` through the oneshot. Returns `false`
  (fail closed) when there is no fsync-able file sink or the writer is gone.
- The eval handler branches on `buffer.mandatory_durable()`
  (`enabled && audit_required`): mandatory → `await log_durable`, and on `false`
  return **`503`** *before* the response cache insert, so a non-durably-logged
  allow is never served. Best-effort mode keeps the fire-and-forget `log()` with
  zero added latency.
- **Deviation resolved (stdout-only mandatory):** stdout cannot be fsynced, so a
  mandatory + stdout-only config would `503` on every request. Rather than a
  per-request outage, `DecisionLogConfig::validate()` now requires a file sink
  (`REAPER_DECISION_LOG_FILE`) whenever `audit_required` — the agent fails **fast
  at startup** with a clear config error. Container `stdout`→Vector deployments
  mount a file path and tail it. Doc updated in `DECISION_LOG_PIPELINE.md`.

## A5 — redaction-on-by-default + redactable `resource`  *(landed)*
Explicit redaction posture at enable time; allow `resource` redaction; ship a
GDPR-compliant default profile. **Closes SEC R2-5.**

**Design (landed).**
- **Explicit posture, enforced at boot:** `REAPER_DECISION_LOG_PRIVACY` =
  `pseudonymize` (the GDPR-friendly profile: implies `hash_principal` +
  `hash_resource`, requires `HASH_SALT`) or `raw` (an explicit, auditable
  opt-out — it echoes in `/decisions/stats`). Enabling decision logging with
  neither a profile nor any fine-grained protection knob fails `validate()`
  with an actionable error. PII-by-default is structurally impossible.
- **`resource` is redactable:** new `hash_resource` knob
  (`REAPER_DECISION_LOG_HASH_RESOURCE`) pseudonymizes `resource` with the same
  HMAC salt, **domain-separated** (`HMAC(salt, "resource\0" || value)`) so equal
  principal/resource strings never produce correlatable tokens; the replay
  blob's resource copy carries the same pseudonym (join-key consistency across
  views). Principal keeps the legacy un-prefixed form so existing pseudonyms
  stay stable across versions.
- Profile semantics live in BOTH `from_env` (preset expansion) and
  `DataProtection::from_config` (so programmatic configs get identical
  behavior). Unknown `PRIVACY` values are ignored with a warning, which leaves
  the posture unchosen → startup error (never silently raw).
- Deploy surfaces updated: compose exposes `DECISION_LOG_PRIVACY`/`_HASH_SALT`
  (no default — the operator must choose); Helm adds `decisionLogs.privacy` +
  `hashResource`; the demo `full.yaml` profile makes its choice explicit
  (`privacy: raw` with a production-switch comment).
