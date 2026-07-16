# Pilot, Compliance Evidence, Go-to-Market & Evolve-in-Prod Readiness

> **STATUS: PLANNED** — round-3 launch-gating plan. This plan ships **no product
> code**. It is process, docs, and cross-team coordination that turns the
> code-level closures of round-3 plans 01–06 into (a) a runnable design-partner
> pilot, (b) a compliance evidence pack a bank's review board can consume, (c) a
> marketing-claims matrix where every headline claim is tied to a gate that
> defends it, and (d) a written operating model that lets Reaper keep changing
> safely once customers depend on it. It is the *last* plan to green-light, not
> the first to start: its deliverables assemble evidence that 01–06 produce.

**Readiness gate:** External launch — the seam between "engineering says READY"
(round-3 verdict path) and "a real design partner runs it in anger" + "a
regulated buyer's architecture/security review board signs off on evidence, not
assertions." This is the gate the roadmap's Definition-of-Ready checklists
*point at* but do not themselves cross.
**Priority:** P1 — gates external launch, not internal correctness. Nothing here
blocks a merge; everything here blocks a signed pilot and a defensible claim.
**Findings addressed:** produces no new code finding. Consumes the closures of
round-3 SEC P0-1..4/P1-b, CICD C1/C2..C6, PROD R3-1, EVO E-01, TEST T1..T4 (via
plans 01–06) and packages them as launch evidence. Directly closes PROD **R3-10**
(no control-plane SLO/error-budget doc; no support-bundle tool) and the review's
standing instruction that security must **not** be marked "done" on internal
review alone (round-3 `02-security.md`).

---

## 1. Goal

Cross the external-launch gate with evidence, not optimism. Concretely, produce
four artifacts and run two live exercises:

1. **Pilot-readiness kit** — a reference deployment, a design-partner onboarding
   runbook, a supported-configuration matrix, a diagnostics/support bundle, an
   SLA/SLO sheet tied to the perf gates, and the **first executed** DR game-day
   (the procedure shipped in plan 11 but was never run).
2. **Compliance evidence pack** — Reaper's controls mapped to SOC 2 Type II / ISO
   27001 / DORA / FCA-PRA SS2/21, each control marked *evidence-exists* vs
   *must-produce*, with named owners, plus an **external penetration test**
   commissioned as the independent check on plan 01.
3. **Marketing-claims matrix** — every headline claim bound to the specific
   gate/evidence that defends it; a claim we cannot evidence does not ship. Plus
   competitive positioning and an explicit "what we do NOT yet claim" list.
4. **Evolve-in-prod operating model** — the guardrails (fitness-function suite,
   staged-rollout discipline for the control plane itself, API+DSL versioning &
   deprecation policy, dogfooding, SLO/error-budget model) that keep Reaper
   changeable once a customer's authorization decisions depend on it.

**This is a coordination/documentation plan.** The only "code" it may touch is a
diagnostics `support-bundle` CLI verb (S) and wiring three dead fitness suites
into CI (EVO E-06, trivially small) — everything else is authored prose,
scheduled exercises, and a claims register. It is explicit about depending on
plans 01–06 *landing first*.

**Non-goals:** building any 01–06 feature here; multi-region active/active;
SAML; a public trust portal/SOC 2 *audit* (this produces the evidence a Type II
audit consumes, not the audit itself).

---

## 2. Current state (evidence) — file:line

- **Deployment surface already exists** — `deploy/helm/reaper/` (Chart + values +
  5 profiles: `profiles/{engine,platform,full,managed-stack,engine-uds-sharded}.yaml`),
  `deploy/kubernetes/`, `deploy/doks/`. There is a deployable substrate; there is
  no *reference pilot* pinned to a supported-config matrix.
- **HA/DR procedures shipped but never executed.** `docs/deployment/CONTROL_PLANE_HA_DR.md`
  §3 sets numeric targets (failover ≤60s, RPO ≤5min, RTO ≤30min) and §8 is a
  four-scenario game-day script — but the planned-vs-actual table (`:314-315`) is
  **empty**; plan 11's own header states "the exit checklist's measured RPO/RTO
  boxes tick on the first game-day run." That run has not happened.
  `docs/deployment/FLEET_UPGRADE_RUNBOOK.md` is likewise written-not-rehearsed.
- **No control-plane SLO/error-budget doc; no support-bundle tool** — round-3
  `04-product-architecture.md` R3-10; absence-checked (`docs/deployment/` has no
  SLO doc, `tools/` has no `support-bundle`). This is the one code-adjacent gap
  this plan may close directly.
- **Security assessed by internal review only.** Round-3 `02-security.md` returned
  NOT READY with 4 authz P0s; even once plan 01 closes them, the synthesis is
  explicit that "every route has an auth check" was the *wrong* fitness function.
  No external/independent test exists (absence: no pentest report in `docs/security/`).
- **The regression backstop is real but has holes.** The blocking fitness suite —
  `tests/api_contract.rs` parity, `perf-gate.yml` paired-A/B, compiled≡AST
  differential, delta≡rebuild determinism — is strong (EVO "done well"). Its
  *missing arms* are exactly the launch-critical ones: a **tenant-authz** contract
  gate (SYNTH path-back #1, plan 01), a **DSL decision-corpus** gate (EVO E-01,
  plan 04), **blocking** absolute/cold-start SLO (TEST T4, plan 05), and three
  **dead** interner suites (EVO E-06, CI-wiring only).
- **Marketing claims are un-indexed.** `README.md:8-14` asserts sub-µs latency,
  zero-downtime updates, OPA-style decision logging, enterprise reliability — with
  no artifact tying each claim to the gate that proves it. Several are currently
  *undefended* by the round-3 findings (e.g. multi-tenant isolation, pending plan 01).
- **Dogfooding deferred.** Roadmap `00-ROADMAP.md:18` and plan 01 header:
  "Phase D dogfooding deferred" — Reaper does not yet authorize its own control
  plane. The evolve-in-prod posture wants this as the ultimate fitness function.

---

## 3. Definition of Done — testable checkboxes

**Pilot readiness**
- [ ] A **green-lit pilot checklist** exists and every item is checked or has a
      dated waiver signed by the accountable owner.
- [ ] A **reference deployment** (one named Helm profile + values overlay) is
      stood up in a clean cluster from the runbook by someone who did not write it.
- [ ] A **design-partner onboarding runbook** takes a new tenant from org-create →
      SSO → first policy → deploy → verified decision, end to end.
- [ ] A **supported-configuration matrix** (DB, deploy profile, policy language,
      identity, transport, arch) states what is Supported / Preview / Unsupported.
- [ ] A **`reaper support-bundle`** diagnostics capture exists (versions, config
      redacted, health/metrics snapshot, recent decisions stats, fleet
      convergence) and is documented for pilot support escalation.
- [ ] An **SLA/SLO sheet** ties customer-facing objectives to the *blocking* gates
      (`perf-gate.yml`, absolute-SLO once T4 blocks) and the HA/DR RPO/RTO numbers.
- [ ] The **first DR game-day is executed**: `CONTROL_PLANE_HA_DR.md:314-315`
      table filled with *actual* RPO/RTO, misses filed as tracked issues.

**Compliance evidence pack**
- [ ] A **control→evidence index** maps SOC 2 (CC6/CC7/CC8), ISO 27001 (A.5/A.8/A.9),
      DORA (ICT-risk, third-party integrity, resilience testing) and FCA-PRA SS2/21
      to Reaper controls, each marked *evidence-exists* (with a `file:`/artifact
      link) or *must-produce* (with an owner + due date).
- [ ] An **external penetration test** is commissioned against a build with plans
      01–06 landed; scope covers tenant isolation, SSO/IdP-trust, webhook/SSRF, and
      supply-chain artifact integrity. Report received; findings triaged into the
      VULN_RESPONSE SLA.
- [ ] Tenant isolation, audit tamper-evidence, supply-chain provenance, and HA/DR
      RPO/RTO each have a **named artifact** in the index, not a prose assertion.

**Go-to-market**
- [ ] A **marketing-claims matrix** lists every headline claim with the gate/
      evidence that defends it; **no row is "unevidenced."** Any claim without a
      green gate is moved to the "not yet claimed" list before launch.
- [ ] A **competitive-positioning sheet** vs OPA+OPAL/Styra, OpenFGA/SpiceDB,
      Cedar/AVP, drawn from `04-product-architecture.md`'s competitive frame.
- [ ] A published **"what we do NOT yet claim"** list (SAML, multi-region
      active/active, consistency tokens/zookies, continuous native SIEM streaming
      where still DIY, …).

**Evolve-in-prod operating model**
- [ ] A written **evolve-in-prod document** names the four fitness-function arms as
      the regression backstop, the control-plane staged-rollout discipline, the
      API+DSL versioning/deprecation policy, the dogfooding plan, and the
      SLO/error-budget operating loop — each with an owner.
- [ ] The three **dead interner suites are wired into CI** (EVO E-06) and the DSL
      corpus + tenant-authz gates (from plans 04/01) are confirmed **blocking**.

---

## 4. Critical steps — ordered; per step what / where(files) / verify

> Sequencing rule: steps that *package evidence 01–06 produce* cannot complete
> until the corresponding plan lands. They can be **drafted** in parallel (the
> template, the index skeleton, the claims matrix rows) and **closed** as each
> dependency merges. This is called out per step.

### Workstream A — Pilot readiness / deploy

**A1 — Reference deployment + supported-config matrix.**
- *What:* Pick one canonical pilot topology (recommend `managed-stack.yaml`
  profile + managed HA Postgres per plan 11 ADR-1). Author a supported-config
  matrix (DB {managed-PG / CNPG / SQLite-dev}, profile, language {DSL / Cedar-preview},
  identity {OIDC}, transport {HTTP/SSE}, arch {amd64 / arm64}).
- *Where:* new `docs/deployment/REFERENCE_DEPLOYMENT.md`, `docs/deployment/SUPPORTED_CONFIGURATIONS.md`; cite `deploy/helm/reaper/profiles/managed-stack.yaml`.
- *Verify:* a clean-cluster stand-up from the doc by a non-author succeeds; every matrix cell is Supported/Preview/Unsupported with a reason.

**A2 — Design-partner onboarding runbook.**
- *What:* org-create → SSO bind → SCIM → policy source (BYO git) → author → deploy
  → verify a live decision → read the audit trail. Include rollback/off-ramp.
- *Where:* new `docs/getting-started/DESIGN_PARTNER_ONBOARDING.md`.
- *Verify:* walk it end-to-end against the A1 reference deploy; each step observable.

**A3 — Diagnostics/support bundle.**
- *What:* a `reaper support-bundle` CLI verb capturing build versions, redacted
  config, `/health`+`/metrics`+decision-stats snapshots, and fleet data-version
  convergence (feeds on plan 06's convergence read model, R3-5). This is the only
  net-new code (S).
- *Where:* `tools/reaper-cli/`; documented in `docs/deployment/OPERATIONS_GUIDE.md`.
- *Verify:* running it on the reference deploy yields a redacted archive with no secrets (grep the output for known secret keys → none).

**A4 — SLA/SLO sheet.**
- *What:* customer-facing objectives (eval p99/p999, availability, hot-swap =
  zero-downtime, RPO/RTO) each pinned to a *blocking* gate or the HA/DR numbers —
  not to aspirations. Explicitly note which SLOs only become defensible once
  TEST T4 (absolute/cold-start) is a blocking gate (plan 05).
- *Where:* `docs/deployment/SLA_SLO.md`; cross-ref `perf-gate.yml`, `CONTROL_PLANE_HA_DR.md`.
- *Verify:* every SLO row cites a gate file or a game-day-measured number.

**A5 — Execute the first DR game-day.**
- *What:* run `CONTROL_PLANE_HA_DR.md` §8 scenarios 1–3 in a real k8s env; record
  actual failover/RPO/RTO; file every miss.
- *Where:* fill `CONTROL_PLANE_HA_DR.md:314-315`; new dated `docs/deployment/dr-gameday-<date>.md` record.
- *Verify:* table populated with measured numbers ≤ targets, or a remediation backlog exists. *Depends on: a provisioned HA-Postgres environment (plan 11 infra).*

### Workstream B — Compliance evidence pack

**B1 — Control→evidence index.**
- *What:* a matrix keyed by framework control → Reaper mechanism → *evidence-exists*
  (artifact link) or *must-produce* (owner+date). Anchor the four load-bearing
  controls: **tenant isolation** (plan 01 backstop + the new tenant-authz contract
  gate), **audit tamper-evidence** (`docs/security/` hash-chain/checkpoints, plan 04-round1),
  **supply-chain provenance** (cosign/SLSA/SBOM from plan 02), **HA/DR RPO/RTO**
  (game-day record from A5).
- *Where:* new `docs/security/COMPLIANCE_EVIDENCE_INDEX.md`.
- *Verify:* zero "assertion-only" rows on the four anchors; each links a file or artifact. *Drafts now; anchors close as 01/02 land.*

**B2 — Commission the external penetration test.**
- *What:* scope + schedule an independent pentest against a build with 01–06
  merged: tenant isolation (the 4 SEC P0s' *fix*, not the patch), IdP-trust/`email_verified`,
  webhook/SSRF, GitHub-App installation-id confusion, and artifact-integrity/registry
  tamper. This is the review's explicit gate: security is **not** "done" on internal
  review alone (`02-security.md`).
- *Where:* tracked issue + `docs/security/PENTEST_SCOPE.md`; results into `VULN_RESPONSE.md` SLA.
- *Verify:* engagement booked; report received; findings triaged. *Hard dependency on plan 01 landing — do not pentest the pre-fix build.*

### Workstream C — Go-to-market enablement

**C1 — Marketing-claims → evidence matrix.** For each headline claim, bind it to
the gate that now defends it; a claim without a green gate does not ship:

| Claim | Defending gate/evidence | Dependency |
|---|---|---|
| Sub-µs eval (p99) | `perf-gate.yml` paired-A/B (blocking); absolute-SLO once TEST T4 blocks | plan 05 for the *absolute* number |
| Tamper-evident audit | hash-chain + signed checkpoints + replay (`docs/security/`, round-1 plan 04) | shipped |
| Zero-downtime hot-swap | atomic Arc swap + fleet-upgrade runbook; game-day zero-5xx roll | A5, plan 11 |
| Multi-tenant isolation | plan 01 tenant-scoping extractor + **tenant-authz contract gate** + pentest (B2) | **plan 01 — undefended until it lands** |
| GitOps policy-as-code | wired sync + GitHub App + signed commits (round-1 plan 09) | shipped |
| Agentic authz | hot-path capability gate (`capability.rs`, `evaluate.rs:203-219`) | shipped |
| Supply-chain provenance | cosign sign + SLSA + image-bound SBOM (plan 02) | **plan 02 — undefended until it lands** |

- *Where:* new `docs/gtm/CLAIMS_MATRIX.md`.
- *Verify:* every marketed claim maps to a green gate; undefended claims are moved to C3.

**C2 — Competitive positioning sheet.**
- *What:* distill `04-product-architecture.md`'s competitive frame — vs OPA+OPAL/Styra
  (Reaper adds managed multi-model data plane, env→env promotion, migration engine,
  agentic capabilities), vs OpenFGA/SpiceDB (own the deficit honestly: **no
  consistency tokens/zookies** yet), vs Cedar/AVP (Reaper owns the whole
  distribution+audit loop).
- *Where:* `docs/gtm/COMPETITIVE_POSITIONING.md`.
- *Verify:* every claimed advantage cites a shipped capability; the SpiceDB zookie deficit is stated, not hidden.

**C3 — "What we do NOT yet claim" list.**
- *What:* SAML, multi-region active/active, consistency tokens, continuous native
  SIEM streaming (still Vector-DIY where R3-2 open), PR-mode commit-back, per-env
  scoped RBAC — each with the plan/finding that would unlock it.
- *Where:* `docs/gtm/NOT-YET-CLAIMED.md`; cross-linked from C1.
- *Verify:* every C1 row that lacks a green gate appears here instead.

### Workstream D — Evolve-in-prod posture

**D1 — Lock the fitness-function suite as the regression backstop.**
- *What:* document the four arms as the contract that lets Reaper change safely:
  (1) **tenant-authz contract gate** (plan 01), (2) **DSL decision-corpus gate**
  (plan 04, EVO E-01), (3) **perf A/B + absolute SLO** (`perf-gate.yml` + plan 05 T4),
  (4) **supply-chain gate** (cargo-deny/audit + plan 02 signing/provenance). Wire
  the three dead interner suites (EVO E-06) — the only code change here.
- *Where:* `docs/development/FITNESS_FUNCTIONS.md`; `ci.yml` (name the three suites).
- *Verify:* each arm is a *blocking* CI job; the interner suites now execute (CI log shows non-zero tests).

**D2 — Control-plane staged-rollout + API/DSL versioning & deprecation policy.**
- *What:* feature-flag/staged-rollout discipline for the *control plane itself*
  (canary the management plane the way we canary policy); a public API + DSL
  version & deprecation policy (DSL carries a language version per plan 04; API is
  contract-gated per round-1 plan 07 — write the *deprecation window* rules).
- *Where:* `docs/development/VERSIONING_AND_DEPRECATION.md`; `docs/deployment/CONTROL_PLANE_ROLLOUT.md`.
- *Verify:* a documented deprecation window + a control-plane canary procedure exist and are referenced by the runbooks.

**D3 — Dogfooding: Reaper authorizes its own control plane (Plan 01 Phase D).**
- *What:* scope the long-deferred dogfood — the management plane's own admin
  actions gated by a Reaper policy — as the ultimate fitness function. This plan
  *schedules and specifies* it; the build is plan 01's Phase D.
- *Where:* section in `docs/development/FITNESS_FUNCTIONS.md`; tracked issue against plan 01.
- *Verify:* a phased dogfood proposal exists with a first milestone (e.g. audit:erase gated by a self-hosted policy).

**D4 — SLO/error-budget operating model.**
- *What:* define control-plane SLOs + error budgets and the operating loop (budget
  burn → freeze feature rollout → focus on reliability). Closes PROD R3-10's SLO half.
- *Where:* `docs/deployment/SLO_ERROR_BUDGET.md`; feeds the A4 SLA sheet.
- *Verify:* SLOs are numeric, tied to A4, with a written budget-exhaustion policy.

---

## 5. Dependencies

This plan is **downstream of every other round-3 plan.** Mapping (round-3 numbering
per `08-SYNTHESIS.md` path-back; confirm against the round-3 roadmap):

| Needs | To close | Because |
|---|---|---|
| **Plan 01** (tenant-isolation authz backstop; SEC P0-1..4/P1-b + tenant-authz contract gate; Phase D dogfood) | C1 "multi-tenant isolation" claim, B1 tenant-isolation anchor, B2 pentest scope, D1 arm #1, D3 | You cannot market or pentest isolation before it is fixed *and* backstopped |
| **Plan 02** (release/supply-chain integrity: cosign, SLSA, image-bound SBOM, scan-before-push, SHA-pin) | C1 provenance claim, B1 supply-chain anchor, D1 arm #4 | DORA/SLSA evidence and the provenance claim depend on signing existing |
| **Plan 03** (decision-quality auto-rollback; PROD R3-1) | A4 SLO "safe-to-act" wording, C1 safety framing | The advertised safety pillar must watch decision quality before it is claimed |
| **Plan 04** (DSL as managed contract; EVO E-01) | D1 arm #2, D2 DSL versioning, C1 DSL-stability | A sellable "decade-stable language" needs a version + frozen corpus |
| **Plan 05** (CLI + testing edges; blocking absolute/cold-start SLO, CLI release, DSL builtins oracle; TEST T1..T4) | A4 absolute SLO, C1 sub-µs *absolute* claim, A2 CLI onboarding | The CLI is the pilot's primary entry point and absolute latency must be gated |
| **Plan 06** (operational completeness: continuous SIEM shipper/Kafka, durable replay, event bridge, fleet convergence; PROD R3-2..R3-5) | A3 support-bundle convergence view, C3 SIEM "not-yet" line | Convergence read model feeds diagnostics; SIEM streaming gates a claim |
| **Plan 11** (already shipped) | A5 game-day, A4 RPO/RTO | Procedures exist; this plan *executes* them |

**External dependencies:** a provisioned HA-Postgres k8s environment (A5); a
booked third-party pentest vendor (B2); a design partner willing to run the pilot
(A1/A2 validation).

---

## 6. Testing & verification

This plan's "tests" are exercises and completeness checks, not unit tests:

1. **Non-author stand-up** of the reference deploy from A1/A2 docs — the runbook
   is verified only when someone who didn't write it succeeds.
2. **First DR game-day executed** (A5) with measured RPO ≤ 5 min / RTO ≤ 30 min or
   a filed remediation backlog — the capstone acceptance test.
3. **Support-bundle secret scan** — the A3 archive contains no unredacted secrets.
4. **Claims-matrix completeness** (C1) — zero rows marked "unevidenced"; a CI/doc
   lint could assert every claim line links a gate file.
5. **Evidence-index completeness** (B1) — the four anchors each link an artifact,
   not prose.
6. **Pentest report received and triaged** (B2) — findings routed into VULN_RESPONSE SLA.
7. **Fitness arms are blocking** (D1) — the four gates are required checks; the
   three interner suites run in CI.

---

## 7. Effort & phasing — S/M/L

Almost entirely **process/docs/coordination.** Only two small code touches:
`support-bundle` (A3, S) and wiring three dead CI suites (D1, XS).

- **Phase A (M) — draft-in-parallel while 01–06 build.** Author every template and
  index skeleton, the supported-config matrix, onboarding runbook, claims-matrix
  rows, competitive/not-yet-claimed sheets, and the evolve-in-prod docs. None of
  this blocks on 01–06 *merging*; it blocks on the *design* being known, which it
  is. Land the two small code changes.
- **Phase B (S–M) — close as dependencies land.** As each of 01–06 merges, flip its
  evidence rows from *must-produce* to *evidence-exists*, and its claim rows from
  "undefended" to a green gate.
- **Phase C (M) — execute the live exercises.** Run the first DR game-day (needs an
  environment) and commission + receive the pentest (needs 01 landed + a vendor;
  the long-lead item — book it early even though it runs late).

Overall: **M** for the arc, but calendar-bound by the pentest lead time and the
01–06 merge train, not by author-hours. Sequence: draft A → close as 01–06 land →
execute the exercises → green-light the pilot checklist last.

---

## 8. Key decisions (ADR-style)

**ADR-1: Evidence over assertion — no claim ships without a defending gate.**
- *Context:* round-3's core lesson is that the codebase's own fitness function for
  authz was checking the wrong property; "we reviewed it" is not evidence.
- *Decision:* every marketed claim and every compliance control links a *gate,
  artifact, or measured exercise*. A claim with no green gate moves to
  "not-yet-claimed"; a control with no artifact is *must-produce* with an owner.
- *Consequence:* marketing is bounded by what CI/exercises defend; the launch is
  honest and the review board gets links, not adjectives.

**ADR-2: Security is not "done" on internal review — commission an external pentest.**
- *Decision:* independent penetration test is a *required* launch gate, scoped to
  the exact defect classes plan 01 fixes, run against a build with 01–06 landed.
- *Consequence:* the isolation claim is defended by an outside party, matching what
  a bank's third-party-risk process actually requires.

**ADR-3: This plan builds no features — it packages them.** Any temptation to fix
a residual R3-2..R3-10 P2/P3 here is redirected to plan 06 / a follow-up. The only
code permitted is `support-bundle` and CI wiring, both in service of *evidence*.

**ADR-4: The fitness-function suite is the product's evolve-in-prod contract.** The
four arms (tenant-authz, DSL corpus, perf+SLO, supply-chain) are declared the
regression backstop of record; changing them requires the same review as changing
a public API. Dogfooding (D3) is the aspirational fifth arm.

---

## 9. Risks & rollback

- **Risk: launching before 01–06 land.** The claims matrix would carry undefended
  rows (isolation, provenance). *Mitigation:* C1 is a hard gate — undefended claims
  auto-move to C3; the pilot checklist cannot be green-lit with an open anchor in
  B1. *Rollback:* delay the external announcement; a design-partner pilot under NDA
  can still run on a CONDITIONAL build with scoped claims.
- **Risk: the first DR game-day misses RPO/RTO.** *Mitigation:* A5 files misses as
  tracked issues; the SLA sheet (A4) publishes only *measured* numbers, so a miss
  changes the published SLA rather than shipping a false one. *Rollback:* none
  needed — measuring reality is the point.
- **Risk: pentest lands late and finds a P0.** *Mitigation:* book the vendor in
  Phase A (long lead) even though it runs in Phase C; route findings through the
  existing VULN_RESPONSE SLA. *Rollback:* a P0 pentest finding re-opens the launch
  gate — that is the gate working, not failing.
- **Risk: docs drift from reality** (the exact failure EVO E-05 flagged for
  architecture docs). *Mitigation:* every doc cites `file:line`/artifacts so drift
  is detectable; the claims/evidence indexes are the canonical source and are
  reviewed at each 01–06 merge. *Rollback:* none — indexes are additive.
- **Risk: "evolve-in-prod" becomes shelfware.** *Mitigation:* D1 makes the four
  arms *blocking CI*, D4 makes the SLO/error-budget an operating loop with a
  freeze trigger, D2 makes deprecation a written window — mechanisms, not intentions.
- **Overall posture:** this plan changes no runtime behavior and ships no feature,
  so there is nothing to roll back in production. Its only failure mode is
  *launching on incomplete evidence* — which every gate here exists to prevent.
