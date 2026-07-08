# Policy Integrity & Distribution Hardening
**Readiness gate:** NOT READY → CONDITIONAL (closes the distribution-plane P0/P1s) / **Priority:** P0 / **Findings closed:** Security P0-2, Security P1-1; Synthesis #3, #6; contributes to Synthesis #2 (two-person control on promotion) and Product F4 (change record on promote).

---

## 1. Goal

Make it impossible for any code path — pull *or* push — to load a policy bundle into a running agent unless it carries a valid, current, non-revoked signature from a pinned key, and make the act of promoting a bundle a governed, two-person, verifiable, recorded event.

Three concrete outcomes:
1. **One verification chokepoint.** Every in-process bundle/policy load entry point routes through a single `verify_and_load` function that fails closed. Today the *pull* path verifies (`sync.rs:517-518` → `verify_bundle_download` → `reaper_core::bundle_signing::verify_bundle`) but the *push* path (`handlers/policies.rs:308-383` `deploy_bundle`, `:395` `load_bundles_atomic`) parses bytes and hot-swaps with **no signature check at all** (Security P0-2).
2. **Anti-rollback + revocation.** The signed envelope gains a monotonic version and a validity window; the agent persists the highest-applied version and refuses regressions; a revocation list (by bundle hash and by `key_id`) is checked at load. Today `BundleSignature` (`bundle_signing.rs:75-86`) carries no version/expiry/revocation, and freshness is dedupe-by-checksum only (`client.rs:405-432`), so a compromised store/CDN/proxy can replay an old signed-but-revoked bundle (Security P1-1).
3. **Governed promotion.** Promotion requires authorization, two-person approval, pins the exact bundle+data version, produces an immutable change record, and supports a verifiable rollback. Today `promote_bundle` (`api/bundles.rs:200-207`) takes `State`/`Path`/`Json` only — no auth, no approver, no change record — and IDOR-reaches any bundle by global UUID (`_org_id` discarded, `api/bundles.rs:113,124,145`).

Reuse the good primitives: the signing module (`bundle_signing.rs` — two fail-closed checks, constant-time compare, algorithm + `key_id` pinning, no `alg:none`), the fail-closed pull-path policy matrix (`verify_bundle_download`, `sync.rs:548-588`), and the secure default `require_signed_bundles = true` (`config/settings.rs:350`).

---

## 2. Current state (evidence) — file:line

**Signing primitive (good, reuse as-is):**
- `crates/reaper-core/src/bundle_signing.rs:265-297` — `verify_bundle(bytes, sig, key, expected_key_id)`: parses algorithm, enforces algorithm match, optional `key_id` pin, SHA-256 integrity (constant-time, `:289`), then authenticity (`:296`). Fail-closed, no `alg:none`.
- `bundle_signing.rs:75-86` — `BundleSignature { algorithm, key_id, sha256, signature }`. **No version, no not-before/expiry, no revocation fields.** ← the schema gap for P1-1.
- `bundle_signing.rs:251-258` — `sign_bundle(bytes, key, key_id)` produces the envelope. `SIGNATURE_HEADER = "x-reaper-bundle-signature"` (`:44`).

**Pull path (verifies — the model to copy):**
- `services/reaper-agent/src/management/sync.rs:125-132` — `verify_download` reads `require_signed_bundles`, the parsed `verifying_key`, and `bundle_key_id` pin from config.
- `sync.rs:548-588` — `verify_bundle_download(require, key, key_id_pin, download)`: the fail-closed policy matrix (key+sig→verify; key,no-sig→reject if require; no-key→reject if require).
- `sync.rs:319, 518` — the two call sites; `sync_bundle` (`:488-537`) calls `verify_download` **before** applying (`:517-518`).
- `sync.rs:70-99` — pinned `VerifyingKey` parsed once from `config.bundle_public_key`/`bundle_signature_algorithm`; bad key logs and leaves `None` so verification fails closed.
- `services/reaper-agent/src/management/client.rs:405-432` — `check_for_update` dedupes by `current_bundle_id`/`checksum` only. **Nothing rejects an older validly-signed bundle** (rollback gap). `set_current_bundle` (`:435-439`) tracks only `(bundle_id, checksum)` in memory, not persisted, not a monotonic version.

**Push path (does NOT verify — the P0):**
- `services/reaper-agent/src/handlers/policies.rs:308-383` — `deploy_bundle`: `PolicyBundle::from_bytes(&payload.bundle)` → `deploy_bundle_with_store(...)`. No `BundleSignature`, no `verify_bundle`, no `require_signed_bundles`.
- `handlers/policies.rs:395-...` — `load_bundles_atomic`: same, for the full-replace path; loops `PolicyBundle::from_bytes` with no verification.
- `services/reaper-agent/src/types.rs:122-143` — `DeployBundleRequest { bundle: Vec<u8>, version: String, force: bool }` and `LoadBundlesRequest { bundles: Vec<Vec<u8>> }` — **no signature field** in either request.

**What the handler CAN reach for the chokepoint:** `AgentState` (`state.rs:22-47`) holds `agent_config: ReaperAgentConfig`; `config.management` (a `ManagementSettings`, used at `main.rs:299-316`) carries `bundle_public_key`, `bundle_signature_algorithm`, `bundle_key_id`, `require_signed_bundles`. So the deploy/load handlers already have access to the same signature config the SyncService uses — the pinned key just needs to be parsed once and shared.

**Management-side signing + promotion:**
- `services/reaper-management/src/bundle/service.rs:45-46` — `sign(bytes)` → `bundle_signing::sign_bundle`. `:216` `compile`, `:344` `promote`, `:425-440` `download` (deserializes stored `BundleSignature`).
- `services/reaper-management/src/api/bundles.rs:200-207` — `promote_bundle`: no `RequireAuth`, no approver, no change record; `_org_id` discarded → acts on global `bundle_id` (IDOR, P0-3b). `:60-63` route.
- `api/bundles.rs:220-254` — `download_bundle` ships the detached signature via `SIGNATURE_HEADER` (`:244`).

**Config default (good, keep):** `crates/reaper-core/src/config/settings.rs:326,350` — `require_signed_bundles: true` by default; `:308-319` the key/alg/key_id fields.

---

## 3. Definition of Done — testable checkboxes

- [ ] A single function (`verify_and_load_bundle`) is the **only** path that inserts a bundle into `PolicyEngine`; `deploy_bundle`, `load_bundles_atomic`, and the SyncService apply-step all call it. Verified by grep: no `deploy_bundle_with_store` / `to_enhanced_policy_with_store` call outside that function.
- [ ] `POST /api/v1/bundles/deploy` and `/bundles/load` **reject an unsigned bundle** with 400/422 when `require_signed_bundles = true` (the default). (unsigned push → rejected)
- [ ] The same endpoints reject a bundle whose signature is valid but whose **envelope version ≤ the highest version this agent has applied** for that policy/bundle lineage. (replay old signed bundle → rejected)
- [ ] The same endpoints reject a bundle whose **`not_before` is in the future or `expires_at` is in the past**.
- [ ] The same endpoints reject a bundle whose **bundle hash or signing `key_id` appears on the revocation list**. (revoked key → rejected)
- [ ] The highest-applied version is **persisted** across agent restart (survives process bounce; a downgrade after restart is still rejected).
- [ ] Envelope schema `BundleSignature` carries `envelope_version` (schema tag), `version` (monotonic u64), `not_before`, `expires_at`, and `bundle_id`; the signature covers these fields (they are inside the signed message, not appended to the JSON envelope only). Old-format envelopes without these fields are rejected when `require_signed_bundles = true`.
- [ ] A revocation list is fetched, cached, signed by the control plane, and checked at load; a stale/unreachable list **fails closed** in `Enforce` mode and is configurable.
- [ ] `promote_bundle` requires `RequireAuth` + `RequireScope(BundlePromote)`, enforces `user.org_id == bundle.org_id` (or platform Admin), and returns 404 on org mismatch (closes IDOR for the promote verb).
- [ ] Promotion requires a **second distinct approver** before the rollout is triggered (two-person control), enforced server-side, not client-advisory.
- [ ] Every promotion writes an immutable **change record** (actor, approver, bundle_id, bundle version, data version, from/to status, timestamp) to the management audit log.
- [ ] Rollback re-promotes a prior recorded bundle version through the *same* verified path and is itself a recorded, approved change (verifiable rollback).
- [ ] Unit + integration tests cover: unsigned push rejected; replayed old version rejected; expired envelope rejected; revoked hash rejected; revoked key_id rejected; happy-path signed+current+authorized promote succeeds.

---

## 4. Critical steps — ordered

### Step 1 — Extend the signed envelope schema (anti-rollback + validity)
**What:** Add fields to `BundleSignature` and fold them into the signed message so they are authenticated, not just decorative.

New/changed envelope schema (`bundle_signing.rs:75-86`):
```
BundleSignature {
    envelope_version: u8,        // NEW: schema tag, currently 2; v1 = legacy (algorithm/key_id/sha256/signature only)
    algorithm: String,           // unchanged
    key_id: String,              // unchanged (already usable as revocation subject)
    bundle_id: String,           // NEW: UUID this envelope is bound to (prevents cross-bundle replay)
    version: u64,                // NEW: monotonic per bundle lineage; agent refuses non-increasing
    not_before: String,          // NEW: RFC3339; reject if now < not_before
    expires_at: String,          // NEW: RFC3339; reject if now > expires_at
    sha256: String,              // unchanged (integrity of bundle bytes)
    signature: String,           // now signs canonical(metadata_fields) || bundle_bytes
}
```
Signing change: `sign_bundle` signs a canonical serialization of `{envelope_version,bundle_id,version,not_before,expires_at,sha256}` concatenated with the bundle bytes, so tampering with any metadata field breaks authenticity. `verify_bundle` gains parameters/logic to (a) recompute over the same canonical bytes, (b) enforce `not_before`/`expires_at` against a caller-supplied `now`, (c) return the parsed `version`/`bundle_id` to the caller so the anti-rollback check can run outside the crypto core. Add `SignatureError::Expired`, `SignatureError::NotYetValid`, `SignatureError::EnvelopeVersionUnsupported`, `SignatureError::BundleIdMismatch`.
**Where:** `crates/reaper-core/src/bundle_signing.rs` (`BundleSignature`, `sign_bundle`, `verify_bundle`, `SignatureError`). Management signer `services/reaper-management/src/bundle/service.rs:45-46` supplies `version`/`bundle_id`/window at sign time (version sourced from the bundle's monotonic version column; add one if absent).
**Verify:** Existing `bundle_signing.rs` tests still pass for authenticity/integrity; add tests: expired envelope rejected, future `not_before` rejected, tampered `version` field breaks signature, `bundle_id` mismatch rejected, legacy v1 envelope rejected when strict.

### Step 2 — Build the single verification chokepoint
**What:** Introduce one function all in-process loads route through:
```
fn verify_and_load_bundle(
    state: &AgentState,
    raw: &[u8],
    sig: Option<&BundleSignature>,
    ctx: LoadContext,   // Push { force } | Pull | AtomicReplace
) -> Result<AppliedVersion, LoadError>
```
It performs, in order, fail-closed: (1) resolve the pinned `VerifyingKey` + `require_signed_bundles` + `bundle_key_id` pin (parse once at startup, store on `AgentState`; reuse the parse logic at `sync.rs:70-99`); (2) run the pull-path matrix `verify_bundle_download` semantics (extract that logic into a shared helper so push and pull share it verbatim); (3) `verify_bundle` (integrity + authenticity + window from Step 1); (4) **anti-rollback** check against the persisted highest-applied version (Step 3); (5) **revocation** check (Step 4); (6) only then `PolicyEngine::deploy_bundle_with_store` / `to_enhanced_policy_with_store`. Move the existing `verify_bundle_download` free function (`sync.rs:548-588`) into a shared module (e.g. `reaper-agent/src/management/verify.rs`) so both the sync service and the HTTP handlers depend on the same code — no second implementation.
**Where:** new `services/reaper-agent/src/management/verify.rs` (or `handlers/load.rs`); refactor `handlers/policies.rs:308-383` (`deploy_bundle`) and `:395` (`load_bundles_atomic`) to call it; refactor `sync.rs:517-518` to call the same function. Add parsed `verifying_key` + signature policy to `AgentState` (`state.rs:22-47`) so handlers don't re-parse.
**Verify:** grep confirms `deploy_bundle_with_store`/`to_enhanced_policy_with_store` appear only inside the chokepoint. Integration test: unsigned `POST /bundles/deploy` → rejected; signed+current → applied.

### Step 3 — Anti-rollback (persisted monotonic version)
**What:** Persist per-lineage highest-applied `version` on the agent (keyed by `bundle_id` or policy lineage id). On load, reject if `incoming.version <= highest_applied` unless `ctx == Push{force:true}` AND the caller is authorized (force still requires a valid signature; force only overrides monotonicity, never the signature). Replace the checksum-only dedupe in `client.rs:405-432` with a version-aware check. Persist to a small on-disk file / sled / the existing policy cache dir (`AgentState.policy_cache`) so it survives restart.
**Where:** `services/reaper-agent/src/management/client.rs:405-439` (`check_for_update`, `set_current_bundle`); persistence next to `policy_cache` (`state.rs:36`); the check lives inside `verify_and_load_bundle` (Step 2).
**Verify:** Apply v5, then attempt v4 → rejected; restart agent, attempt v4 again → still rejected (persisted).

### Step 4 — Revocation list (format + distribution)
**What:** A signed revocation document the agent fetches and caches, checked at load.

Revocation list format (JSON, itself signed with the bundle signing key so a CDN/proxy can't forge it):
```
RevocationList {
    issued_at: String,            // RFC3339
    serial: u64,                  // monotonic; agent rejects a list older than the last seen serial
    next_update: String,          // RFC3339; after this, list is "stale"
    revoked_bundle_hashes: [String],   // lowercase-hex sha256 of bundle bytes
    revoked_key_ids: [String],         // key_ids whose bundles are all distrusted
    signature: BundleSignature,        // detached signature over the canonical list body
}
```
Distribution: **list-pull, not OCSP-style per-request.** The agent pulls the list from the control plane (`GET /orgs/{org}/revocations`) on the existing sync poll/SSE cadence (`sync.rs:151-163`) and caches it; the management plane serves it signed and re-signs on change. Rationale in §8. Staleness policy: if the cached list is past `next_update` and cannot be refreshed, behavior follows a config knob mirroring the data-plane `StalenessMode` (`state.rs:58-63`) — default `Monitor` (warn), with `Enforce` (fail-closed: refuse all loads) available for regulated deployments. The check: a bundle is refused if its `sha256` ∈ `revoked_bundle_hashes` or its `key_id` ∈ `revoked_key_ids`.
**Where:** new management endpoint `services/reaper-management/src/api/` (revocations) + storage; agent fetch/cache in `services/reaper-agent/src/management/` (new `revocation.rs`); check invoked inside `verify_and_load_bundle` (Step 2). Reuse `bundle_signing::verify_bundle` to validate the list's own signature.
**Verify:** Add `key_id=k1` to `revoked_key_ids`, push a k1-signed bundle → rejected; revoke a specific bundle hash → that bundle rejected, a different current bundle accepted; stale list in `Enforce` mode → all loads refused.

### Step 5 — Govern promotion (authz + two-person + change record + rollback)
**What:**
- Add `RequireAuth` + `RequireScope(BundlePromote)` to `promote_bundle` and scope the bundle lookup by `org_id` (fix IDOR: bind `_org_id`, return 404 on mismatch) — mirrors the correct pattern at `datastore.rs:118`.
- Model promotion as a **change request**: `POST …/promote` creates a pending change request (bundle_id + pinned bundle version + pinned data version + requester), it does **not** immediately start the rollout.
- Two-person control: a second, distinct principal calls `POST …/change-requests/{id}/approve`; the server rejects self-approval (`approver_id != requester_id`). Only on approval does the existing rollout machinery (`api/deployments/*`) fire. Reuse the strategy `require_approval` gate if present.
- Change record: on create/approve/execute, write an immutable audit row (actor, approver, org, bundle_id, bundle version, data version, from_status→to_status, decision_id, timestamp) to the management audit log.
- Verifiable rollback: rollback = create a change request that re-promotes a *previously recorded* bundle version; it flows through the same approval + the same agent-side verified load path (Steps 1-4), so a rollback target that is now revoked or expired is refused. Record it as its own change.
**Where:** `services/reaper-management/src/api/bundles.rs:200-207` (`promote_bundle`), `services/reaper-management/src/bundle/service.rs:344` (`promote`), `services/reaper-management/src/api/deployments/*` (rollout trigger), management audit log module. The **who-may-promote / who-may-approve** decision depends on the roles/scopes delivered by plan `01-authn-authz-foundation.md` (see §5).
**Verify:** anonymous promote → 401; promote in org A of a bundle in org B → 404; requester approving own request → 403; approved-by-second-user → rollout starts and change record exists; rollback of a now-revoked version → refused at the agent with a recorded reason.

### Step 6 — Deployment surface for signatures on the push path
**What:** Extend `DeployBundleRequest`/`LoadBundlesRequest` (`types.rs:122-143`) to carry the `BundleSignature` (body field, or accept it via the `SIGNATURE_HEADER` the download path already uses, `bundle_signing.rs:44`). The push handlers pass it into `verify_and_load_bundle`. When `require_signed_bundles = true` and no signature is present → reject.
**Where:** `services/reaper-agent/src/types.rs:122-143`; `handlers/policies.rs:308,395`.
**Verify:** signed push with valid envelope succeeds; push omitting the signature field rejected under default config.

---

## 5. Dependencies

- **`01-authn-authz-foundation.md` (hard dependency for Step 5).** Governed promotion needs *identity* and *scopes*: `RequireAuth`, a `BundlePromote`/`BundleApprove` scope, `user.org_id`, and platform-Admin distinction. This plan assumes those primitives exist (the security review confirms the pattern already exists at `datastore.rs:118` and is simply not applied to `bundles.rs`). Steps 1-4 (envelope, chokepoint, anti-rollback, revocation) are **independent of plan 01** and can land first — they are pure data-plane integrity and do not need the human-auth work.
- **Agent-plane auth (plan 01) is complementary but not blocking here.** Even with the push path unauthenticated today, routing it through signature verification (Steps 1-3,6) removes the P0-2 takeover: an attacker can reach the endpoint but cannot get an unsigned/rolled-back/revoked bundle applied. Auth on the agent (plan 01) is defense-in-depth on top.
- **Shares the signing key material** with the existing pull path and `bundle/service.rs` signer — no new key system; extends the existing one.
- **Reuses:** `bundle_signing.rs` primitive, `verify_bundle_download` matrix, `require_signed_bundles=true` default, the SSE/poll sync cadence, the rollout/strategy machinery in `api/deployments/*`, and the management audit log.

---

## 6. Testing & verification

**Unit (crate `reaper-core`, `bundle_signing.rs`):**
- Envelope round-trip with new fields; tampering any metadata field breaks authenticity.
- Expired `expires_at` → `SignatureError::Expired`; future `not_before` → `NotYetValid`.
- Legacy v1 envelope rejected when strict; accepted only if `require_signed_bundles=false`.

**Integration (agent, the three headline cases from the brief):**
- **Replay an old signed bundle → rejected.** Apply v5 (valid signature), then POST a genuinely-signed v4 → 409/422 anti-rollback; repeat after agent restart → still rejected (persistence).
- **Unsigned push → rejected.** `POST /api/v1/bundles/deploy` and `/bundles/load` with no signature under default `require_signed_bundles=true` → 400/422; confirm no policy swap occurred (`GET /api/v1/policies` unchanged).
- **Revoked key → rejected.** Publish a revocation list with the bundle's `key_id`; push a correctly-signed, current-version bundle by that key → rejected. Same for a single revoked bundle hash. Stale list in `Enforce` → all loads refused.
- **Chokepoint proof:** grep test / architecture test asserting `deploy_bundle_with_store` and `to_enhanced_policy_with_store` are only called from `verify_and_load_bundle`.
- **Pull path regression:** existing `sync.rs` tests (`:590+`, `wrong_key_id_pin_is_rejected` at `:655`) still pass after refactor to the shared helper.

**Integration (management, promotion governance):**
- Anonymous promote → 401; cross-org promote (IDOR) → 404; self-approval → 403; two-person happy path → rollout fires + audit record present.
- Rollback to a now-revoked/expired version → refused at agent, refusal recorded.

**End-to-end:** management sign → promote (approved) → agent pull → verified load → decision reflects new bundle; then attempt a downgrade via the push endpoint → rejected while the pulled current version keeps serving.

---

## 7. Effort & phasing

- **Phase A (S–M): Steps 1, 2, 6 — envelope schema + chokepoint + push-path signature field.** Closes Security P0-2 (the signing bypass) — the highest-severity item. Mostly refactor + additive fields; the signing primitive already exists.
- **Phase B (M): Step 3 anti-rollback (persistence) + Step 4 revocation list.** Closes Security P1-1. New but small subsystems (persisted counter, signed list fetch/cache). Revocation list is the larger part (new endpoint + agent fetch/cache + staleness policy).
- **Phase C (M, gated on plan 01): Step 5 governed promotion.** Closes the promotion-authorization + two-person + change-record + verifiable-rollback goals (Synthesis #3/#6, Product F4). Depends on plan 01 auth primitives; the rollout machinery it terminates in already exists, so this is a governance layer, not a rewrite.

Overall: **M** (Phase A alone removes the P0 quickly; B and C follow).

---

## 8. Key decisions (ADR-style)

**ADR-1 — Envelope versioning scheme.** *Decision:* add an `envelope_version: u8` schema tag (v2) plus a per-lineage monotonic `version: u64` that is **inside the signed bytes**, and a `bundle_id` binding. *Alternatives:* (a) reuse the bundle's own `metadata.policy_version` string — rejected: not authenticated, string-typed, not guaranteed monotonic; (b) use a signed timestamp only for anti-rollback — rejected: clock skew makes it fragile and a fast re-issue could tie. *Rationale:* a monotonic integer in the signed message gives an unambiguous "not-older-than" test; `bundle_id` binding stops cross-lineage replay; the schema tag lets us reject legacy v1 envelopes explicitly and evolve later. *Trade-off:* the control plane must own a monotonic version source per bundle lineage (add a column if absent).

**ADR-2 — Revocation distribution: list-pull vs OCSP-style.** *Decision:* **signed revocation list, pulled on the existing sync cadence and cached**, checked locally at load. *Alternatives:* OCSP-style per-load online check — rejected: puts a network round-trip and a hard control-plane dependency on the sub-µs load/hot path, fails badly for air-gapped/edge agents, and creates a DoS lever. *Rationale:* agents already poll/SSE for bundle promotions (`sync.rs:151-163`); revocation rides the same channel with no new hot-path dependency; a signed list with a monotonic `serial` and `next_update` prevents a stale-list replay and lets operators choose fail-open (`Monitor`) vs fail-closed (`Enforce`) mirroring the data-plane `StalenessMode`. *Trade-off:* revocation is eventually-consistent (bounded by poll interval), not instant — acceptable and documented; `Enforce` mode plus a short `next_update` tightens it for regulated deployments.

**ADR-3 — Two-person-control mechanism.** *Decision:* enforce separation **server-side** via a change-request object: create (requester) and approve (a distinct principal) are separate authenticated calls; the server rejects `approver_id == requester_id`; only approval triggers the rollout. *Alternatives:* (a) client/UI-advisory dual-control — rejected: not enforceable, not auditable; (b) N-of-M threshold signatures on the bundle itself — rejected as over-engineered for v1, but the change-request model leaves room to add it. *Rationale:* reuses the existing rollout/approval primitives (`api/deployments/*`, strategy `require_approval`) and the management audit log, so the governance layer sits on top of proven machinery; every transition is an immutable record answering "who promoted what, approved by whom, at time T" — the regulator's question. *Dependency:* the identity/scope model from plan 01.

---

## 9. Risks & rollback

- **Envelope schema change breaks in-flight bundles.** Mitigation: `envelope_version` tag + a bounded migration window where the management plane re-signs stored bundles into v2; agents accept v1 **only** while `require_signed_bundles=false` (non-production) and hard-reject v1 once strict. Ship the crate change (Step 1) before flipping any agent to strict-v2.
- **Anti-rollback could brick a legitimate emergency downgrade.** Mitigation: `Push{force:true}` overrides monotonicity but **never** the signature/revocation checks, and every force is authorized + audited (via Step 5). Documented runbook: to move backward, re-sign the older policy content as a *new higher* version (the correct pattern) rather than force-downgrade.
- **Revocation list becomes a new availability dependency.** Mitigation: default `Monitor` (warn, keep serving last-good) so a revocation outage never takes down enforcement; `Enforce` is opt-in for shops that prefer fail-closed. Signed list + monotonic `serial` prevents a downgrade-the-list attack.
- **Refactor to a single chokepoint risks regressing the working pull path.** Mitigation: extract `verify_bundle_download` into the shared module unchanged and keep its existing tests (`sync.rs:590+`) green; add architecture test asserting no bypass call sites.
- **Rollback of this workstream:** each phase is independently revertible. Phase A is additive (new fields default-absent → treated as unsigned → rejected only under strict mode, which is already the default; if a hotfix is needed, operators can set `require_signed_bundles=false` to restore prior push behavior, at the cost of the P0 reopening — a conscious, logged operator choice). Phases B and C are new endpoints/checks that can be feature-flagged off without touching the eval hot path.
