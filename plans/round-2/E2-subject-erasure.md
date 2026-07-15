# Workstream E2 — GDPR Subject-Erasure

Round-2 remediation (`reviews/round-2/`, backlog `plans/round-2/00-NEXT-BACKLOG.md`).
*Closes PROD R2-3.* Reaper has no erase-by-subject anywhere; UK DPA-2018 / GDPR
Art. 17 ("right to erasure") requires a way to remove a data subject's personal
data across the decision-log store **and** the authoring DataStore, guarded by a
legal hold, with a durable receipt proving the erasure happened.

This doc is updated as each slice lands.

---

## STATUS (2026-07-14) — slice 1 + all three follow-ups landed; E2 complete

**Decisions locked (with the maintainer):**
1. Decision-log strategy = **redact-in-place** (option B below).
2. Authority = a dedicated **`audit:erase`** scope (not conferred by `org:admin`).

**Landed** (PR #61 — merged into `main` before the next session):
- `audit:erase` scope — `auth/scopes.rs`, separation-of-duties test.
- `DecisionStore::erase_subject` — redact-in-place `ALTER…UPDATE` over the
  PII columns, hold-honoring, chain-preserving (`decisions/mod.rs`), SQL-builder
  unit tests.
- `POST /orgs/{org}/audit/erasure` — `api/audit.rs`: `audit:erase`-gated,
  `idempotency::run`, fetches active holds → `erase_subject`, hard-deletes the
  subject's entity across every org DataStore (`datastore_ids_for_org` +
  `delete_entity_cascade`), writes an `audit.subject_erasure` receipt to the
  audit trail. Typed DTOs + ProblemDetails → api_contract gate passes.

**Remaining — the follow-up pieces:**
1. **Pseudonymisation salt.** *(LANDED — follow-up #1.)* Under
   `PrivacyProfile::Pseudonymize`, decision-log `principal`/`resource` are
   `sha256:<hmac>` of the plaintext with a per-tenant `hash_salt` held agent-side.
   The erasure endpoint now accepts an optional `pseudonym_salt` in the request
   body (the tenant's `REAPER_DECISION_LOG_HASH_SALT`); when supplied it derives
   the subject's principal token (`policy_engine::pseudonymize`) and resource token
   (`pseudonymize_domain(_, "resource", _)`) and passes them as
   `decisions::SubjectPseudonyms` to `erase_subject`. `build_erase_subject_sql`
   unions the hashed-column terms with the plaintext selector — one call covers a
   plaintext *or* a pseudonymised tenant (a token can never equal a plaintext id,
   so the union is exact). The salt is used only to derive the match tokens: it is
   never persisted, echoed in the receipt (`matched_pseudonyms: bool`), or written
   to the audit trail, and it is excluded from the idempotency fingerprint (only
   the `pseudo`/`plain` marker enters it, so a retry that adds the salt is treated
   as a materially different request rather than replaying a narrower result).
2. **Immutable archive + published DataStore versions.** *(LANDED — follow-up
   #2.)* The WORM/NDJSON archive (`VerifyMode::ByteExact` source) and immutable
   published data-bundle versions (`get_version_document`) are append-only and NOT
   rewritten by erasure. Resolved as a **documented, receipted exemption** (the
   only sound option: the WORM sink is S3 Object-Lock in COMPLIANCE mode —
   un-rewritable even by root, *by design* — and published versions are
   checksum-sealed snapshots agents may still run, so rewriting either defeats the
   guarantee it exists for). The receipt now carries `verification_posture:
   "linkage"` and an `exemptions[]` list (`immutable_exemptions`, pure + unit
   tested): a `decision_log_worm_archive` exemption whenever the decision-log
   redaction was submitted, and a `published_datastore_versions` exemption whenever
   the org has authoring DataStores — each with its lawful basis and disposition
   (ages out with retention / superseded by next publish; crypto-shred the tenant
   key to render the archived `input_data` irrecoverable pre-expiry). Full posture
   + verification guidance: `docs/security/SUBJECT_ERASURE.md`.
3. **Dedicated erasure-receipt table.** *(LANDED — follow-up #3.)* Added
   `audit_erasure_requests` (migrations `025_subject_erasure.sql` /
   `0018_subject_erasure.sql`, registered in `connection.rs`) + the
   `AuditErasureRepository` (`record` / `list_for_org`, borrowed
   `NewErasureRecord` input, `ErasureRecord` output). The endpoint persists one
   row per completed erasure best-effort *inside* the idempotency-guarded op
   (so a history hiccup never fails an irreversible erasure, and a key replay
   never double-inserts): the full `ErasureReceipt` JSON verbatim in `receipt`,
   with `decision_log_status`/`holds_honored`/`matched_pseudonyms`/
   `datastore_status`/`datastores_scanned`/`entities_deleted`/
   `verification_posture` decomposed for querying. New read path
   `GET /orgs/{org}/audit/erasures` (org-admin-gated — reading *who was erased*
   is a compliance read like holds/retention, distinct from the `audit:erase`
   write scope), newest-first, `?limit` default 100 / hard cap 500. Typed DTOs +
   ProblemDetails → api_contract gate passes; integration test
   `subject_erasure_history_records_and_lists` covers record + list ordering +
   tenant scoping.

---

## What already exists (reuse, don't rebuild)

The compliance substrate from Plan 04 + round-2 A1/A2 is directly reusable:

- **Legal hold + retention**, end to end: `HoldFilter`
  (`services/reaper-management/src/decisions/mod.rs:88`), `LegalHold` +
  `AuditGovernanceRepository::{create_hold,active_holds,release_hold}`
  (`db/repositories/audit_governance.rs`), tables `audit_legal_holds` /
  `audit_retention` (`migrations/014_audit_governance.sql`,
  `migrations_pg/0007_audit_governance.sql`), and a **hold-aware purge** that
  emits one `NOT (…)` exclusion per active hold (`decisions/mod.rs:build_purge_sql`,
  orchestration in `decisions/purge.rs:56`). A blanket hold refuses the purge
  outright (`PurgeOutcome::SkippedBlanketHold`).
- **DataStore cascade delete**: `DatastoreRepository::delete_entity_cascade`
  (`db/repositories/datastore.rs:343`) transactionally deletes the entity row,
  its tuples (as `subject` or `object`), its role-bindings, and writes
  change-log tombstones so agents converge on the removal.
- **Admin authz + audited writes**: `authorize_admin` + `write_audit`
  (`api/audit.rs:52,82`), `org:admin`-gated, the natural home for the endpoint.
- **Idempotency**: `idempotency::run(db, headers, scope, scope_id, fingerprint, op)`
  (`api/idempotency.rs:77`) — already wraps the destructive `apply_migration`
  (`api/datastore.rs:1046`). Erasure is irreversible + fans out, so it must use this.
- **Hash-chain verifier** with a `Linkage` mode (`decision_log.rs:VerifyMode`)
  that survives in-place content redaction — see the design decision below.

## The core design decision — erase vs. redact under the audit hash chain

The decision-log store is tamper-evident: every row carries `seq` / `prev_hash`
/ `entry_hash`, and `principal` / `resource` / `input_data` are **inputs to
`entry_hash`** (`decision_log.rs:canonical_bytes`). Subject erasure must mutate
*scattered interior rows* — exactly what the chain is built to detect. Three
options the code permits:

| Strategy | Chain impact | GDPR fit | Cost |
|----------|-------------|----------|------|
| **A. Hard-delete** the rows (`ALTER TABLE … DELETE`) | Breaks completeness: `verify_records` → `chain_broken` + `checkpoint_invalid` on any covered range (deleted `seq` leaves a `prev_hash` gap; checkpoint `count` mismatches). Retention purge only gets away with this by deleting *whole ranges + their checkpoints together* under a no-holds condition — scattered erasure has no such boundary. | Strongest ("truly gone") | Destroys attestation for the **non-erased** rows sharing the range. Unacceptable for an auditable store. |
| **B. Redact-in-place** (`ALTER TABLE … UPDATE` the PII columns to a tombstone, leave `seq`/`prev_hash`/`entry_hash` intact) | **Passes `VerifyMode::Linkage`** (never recomputes content hashes — `decision_log.rs:441`); completeness + ordering stay provable. **Fails `VerifyMode::ByteExact`** vs the WORM/NDJSON archive (content no longer matches `entry_hash`). | Art. 17 is satisfied when the personal data is rendered irrecoverable; the audit metadata (that *a* decision happened, when, its outcome) is legitimately retained. | Moderate. Requires documenting the store's verification posture as linkage-based, and reaching (or exempting) the immutable archive. |
| **C. Crypto-shred** (destroy the per-tenant/subject AES key for the encrypted `input_data`/`replay_input` envelopes) | Ciphertext bytes unchanged → **both** verify modes still pass. | Only covers the encrypted blobs, not the plaintext `principal`/`resource` columns — partial unless those are also encrypted/pseudonymised. | Low for what it covers; incomplete alone. |

**Recommendation: B (redact-in-place) as the default decision-log strategy**,
because it is the only option that both satisfies erasure *and* preserves the
completeness guarantee the whole audit workstream (A1–A3) was built to provide.
Where a tenant runs `PrivacyProfile::Pseudonymize` (A5), `principal`/`resource`
are already `sha256:<hmac>` and the input blobs are AES-encrypted, so B naturally
composes with C (shred the key) for defence in depth. Hard-delete (A) is offered
only as an explicit, hold-checked escape hatch for tenants who accept
completeness gaps. **This is the one product/compliance decision to confirm
before slice 2.**

### The pseudonymisation-salt wrinkle *(resolved — follow-up #1)*
Under `PrivacyProfile::Pseudonymize`, stored `principal`/`resource` are
HMAC-SHA-256 of the plaintext with a per-tenant `hash_salt` that is
`skip_serializing` and lives **agent-side** (`decision_log.rs:1175`). To match a
subject in that tenant's rows, the control plane must hash the input subject with
the same salt. **Resolved via salt-in-request:** the erasure caller (who already
holds the privileged `audit:erase` scope) supplies the salt as `pseudonym_salt`;
the endpoint derives the tokens and matches the hashed columns alongside the
plaintext ones. This avoids a persistent salt escrow (a second copy of the secret
in the control-plane DB) and a synchronous agent round-trip; the trade-off is the
salt transits the (TLS-protected) request for that one call and is never stored.
The datastore path (entity ids are plaintext) is unaffected.

## Subject model

There is no first-class "data subject" type. A subject is a string matched across
two namespaces by convention:
- **Decision logs** — `principal` (and, PII-bearing, `resource`) columns.
- **DataStore** — `entity_id` (the `adm_entities` row) plus tuple `subject`/`object`
  and role-binding `subject` sides (all handled by `delete_entity_cascade`).

The endpoint takes an explicit subject selector (`principal` value and/or
`entity_id`, optional namespace scoping) rather than inferring a mapping that the
system does not record.

## Work breakdown (sliced by PR)

**Slice 1 — scaffolding + DataStore erasure (decision-independent).** *This slice.*
- Migrations `025_subject_erasure.sql` / `0018_subject_erasure.sql`: an
  `audit_erasure_requests` receipt table (id, org_id, subject, requested_by,
  requested_at, status, datastore_deleted, decisions_affected, strategy, completed_at).
- Domain model `ErasureRequest` / `ErasureReceipt` + repository.
- `POST /orgs/{org}/audit/erasure` in `api/audit.rs`: `org:admin` (or a new
  `audit:erase` scope — see open questions), wrapped in `idempotency::run`
  (`fingerprint(["audit.erase", org, subject])`), **legal-hold guarded**
  (refuse if a blanket hold or any hold whose filter covers the subject is
  active — mirror `run_org_purge`), `write_audit` with a new
  `audit.subject_erasure` action, returns the receipt.
- DataStore erasure: call `delete_entity_cascade` per namespace for the subject's
  `entity_id`; record `datastore_deleted` in the receipt.
- Decision-log step returns `strategy: pending` in slice 1 so the endpoint never
  claims to have erased the audit store before the strategy is wired.
- Tests: hold-guard refusal, idempotent replay, datastore cascade, receipt shape.

**Slice 2 — decision-log erasure (needs the strategy decision).**
- New `DecisionStore::erase_subject` beside `purge_expired`, reusing the
  parameterised hold-aware WHERE-builder, emitting the chosen mutation
  (redact-in-place `UPDATE` by default; hard-delete behind an explicit flag).
- Salt handling for pseudonymised tenants.
- SQL-builder unit tests (as `build_purge_sql` is tested — no live ClickHouse
  needed for the shape); document the linkage-based verification posture.

**Slice 3 — archive + historical versions (follow-up).**
- Reconcile with the immutable WORM/NDJSON archive and immutable published
  DataStore versions (both append-only today) — either an archive-rewrite path
  or a documented, receipted exemption.

## Open questions (confirm before/within the relevant slice)
1. **Decision-log strategy** (gates slice 2): redact-in-place (recommended) vs
   crypto-shred vs hard-delete escape hatch.
2. **Scope**: reuse `org:admin`, or add a dedicated `audit:erase` (following the
   `bundle:approve`/`bundle:promote` separation-of-duties precedent)?
3. **Pseudonymisation salt** access for control-plane-side subject matching.
4. **Legal-hold semantics**: holds today guard only decision logs. Does a hold
   also block the DataStore erasure, or are they independent stores?
