# Reaper Enterprise Readiness Review — Round 3 Synthesis

**Date:** 2026-07-16 · **Scope:** engine, services, functions, APIs (UI out of scope) ·
**Reviewers:** 7 (4 original + 3 new: evolutionary architect, testing guru, CI/CD expert) ·
**Bar:** would a UK bank's platform-security review board, SRE org, and enterprise
architecture council approve this for production authorization decisions?

---

## Overall verdict: **NOT READY**

The gate is "worst persona wins." Two personas returned **NOT READY**:

| # | Persona | Verdict | P0 | P1 |
|---|---|---|---|---|
| 01 | Performance Engineer | READY | 0 | 0 |
| 02 | **Security Engineer** | **NOT READY** | **4** | 2 |
| 03 | Code Quality & API | CONDITIONAL | 0 | 0 |
| 04 | Product Architect | CONDITIONAL | 0 | 1 |
| 05 | Evolutionary Architect (Fowler) | CONDITIONAL | 0 | 1 |
| 06 | Testing Guru | CONDITIONAL | 0 | 4 |
| 07 | **CI/CD & Release Eng.** | **NOT READY** | **1** | 5 |

The verdict is not close and it is not a matter of judgement: there are **five P0s**
across two independent lanes — four cross-tenant / account-takeover defects in the
authorization layer, and one supply-chain integrity hole in the release pipeline.
Either lane alone fails a bank review. This is a **regression in the authorization
layer specifically** — rounds 1 and 2 hardened the anonymous-attacker perimeter and
the audit pipeline (both verified closed this round), but the tenant-isolation layer
the round-2 auth gateway explicitly delegated to handlers was never uniformly
enforced, and five route groups skipped the check.

**Important nuance for leadership:** the *engineering underneath is strong and the
prior remediation was real, not cosmetic.* Every round-1 and round-2 blocker that
was claimed closed was verified closed against source. The core evaluation engine,
audit tamper-evidence, hot-swap, and perf gates are genuinely good. The failures are
concentrated, not systemic — but they are exactly the failures that matter most for
this product, because **Reaper is the authorization infrastructure**: a tenant-isolation
break here is an authorization break in every application that trusts it.

---

## The single most important cross-cutting theme

**The authorization / tenant-isolation layer is the weakest subsystem, and it is
weak in a structurally predictable way.** The round-2 auth gateway authenticates
correctly but does **not** authorize — it delegates per-resource tenant scoping to
individual handlers. That pattern has no backstop: any handler that forgets the check
silently becomes a cross-tenant hole, and the contract test only proves routes are
*authenticated*, not *authorized-to-the-right-tenant*. Three reviewers hit this from
different angles:

- **Security** found four P0s and two P1s that are all instances of it (missing/incorrect
  per-tenant authz).
- **Product** found the auto-rollback safety pillar wired to the wrong signal — a
  governance/control-integrity gap in the same promotion/deployment subsystem.
- **Code & API** independently rated its scope "no open P0/P1," having enumerated
  routes for *authentication* — which is precisely the blind spot: route-presence
  checks pass while per-tenant authorization fails. **Adjudication:** security's
  deeper token→query→resource trace wins over code-&-api's route enumeration; the
  divergence itself is evidence that "every route has an auth check" is the wrong
  fitness function — it must be "every route authorizes the resource's owning tenant."

Secondary cross-cutting themes:

- **Supply-chain / release integrity (CI/CD P0 + 5 P1s).** Nothing binds
  "scanned/reviewed" to "what a customer pulls": no signing, no provenance, SBOM not
  artifact-bound, and the image scan runs *after* the push on release.
- **The policy DSL is an unmanaged public contract** (Evolutionary P1) that is also
  **under-verified** (Testing P1: builtins outside the differential oracle; Security
  P2: non-verifying JWT builtin). Customers write policy in it and auditors read it;
  it silently changed syntax v1→v2 with no language version and a published reference
  that no longer parses.
- **The CLI is both untested and unshippable** — Testing P1 (zero active CLI
  integration tests), Code P2 (same), and CI/CD P1 C4 (the release job references a
  non-existent toolchain action, so the advertised CLI tarballs were never built).
  The customer's primary CI/CD entry point is unverified end to end.
- **The "fast" star is defended only relatively.** The blocking perf gate is a strong
  paired-A/B *delta* gate, but absolute SLA and cold-start are nightly/non-blocking
  and p999 is in no blocking gate (Testing P1 + Performance P2).

---

## Top 10 findings across all reports (ranked by risk to enterprise adoption)

| Rank | ID | Sev | Source | Location | Finding |
|---|---|---|---|---|---|
| 1 | SEC P0-1 | P0 | Security | `auth/sso/broker.rs:67-70` | OIDC login adopts an account by global email with **no `email_verified` check** — a self-service org admin points their org's SSO at an IdP they control, asserts a victim's email, and inherits the victim's identity/roles across every org. Full account takeover. |
| 2 | SEC P0-2 | P0 | Security | `api/webhook_subscriptions.rs:133-357` | Six CRUD handlers have **zero auth/scope/tenant check** — any authenticated user manages any org's webhook subscriptions by slug and exfiltrates its event stream. |
| 3 | SEC P0-3 | P0 | Security | `sync/git.rs:108-118`, `api/sources.rs:258,573` | `installation_id` flows unvalidated into source config; the shared GitHub App mints a **victim tenant's** installation token, cloning their private repos into the attacker's org. |
| 4 | SEC P0-4 | P0 | Security | `api/webhooks.rs:153` | Public bundle-update webhook verifies signature only `if webhook_secret.is_some()` — a secretless source yields **unauthenticated SSRF + credential exfil**. |
| 5 | CICD C1 | P0 | CI/CD | `docker.yml`, `release.yml` | **No signing (cosign/minisign), no SLSA/provenance, no image-bound SBOM** on any shipped artifact; `scan-images needs: build-images`, so Trivy runs *after* the image is pushed to ghcr. Nothing ties "reviewed/scanned" to "what a customer pulls." |
| 6 | PROD P1-new | P1 | Product | `db/.../agent_deployment.rs:141`, `deployment/service/mod.rs:582` | The advertised autonomous **auto-rollback fires on bundle-*apply* failure rate, not runtime decision quality** — a valid policy that deploys cleanly then denies/allows wrongly (the likeliest bank failure) never self-reverts. Right loop, wrong signal. |
| 7 | SEC P1-b | P1 | Security | deployment rollout/pin handlers | Handlers authorize the path org but mutate a **global-UUID resource with no resource-org recheck** — cross-tenant deployment mutation. (Same class as #1–4.) |
| 8 | EVO E-01 | P1 | Evolutionary | `reap/` grammar & bundle versioning | **Policy DSL is an unmanaged public contract**: silent v1→v2 syntax change, no language version, no frozen decision corpus, published reference no longer parses. Silent semantic drift changes deployed authorization decisions. |
| 9 | TEST T1/T2 | P1 | Testing | `mutation.yml:69-77`; DSL builtins | Can't fully stand behind the **logic star**: mutation testing is **advisory** (adequacy metric can never fail) and DSL builtins (jwt/regex/time/comprehension) sit **outside the differential oracle**, relying on example tests only. |
| 10 | CICD C4 + TEST T3 | P1 | CI/CD + Testing | `release.yml:181`; `tools/reaper-cli` | The **CLI is unshippable and unverified**: release uses non-existent `dtolnay/rust-action@stable` (CLI tarballs never produced) and the CLI has **zero active integration tests** (BDD is a disabled empty stub). |

**Just below the line (would be 11–14):** SEC P1 SSRF cluster on API-source/bundle-URL
+ universal redirect-following; CICD C5 zero SHA-pinned actions (`@master`) under a
compromised-CI threat model; TEST T4 absolute/cold-start perf not blocking-gated;
PERF P2 pruning index inoperative for ABAC/ReBAC shapes (per-request O(N log N) +
256-candidate denial cliff at scale).

---

## Where reviewers agreed and disagreed

- **Strong agreement** that prior remediation is real: performance, security, code,
  product, and evolutionary reviewers all independently verified round-1/round-2
  closures against source rather than trusting the STATUS banners.
- **The one apparent disagreement** — Code & API rating "no open P0/P1" while Security
  found four P0s — is not a contradiction but a scoping artifact (auth-*presence* vs
  auth-*correctness-per-tenant*), adjudicated above in favour of Security. It is the
  most important lesson of this round: the codebase's own fitness function for authz
  is checking the wrong property.
- **Performance's lone READY** is correctly scoped to the eval hot path and does not
  soften the overall verdict; its own P2s (ABAC/ReBAC pruning, capability-gate verify
  cost) are real but sub-blocking.

---

## If a UK bank's architecture review board saw this tomorrow, the first three things they would reject it for

1. **Cross-tenant isolation failures in the authorization product itself.** Four
   independent P0s — OIDC account takeover, unauthenticated webhook-subscription CRUD,
   cross-tenant GitHub-App repo theft, and a secretless-webhook SSRF — mean tenant A
   can reach tenant B's identity, data, and secrets. For a product whose entire value
   proposition is *correct authorization*, this is disqualifying on sight; it fails
   SOC 2 CC6 (logical access), ISO 27001 A.9, and multi-tenancy assurance outright.
2. **A release/supply-chain pipeline that cannot prove what it ships.** Unsigned,
   un-attested artifacts with no image-bound SBOM and a vulnerability scan that runs
   *after* publication fail DORA ICT-risk and third-party-integrity expectations and
   SLSA provenance requirements — a regulated bank cannot attest the software it runs.
3. **The operational-resilience and correctness-assurance story doesn't hold up.**
   The advertised autonomous safety net (auto-rollback) watches the wrong signal, so a
   bad *decision* never self-reverts; and the correctness/performance claims are only
   partially defended (mutation testing advisory, DSL builtins un-oracled, DSL an
   unversioned contract, absolute/cold-start latency ungated) — so the vendor cannot
   evidence the operational-resilience assertions FCA/PRA SS2/21 and DORA require.

---

## Path back to CONDITIONAL, then READY (recommended sequencing)

1. **Close the four authz P0s and the SSRF/deployment P1s**, and — critically —
   **replace the delegated-authz pattern with a structural backstop**: a
   tenant-scoping extractor enforced centrally, plus a fitness function in
   `tests/api_contract.rs` that fails when a mutating route lacks resource-tenant
   authorization (not just authentication). This turns the whole finding-class off.
2. **Fix release integrity (CICD P0 + P1s):** sign images/binaries/bundles (cosign),
   generate build provenance/SLSA attestation, bind the SBOM to the published digest,
   move the Trivy gate *before* push, scan the exact multi-arch digest that ships, fix
   the broken CLI-release toolchain ref, and SHA-pin all actions.
3. **Reconnect the safety pillar:** feed the decision buffer's live allow/deny/eval-error
   rate into the existing auto-rollback trigger.
4. **Make the DSL a managed contract:** add a language version + magic/version/fail-closed
   reject (the pattern already exists one layer down in the bundle format), freeze a
   decision-corpus regression suite, and republish a parsing reference.
5. **Verify the customer edges:** CLI integration tests + a working CLI release build;
   promote mutation testing and absolute/cold-start SLO to blocking; bring DSL builtins
   under the differential oracle.

**The single most important next move:** close the tenant-isolation P0s *and* install
the structural authz backstop — everything else is finishing work on a fundamentally
sound system, but this class of defect must be made impossible to reintroduce, not
merely patched four times.
