# Reaper Enterprise Readiness Review — Round 2 Synthesis

*Orchestrator synthesis across five reviewers. Companion to `01-performance.md`,
`02-security.md`, `03-code-and-api.md`, `04-product-architecture.md`, and the
strategist `06-future-architecture.md`. This is a RE-REVIEW: round 1
(`reviews/00`–`05`) returned **NOT READY** with three P0s and spawned a 12-plan
remediation roadmap (`plans/`), all of which shipped (PRs #10–#48). Review only
— no code was modified.*

---

## Overall verdict: **CONDITIONAL**

The gate rule is "worst persona verdict wins unless justified." Three personas
returned CONDITIONAL, one returned READY-WITH-CONDITIONS (a softer CONDITIONAL),
and the strategist runs a separate fitness lens. **No reviewer found a single
P0.** That is the headline: round 1 had three P0s that let an anonymous attacker
control live authorization; every one is verified genuinely closed in code (not
cosmetically) — router-level default-deny gateway defaulting `Enforcing`
(`main.rs:297-300`), agent push-deploy verifying ed25519 signatures fail-closed
*before parse* with `require_signed_bundles=true` default, and an IdP-agnostic
SSO/SCIM broker with a structural "no IdP group confers platform admin"
invariant.

What remains is **finishing work on 70–90%-built pillars**, concentrated in one
place — audit integrity operationalization — plus a cluster of
authorization-scope, concurrency, and pagination gaps that a bank review board
*will* catch. CONDITIONAL means: not deployable into a regulated production
authorization path *today*, but the distance to READY is a focused sprint, not
another 12-plan roadmap.

| Reviewer | Verdict | P0 | P1 | P2 | P3 |
|---|---|---|---|---|---|
| Performance | CONDITIONAL | 0 | 1 | 3 | 5 |
| Security | CONDITIONAL | 0 | 5 | 3 | 3 |
| Code & API | CONDITIONAL | 0 | 1 | 6 | 4 |
| Product Architecture | READY w/ CONDITIONS | 0 | 4 | 7 | 5 |
| Future Architecture | *(strategy — no gate)* | — | — | — | — |
| **Aggregate** | **CONDITIONAL** | **0** | **11** | **19** | **17** |

Round-1 → round-2 trajectory: **3 P0 → 0 P0.** The three original P0s
(anonymous authz control, unsigned distribution, tenant-isolation holes) are
closed and independently re-verified. The 11 P1s are all "the pillar exists and
works; the last mile is unshipped," not "the pillar is missing."

---

## Top 10 findings, ranked by risk to enterprise adoption

**1. Audit integrity is real but not operationalized end-to-end — P0-adjacent in the default deployment.**
*(SEC R2-2/R2-3/R2-4, PROD R2-10, PERF R2-P2-2)* — The hash chain + signed
Ed25519 checkpoints are genuine and well-tested at the unit level, but: (a) the
verifiers (`verify_chain`/`verify_checkpoint`, `decision_log.rs:139,339`) are
called **only from unit tests** — no CLI, endpoint, or scheduled verifier ships,
so a regulator cannot be handed a "prove our audit is intact" tool; (b) chain
`seq` is assigned pre-enqueue and can diverge from writer order under
concurrency, and ClickHouse's `ORDER BY` + ReplacingMergeTree dedup **does not
preserve write order**, so the chain is *not verifiable from the queryable
store*; (c) checkpoints ship to the **same mutable ClickHouse** as decisions, the
immutable S3/WORM sink is **commented out by default** (`vector.toml:97-107`),
and there is no cross-boot `chain_id` linkage — so an insider with store write
access can delete a boot's decisions *and* its covering checkpoints undetectably;
(d) decisions are **served before durable persistence** (async writer), leaving a
bounded served-allow loss window on sink failure even in mandatory mode. This
cluster is the single biggest barrier and the first thing a bank rejects.

**2. Fleet propagation routes lack fine-grained authorization — intra-tenant privilege escalation.**
*(SEC R2-1)* — Rollout, rollback, approve-wave, cancel, and pin
(`api/deployments/rollouts.rs`, `pins.rs`) gate only on `org.id` membership or
`Admin`, with **no scope check** — unlike `bundle:promote` and every sibling
route. Any org member holding *any* token (including a read-only service token)
can trigger a fleet-wide rollback or pin. On the propagation surface — the one
that rewrites what every agent enforces — this is the highest-severity access
gap remaining.

**3. No autonomous auto-rollback — the flagship pillar detects but cannot act.**
*(PROD R2-1)* — Thresholds, error-rate signal, trigger evaluation, and the
rollback action all exist (`rollback_config.rs:274-364`), but **nothing invokes
them** — none of the five `main.rs` background tasks is a rollout supervisor. A
bad policy spiking denials at a bank waits for a human to poll an endpoint. ~90%
built; missing only the supervising loop (and the leader-election pattern it
needs already exists in the codebase). **This is the single most important next
move** — it is the last safety gap on the strongest, most-differentiated pillar,
and has no operational workaround.

**4. Unbounded ABAC/ReBAC list endpoints — DoS on the largest tables.**
*(CODE R2-01)* — Entity, role-binding, and tuple lists
(`api/datastore.rs:293,449,553`) return `count: len()` with no `LIMIT` — the
biggest tables in the system, unpaginated, while `policies` is correctly keyset-
paginated. Multi-MB responses / full DB scans; a false "Phase-E pagination
closed" claim.

**5. The headline sub-microsecond SLO is never measured on the real request path.**
*(PERF R2-P1-1)* — Plan 08's load harness is deferred; the blocking perf gate
benches only two in-process criterion micro-suites, never the request-total HTTP
handler at representative load (10k policies × 5k rps). The request-total
histogram exists but nothing drives it in CI. A bank cannot approve a latency SLA
on unmeasured assumptions.

**6. PII redaction is opt-in and `resource` is never redactable — GDPR exposure by default.**
*(SEC R2-5)* — All privacy knobs default off (`decision_privacy.rs`), and the
`resource` field has no redaction path at all. A default `mode=full` deployment
ships raw principals and request resources to the audit store. UK DPA-2018 /
GDPR problem out of the box.

**7. No native SIEM connectors and no GDPR subject-erasure endpoint.**
*(PROD R2-2, R2-3)* — Audit egress is NDJSON/JSON → Vector → ClickHouse only; no
native Kafka/Splunk-HEC/CEF/OCSF (the S3 sink is commented out). And there is no
erase-by-subject endpoint anywhere in the tree — a DSAR cannot be honored through
the product. Both are SOC/compliance onboarding blockers for a regulated buyer.

**8. Optimistic concurrency is dormant by default and Plan 12 reintroduced lost-update gaps.**
*(CODE R2-02/R2-03/R2-04/R2-05)* — `require_if_match` defaults **false**
(warn-only, `config/server.rs:39`), so lost-update is reachable in the default
config that a "READY" build would ship; policy ETag is content-hash so
metadata-only edits silently lose updates even with enforcement on; and the new
`apply_migration` has **no `model_version` optimistic guard** and takes no
Idempotency-Key, so concurrent migrations or a retried timeout can clobber the
model / double-apply + double-propagate.

**9. The mandated DSL is not prunable — evaluate-all mass-denies or degrades at scale.**
*(PERF R2-P2-1)* — The Phase-A pruning index prunes only the **deprecated**
`Simple` language; DSL/Cedar are always `unprunable`, so DSL evaluate-all at >256
policies returns blanket `candidate_cap_exceeded` denials or (cap raised) runs a
full scan plus per-request `O(N log N)` sort/dedup. The SLO row is unmet for the
language customers must use. Notably, `partial_evaluation.rs` — the exact
technique that would make DSL prunable — exists in-tree but **dead** (benches
only), alongside two other unwired parallel engines (`indexed_engine.rs`,
`optimized_engine.rs`).

**10. Operator fail-open footguns and no signed air-gap export.**
*(SEC R2-6/R2-7, PROD R2-4)* — The gateway has selectable `Disabled`/`LogOnly`
modes and the agent has `allow_unauthenticated`/`open_data_plane` flags — all
default-safe and warned, and the agent's `validate_exposure` refuses an
unauthenticated non-loopback bind, but they remain one-config-line paths to an
anonymous plane. Separately, there is no signed export/import for air-gapped
enclaves (`compile` doesn't sign; CLI deploy sends no signature field), so a
disconnected bank cannot receive policy via sneakernet with verifiable
provenance.

---

## Cross-cutting themes

- **Audit is the weakest subsystem — three reviewers converge on it.** Security
  (5 of its P1s touch audit), Product (R2-10), and Performance (R2-P2-2, the
  `Block`-mode reactor stall) all independently land on the audit pipeline. The
  primitives are excellent; the *operationalization* (a running verifier, an
  immutable independent checkpoint anchor, durable-before-serve, redaction-on) is
  the gap. If one workstream is funded next, it is audit hardening.

- **The propagation/deployment surface is under-governed AND can't self-heal.**
  Security R2-1 (anyone can trigger it) and Product R2-1 (it can't auto-revert)
  are the same subsystem viewed from two angles: the fleet-distribution pillar is
  the product's strongest differentiator and its two remaining safety holes both
  live here. Fix them together.

- **Fast-shipped roadmap code carries a concurrency-correctness tax.** The Plan
  12 migration engine (CODE R2-04/R2-05) and the warn-only concurrency default
  (R2-02/R2-03) show the remediation sprint optimized for feature-complete over
  race-complete. None is a P0, but collectively they say "the optimistic-
  concurrency story shipped as scaffolding, not as an enforced invariant."

- **Dead code is an overclaim signal, not just hygiene.** Four engine modules
  (`arena.rs`, `indexed_engine.rs`, `optimized_engine.rs`,
  `partial_evaluation.rs`) are wired only to benches — and the strategist
  independently flags the same "three parallel engines + three policy languages"
  accretion. The most useful unwired asset (partial evaluation) is exactly what
  finding #9 needs. This is complexity being carried without being paid down.

- **Strategist's orthogonal warning:** the tactical reviewers are all looking
  *down* at defects; the strategist looks *forward* and says the real risk is
  strategic, not defective — Reaper is hardening a **commoditizing eval core**
  while the differentiating axis of the next five years (**authorization for
  non-human / agentic AI actors**, distributed via the **wasm target the source
  tree is 80% ready for**) sits unclaimed. Worth holding in view while triaging
  the P1 list, so the next roadmap isn't purely a thirteenth enterprise
  integration.

---

## If a UK bank's architecture review board saw this tomorrow, the first three rejections

1. **"Prove this decision was made under policy X at time T — and prove the log
   is complete and unaltered."** Today you cannot hand them a verifier (it exists
   only as a unit-tested library function), the chain isn't verifiable from the
   queryable ClickHouse store, and in the *default* deployment an insider with
   store write access can delete a boot's decisions and its checkpoints together
   without detection because the immutable WORM anchor is commented out. This is
   the board's job-one question and the product cannot yet answer it end-to-end.

2. **"Who can change what every enforcement point does, and what happens when a
   change goes wrong?"** Any authenticated org member — including a read-only
   service token — can trigger a fleet-wide rollout, rollback, or pin (no
   fine-grained scope), and a bad policy that starts denying legitimate traffic
   will not auto-revert because the detection loop has no supervisor to act on it.
   A regulated change-control review rejects an under-authorized, non-self-healing
   propagation surface.

3. **"Show me the latency SLA holds under load, and that the data-plane APIs are
   safe at scale."** The headline sub-microsecond SLO has never been measured on
   the real request path (only in-process micro-benches), and the largest tables
   in the system (ABAC entities, ReBAC tuples) list without pagination — an
   unproven performance claim and an availability foot-gun on the same surface a
   bank would load-test first.

*None of these is a P0 and none reopens a round-1 P0. All three are closable
without architectural change — which is precisely why this review is CONDITIONAL,
a decisive step up from round 1's NOT READY, and why the honest one-line summary
is: the platform is architecturally sound and enterprise-shaped; it now needs one
focused hardening sprint on audit-verifiability, propagation authz + auto-
rollback, pagination, concurrency defaults, and a measured SLO before it can carry
production authorization for a regulated bank.*
