# DSL as a Managed Public Contract

**Readiness gate:** Moves the correctness/evolvability pillar **CONDITIONAL → READY**. The policy DSL is the one artifact customers *author* and auditors *read*; today it is an unmanaged public contract that has already drifted (v1→v2) with no language version, no frozen decision corpus, and a published reference that no longer parses. This protects deployed policies from silent semantic drift.
**Priority:** P1 (Round-3 Evolutionary-Architecture E-01; Testing T2).
**Findings closed:** E-01 (DSL unversioned/undefended public contract; reference documents non-parsing v1 syntax), E-03 (deprecated-but-advertised, unsafe-by-design `Simple` language), and T2 (DSL builtins sit outside the differential/property oracle). Reaffirms E-02's "keep the two DSL surfaces" verdict by formalizing their tiering rather than converging them.

---

## 1. Goal

Make the `.reap` policy language a **managed, versioned, decision-frozen contract** so that a policy a customer commits to their repo today keeps producing the **same authorization decision** on every future engine version — or, if a decision must ever change, it changes *loudly*, behind an explicit language-version bump, never silently.

Concretely: (1) every published policy declares a **language version**, carried in source and in the compiled bundle, and an engine that does not implement that version **fails closed** (the exact discipline the bundle wire format already uses one layer down); (2) a **frozen `(policy, data, request) → expected decision` corpus** is checked in and gated blocking in CI as the *fitness function for semantic stability* — any decision drift fails the build; (3) the **builtins** (jwt/regex/time/comprehensions/string/math) are brought under the differential/property oracle so their correctness is machine-checked, not example-checked; (4) the **authoritative grammar + semantics reference** is regenerated from / verified against the real parser so it parses; (5) a written **backward-compatibility & deprecation policy** defines how to add a keyword/operator/builtin *without* changing any deployed policy's decision, with a deprecation window and machine-emitted warnings; and (6) the **two DSL surfaces** (compiled `ReaperDSLEvaluator` + AST `ReapAstEvaluator`) are documented as an intentional tiered-compilation asset while the deprecated, unsafe `Simple` language is gated behind an explicit opt-in and de-advertised.

**Non-goals:** changing evaluator semantics or the bundle format (nothing in `crates/policy-engine/src/reap/bundle.rs` or the evaluators changes behavior — this plan *pins* current behavior, it does not alter it); converging the two DSL surfaces (E-02 explicitly rules that out — the interpreter-as-compiler-oracle is an asset); designing DSL v3 (this plan makes v3 *possible to ship safely*, it does not author it); removing `Simple` outright (gate + de-advertise now; removal rides the separate JSON-rule migration).

---

## 2. Current state (evidence) — file:line

- **No language version exists anywhere.** The grammar header `crates/policy-engine/src/reap.pest:1-2` is prose; a full-repo search for `language_version|dsl_version|grammar_version|syntax_version` returns zero hits (E-01 absence check). The only "version" concepts are the *policy content* metadata string (`reap.pest:20-22`, a free `metadata_field`) and the redeploy counter `EnhancedPolicy.version` — **neither versions the language.** A `.reap` file cannot declare which grammar/semantics it targets, and the engine has no notion of which version it implements.
- **The drift already happened, in the field.** `docs/development/DSL_V2_DESIGN.md` records a v1→v2 evolution (structured `input` document + violation messages, both "✅ IMPLEMENTED"). The grammar now requires `if` (`reap.pest:35` — `rule = { "rule" ~ ident ~ "{" ~ decision ~ message_clause? ~ "if" ~ condition ~ "}" }`). Meanwhile the **customer-facing reference** `docs/reference/reap-language.md` still documents v1: `package` (`:18,58,61`), `metadata { }` (`:21,67`), and **32 `when { }` clauses** (`grep -c "when {"` → 32) — **none of which the current parser accepts.** The published contract a customer would write against does not parse.
- **The fix pattern already exists one layer down — copy it up.** The bundle wire format versions correctly: magic `b"REAP"` + `FORMAT_VERSION` (`reap/bundle.rs:55,57`) and a **fail-closed reject of newer versions** (`bundle.rs:111-119`: `if bundle.metadata.version > Self::FORMAT_VERSION { return Err(...) }`). The signing envelope does the same: `ENVELOPE_V2` (`crates/reaper-core/src/bundle_signing.rs:73`) with an explicit `EnvelopeVersionUnsupported { got, required }` error (`bundle_signing.rs:164-165`) and an anti-rollback monotonic `version`. The language is the only public contract *without* this discipline.
- **A decision corpus exists but is not *frozen*.** `crates/policy-engine/tests/policy_library_tests.rs` runs 14 checked-in manifests under `policy-library/` (`:48` library root; `:63` `every_library_scenario_meets_its_manifest`), each pinning `(principal, action, resource) → expect` and running **both** AST and compiled paths against the expectation (`:1-8` doc comment). It is gated blocking in CI (`.github/workflows/ci.yml:1069-1071`). **But it verifies *current* correctness, not *cross-version* stability:** a change that alters a decision and updates the manifest in the same PR sails through — there is no immutability discipline, no language-version tag on a case, and no "this decision must never change" gate. It is a golden corpus, not a *frozen* one.
- **Builtins are outside the correctness oracle.** The differential generator's atom set (`crates/policy-engine/tests/differential_parity_tests.rs:165-206`) covers attr compare, cross-entity eq, context, `in`, badge type-strictness, `rebac::related/reachable` — but **not** `jwt::*`, `regex::*`, `time::*`, comprehensions, string, or math. Those builtins live in `crates/policy-engine/src/reap/ast_evaluator/builtin_functions/{jwt,regex,time,math,string,json}.rs` and are dispatched by a hardcoded `match (namespace, function)` (`reap/ast_evaluator/function_dispatch.rs:26,102-140`). Several are **AST-only** (no compiled twin), so even the compiled≡AST parity net (`compiled_ast_equivalence_tests`, `ci.yml:301-302`) does not reach them. A miscompiled `jwt::verify(...).scope` or `regex::matches(resource)` in a deny rule passes example tests and ships (T2).
- **`Simple` is deprecated, unsafe-by-design, and still advertised + deployable.** `crates/policy-engine/src/evaluators/simple.rs:1-18` self-documents "DEPRECATED — scheduled for removal" and that it **ignores the request action, principal/context, and the rule's `conditions`** ("a rule that should apply to one principal/action applies to all of them"). Yet it remains selectable via `PolicyLanguage::Simple` (`crates/policy-engine/src/engine/types.rs:38`) and is still advertised as one of "three policy languages" in `docs/index.md`, `docs/introduction/key-features.md`, and CLAUDE.md. A customer can deploy a `simple` policy believing it enforces per-principal rules; it silently matches on resource only — a latent authorization surprise.
- **The two DSL surfaces are tiered compilation, not duplication (keep them).** `reap/mod.rs:110-124`: `build` → compiled `ReaperDSLEvaluator`, `build_ast_evaluator` → `ReapAstEvaluator`, `build_preferred` tries compiled then falls back to AST. They are pinned identical by a blocking differential. E-02's explicit verdict: this is textbook JIT+interpreter-oracle — **do not converge.** This plan formalizes the tiering in docs and folds the AST-only builtins into the oracle, closing the one seam where the tiers diverge.

---

## 3. Definition of Done — testable checkboxes

- [ ] **Every published policy carries a language version.** A `.reap` source declares its target version (e.g. a `reaper 2` / `language_version: 2` header), the parser records it, and `PolicyBundle`/`BundleFormat` (`reap/bundle.rs`) carries a `language_version` field alongside the existing `version` (format version). A policy with no declaration is treated as the current implicit version **and** emits a machine-warning directing the author to declare one.
- [ ] **Unknown language versions fail closed.** An engine asked to load a policy whose declared `language_version` exceeds what it implements returns a hard `InvalidPolicy`/`LanguageVersionUnsupported { got, supported }` error — mirroring `bundle.rs:111-119` and `bundle_signing.rs:164-165` — and **never** silently down-levels or best-effort parses. Verified by a test that a v+1 policy is rejected, not misinterpreted.
- [ ] **A FROZEN decision corpus is checked in and gates blocking.** A dedicated, append-only corpus of `(policy, data, request) → expected decision` cases — each tagged with the language version it was authored against — runs in a **new blocking CI job** whose contract is "**no expected decision may change**." Editing an existing frozen expectation requires an explicit, reviewed language-version bump + a documented waiver; the raw edit alone fails a corpus-immutability check (checksum/append-only guard over the frozen file).
- [ ] **Decision drift fails the build.** A deliberately injected semantic change to a shared operator/builtin (applied to *both* compiled and AST paths, so the existing differential can't catch it) turns the frozen-corpus job red. This is proven by a self-test in the same spirit as `perf-gate.yml`'s synthetic +15% regression check.
- [ ] **Builtins are under the oracle.** The differential/property generator (`differential_parity_tests.rs:165-206`) is extended to emit `jwt::*`, `regex::*`, `time::*`, comprehension, string, and math atoms — each checked compiled-vs-AST-vs-independent-oracle — **or** each builtin family gets a dedicated differential/property suite with an independent oracle. Includes negative `jwt::verify` cases (forged signature, expired token, expiry boundary).
- [ ] **The reference parses.** `docs/reference/reap-language.md` is regenerated from / verified against the real grammar; a doc-test or CI check extracts every fenced `.reap` example and asserts it parses under the current parser (the current 32 `when {}` blocks are gone). No published example fails to parse.
- [ ] **A written backward-compat & deprecation policy exists.** `docs/reference/DSL_COMPATIBILITY.md` (new) defines: what counts as a breaking vs additive change (breaking = *any* change to a deployed policy's decision), the rule that additions must be decision-neutral for existing policies, a deprecation window (N releases), and how deprecations surface as machine-warnings at parse/compile time.
- [ ] **`Simple` is gated and de-advertised.** `PolicyLanguage::Simple` is reachable only behind an explicit opt-in feature/flag (default off), and the docs stop listing it as a co-equal authorization language until the JSON-rule migration lands. Deploying `Simple` without the opt-in is rejected with a message pointing at the DSL.
- [ ] **The two-surface tiering is documented as intentional** in the architecture reference, with the stale `evaluators/mod.rs` "not yet used" comment (E-07) corrected.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

### Step 1 — Freeze current decisions *before* touching anything (the safety net first)
- **What:** Promote the existing golden corpus into a **frozen** corpus. Add a `language_version` field to each `policy-library/*/manifest.json` case (or a per-manifest default), and add a `frozen/` tier whose expectations are declared immutable. Introduce a `frozen_decision_corpus_tests.rs` runner that loads every frozen case, evaluates it through **both** AST and compiled paths, and asserts the pinned decision; plus an **immutability guard** (a checksum manifest, e.g. `policy-library/frozen/CHECKSUMS`) so that changing an expectation without also changing the checksum + a language-version bump fails.
- **Where:** `crates/policy-engine/tests/frozen_decision_corpus_tests.rs` (new, modeled on `policy_library_tests.rs:63-130`); `policy-library/frozen/` (seeded from the existing 14 manifests + adversarial edge cases: rule-order, deny-monotonicity, type-strict comparison, rebac reachability, each builtin family). Reuse the `Manifest`/`Case` structs already in `policy_library_tests.rs:17-45`.
- **Verify:** The suite is green on `main`. Flip one expected decision by hand → suite fails on both the decision assertion *and* the checksum guard. Do this step first so every later change is measured against a frozen baseline.

### Step 2 — Add the language-version declaration + fail-closed rejection
- **What:** Add an optional language-version header to the grammar (`reap.pest`) — additive, so existing headerless policies still parse and are assigned the current implicit version. Thread the parsed version into `ReaperPolicy`/`Policy`, and add `language_version: u32` to `BundleFormat` (`reap/bundle.rs`), defaulted for old bundles the same way `default_envelope_version` handles legacy envelopes (`bundle_signing.rs:75,93`). On load/compile, if declared version > implemented, return `LanguageVersionUnsupported { got, supported }` — a hard error, copying `bundle.rs:111-119`.
- **Where:** `crates/policy-engine/src/reap.pest`, `reap/parser/*`, `reap/ast.rs`, `reap/bundle.rs` (field + fail-closed check in `from_bytes`), `crates/reaper-core` error enum. A `const CURRENT_LANGUAGE_VERSION` next to `FORMAT_VERSION` (`bundle.rs:57`).
- **Verify:** A headerless policy parses and is stamped current version (back-compat, mirrors `version_migration_tests.rs:220-238`). A policy declaring `reaper 999` is rejected, not down-levelled. A bundle round-trips its `language_version`. Frozen corpus (step 1) stays green — proving the addition is decision-neutral.

### Step 3 — Bring builtins under the differential/property oracle
- **What:** Extend the differential generator's atom set (`differential_parity_tests.rs:165-206`) to emit `jwt::*`, `regex::*`, `time::*`, comprehension, string, and math expressions, cross-checked compiled-vs-AST-vs-independent-oracle. For AST-only builtins (no compiled twin), add per-family differential/property suites with a hand-written oracle. Add explicit **negative jwt** cases: forged signature rejected, expired token rejected, expiry-boundary (`exp == now`) deny — today `jwt.rs` only decodes a far-future token.
- **Where:** `crates/policy-engine/tests/differential_parity_tests.rs` (generator + oracle); new `builtin_differential_tests.rs` for AST-only families; test seam for a native-injectable clock so time boundaries are deterministic (Testing T6 — `clock.rs` injection is currently wasm32-only). Wire the new suites into `ci.yml` next to `:1090-1094` and into `mutation.yml`'s per-mutant suite list (`:61-67`).
- **Verify:** A hand-injected miscompile in one builtin fails the differential. A forged/expired jwt is denied. The new suites run in the blocking integration job.

### Step 4 — Regenerate the authoritative reference so it parses
- **What:** Rewrite `docs/reference/reap-language.md` to v2 (`if`/`input`/violations), removing `package`/`metadata{}`/`when{}`. Add a CI doc-check that extracts every fenced `.reap` block from the reference (and `policy-syntax.md`) and asserts it parses under the current grammar — so the reference can never again document non-parsing syntax. Prefer generating the grammar section from `reap.pest` (or a golden-tested snippet set) over hand-maintenance.
- **Where:** `docs/reference/reap-language.md`, `docs/reference/policy-syntax.md`; new `crates/policy-engine/tests/reference_examples_parse_tests.rs` (glob the docs, parse each block); CI wire-up.
- **Verify:** Every example in the reference parses. Introduce a deliberately-broken example → the doc-check fails. Cross-check a few examples produce the documented decision via the frozen corpus.

### Step 5 — Write the backward-compat & deprecation policy
- **What:** Author `docs/reference/DSL_COMPATIBILITY.md`: the contract that *additive* changes (new keyword/operator/builtin) must not change the decision of any already-deployed policy (enforced by the frozen corpus, step 1); breaking changes require a language-version bump; a deprecation window of N releases; deprecations emit machine-warnings at parse/compile with a `since`/`removed_in` tag. Add a lightweight `since` annotation convention to the builtin dispatch table (`function_dispatch.rs`) so a deprecation can be surfaced programmatically.
- **Where:** `docs/reference/DSL_COMPATIBILITY.md` (new); a `DeprecationWarning` channel on parse/compile results; optional `since` metadata on builtins.
- **Verify:** A policy using a (test-only) deprecated builtin parses **and** surfaces a structured warning naming the removal version. The policy referenced in the frozen corpus that uses the deprecated form still returns its frozen decision (deprecation ≠ removal).

### Step 6 — Decide the fate of `Simple`; document the two-surface tiering
- **What:** Gate `PolicyLanguage::Simple` behind an explicit opt-in feature (default off); deploying it without the flag is rejected with a message pointing at the DSL. Remove `Simple` from the "three languages" marketing in `docs/index.md`, `docs/introduction/key-features.md`, and CLAUDE.md until the JSON-rule migration lands. Separately, add an architecture-reference section documenting the compiled/AST surfaces as intentional tiered compilation (compiled `ReaperDSLEvaluator` primary, `ReapAstEvaluator` fallback + oracle), and fix the stale `evaluators/mod.rs:21-22` "not yet used" comment (E-07).
- **Where:** `crates/policy-engine/src/engine/types.rs:38` (feature-gate the variant path), `evaluators/simple.rs`, `evaluators/mod.rs`; docs. Maintenance/attack/test-surface note: keeping `Simple` deployable is pure liability — it adds an evaluator with no oracle coverage and an unsafe resource-only match; gating it removes that surface without a breaking removal.
- **Verify:** Deploying `Simple` without the flag is rejected. Docs no longer advertise it. The two-surface doc matches `reap/mod.rs:110-124` reality.

---

## 5. Dependencies

- **Shipped and reused (this plan is mostly assembly on top):** the bundle/envelope versioning pattern to copy (`reap/bundle.rs:55-119`, `bundle_signing.rs:73,164-165`); the existing golden corpus + runner (`policy-library/`, `policy_library_tests.rs`) to freeze; the differential/oracle machinery to extend (`differential_parity_tests.rs`, `compiled_ast_equivalence_tests`); the blocking CI integration + mutation jobs to wire into (`ci.yml:1069-1094`, `mutation.yml:61-67`); the tiered `build_preferred` surface to document (`reap/mod.rs:110-124`).
- **Adjacent, not blocking:** Round-3 T1 (gate mutation testing) strengthens this plan's oracle but is independent; E-02 (unify the duplicated `PolicyLanguage`) pairs naturally with step 6 but is separately tracked; the native-clock test seam (T6) is needed for deterministic `time::*` builtin tests (step 3).
- **No schema/data-plane dependency:** this is a language-contract plan; it touches the engine, the DSL front-end, tests, and docs only — no DB migration, no control-plane change.

---

## 6. Testing & verification (incl. the fitness function for semantic stability)

1. **Frozen-corpus regression (the load-bearing gate):** every `(policy, data, request) → expected decision` case runs through **both** AST and compiled paths; **no expected decision may change** across engine versions; the immutability/checksum guard blocks a silent expectation edit (step 1). Self-tested by a synthetic decision flip that must turn the job red.
2. **Language-version fail-closed:** a policy/bundle declaring an unimplemented version is rejected, never down-levelled; a headerless policy is stamped the current version and still parses (step 2).
3. **Builtin differential/property:** compiled-vs-AST-vs-oracle over generated jwt/regex/time/comprehension/string/math; forged + expired + boundary jwt denials (step 3).
4. **Reference parses:** every fenced `.reap` example in the docs parses under the live grammar; a broken example fails CI (step 4).
5. **Deprecation warning surfaces:** a deprecated builtin parses and emits a structured `since`/`removed_in` warning; its frozen decision is unchanged (step 5).
6. **`Simple` gated:** deploying `Simple` without the opt-in is rejected; frozen corpus unaffected (step 6).
7. **CI wiring proof:** the new frozen-corpus and builtin-differential suites appear in the blocking integration job and in the per-mutant suite list (`mutation.yml`), so oracle rot is caught.

---

## 7. Effort & phasing — S/M/L

- **Phase 1 (S–M) — Freeze + fail-closed version (steps 1–2).** The safety net and the header. Freeze first (pure test/data + a runner), then add the additive version declaration measured against that freeze. Highest value: it stops the bleeding — no future change can silently alter a deployed decision, and unknown versions fail closed. Independently shippable.
- **Phase 2 (M) — Builtins under the oracle + reference regeneration (steps 3–4).** Extends the existing differential generator (mechanical) and rewrites the customer-facing reference with a parse-check so it can't rot again. Closes T2 and the live doc defect.
- **Phase 3 (S) — Compat policy + `Simple`/tiering decisions (steps 5–6).** Mostly writing (the compatibility policy) plus a feature-gate and doc corrections. Small, additive, no behavior change to the DSL itself.

Overall **M** — the design care concentrates in Phase 1 (what exactly "frozen" means and how the immutability guard works); the rest is assembly on machinery that already exists. Each phase is independently shippable and independently valuable (the freeze alone is worth landing before any further DSL change).

---

## 8. Key decisions (ADR-style)

**ADR-1: A frozen decision corpus, not just a golden one.**
- *Context:* `policy-library/` already pins expected decisions and gates in CI, but a PR can change a decision and update the manifest together and pass — the corpus proves *current* correctness, not *cross-version* stability. E-01 names exactly this gap.
- *Decision:* Add a **frozen** tier whose expectations are immutable-by-default, guarded by a checksum/append-only manifest. Changing a frozen expectation requires a deliberate language-version bump + a reviewed waiver; the raw edit alone fails.
- *Consequence:* Semantic stability becomes a *fitness function* — a change that alters an existing decision on *both* compiled and AST paths (which the differential cannot catch) is caught here. The cost is discipline: legitimate semantic changes now cost a version bump, which is the point.

**ADR-2: Copy the bundle-format versioning discipline up to the language.**
- *Options:* (a) invent a bespoke language-versioning scheme; (b) reuse the magic+version+fail-closed-reject pattern the bundle (`bundle.rs:55-119`) and signing envelope (`bundle_signing.rs:73,164-165`) already prove in production.
- *Decision:* **Reuse (b).** Same shape: a declared version, carried in source and bundle, with a hard `LanguageVersionUnsupported` reject of anything newer than implemented, and a defaulted read for legacy artifacts. It is the reference example in the same codebase; a second scheme would be gratuitous divergence.
- *Consequence:* An old engine cannot silently misinterpret a newer policy — it fails closed, the correct posture for an authorization contract.

**ADR-3: Keep the two DSL surfaces; fold builtins into their shared oracle.**
- *Decision:* Do **not** converge `ReaperDSLEvaluator` (compiled) and `ReapAstEvaluator` (AST) — E-02's verdict is that this is tiered compilation with the interpreter as the compiler's oracle, a genuine strength. Instead, close the one seam where they diverge: AST-only builtins with no compiled twin get their own differential/property suites so nothing authz-relevant sits outside an oracle.
- *Consequence:* The asset is preserved and its blind spot (T2) is closed; the maintenance cost of two surfaces is justified by the free correctness oracle it buys.

**ADR-4: Gate `Simple`, don't remove it yet.**
- *Context:* `Simple` is deprecated, unsafe-by-design (resource-only match, ignores action/principal/context — `simple.rs:1-18`), still advertised, and still deployable — but a hard removal is a breaking change and the JSON-rule migration path isn't landed.
- *Decision:* Feature-gate it (default off) and stop advertising it as a co-equal authorization language now; schedule removal with the migration. Assessed cost of keeping it deployable: an evaluator with zero oracle coverage and a latent per-principal authorization surprise — pure liability, removed cheaply by gating without a breaking API change.
- *Consequence:* The unsafe surface is off by default immediately; removal becomes a clean follow-up once migration exists.

---

## 9. Risks & rollback

- **Risk: the freeze bakes in a *current* bug** as an "expected" decision. *Mitigation:* seed the frozen tier from the existing, reviewed golden corpus plus adversarial cases drawn from the same laws the differential already pins (rule-order invariance, deny-monotonicity, type-strict comparison); a genuine bug fix is then an intentional, reviewed language-version bump with a waiver — visible, not silent. *Rollback:* the frozen tier is additive test data; removing the gate is a one-line CI revert if it proves mis-seeded.
- **Risk: the language-version header is itself a breaking change** if it were required. *Mitigation:* the header is **optional and additive** — headerless policies parse and are stamped the current implicit version (proven by the frozen corpus staying green in step 2), exactly as legacy bundles default their envelope version (`bundle_signing.rs:75,93`). *Rollback:* the field defaults; dropping the header requirement is trivial.
- **Risk: builtin oracle coverage is incomplete** and a gap ships. *Mitigation:* the generator extension is measured by mutation testing (wire the new suites into `mutation.yml`) — a surviving mutant in a builtin flags an oracle hole; negative jwt cases are explicit, not generator-dependent. *Rollback:* n/a — pure test addition.
- **Risk: gating `Simple` breaks an existing customer** who deployed it. *Mitigation:* it is deprecated and unsafe already; gate behind an opt-in flag (not an immediate removal) so an existing deployment can re-enable it explicitly while being told to migrate. *Rollback:* the flag default is a one-line change.
- **Risk: the regenerated reference drifts again.** *Mitigation:* the parse-check CI job (step 4) makes a non-parsing example a build failure — the reference *cannot* rot the way it did. *Rollback:* n/a — the check only tightens.
- **Overall posture:** every change here is either additive (version header, frozen tests, builtin oracle, docs) or a default-off gate (`Simple`); none alters an existing decision. The frozen corpus is the backstop that *proves* that claim on every commit — the plan is self-verifying.
