# Reaper — Enterprise Readiness Roadmap

Master index and sequencing for the 12 feature plans in this directory. Each plan
is derived from the enterprise-readiness review in [`../reviews/`](../reviews/)
(start with `reviews/05-SYNTHESIS.md`) and follows one template: *Goal ·
Current state (file:line) · Definition of Done · Critical steps · Dependencies ·
Testing · Effort · Key decisions (ADR) · Risks*.

**Where we start:** the review's overall verdict is **NOT READY** — three
independent P0s let an anonymous network attacker control live authorization
decisions and rewrite policy for any tenant, and there is no SSO/SCIM. Every P0
is *missing wiring around correct primitives*, so the path back is tractable.

**The gate model this roadmap drives toward:**
- **NOT READY → CONDITIONAL** = safe to run for a *design partner / non-regulated pilot*: no anonymous control of authz, signing enforced, identity federated, audit defensible. Delivered by Phase 0 + Phase 1.
- **CONDITIONAL → READY** = passes a *regulated (bank) architecture + security review*: API contract & concurrency safety, engine holds its SLA at scale, GitOps pillar real, first-class environments/approvals, control-plane HA/DR, safe data-model evolution. Delivered by Phase 2 + Phase 3.

## The plan set

| # | Plan | Prio | Moves gate | Findings closed | Effort | Depends on |
|---|------|------|-----------|-----------------|--------|-----------|
| 01 | [AuthN/AuthZ Foundation](01-authn-authz-foundation.md) ✅ **shipped** (PRs #10/#11; Phase D dogfooding deferred — see plan header) | **P0** | →CONDITIONAL | Synth #1,#2; Sec P0-1/P0-3/P0-3b; API-1/API-2 | M | — |
| 02 | [Policy Integrity & Distribution](02-policy-integrity-and-distribution.md) ✅ **shipped** (PRs #12–#14, #16 governed promotion) | **P0** | →CONDITIONAL | Synth #3,#6; Sec P0-2/P1-1 | M | 01 (promotion authz) |
| 03 | [Enterprise Identity: SSO + SCIM](03-enterprise-identity-sso-scim.md) ✅ **shipped** (PR #17; SAML deferred by decision) | **P0** | →CONDITIONAL | Synth #4; Prod F1 | M | 01 (session/RequireAuth seam) |
| 04 | [Audit Integrity & Replay](04-audit-integrity-and-replay.md) ✅ **shipped** (PRs #18–#23: seq+chain, signed checkpoints, mandatory fail-closed + alarms, retention/legal holds, replay capture + counterfactual engine) | P1 | →CONDITIONAL | Sec P1-2; Prod F7/F10 | M–L | 02 (signing primitive for checkpoints); 03 (actor identity) |
| 05 | [Availability & Resilience](05-availability-and-resilience.md) ✅ **shipped** (PR #24: drop panic=abort + CatchPanicLayer, DSL nesting-depth bound, batch cap + spawn_blocking + per-route body limit, safe bundle-download header, fail-open/closed matrix, unwrap/expect clippy ratchet) | P1 (1 borderline P0) | →CONDITIONAL | Synth #8; API-3/API-11; Sec P2-1; Perf P1-2 | S–M | — |
| 06 | [Software Supply Chain](06-software-supply-chain.md) ✅ **shipped** (PR #25: cargo-deny + cargo-audit blocking CI, blocking Trivy image scan, CycloneDX SBOM on release, cargo-fuzz parser targets, deny.toml + vuln-response SLA, dependency-freshness + nightly supply-chain jobs) | P2 (elevated, 3-way) | →CONDITIONAL | Synth #9; Sec P2-2 = API-9 = F9 | S–M | 05 (fuzz targets test 05's DSL depth bound) |
| 07 | [API Governance](07-api-governance.md) ✅ **shipped** (PRs #27–#31: generated OpenAPI 3.1 contracts + blocking parity gate for both planes, single /api/v1 surface + versioning policy, ETag/If-Match optimistic concurrency, Idempotency-Key on propagation POSTs, keyset cursor pagination on every list, RFC 9457 problem+json errors, route conventions; suites verified on SQLite + PostgreSQL) | P1 | →READY | Synth #9; API-4/5/6/7/8/13/14 | M | 01 (auth is a hard prereq) |
| 08 | [Engine Performance to SLA](08-engine-performance-to-sla.md) ✅ **shipped** (PRs #33–#34: served-path pruning index + evaluate-all fan-out cap, sharded decision cache + allocation-free fingerprint, request-total vs engine-slice latency histograms, configurable tokio worker threads, rayon-parallel batch eval, ReBAC thread-local scratch + per-eval traversal budget, blocking paired A/B perf gate) | P1 | →READY | Synth #10; Perf P1-1/P1-2/P2-*/P3-* | M–L | — |
| 09 | [GitOps / Policy-as-Code](09-gitops-policy-as-code.md) ✅ **shipped** (PRs #35–#37: spawn the sync engine + materialize policies/bundles idempotently, shared SSRF guard on the git path, GitHub App install + minted installation tokens (no PAT-in-URL), signed-commit verification, HMAC-verified webhook push, drift detection + commit-back conflict model) | P1 | →CONDITIONAL/READY | Prod F2/F3; Sec P1-3 | S (wire) + M (reshape) | 01 (git creds behind auth) |
| 10 | [Environments & Promotion](10-environments-and-promotion.md) ✅ **shipped** (PRs #38–#40: first-class Environment over namespaces (tier ordering, approval policy, freeze windows), governed env→env promotion via change requests (upward-only, N distinct approvers, requester excluded), pinned data version applied to the target env on apply (fail-closed), keyset-paginated change-record trail, always-two-step promotion, per-env require_change_record gate on direct rollouts with audited admin break-glass, apply-time freeze recheck, opt-in ServiceNow change-record reference/validation) | P1 | →READY | Prod F4 | M | 01, 02 (approval actor + verified promote) |
| 11 | [Control-Plane HA/DR](11-control-plane-ha-dr.md) | P1 | →READY | Prod F5 | M | — |
| 12 | [Data-Model Migration Engine](12-data-model-migration.md) | P1 | →READY | Prod F6 | L | — |

*Effort is relative T-shirt sizing (S≈days, M≈1–3 weeks, L≈1–2 months for a small team), not a commitment — see each plan's §7.*

## Phased sequence

### Phase 0 — Stop the bleeding (P0 blockers; nothing external until these land)
**01 → 02**, in that order (02 needs 01 for "who may promote"). These two remove
every path by which an anonymous actor controls authorization: default-deny auth
on both planes, and signature verification on *every* bundle-load entry point
(the direct-deploy push path currently bypasses it). **This is the critical
path — it is the single most important work in the entire roadmap.**

Ship 05's one S-sized hotfix alongside Phase 0 out-of-band: **remove `panic =
"abort"` from the service release profile + add `CatchPanicLayer`** (a crafted
policy currently aborts the enforcement sidecar → authz outage). It's a few
lines and it's borderline-P0.

### Phase 1 — Make it evaluable by a regulated buyer (CONDITIONAL)
**03 (SSO/SCIM)**, **04 (tamper-evident audit)**, the rest of **05
(availability)**, and **06 (supply-chain gates)**. After Phase 1: identity is
federated and deprovisionable, the audit trail can answer "prove decision X
under policy vX at time T," a crafted policy can't crash a node, and the vendor
security review has an SBOM + blocking dependency scanning. 03 before 04 so the
audit actor maps to a governed corporate identity; 05's DSL depth bound and 06's
parser fuzz targets are **co-delivered** (the fuzzer is the acceptance test for
the bound).

### Phase 2 — GA hardening (CONDITIONAL → READY)
**07 (API governance)**, **08 (engine perf to SLA)**, **09 (GitOps)**, **10
(environments/approvals)**. 07 needs 01's auth; 10 needs 01+02. 09's F2 "wire the
sync engine" is an S-sized quick win that restores a *named product pillar* and
can be pulled earlier if GitOps is a design-partner ask. 08 makes the sub-µs
headline true on the *served* path at policy scale (today it linear-scans and
clones the full policy set).

### Phase 3 — Operational maturity (READY for regulated production)
**11 (control-plane HA/DR)** and **12 (data-model migration engine)**. Not
blocking for a pilot; hard requirements before a bank runs production
authorization data on it. 12 is the largest single plan (safe evolution of live
authorization data is genuinely hard) — start its design early even if built late.

## Dependency graph (build order)

```
Phase 0   01 ───────────────► 02
              │                 │
Phase 1       ├──► 03 ──► 04 ◄──┘ (04 reuses 02's signing primitive + 03's identity)
              │     05 ──► 06     (05 depth-bound tested by 06 fuzz)
Phase 2       ├──► 07
              │    08            (independent)
              ├──► 09
              └──► 10 ◄── 02
Phase 3        11               (independent)
               12               (independent; largest — design early)
```

## Cross-plan linkages (co-deliver / don't duplicate)
- **02 ↔ 01 ↔ 10:** promotion authz + two-person control is *specified* in 02 and 10 but *enforced* by 01's identity/scope layer. Build the auth seam once.
- **04 ↔ 02:** the audit hash-chain's signed checkpoints **reuse `bundle_signing`** — don't build a second signer.
- **05 ↔ 06:** the DSL nesting-depth bound (05) and the parser fuzz target (06) are the same acceptance test; land together.
- **03 ↔ 04:** SSO gives the audit trail (04) and the existing management-action audit log a governed *actor*; sequence 03 first.
- **09 materialization gap:** even after wiring `SyncService`, `sync_source` currently discards the parsed policy files — 09 includes the required materialize-into-policies/bundles step, not just the spawn.

## Definition of Ready — exit checklists

**CONDITIONAL (design-partner ready)** — all true:
- [ ] No unauthenticated route on agent or control plane; cross-tenant/IDOR probes fail (01)
- [ ] Every bundle-load path verifies signature + rejects rollback/revoked bundles (02)
- [ ] Admin identity via OIDC/SAML; SCIM deprovision revokes sessions (03)
- [ ] Decision log is hash-chained, drop-alarmed, and has a mandatory (fail-closed) audit mode (04)
- [ ] A crafted policy/batch cannot abort a node; fail-closed on every failure mode (05)
- [ ] `cargo deny` + `cargo audit` block CI; SBOM emitted; parser fuzzed (06)

**READY (regulated-production ready)** — CONDITIONAL plus:
- [ ] OpenAPI contract in CI; If-Match concurrency; pagination on all lists; RFC 9457 errors (07)
- [ ] Served engine holds the published p99/p999 SLO at target policy scale; perf regression gate is trustworthy (08)
- [ ] GitOps pillar runs end-to-end (git → materialized policy → deployed), GitHub App + signed commits (09)
- [ ] First-class environments with env→env approval gates, change records, freeze windows (10)
- [ ] Control-plane meets RPO ≤ 5 min / RTO ≤ 30 min; fleet upgrade with zero eval downtime; DR game-day passed (11)
- [ ] Model changes run through the migration engine with dry-run impact analysis + rollback (12)

## The single most important next move
**Plan 01, Phase A1:** add the default-deny authentication middleware to
`reaper-management`'s router (and the agent's), factoring the existing
`RequireAuth` logic into a shared `authenticate()`. That one structural change
makes every currently-unprotected handler fail *closed* instead of *open* — it is
the root-cause fix the synthesis names before anything else, and it converts the
worst P0 from "anonymous cross-tenant control" to "authenticated and scoped."

---
*Planning artifacts only — no product code was modified. Base: current `main`.
Review base commit `8abb3b8` (~2 merged PRs behind; delta does not affect any P0/P1).*
