# 05 — Evolutionary Architecture (Fowler lens) — Round 3

**Reviewer:** Subagent 5, Evolutionary Architect (Building Evolutionary
Architecture tradition). **Scope:** coupling/seams, optionality, reversibility,
fitness functions, one-way doors, scaling discontinuities, doc/reality drift —
*not* today's bugs. UI out of scope. Evidence is `file:line` or doc reference;
absence claims name where I looked.

---

## VERDICT: **CONDITIONAL** (0 P0, 1 P1, 6 P2, 4 P3)

The architecture is **more evolvable than most, and it proved it since round 2**:
the two highest-leverage bets the round-2 architect named — agentic authz (F1)
and the wasm build target (F2) — both *shipped*, and they landed **additively
through the existing seams** (a defaulted trait method, additive `PolicyRequest`
fields, feature-gates, a capability token reusing the bundle-signing machinery)
without a core rewrite. That is the strongest possible evidence that the
load-bearing abstraction (an I/O-free, deterministic engine core behind a total
`PolicyEvaluator` trait) is real, not asserted. The single thing that would fail
a decade-horizon review is **not the code — it is the policy *language* as an
unmanaged public contract**: the `.reap` DSL has already silently changed syntax
(v1→v2) with no language version, no frozen decision corpus, and a published
reference that no longer parses. Everything else is compounding evolvability debt
(enum duplication, a deprecated-but-advertised language, hardcoded distribution
transport, stale architecture docs) that is cheap now and expensive later.

---

## Exec summary (≤10 lines)

1. **Round-2's two strategic bets shipped through the seams, additively** — F1
   (capabilities/taint/allow-explain/MCP) and F2 (wasm cdylib + 3-leg parity in
   CI). The I/O-free boundary held. This *validates* the architecture's central claim.
2. **P1 — the DSL is an unversioned, undefended public contract.** No language
   version, no golden policy+expected-decision corpus, and the customer-facing
   grammar reference documents non-parsing v1 syntax while the engine runs v2.
3. **The two "DSL surfaces" are not duplication — they are tiered compilation**
   (compiled `ReaperDSLEvaluator` + AST `ReapAstEvaluator`), pinned identical by a
   blocking differential. Keep it; it is an asset, not debt.
4. **Three-language pluralism is now a liability with a sharp edge:** `Simple` is
   `DEPRECATED — scheduled for removal` *and* unsafe-by-design (ignores
   action/principal/context) yet still advertised as a language and deployable.
5. **`PolicyLanguage` is duplicated** across engine and control plane and
   stringly-typed in sync, with no contract test — coincidental wire agreement.
6. **Distribution transport is hardcoded HTTP/SSE** — no `Transport` port; the
   poll→push→xDS evolution the strategic direction implies needs core surgery.
7. **Storage and SQL-query layers are textbook clean ports** — the portability
   tax is only duplicated per-dialect migration authoring.
8. **The I/O-free firewall is enforced only *indirectly*** (wasm-build), so an
   optional or `cfg`-gated I/O dep would slip past. tokio is already optional-prod.
9. **Doc/reality drift in the primary architecture references** — they describe
   the pre-F1/F2 system; one audit doc still claims "~45% of the vision."
10. **Bundle/wire format is the model of good evolution** — magic + version byte
    + fail-closed reject of newer versions. The DSL should copy this discipline.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| E-01 | **P1** | `crates/policy-engine/src/reap.pest`; `docs/reference/reap-language.md`; `docs/development/DSL_V2_DESIGN.md` | DSL is a public contract with no language version, no frozen decision corpus, and a published reference that documents non-parsing v1 syntax (`when`/`package`/`metadata{}`) while the engine runs v2 (`if`/`input`/violations). | Silent semantic drift on deployed policies is unguarded; customers write against docs that don't parse. The one evolutionary hazard the brief calls P0-class. | Add a language-version declaration + a frozen `policy+expected-decision` golden corpus in CI; regenerate the reference from the grammar. |
| E-02 | P2 | `crates/policy-engine/src/engine/types.rs:38`; `services/reaper-management/src/domain/policy.rs:13`; `services/reaper-sync/src/server_client.rs:34` | `PolicyLanguage` is defined **twice** (engine enum vs management enum) and **stringly-typed** in sync; no contract test asserts wire agreement. Cedar is feature-gated in the engine but unconditional in the management enum. | Adding/renaming a language is a multi-seam change; drift is caught only in production. | Single source of truth (shared type in `reaper-core`) or a serde round-trip contract test across all three representations. |
| E-03 | P2 | `crates/policy-engine/src/evaluators/simple.rs:3-16`; `docs/index.md:24`, `docs/introduction/key-features.md:11`, others | `Simple` is `DEPRECATED — scheduled for removal` **and** ignores action/principal/context ("a rule that should apply to one principal/action applies to all"), yet is advertised as one of "three policy languages" and remains deployable via `PolicyLanguage::Simple`. | A customer can deploy a `simple` policy believing it enforces per-principal rules; it silently matches on resource only. Conceptual-integrity + latent authorization surprise. | Gate `Simple` behind an explicit opt-in feature or remove it; stop advertising it as an authorization language until the JSON-rule migration lands. |
| E-04 | P2 | `services/reaper-agent/src/management/client.rs:33`, `management/sse.rs:23`; `services/reaper-sync/src/server_client.rs:8` | Distribution transport is concrete `reqwest`+SSE end-to-end; **no `Transport`/`Subscription`/`DeltaSource` port**. | poll→push→gRPC/xDS-streaming (the direction "edge/agentic gate" implies) requires rewriting agent+sync internals — the least portable extension point. | Introduce a thin transport trait now (even with one HTTP/SSE impl) so the seam exists before it is needed. |
| E-05 | P2 | `docs/architecture/ARCHITECTURE.md`, `ARCHITECTURE_SUMMARY.md`; `docs/architecture/REAPER_MANAGEMENT_AUDIT.md:11` | Primary architecture docs omit F1/F2 (capabilities, taint, wasm, MCP); the management audit still claims "~45% of the vision / central decision-log ingestion essentially absent / billing a pure Stripe placeholder" after A/E workstreams shipped exactly those. | The team's and a buyer's mental model of the system is wrong — itself an evolvability risk. | Refresh the architecture references to the post-F1/F2 tree; date-stamp or retire the stale audit. |
| E-06 | P2 | `.github/workflows/ci.yml:289,305-310`; `crates/policy-engine/tests/{eval_interner_bounding,rebac_interner_bounding,migration_rename_interner}_tests.rs` | Three of four interner-bounding suites are **never executed in CI** (CI runs `--workspace --lib` + explicitly named `--test` targets only). Only `context_interner_leak_tests` is wired. | The memory-bound invariants behind the "~60% memory reduction" claim are advisory/dead; unbounded-growth regressions ship silently. | Name the three suites in the integration-tests job. |
| E-07 | P3 | `crates/policy-engine/src/evaluators/mod.rs:21-22` | Comment "`ReaperDSLEvaluator` … not yet used … kept for future features" — it is the **primary served evaluator** via `build_preferred`. | Misleads maintainers about what the hot path runs. | Delete/fix the comment. |
| E-08 | P3 | `deny.toml:126` (`bans = []`); `.github/workflows/ci.yml:595-640` | The I/O-free firewall is enforced only **indirectly** by the wasm build, which catches only *unconditional* native deps. `tokio` is already an optional-prod dep (`Cargo.toml:60`); a new `cfg(not(wasm32))`-gated or optional I/O dep would slip past. | The single most important architectural invariant has no positive assertion. | Add an explicit `cargo-deny` ban (tokio-rt/sqlx/reqwest/axum in `policy-engine` non-dev deps) or a small dependency-graph architecture test. |
| E-09 | P3 | `services/reaper-management/migrations/` (26 files) vs `migrations_pg/` (19 files) | SQL schema is authored **twice** per dialect. | Dual-authoring tax; the two dialects can silently diverge. | Treat SQLite as a test double with a generated/checked-subset schema, or add a cross-dialect schema-parity check. |
| E-10 | P3 | `services/reaper-management/src/integrations/servicenow.rs:55`, `integrations/mod.rs:5` | ITSM integration is a single concrete `ServiceNowClient` with no `ChangeManagement`/`Itsm` trait. | A second ITSM vendor (Jira/PagerDuty) needs core surgery to introduce the missing port first. | Extract the port when the second vendor is on the roadmap (not before). |

---

## Detailed findings (P1)

### E-01 (P1) — The policy DSL is an unversioned, undefended public contract, and it has *already* drifted

A policy language is the one artifact customers author and commit to their own
repos; its semantics are a public API. Judged as such, the `.reap` DSL fails the
evolvability bar on every axis except its binary packaging:

**No language version exists.** A full-repo search for `language_version`,
`dsl_version`, `grammar_version`, `syntax_version` returns zero hits. The
grammar file `crates/policy-engine/src/reap.pest` carries no version marker
(header `:1-2` is prose). The only "version" concepts are the *policy content*
version (metadata string, `reap.pest:21`) and the redeploy counter
(`EnhancedPolicy.version`) — neither versions the language. A `.reap` file cannot
declare which grammar/semantics it targets, and the engine has no notion of which
version it implements.

**The drift is not hypothetical — it already happened.**
`docs/development/DSL_V2_DESIGN.md` documents a v1→v2 evolution with "Phase 1 —
Structured `input` document — ✅ IMPLEMENTED" and "Phase 2 — Violations with
messages — ✅ IMPLEMENTED." The grammar now requires `if` (`reap.pest:35`), the
`input` entity, and violation messages. Meanwhile the **customer-facing language
reference** `docs/reference/reap-language.md` still documents v1: `when` clauses,
a `package` keyword, and a `metadata { }` block (grammar-agent cites
`reap-language.md:56-93`) — **none of which the current parser accepts.** The
published contract a customer would write against does not parse. That is the
semantic-drift hazard manifested, silently, with no version to distinguish the two.

**Nothing freezes decisions.** There is no golden corpus of
`frozen policy + fixed inputs → expected decision` (searched `golden|frozen|
snapshot|expected.decision` — only unrelated data-store snapshots). The closest
test (`version_migration_tests.rs:220-238`) only asserts the *current* grammar
still parses a policy; it pins no decision. The blocking compiled≡AST differential
(`compiled_ast_equivalence_tests.rs`, `differential_parity_tests.rs`) and nightly
mutation testing are excellent, but they guard *compiler-vs-interpreter agreement*
— they do **not** catch a change to an existing operator/builtin's semantics
applied to *both* paths. Builtins/methods are added by editing hardcoded matches
(`ast_evaluator/function_dispatch.rs:578`, `ast.rs:365-410`) with no `since`
tag, no stability tier, no deprecation path.

**Why P1 and not P0:** blast radius is bounded — an *unknown* builtin/keyword is
a hard `InvalidPolicy` error, so additive syntax can't silently break old
policies, and the differential catches a large class of regressions. But the
*semantics* of an existing construct can drift undetected, and the mis-documented
reference is a live contract defect. Before this is sold as a decade-stable
authorization language to a regulated buyer, it needs: (1) a language-version
declaration carried in source and bundle; (2) a frozen decision corpus in CI;
(3) a reference generated from (or contract-tested against) the grammar.

**The fix pattern already exists in the same codebase** — the bundle wire format
does versioning correctly (magic `b"REAP"` + `FORMAT_VERSION` + fail-closed reject
of newer versions, `reap/bundle.rs:55-119`; envelope `ENVELOPE_V2` with
`EnvelopeVersionUnsupported`, `bundle_signing.rs:73,450-465`). Copy that
discipline up one layer to the language.

---

## Assessment against the brief's prompts

**Module boundaries & the I/O-free core.** Verified and holding. `policy-engine`
production deps are serde/dashmap/pest/regex/bumpalo/rayon/crypto with no
tokio-rt/sqlx/reqwest/axum unconditionally (dev-deps only); `reaper-wasm` builds
the same core to `cdylib` for `wasm32-unknown-unknown` and a 3-leg parity suite
(native wrapper + wasm-in-Node + library manifests) runs in CI. The boundary is
now *load-bearing in production* (wasm ships), not just aspirational. Caveat E-08:
the firewall is enforced indirectly.

**The two DSL surfaces — asset, not debt.** This is the brief's explicit
strategic question. The two are **not** two languages: `reap/` is the
parser/compiler front-end; `ReaperDSLEvaluator` (compiled) is what it targets;
`ReapAstEvaluator` (AST) is a full-feature interpreter fallback. `build_preferred`
(`reap/mod.rs:133`) tries the compiled tier and falls back to AST, and the two are
pinned to identical decisions by a **blocking** differential (example-based +
proptest, `ci.yml:302,1089`). This is textbook tiered compilation (JIT + interpreter
oracle), and the interpreter doubling as the compiler's correctness oracle is a
genuine strength. Keep it. Do **not** converge them.

**Extension points.** New evaluator/language: the `PolicyEvaluator` trait is
clean and total, and evolved *additively* (`evaluate_matched`/`evaluate_named`
defaulted, `resource_index_terms` defaulted to safe `None`). But end-to-end wiring
is multi-seam: `build_evaluator_with_data` (`engine/policy.rs:173`) exhaustively
matches `PolicyLanguage`, which is duplicated in the control plane (E-02), plus
bundle format + CLI. New storage backend: **clean port** (`BundleStorage` trait,
one factory, `storage/traits.rs:83`). New SIEM sink: enum-dispatch (moderate). New
distribution transport: **hardcoded** (E-04). New ITSM: **hardcoded** (E-10).

**Fitness functions — strong where wired, with two holes.** Enforced-blocking:
api-contract parity with ratchets (`api_contract.rs`, `ci.yml:221`), perf A/B gate
(10%/50% micro, 1.25x served p99, `perf-gate.yml`), compiled≡AST equivalence,
delta≡rebuild determinism proptest, context-interner leak, PR fuzz smoke. Advisory/
nightly: perf-tracking, slo-harness (multiplier 250 — order-of-magnitude only),
mutation (report-only). **Should be fitness-functioned but isn't:** (a) the
I/O-free dependency-direction rule as a *positive* assertion (E-08); (b) three
interner-bounding suites are dead in CI (E-06); (c) the DSL golden decision corpus
(E-01); (d) a `PolicyLanguage` wire-agreement test (E-02).

**Scaling discontinuities.** The agent is a clean quantum that scales
horizontally (in-process DashMap store, delta-sync, fail-closed staleness budget);
10k+ reapers is "tune a constant" *for the read path* but the SSE fan-out +
hardcoded HTTP transport (E-04) is where the control plane's push cost concentrates.
10^8 ReBAC tuples: the bounded-BFS budget (`data/relationships.rs`) protects eval
latency but the in-process store is per-agent memory-bound — a genuinely large
tuple set forces an external relationship store, which is a re-architecture, not a
tune. Multi-region control plane and consistency tokens remain deferred (correctly
flagged, not yet built). Control-plane monolith: still one binary; fracture lines
(identity/SSO, policy/GitOps, distribution, audit/replay, billing) visible but
decomposition is correctly *not* done yet.

**Doc/reality drift.** E-05. Also the language reference (E-01) and the stale
evaluator comment (E-07).

---

## Evolvability scorecard

| Dimension | Rating | One-line evidence |
|---|---|---|
| Module seams / I/O-free core | **Strong** | `policy-engine` prod deps carry no I/O; wasm cdylib ships the same core (`reaper-wasm`, `ci.yml:600`). Enforcement only indirect (E-08). |
| DSL language evolution | **Weak** | No language version, no frozen decision corpus, published reference doesn't parse (E-01). |
| Extension points | **Adequate** | Trait clean + additive; storage a clean port; but language wiring multi-seam and transport/ITSM hardcoded (E-04, E-10). |
| Fitness functions | **Strong** (with holes) | Blocking api-contract, perf A/B, compiled≡AST, delta≡rebuild; but dead interner suites + no dep-direction assertion + no DSL corpus. |
| Data-model evolution (migration engine) | **Adaptable** | Typed `ModelTransform` + planner + rollback + PG versioned/checksummed migrator; tax is dual-dialect authoring (E-09). |
| Distribution evolvability | **Weak** | HTTP/SSE hardwired end-to-end, no transport port (E-04). |
| Storage portability | **Strong** | `BundleStorage` textbook port; sqlx `Any` a clean query-level port (`storage/traits.rs:83`, `db/connection.rs:171`). |

---

## One-way-door register

| Decision | Reversibility | Made prematurely? | Escape hatch | Recommendation |
|---|---|---|---|---|
| **sqlx `Any` (SQLite-dev / PG-prod duality)** | Med | No — a real dev-convenience seam | Query layer is a clean port; only migrations are dual-authored (E-09) | Keep; relegate SQLite to an explicit test double, budget a PG-native repo layer as audit/tenant volume grows. |
| **`.rbb`/`.rpp` + signing envelope wire format** | Low | No | **Escape hatch is built in** — magic + version + fail-closed reject (`bundle.rs:55-119`, `bundle_signing.rs:450`) | None. This is the reference example; copy it to the DSL (E-01). |
| **Three-language pluralism** (`Simple` deprecated/unsafe, Cedar feature-gated/half-wired, DSL primary) | Med | Yes — pluralism outran depth | Cedar already behind a feature flag; `Simple` removal path noted but unexecuted | Decide: "DSL company that imports Cedar," not "multi-language runtime." Gate/remove `Simple` (E-03); stop advertising three co-equal languages. |
| **Hardcoded HTTP/SSE distribution transport** | Low | Yes-ish — the seam is cheap now, costly to retrofit | None today (E-04) | Introduce a `Transport` trait with one HTTP impl before the push/xDS ask lands. |
| **In-process DashMap store (agent) + single-region control plane** | Agent: High (correct quantum) / Control plane: Med | No | Agent store is the right choice; multi-region CP explicitly deferred | Leave the agent store; keep multi-region + consistency-tokens on the register with a review date, not defaulted-permanent. |
| **DSL semantics unversioned** (E-01) | Med (additive fix possible) | Yes | None yet — no version, no golden corpus | Add language version + frozen decision corpus *before* the next semantic change, or the drift becomes irreversible-in-the-field. |

---

## Opportunity roadmap (highest-leverage architectural moves, ordered)

1. **★ Version and freeze the DSL as a public contract (E-01).** Add a language
   version carried in source + bundle, a golden `policy+inputs→decision` corpus in
   CI, and a generated-from-grammar reference. **This is the single most important
   move** — it converts the language from an unmanaged liability into the durable,
   sellable contract the rest of the platform's optionality depends on, and it is
   cheap (the bundle format already shows the pattern). Everything F1/F2 opened up
   (embedding, edge, MCP) is only as trustworthy as the language's stability guarantee.
2. **Make the I/O-free boundary a *positive* fitness function (E-08).** An explicit
   cargo-deny ban or dep-graph test. The whole platform's future optionality
   (wasm/edge/embedded/replay) rests on this one invariant; today it's guarded by a
   side effect. One config change buys the guarantee.
3. **Introduce a `Transport` port on the distribution path (E-04).** The strategic
   direction (agentic edge gate, xDS-style streaming) needs push/gRPC; retrofitting a
   trait after the concrete impl has metastasized across agent+sync is far costlier
   than extracting it now with a single HTTP implementation behind it.
4. **Resolve the three-language decision explicitly (E-02, E-03).** Unify
   `PolicyLanguage` to one source of truth, gate/remove the deprecated-unsafe
   `Simple`, and formalize Cedar as an *import* path behind its feature. This pays
   down the "capability breadth outrunning depth" debt the round-2 architect named.
5. **Wake the dead fitness functions and refresh the architecture docs (E-06,
   E-05).** Cheap hygiene that restores the team's true mental model and defends the
   memory claims.

---

## Absence checks (where I looked and found nothing)

- **Language version / grammar version:** searched `language_version|dsl_version|
  grammar_version|syntax_version` repo-wide (incl. `reap.pest`, `bundle.rs`) — none.
- **Frozen/golden decision corpus for the DSL:** searched `golden|frozen|snapshot|
  expected.decision` — only unrelated data-store/decision snapshots; no
  policy→expected-decision freeze.
- **Consistency tokens / zookies:** searched `zookie|consistency.token|snapshot.
  token|new.enemy` — present only as *deferred* backlog notes
  (`docs/development/DATA_PLANE_PLAN.md:200-225`, `DATA_PROTECTION_PEP_DESIGN.md:408`);
  zero implementation. Correctly deferred, not silently missed.
- **Distribution `Transport`/`Subscription` trait:** grepped `reaper-agent/src/
  management/` and `reaper-sync/src/` — only concrete `reqwest`/SSE, no port.
- **ITSM/ChangeManagement trait:** `integrations/` holds only `ServiceNowClient`;
  no `connectors/` directory exists.
- **Shared `PolicyLanguage` type / wire-agreement test:** searched management
  `tests/` and sync — none; enum is defined twice + a stringly-typed sync copy.
- **Cedar upgrade-semantics note:** searched `docs/` for Cedar breaking-change /
  upgrade analysis — none; pinned `4.2` with no deployed-policy compatibility note.

## What's done well (≤5)

- **Round-2's two strategic bets shipped additively through the seams** (F1
  agentic authz, F2 wasm cdylib) — the strongest possible proof the architecture
  evolves without a rewrite. `evaluate_named` defaulted, `PolicyRequest` actor/
  provenance additive, capability token reusing `bundle_signing`.
- **Tiered-compilation DSL with the interpreter as the compiler's oracle**, pinned
  by a blocking compiled≡AST differential — a correctness architecture most
  engines lack.
- **The bundle/wire format is the reference model of good evolution** — magic +
  version + fail-closed reject of newer versions (`bundle.rs:55-119`,
  `bundle_signing.rs:450`).
- **Storage and SQL-query layers are clean ports** (`BundleStorage` trait; sqlx
  `Any` + `numbered_placeholders` helper), so backend/dialect change is localized.
- **A genuine, blocking fitness-function suite** (api-contract parity, perf A/B,
  delta≡rebuild determinism) — architectural integrity held by tooling, not by
  hoping.

---

## What I did NOT cover

- I did not re-audit the round-1/round-2 *tactical* closures (auth, audit-chain
  crypto, HA/DR) except where they touch evolvability — those are personas 06/07
  and the tactical reviewers.
- I did not deeply verify the decision-log *entry* schema's own versioning (only
  that it carries policy/data/model versions); replay-schema forward-compat is worth
  a dedicated check by the testing/security personas.
- I did not benchmark or re-measure any performance claim (persona 06/perf).
- Cedar evaluator internals and the eBPF crate were out of my time budget beyond
  their coupling surface.
- I relied on three sub-agent sweeps (fitness functions, DSL grammar/versioning,
  storage/transport seams) whose `file:line` citations I spot-verified against the
  tree but did not exhaustively re-open.
