# Reaper — Enterprise Readiness Roadmap (Round 3)

Master index and sequencing for the round-3 remediation set. Each plan is derived
from the round-3 review in [`../../reviews/round-3/`](../../reviews/round-3/)
(start with [`08-SYNTHESIS.md`](../../reviews/round-3/08-SYNTHESIS.md)) and follows
the house template: *Goal · Current state (file:line) · Definition of Done ·
Critical steps · Dependencies · Testing · Effort · Key decisions (ADR) · Risks*.

> **STATUS: 🚧 IN PROGRESS (opened 2026-07-16)** — round-3 review merged
> (PR #76). Overall verdict **NOT READY** on **five P0s** across two lanes:
> four cross-tenant / account-takeover defects in the authorization layer, and
> one supply-chain / release-integrity hole. Rounds 1–2 (24 plans, PRs #10–#48)
> are verified genuinely closed; the engine, audit, HA/DR, GitOps, and perf
> gates are strong. Round-3 is **narrow and deep**: it does not add features —
> it closes a concentrated cluster of authz + release-integrity defects and,
> critically, installs the *fitness functions* that stop the whole class from
> ever reappearing, so we can evolve safely once in production.

## Where we start — and why it's tractable

The round-3 failures are **not systemic rot**. The root cause is a single
structural gap: the round-2 auth gateway *authenticates* but delegates per-tenant
*authorization* to individual handlers, with no backstop — so any handler that
forgets the check becomes a silent cross-tenant hole, and the existing contract
test proves routes are *authenticated*, not *authorized to the right tenant*.
Five route groups skipped the check. Every P0 is "correct primitive, missing or
mis-wired enforcement," so the path back is fast — **provided we fix the pattern,
not just the four instances.** The same discipline applies to release integrity
(sign + attest + scan-before-publish) and to the correctness/perf claims (turn
each load-bearing claim into a blocking gate).

## The gate model this roadmap drives toward

- **NOT READY → CONDITIONAL** = safe for a *design-partner / non-regulated pilot*:
  no cross-tenant access, release artifacts signed + provenance-attested +
  scanned-before-publish. Delivered by **Phase 0**.
- **CONDITIONAL → READY** = passes a *regulated (bank) architecture + security
  review*: the safety pillar reverts on bad *decisions* not just failed applies,
  the DSL is a versioned contract that can't silently drift, every headline claim
  is defended by a machine check, and the GA edge/scale hygiene is closed.
  Delivered by **Phase 1 + Phase 2**.
- **READY → LAUNCHED** = externally deployable and marketable: pilot runbook,
  compliance evidence pack, an independent external pen-test, a defensible
  marketing-claims matrix, and a written *evolve-in-prod* operating model.
  Delivered by **Phase 3**.

## The plan set

| # | Plan | Prio | Moves gate | Round-3 findings closed | Effort |
|---|------|------|-----------|-------------------------|--------|
| 01 | [Tenant Isolation & AuthZ Backstop](01-tenant-isolation-and-authz-backstop.md) | **P0** | →CONDITIONAL | SEC P0-1, P0-2, P0-3, P0-4, P1-b; Synth #1,2,3,4,7 | M |
| 02 | [Release Integrity & Provenance](02-release-integrity-and-provenance.md) | **P0** | →CONDITIONAL | CICD C1(P0), C2–C6; Synth #5 | S–M |
| 03 | [Decision-Quality Auto-Rollback](03-decision-quality-rollback.md) | P1 | →READY | PROD P1-new; Synth #6 | M |
| 04 | [DSL as a Managed Contract](04-dsl-managed-contract.md) | P1 | →READY | EVO E-01; TEST builtins; Synth #8 | M |
| 05 | [Verification We Can Stand Behind](05-verification-and-gates.md) | P1 | →READY | TEST T1–T4; CODE CLI; Synth #9,10 | M |
| 06 | [GA Hardening](06-ga-hardening.md) | P2 | →READY | CODE P2s; PERF P2 (ABAC/ReBAC, cap-gate) | S–M |
| 07 | [Pilot, Compliance & Go-to-Market](07-pilot-and-gtm-readiness.md) | P1* | →LAUNCHED | Synth path-back; PROD gap register | M (mostly process/docs) |

*Effort is relative T-shirt sizing (S≈days, M≈1–3 weeks, L≈1–2 months for a small
team), not a commitment. Plan 07 is P1 for **launch**, not for internal correctness.*

## Phased sequence

### Phase 0 — Stop the bleeding (P0 blockers; nothing external ships until these land)
**01 → 02**, in parallel where possible (they touch different surfaces).

- **01 is the single most important work in the entire round.** It closes the four
  cross-tenant P0s *and* replaces the delegated-authz pattern with a central
  resource-ownership guard plus a **contract fitness function that fails CI when a
  mutating route lacks resource-tenant authorization**. Patching four handlers
  without this leaves the class alive; do not stop at the four instances.
- **02** makes the release pipeline able to *prove what it ships*: sign
  images/binaries/bundles (cosign), attach SLSA provenance, bind the SBOM to the
  published digest, and move the Trivy gate **before** publish — plus fix the
  broken CLI-release toolchain reference so the advertised binaries actually exist.

**Exit criterion for Phase 0 → CONDITIONAL:** an external attacker or malicious
tenant has no cross-tenant path, and every shipped artifact is signed, attested,
and scanned-before-publish. This unlocks a **non-regulated design-partner pilot**.

### Phase 1 — Earn the claims (P1s; reach a defensible regulated posture)
**03, 04, 05** — independent, run in parallel.

- **03** reconnects the advertised auto-rollback safety pillar to the *right signal*
  (live decision-quality + SLO breach, not just apply-failure), with canary
  decision-diffing and anti-flap guardrails.
- **04** turns the policy DSL into a **managed, versioned contract**: language-version
  headers with fail-closed rejection, a frozen decision-corpus regression gate that
  fails on any semantic drift, a republished parsing reference, a decision on the
  two DSL surfaces, and builtins brought under the oracle.
- **05** makes every load-bearing claim (ready / correct-logic / fast) defended by a
  **blocking** check: mutation testing promoted to a gate, tenant-isolation + audit
  invariants under test, the CLI actually tested, and absolute-SLA / cold-start /
  p999 gated on the served path.

### Phase 2 — GA hardening (P2s; close the edges and scale cliffs) → READY
**06** — outbound timeouts everywhere, finish pagination, fix the ABAC/ReBAC pruning
cliff (the 256-candidate denial cliff at scale), cache the capability-gate verify off
the hot reactor, and neutralize the misleading billing stub.

**Exit criterion for Phase 1+2 → READY:** passes a bank architecture + security
review on paper — the failures a regulator would reject are closed and gated.

### Phase 3 — Deploy & market (turn READY into LAUNCHED)
**07** (depends on 01–06 landing) — pilot runbook + reference deployment + first real
DR game-day; a compliance evidence pack mapped to SOC 2 / ISO 27001 / DORA / FCA-PRA;
an **external penetration test** as the independent check on Plan 01 (security is not
"done" on internal review alone); a marketing-claims matrix where every headline claim
is tied to the specific gate that now defends it; and the **evolve-in-prod operating
model** — the fitness-function suite as the standing regression backstop, staged
rollout for the control plane itself, public API + DSL versioning/deprecation policy,
dogfooding (Reaper authorizing its own control plane), and an SLO/error-budget model.

## Critical path & prioritisation (the short version)

1. **Plan 01 first, always.** It is the difference between "leaks across tenants"
   and "safe to show anyone." The structural backstop is the highest-leverage line
   of work in the round — it converts a recurring finding-class into a CI failure.
2. **Plan 02 in parallel.** A bank cannot attest software it can't prove the
   provenance of; this is cheap relative to its weight and unblocks any external
   distribution.
3. **Plans 03–05 are the "earn READY" band** — they convert marketing claims from
   assertions into evidence. Sequence by team capacity; none blocks another.
4. **Plan 06** is finishing work — real, GA-blocking, but low-risk.
5. **Plan 07** is the launch gate and the answer to "no product is ever finished":
   it institutionalises the fitness functions from 01/04/05 as the mechanism that
   lets Reaper change safely while customers depend on it.

## Guiding principle — solid now, evolvable later

Every P0/P1 fix in this round ships **with the automated guardrail that keeps it
fixed** — a tenant-authz contract gate (01), a scan-before-publish + signing gate
(02), a decision-corpus semantic-stability gate (04), and promoted correctness/perf
gates (05). That is deliberate: the goal is not a frozen "finished" product but a
**locked floor we can build up from in production** without silently regressing the
properties a bank is trusting us for.
