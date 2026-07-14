# Subject Erasure (GDPR Art. 17) — Posture & Verification

*Round-2 workstream E2. Endpoint: `POST /orgs/{org}/audit/erasure`
(`services/reaper-management/src/api/audit.rs`), scope `audit:erase`.*

Reaper erases a data subject across the two stores that hold personal data — the
central decision-log store and the authoring DataStore — while keeping the
tamper-evident audit chain provable. This document records **what erasure does,
what it deliberately does not rewrite, and how to verify the store afterwards.**

---

## What erasure does

| Store | Action | Mechanism |
|-------|--------|-----------|
| **Decision-log** (ClickHouse, queryable) | Redact in place | `ALTER TABLE decisions UPDATE` tombstones `principal`/`resource`/`context`/`input_data`/`replay_input`, leaving `seq`/`chain_id`/`prev_hash`/`entry_hash` untouched. `DecisionStore::erase_subject`. |
| **Authoring DataStore** (live) | Hard delete (cascade) | `delete_entity_cascade` removes the entity row, its tuples (subject/object), its role-bindings, and writes change-log tombstones so agents converge on the removal. |

The subject is matched by its **plaintext** identifier on the `principal` and
`resource` columns and by `entity_id` in the DataStore. Under the
**`pseudonymize` privacy profile** those decision-log columns hold
`sha256:<hmac>` tokens, not plaintext (per-tenant salt, held agent-side); the
caller supplies the tenant's `REAPER_DECISION_LOG_HASH_SALT` as `pseudonym_salt`
in the request and erasure additionally matches the derived tokens (E2
follow-up #1). The salt is used only to derive the match tokens for that one
call — never persisted, never echoed in the receipt or audit trail.

Active **legal holds** are honored exactly as in a retention purge: a held row is
never redacted, and an active *blanket* hold defers the decision-log redaction
entirely (a lawful basis to retain overrides the erasure — `DeferredBlanketHold`).

Every erasure is **idempotent** (`Idempotency-Key`) and writes an
`audit.subject_erasure` **receipt** to the audit trail as durable proof.

---

## Why redact-in-place, not delete

The decision-log store is tamper-evident: `principal`/`resource`/`input_data`
are inputs to each row's `entry_hash`, and rows are chained by
`seq`/`prev_hash`. Hard-deleting scattered interior rows would break chain
completeness (`verify_records` → `chain_broken` + `checkpoint_invalid`) for the
**non-erased** rows sharing the range — destroying the attestation the whole
audit workstream (A1–A3) exists to provide.

Redact-in-place instead overwrites only the PII columns and leaves the chain
columns intact, so:

- **`VerifyMode::Linkage` still passes** — it verifies chain *linkage* over the
  stored `prev_hash`/`entry_hash` values plus checkpoint signatures, never
  recomputing content hashes. Completeness, ordering, and checkpoint integrity
  stay provable. This is the mode the store-backed verifier
  (`GET …/audit/verify`) already runs.
- **`VerifyMode::ByteExact` over the redacted queryable store no longer matches**
  — the content changed, so recomputed hashes differ from the stored ones. That
  is expected: ByteExact is not meant to run over the queryable store; it is the
  **WORM archive's** job (`reaper-cli audit verify --file`).

### Post-erasure verification posture: **Linkage-based**

After any erasure the decision store's authoritative verification is
**Linkage**. The erasure receipt records `verification_posture: "linkage"` to
make this explicit. Operators and auditors verify a redacted range with the
store-backed verifier (Linkage); the WORM archive (below) remains the ByteExact
anchor over the *original* bytes.

---

## Immutable surfaces: documented, receipted exemptions

Two surfaces are **append-only and are NOT rewritten** by erasure. This is not
an oversight — both are immutable *by design*, and rewriting either would defeat
the guarantee it exists to provide. Erasure therefore **discloses** them on the
receipt (`exemptions[]`, each with a `surface`, `reason`, and `disposition`)
rather than silently leaving them out of scope.

### 1. Decision-log WORM archive (`decision_log_worm_archive`)

The S3 Object-Lock (**COMPLIANCE mode**) WORM sink and any NDJSON archive
(`deploy/decision-logs/vector.toml`, Helm `decisionLogs.worm.enabled`) receive
the same decisions + checkpoints as ClickHouse. Object-Lock COMPLIANCE mode
means **even a bucket admin — even root — cannot alter or delete an object
before its retention window expires.** That immutability is the entire point: it
is the independent copy a regulator runs `reaper-cli audit verify --file`
ByteExact against, and cross-boot genesis linkage makes deleting a whole writer
boot detectable. A rewrite path is therefore *impossible by construction*, and
intentionally so.

- **Lawful basis to retain:** GDPR Art. 17(3)(b) — retention required for
  compliance with a legal obligation (a complete, tamper-evident audit trail).
- **Disposition:** the subject's bytes age out when the Object-Lock retention
  window expires (set to the strictest applicable window, e.g. SOX 7y / HIPAA
  6y).
- **Data minimization already applied:** under the `pseudonymize` profile the
  archive holds only HMAC tokens (`principal`/`resource`) and AES-256-GCM
  ciphertext (`input_data`/`replay_input`). **Crypto-shredding** the tenant key
  renders the archived `input_data` irrecoverable even in the WORM copy — the
  defence-in-depth path for tenants who want the archived explain-tier data gone
  before retention expiry.

### 2. Published DataStore versions (`published_datastore_versions`)

Published data-bundle versions (`adm_versions`, `get_version_document`) are
immutable, **checksum-sealed** materialized snapshots — exactly what deployed
agents load and may still be running. Erasure hard-deletes the **live** entity
(cascade), but historical published versions are not rewritten: doing so would
break the version checksum and the immutability contract agents rely on to
detect a tampered bundle.

- **Basis to retain:** technical immutability + integrity contract; the versions
  are point-in-time snapshots, not the current record.
- **Disposition:** superseded by the next publish (which materializes from the
  now-erased live store); historical versions age out under data-bundle version
  retention.

---

## The receipt

`POST /orgs/{org}/audit/erasure` returns — and writes to the audit trail — an
`ErasureReceipt`:

```jsonc
{
  "subject": "alice@example.com",
  "decision_log": { "status": "submitted", "holds_honored": 1, "matched_pseudonyms": true },
  "datastore":    { "status": "erased", "datastores_scanned": 2, "entities_deleted": 1 },
  "verification_posture": "linkage",
  "exemptions": [
    { "surface": "decision_log_worm_archive",      "reason": "…", "disposition": "…" },
    { "surface": "published_datastore_versions",   "reason": "…", "disposition": "…" }
  ],
  "requested_by": "user-123"
}
```

`exemptions` is a pure function of what the live-store steps did
(`immutable_exemptions`): the WORM-archive exemption is disclosed whenever the
decision-log redaction was submitted; the published-versions exemption whenever
the org has authoring DataStores. A DSAR responder can hand the receipt to a data
subject or regulator as a complete, honest account of what was erased immediately
and what is retained-by-exemption with its basis and time-bound.

---

## Related

- `plans/round-2/E2-subject-erasure.md` — design + slice history.
- `docs/architecture/DECISION_LOG_PIPELINE.md` — the ClickHouse + WORM pipeline.
- `crates/policy-engine/src/decision_log.rs` — `VerifyMode`, `verify_records`.
- `docs/security/VULN_RESPONSE.md` — advisory triage (unrelated, sibling doc).
