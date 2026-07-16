# GA Hardening — Edge Hygiene & Scale Cliffs

> **STATUS: 📋 PLANNED** — round-3 remediation. This is the P2 *finishing* cluster:
> the residual closure-completeness and scale-cliff items that the third-pass code
> (`reviews/round-3/03-code-and-api.md`) and performance (`reviews/round-3/01-performance.md`)
> reviews leave open. None is a new pillar; each is the last mile of a pillar that
> already shipped. This plan does **not** touch the round-3 P0 lanes
> (tenant-isolation authz, release/supply-chain integrity) — those are owned by
> sibling round-3 plans and are the hard gate to CONDITIONAL. This plan is what
> converts **CONDITIONAL → READY** once those land.

**Readiness gate:** Blocks CONDITIONAL → READY. A regulated architecture/SRE review will not pass a product that (a) parks request tasks forever on a hung upstream, (b) ships a list endpoint that returns the whole fleet in one array, (c) has a documented latency SLO that silently inverts into a *denial* cliff for the exact policy shapes the product is sold on, (d) burns a core on redundant crypto on the reactor, or (e) advertises a billing flow that fabricates checkout URLs.
**Priority:** P2 (GA-blocking cluster). No P0/P1 in this plan's scope; every item is finishing work on a sound system.
**Findings closed:** Code R3-01, R3-02, R3-04, R3-05, R3-06, R3-07; Performance R3-P2-1, R3-P2-2, R3-P3-1 (folded in — it is one line and on the same fan-out path). Synthesis "just below the line": PERF P2 (pruning inoperative for ABAC/ReBAC).

---

## 1. Goal

Close the round-3 P2 residue so the READY checklist is *honestly* true, not true-with-asterisks:

1. **Universal client-timeout policy** — no outbound HTTP call anywhere in the workspace can hang the awaiting task. Every `reqwest` client is built through one helper with a mandatory timeout; a grep-lint keeps bare `Client::new()` out of non-test code permanently.
2. **Pagination is actually universal** — every list endpoint, including `/orgs/{org}/pins` and the config-cardinality lists, enforces a bounded page size, so Plan 07's "every list endpoint is paginated" claim is finally not overstated.
3. **ABAC/ReBAC evaluate-all holds its SLA at scale** — the served-path pruning index becomes effective for attribute/relation policy shapes (not only literal-resource RBAC), removing both the per-request O(N log N) sort and the 256-candidate denial cliff. No silent availability incident at 10k realistic policies.
4. **Capability verification is cached / off the hot reactor** — steady-state agentic traffic becomes a hash lookup; the cold verify moves off the tokio worker; an asymmetric-cost authenticated-DoS vector is bounded.
5. **The billing stub cannot mislead** — the fabricated-checkout surface is feature-gated off by default and excluded from the published contract until implemented.
6. **Public contract hygiene** — growable public SDK/core enums are `#[non_exhaustive]`; the untyped-contract ratchet baselines are driven down; the bundle ETag stops being a wall-clock string.

Non-goal: implementing real Stripe billing; rewriting the DSL compiler; the round-3 P0 authz/supply-chain work (sibling plans). This plan changes *robustness and honesty of the contract*, not product surface.

## 2. Current state (evidence) — file:line

- **Outbound calls with no timeout (R3-01 / R2-07 half-fixed).** Bare `reqwest::Client::new()` (no default timeout, no per-request `.timeout()`) survives at:
  - `services/reaper-management/src/decisions/mod.rs:255` — the ClickHouse `DecisionStore` client; `run()` (`:275-298`) sets no per-request timeout. This is the **audit read/query path** (`/orgs/{org}/decisions*`); a wedged ClickHouse parks every decision-query task with no deadline.
  - `services/reaper-management/src/api/oauth/github.rs:124` and `:412` — OAuth token exchange + user fetch.
  - `services/reaper-management/src/sync/github_app.rs:119` — GitHub App installation-token minting.
  - `services/reaper-agent/src/management/sse.rs:154` — agent→management SSE request (found this round; same class, not previously flagged).
  - Degrading fallbacks: `sync/bundle_url.rs:71`, `sync/s3.rs:57`, `sync/api.rs:38` each do `builder…unwrap_or_else(|_| reqwest::Client::new())` — a builder error silently drops the timeout property.
  - Timeouts *are* present at ServiceNow (`integrations/servicenow.rs:62`), JWKS (`auth/jwks.rs:149`), SSO (`api/auth/sso.rs:557`), webhook (`webhook/service.rs:61`), SIEM (`siem/mod.rs:69`) — proving the pattern exists; it is just not universal.
- **Not every list endpoint is paginated (R3-02).** `list_pins` returns `Json<Vec<PinResponse>>` with no `PageQuery` (`api/deployments/pins.rs:152`); its repo query has **no `LIMIT`** (`db/repositories/deployment/pins.rs:85`, `SELECT … ORDER BY … fetch_all`). One pin per pinned agent → at the fleet scale the product targets (thousands of agents) this returns the whole set in one array. Also unbounded (config-cardinality, lower urgency): `list_environments` (`environments.rs:44`), `list_webhooks` (`webhook_subscriptions.rs:133`), `list_strategies` (`deployments/strategies.rs:32`), `list_agent_subscriptions` (`namespaces.rs:505`), `capabilities.rs`, `revocations.rs`. The big-table lists (entities/tuples/bindings/decisions) are already keyset-correct (`db/repositories/datastore.rs:541,702,869`) — the primitives (`PageQuery`/`Paginated`) are proven on `agents`/`policies`.
- **Pruning index inoperative for ABAC/ReBAC (R3-P2-1).** `compiled_resource_index_terms` extracts terms only from `CompiledCondition::ResourceIdEquals` (`reaper_dsl/mod.rs:1867-1917`). Attribute/relation predicates (`resource.type == "document"`, `resource.owner == principal`, `has_relation(...)`, wildcards, dynamic ids) return `None` → the policy lands in `unprunable` (`engine/mod.rs:170-173`). `candidate_policy_ids` (`engine/mod.rs:337-351`) then returns the resource bucket **plus every unprunable policy** and does `ids.sort(); ids.dedup()` per request. At 10k realistic DSL policies: `unprunable.len() == 10_000` → ~O(10⁴·log 10⁴) ≈ 130k comparisons/request, and with the default `max_candidate_policies = 256` (`settings.rs:362`) every evaluate-all request trips `candidate_cap_exceeded` → **blanket deny** (`evaluate.rs:358`). The soundness is fine; the **coverage** is the defect. `crates/policy-engine/src/partial_evaluation.rs` exists in-tree and **unused** — it is exactly the asset needed (R3-P3-3).
- **Redundant per-candidate arc-swap load (R3-P3-1).** `evaluate_set` calls `self.get_policy(id)` per candidate (`engine/mod.rs:497-511`) → one `ArcSwap::load` per policy in a fan-out, instead of loading the `ActiveSet` snapshot once. Same fan-out path; fold in here.
- **Capability verify: uncached, inline on the reactor (R3-P2-2).** `evaluate_policy` calls `capability_gate::enforce(...)` at `evaluate.rs:207` — directly in the async handler, **not** `spawn_blocking`. `enforce` → `verify_capability` (`capability_gate.rs:71`) → `verify_at` (`capability.rs:348-350`): hex-decode signature, rebuild canonical message, `verifying_key.verify_raw(...)` — a full ed25519 verify (~30-50µs CPU) **on every agentic request**, with **no verdict cache** (`verify.rs:142-158`). Redundant (same capability re-verified every call: ~0.15-0.25 core/s at 5k rps), reactor-blocking, and asymmetric-cost DoS (garbage signature still costs a full verify; `/api/v1/messages` is behind `bearer_jwt` so this is an *authenticated* DoS). The SLO harness scenarios are non-agentic, so this path has never been load-tested.
- **Billing is a published stub (R3-04).** `/orgs/{org}/billing/checkout` and `/portal` are mounted (`api/mod.rs:86`), OpenAPI-documented, and return fabricated `cs_placeholder_*` sessions (`billing/service.rs:198-210`); `/webhooks/stripe` is a no-op that returns 200 **without verifying the signature** (`billing/service.rs:298-323`). The contract advertises a working billing flow that does not exist.
- **Public-enum / doc discipline still open (R3-05 / R2-09).** Only `ApiError` is `#[non_exhaustive]` (2 hits workspace-wide). `PolicyLanguage` (`engine/types.rs:38`), SDK `Decision`/`Source` (`reaper-sdk/src/types.rs:38,48`), `Transport` (`reaper-sdk/src/transport.rs:11`), and `reaper-core` public enums are unsealed; no `#[deny(missing_docs)]` anywhere, including the published SDK.
- **Contract-quality ratchet baselines still high (R3-06 / R2-06 partial).** The `contract_is_publishable` gate hard-fails on an undocumented error model, but the shipping spec still carries **94 untyped success bodies** and **129 operations with no documented 4xx** (`tests/api_contract.rs:226-227`, `UNTYPED_SUCCESS_BASELINE=94`, `MISSING_4XX_BASELINE=129`).
- **Bundle ETag is a wall-clock string (R3-07 / R2-10).** `updated_at.to_rfc3339()` (`api/bundles.rs:218,281`) — sub-resolution rapid edits share a tag, defeating the `WHERE updated_at=$expected` guard. Policies already migrated to a monotonic `row_version` (`db/repositories/policy.rs:380-404`); bundles did not.

## 3. Definition of Done — testable checkboxes

- [ ] **No unbounded outbound call.** A single `http_client(timeout)` (and/or `http_client_default()`) helper is the only constructor of a `reqwest::Client` in non-test code. All eight sites in §2 build through it. A `ci.yml` grep-lint fails the build on `reqwest::Client::new()` (or `ClientBuilder…build().unwrap_or_else`) outside `#[cfg(test)]`. The ClickHouse `run()` sets a per-request `.timeout()`.
- [ ] **Every list endpoint is paginated.** `list_pins` returns `Paginated<PinResponse>` with a keyset cursor and the repo query has `ORDER BY (created_at, id) LIMIT $n`. `environments`, `webhooks`, `strategies`, `agent_subscriptions`, `capabilities`, `revocations` each enforce a default page size (50) and hard max (200), reject `limit>max` with 400, and return `next_cursor`. The `tests/api_contract.rs` list-endpoint audit (see step) enumerates every `#[utoipa::path]` GET that returns a collection and asserts it carries a `PageQuery` param + `Paginated` response — Plan 07 Phase-E's claim becomes machine-checked.
- [ ] **ABAC/ReBAC evaluate-all holds SLA at scale.** With 10k attribute/relation policies and evaluate-all enabled, no request returns `candidate_cap_exceeded` purely because the policies are unprunable; `candidate_policy_ids` does not sort/dedup the full unprunable set per request; the evaluate-all p99 stays within the §3-SLO evaluate-all row (≤25µs) in a new `slo-harness.yml` ABAC/ReBAC scenario. A property test asserts the type/prefix prefilter is still a *superset* of the true match set (never fails open), mirroring the existing `reaper_dsl/tests.rs:301-408` literal-resource property.
- [ ] **Capability verify is cached / off-hot-path.** A positive-verdict cache keyed on `(capability_id, key_id, signature_bytes, expiry, revocation_generation)` turns a repeated capability into a hash hit (no `verify_raw`); the cache invalidates when the revocation generation bumps. The cold verify runs off the reactor (`spawn_blocking` above a threshold) and/or a per-tenant cap-verify rate limit bounds garbage-signature floods. A new agentic (capability-per-request) `slo-harness.yml` scenario produces a number; a benchmark shows steady-state agentic throughput is not gated by ed25519 cost.
- [ ] **Billing stub cannot mislead.** The billing router is behind a `billing` cargo feature / `enable_billing` config, **default off**; when off the routes are unmounted and excluded from `/openapi.json` (parity gate green in both configs). The Stripe webhook, if mounted at all, verifies the signature or returns 501, never a silent 200. When on, the spec marks the operations `x-experimental`.
- [ ] **Enums sealed, ETag monotonic, ratchets lowered.** `#[non_exhaustive]` on `PolicyLanguage`, SDK `Decision`/`Source`/`Transport`, and the growable `reaper-core` public enums; `#[deny(missing_docs)]` (or `#[warn(...)]` with a CI `-D`) on `reaper-sdk` and `reaper-core`. Bundle ETag derives from a monotonic `row_version` column, not `updated_at`. `UNTYPED_SUCCESS_BASELINE` and `MISSING_4XX_BASELINE` in `tests/api_contract.rs` are strictly lower than 94/129 (target: 0; this plan commits to a measurable ratchet-down, phased).

## 4. Critical steps — ordered; per step what/where(files)/verify

1. **Universal HTTP client helper + grep-lint (R3-01).**
   - What: Add `http_client(timeout: Duration) -> reqwest::Client` (and a `http_client_default()` using a workspace default, e.g. 10s connect / 30s total) in a shared module (e.g. `reaper-management/src/http.rs`; a mirror for `reaper-agent`, or a tiny helper in `reaper-core`). Route all eight sites in §2 through it. For the sync builders, remove the `unwrap_or_else(|_| Client::new())` fallback — a builder error must propagate, not degrade to no-timeout. Set a per-request `.timeout()` inside the ClickHouse `run()` for defense-in-depth. Add a `ci.yml` step: `grep -rn 'reqwest::Client::new()' --include='*.rs' | grep -v '#\[cfg(test)\]'` → fail if non-empty (or a clippy `disallowed-methods` entry in `clippy.toml`).
   - Where: new `services/reaper-management/src/http.rs`; `decisions/mod.rs:255,275-298`, `api/oauth/github.rs:124,412`, `sync/github_app.rs:119`, `sync/bundle_url.rs:71`, `sync/s3.rs:57`, `sync/api.rs:38`; `services/reaper-agent/src/management/sse.rs:154`; `.github/workflows/ci.yml` (or `clippy.toml`).
   - Verify: grep-lint green; a unit/integration test points a client at a black-hole port and asserts it errors within `timeout + ε`, not hangs.

2. **Paginate `/orgs/{org}/pins` (R3-02, primary).**
   - What: Thread the existing `PageQuery` extractor into `list_pins`; return `Paginated<PinResponse>` with a keyset cursor over `(created_at, agent_id)`. Change `PinRepository::list` to `… WHERE a.org_id=$1 ORDER BY vp.created_at, vp.agent_id LIMIT $n` with a cursor predicate, exactly as `agents`/`policies`/datastore lists already do.
   - Where: `api/deployments/pins.rs:152`, `db/repositories/deployment/pins.rs:85`.
   - Verify: seed 500 pins; `limit=10000` → 400; page through with cursors, each pin exactly once, no drift after an insert.

3. **Paginate the config-cardinality lists (R3-02, consistency).**
   - What: Same `PageQuery`/`Paginated` treatment for `list_environments`, `list_webhooks`, `list_strategies`, `list_agent_subscriptions`, `capabilities`, `revocations`. Add a `LIMIT` to each repo query.
   - Where: `environments.rs:44`, `webhook_subscriptions.rs:133`, `deployments/strategies.rs:32`, `namespaces.rs:505`, `capabilities.rs`, `revocations.rs`, and their repos.
   - Verify: per-endpoint default-size + limit-cap tests. Add the **list-endpoint fitness function** to `tests/api_contract.rs`: enumerate collection-returning GETs from the spec and assert each has a bounded page param — makes a future unpaginated list a red build (the durable control that keeps Plan 07's claim honest).

4. **Make ABAC/ReBAC policies prunable — resource-type/prefix index tier (R3-P2-1).**
   - What: Extend term extraction so a compiled condition yields resource *type* and literal-*prefix* buckets, not only `ResourceIdEquals`. Wire in the in-tree, currently-dead `partial_evaluation.rs`: extract `resource.type == T` and literal path prefixes into a second index tier on `ActiveSet`; a policy is prunable if it is bounded by id **or** type/prefix. Keep the extraction a provable *superset* of the true match set (unrecognized shape → still `unprunable`, never a spurious `Some`). Pre-build a **sorted, deduped** `unprunable` id slice at swap time in `build`/`store` so `candidate_policy_ids` no longer sorts per request — it merge-walks pre-sorted slices instead.
   - Where: `reaper_dsl/mod.rs:1844-1917` (extraction), `engine/mod.rs:170-173,337-351,372-404` (index build + candidate assembly), `crates/policy-engine/src/partial_evaluation.rs` (wire in or delete-and-inline).
   - Verify: property test — for any request resource, the type/prefix prefilter's candidate set ⊇ the true-matching set (extends `reaper_dsl/tests.rs:301-408`). Bench: 10k ABAC policies, evaluate-all, no `candidate_cap_exceeded`, no per-request O(N log N). Nightly `slo-harness.yml` ABAC/ReBAC evaluate-all row within SLO. Until this lands, document the evaluate-all SLO row as "literal-resource policies only" and keep `allow_evaluate_all` default `false`.

5. **Load `ActiveSet` once per `evaluate_set` (R3-P3-1).**
   - What: Load `self.active.load()` once at the top of `evaluate_set` and index the snapshot directly instead of calling `self.get_policy(id)` (one `ArcSwap::load` + guard alloc) per candidate.
   - Where: `engine/mod.rs:497-511`.
   - Verify: micro-bench at a 256-candidate fan-out shows N→1 arc-swap loads; existing eval correctness tests stay green.

6. **Cache capability verdicts + move crypto off the reactor (R3-P2-2).**
   - What: Add a bounded, sharded verdict cache keyed on `(capability.id, key_id, signature_bytes, expiry, revocation_generation)` storing only *positive* verdicts with a short TTL; a hit skips `verify_raw`. Fold the revocation generation into the key so a revocation-list bump invalidates without a scan. Offload the cold `verify_raw` to `spawn_blocking` above a size/batch threshold (mirrors the R2-P2-2 audit fix pattern). Add a per-tenant cap-verify rate limit to bound garbage-signature floods.
   - Where: `services/reaper-agent/src/capability_gate.rs:71`, `services/reaper-agent/src/management/verify.rs:142-158`, `crates/reaper-core/src/capability.rs:320-380`, `evaluate.rs:207`.
   - Verify: unit test — same capability twice → one `verify_raw`; revocation-generation bump → re-verify. New agentic (capability-per-request) `slo-harness.yml` scenario; load test shows steady-state throughput not gated by ed25519 cost and garbage-signature flood is rate-limited.

7. **Feature-gate the billing stub (R3-04).**
   - What: Put the billing router behind a `billing` cargo feature / `enable_billing` config flag, **default off**. When off, do not mount `api/mod.rs:86` billing routes and exclude them from the OpenAPI root so the parity gate is green with the surface absent. Ensure the Stripe webhook either verifies the signature or returns 501 — never a silent 200 on an unverified payload. When on, tag the operations `x-experimental` in the spec.
   - Where: `api/mod.rs:86`, `api/billing.rs:126,189,281`, `billing/service.rs:198-323`, `config/server.rs`, `tests/api_contract.rs` (parity must pass in both configs).
   - Verify: default build → `/orgs/{org}/billing/*` returns 404, absent from `/openapi.json`; feature-on build → present, marked experimental; webhook with a bad signature → 4xx/501, not 200.

8. **Seal enums, monotonic bundle ETag, ratchet-down (R3-05/06/07).**
   - What: (a) `#[non_exhaustive]` on `PolicyLanguage` (`engine/types.rs:38`), SDK `Decision`/`Source` (`reaper-sdk/src/types.rs:38,48`), `Transport` (`reaper-sdk/src/transport.rs:11`), growable `reaper-core` enums; add `#![deny(missing_docs)]` to `reaper-sdk` + `reaper-core` (backfill docs). (b) Add a monotonic `row_version` column to bundles (migration), bump on every write, derive the ETag from it (mirror policies). (c) Lower `UNTYPED_SUCCESS_BASELINE`/`MISSING_4XX_BASELINE` by adding typed DTOs + a shared `responses(...)` 4xx fragment to the highest-traffic operations; commit the new (lower) baselines so the ratchet can only tighten.
   - Where: `engine/types.rs:38`, `reaper-sdk/src/{types.rs,transport.rs,lib.rs}`, `reaper-core/src/lib.rs`, `api/bundles.rs:218,281` + bundle repo + `db/migrations/NNN_bundle_row_version.sql`, `tests/api_contract.rs:225-227`.
   - Verify: `cargo build` fails on an undocumented public item in the two crates; downstream `match` on a sealed enum requires a wildcard arm; two sub-resolution bundle edits get distinct ETags and the second `If-Match` on the stale tag → 412; the two baselines in CI are strictly below 94/129.

## 5. Dependencies

- **Round-3 P0 lanes (hard gate, sibling plans):** the tenant-isolation authz backstop and the release/supply-chain integrity work must land for the *overall* gate to reach CONDITIONAL. This plan is safe to build in parallel but does not move the gate to READY until those close — READY is "CONDITIONAL plus this cluster."
- **Plan 07 (API governance) primitives:** steps 2-3 reuse the shipped `PageQuery`/`Paginated` keyset extractor and the `contract_is_publishable`/parity gate in `tests/api_contract.rs`. Step 7's spec-exclusion relies on the utoipa-axum single-tree.
- **Plan 08 (engine perf) + `slo-harness.yml`:** step 4 extends the served-path pruning index that Plan 08 built; steps 4/6 add scenarios to the existing SLO harness and paired-A/B gate rather than building new infra.
- **Plan 12 (migration engine) `row_version` pattern:** step 8b mirrors the policy `row_version` mechanism already shipped (`db/repositories/policy.rs:380-404`).
- **F1 agentic-authz (`plans/round-2/F1-agentic-authz.md`):** step 6 hardens the capability gate that F1 introduced; must not regress its correctness.

## 6. Testing & verification

- **Timeout tests** (step 1) — black-hole-port client errors within the deadline; grep-lint red on a reintroduced bare `Client::new()`.
- **Pagination tests + fitness function** (steps 2-3) — per-endpoint default/cap enforcement, cursor no-drift-under-insert, and the contract-test enumeration that fails on any unpaginated collection GET.
- **Prunability property + scale tests** (step 4) — superset property (never fails open), 10k-ABAC evaluate-all with no candidate-cap denial, nightly SLO-harness ABAC/ReBAC row within budget. This is the primary correctness gate for the change.
- **Capability-cache tests + agentic load** (step 6) — single-verify-on-repeat, revocation-generation invalidation, new agentic SLO scenario, garbage-signature rate-limit test.
- **Billing gate tests** (step 7) — parity green in both feature configs; default-off surface absent; webhook rejects unverified payloads.
- **Contract/enum tests** (step 8) — `deny(missing_docs)` build failure, sealed-enum wildcard requirement, distinct bundle ETags on rapid edits, lowered baselines committed.
- **Regression:** existing `tests/api_contract.rs` parity + publishability gate, `perf-gate.yml` paired A/B, and the eval correctness suites must all stay green.

## 7. Effort & phasing — S/M/L

- **Phase A (S):** Universal HTTP timeout helper + grep-lint (step 1). Small, high-value, no behavior change to healthy paths; ship first.
- **Phase B (S-M):** Pagination — pins + config lists + the fitness function (steps 2-3). Mechanical, broad, reuses Plan 07 primitives.
- **Phase C (M-L):** ABAC/ReBAC prunability + the once-per-fan-out arc-swap load (steps 4-5). The one genuinely non-trivial item — wiring `partial_evaluation.rs` and proving the superset property. Highest performance leverage.
- **Phase D (M):** Capability verdict cache + off-reactor verify + rate limit (step 6). Correctness-sensitive (must not fail open on revocation).
- **Phase E (S-M):** Billing gate (step 7) + enum sealing / monotonic ETag / ratchet-down (step 8). Mostly mechanical; ratchet-down is incremental.

Overall: **M** in aggregate (Phase C dominates). Each phase independently shippable and independently revertable.

## 8. Key decisions (ADR-style)

- **ADR-1: One `http_client()` helper + a lint, not per-site fixes.** Fixing the eight current sites without a lint guarantees the ninth reappears. The durable control is the grep/clippy `disallowed-methods` gate that makes a bare `Client::new()` a red build. Consequence: all HTTP construction routes through one function; the sync `unwrap_or_else` degradation is deleted (a builder error propagates rather than silently dropping the timeout).
- **ADR-2: Keyset (not offset) pagination for pins.** Pins are fleet-cardinality and mutate under concurrent agent registration; offset drifts. Reuse the proven `(created_at, id)` keyset. Config-cardinality lists (environments/webhooks/strategies) get the same envelope for contract consistency even though they are small — one shape, one fitness function. Consequence: opaque cursors, already documented as non-decodable in Plan 07.
- **ADR-3: Resource-*type*/prefix index tier via `partial_evaluation.rs`, superset-sound.** Chosen over (a) raising `max_candidate_policies` (masks the O(N) fan-out and the sort, doesn't fix the cliff) and (b) documenting the SLO row as literal-only forever (concedes the product's headline use case). The extraction stays a provable superset so it can never fail open — an unrecognized shape remains `unprunable`. Consequence: the in-tree dead engine asset is wired in (or inlined and the file deleted, per R3-P3-3), and the `unprunable` id slice is pre-sorted at swap time so the per-request sort disappears.
- **ADR-4: Cache *positive* verdicts keyed with the revocation generation; `spawn_blocking` the cold path.** Caching only positive verdicts with the revocation generation in the key means a revocation bump invalidates without a per-entry scan and a still-invalid capability never gets a cached pass. The cold verify moves off the reactor above a threshold, mirroring the R2-P2-2 audit fix. Consequence: steady-state agentic traffic is a hash lookup; a per-tenant rate limit bounds the asymmetric-cost DoS that the cache alone does not (garbage signatures never cache-hit).
- **ADR-5: Billing default-off behind a feature/config, excluded from the spec.** Chosen over leaving it mounted with an `x-experimental` tag only — a documented endpoint that fabricates checkout URLs is a contract lie a buyer can call. Default-off + spec-exclusion means the published contract describes only what works; turning the feature on is an explicit operator choice. Consequence: the parity gate must pass in both configs (asserted in CI).
- **ADR-6: Ratchet the untyped-contract baselines down, not to zero in one PR.** 94 untyped bodies + 129 missing-4xx is a large mechanical surface; a big-bang typing PR is unreviewable and risky. Commit a strictly-lower baseline each PR so the gate can only tighten, targeting 0. Consequence: the READY claim is "ratchet installed and moving," with the residual surface visible and bounded, not hidden.

## 9. Risks & rollback

- **Risk: the prunability prefilter fails open** (a policy that should match is pruned out → wrong *allow*/silent *deny*). Mitigation: the extraction is a provable superset with a property test asserting `candidates ⊇ true-matches` for arbitrary resources (extends the existing literal-resource property); an unrecognized shape stays `unprunable`. This is the one item where a bug is a correctness (not just latency) defect — it gets the property test as a hard gate. Rollback: `use_pruning_index` config flag already exists (`settings.rs`); disable the type tier and fall back to literal-only + full unprunable set.
- **Risk: capability cache serves a stale positive after revocation.** Mitigation: the revocation generation is in the cache key, so a revocation bump makes every prior entry a miss; TTL is short. Rollback: feature-flag the cache off → inline verify (current behavior).
- **Risk: `#[deny(missing_docs)]` blocks the build on undocumented items across two crates.** Mitigation: land as `#[warn(...)]` first, backfill docs, then flip to `deny` in CI. Rollback: keep `warn`.
- **Risk: adding a bundle `row_version` column needs a migration on a live table.** Mitigation: additive column with a default; backfill is a no-op (new writes bump it). Reuse the exact pattern proven for policies. Rollback: ETag falls back to `updated_at` (current behavior) if the column is absent.
- **Risk: feature-gating billing breaks a consumer that called the stub.** Mitigation: it returns *fabricated* sessions today — no real consumer can depend on it; default-off is safe. Rollback: flip the feature on.
- **Risk: universal timeout is too aggressive for a legitimately slow upstream** (e.g. a large ClickHouse audit query). Mitigation: per-call timeout override on the helper; set the audit-query timeout generously (and separately from the connect timeout). Rollback: raise the specific call's timeout.
- **General rollback:** every phase is independently revertable; Phases A/B/E carry near-zero runtime risk, Phase C is the only one that touches the eval hot path and it is guarded by the superset property test and the `use_pruning_index` flag.
