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

## A3 — immutable checkpoint anchor + cross-boot linkage  *(pending)*
Enable the S3/WORM checkpoint sink by default; sign a genesis linking each boot's
`chain_id` to the prior, so an insider can't delete a boot's decisions +
checkpoints together undetectably. **Closes SEC R2-3.**

## A4 — durable-before-serve for mandatory-audit mode  *(pending)*
Move the durable hand-off off the reactor; no served-allow loss window and no
`Block`-mode reactor stall. **Closes SEC R2-4, PERF R2-P2-2.**

## A5 — redaction-on-by-default + redactable `resource`  *(pending)*
Explicit redaction posture at enable time; allow `resource` redaction; ship a
GDPR-compliant default profile. **Closes SEC R2-5.**
