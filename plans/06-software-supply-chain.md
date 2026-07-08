# Software Supply Chain

**Readiness gate:** Third-party / supply-chain risk (SOC 2 CC7/CC8, DORA Art. 28 ICT third-party risk, SSDF / EO 14028 SBOM expectation). Blocks any regulated buyer's vendor security review.
**Priority:** P2 by CVSS, but **elevated** by 3-way reviewer convergence (Security P2-2 = Code API-9 = Product F9). Cheap to fix, universally expected.
**Findings closed:** Synthesis #9 (honourable mention); Security P2-2; Code API-9; Product F9.

---

## 1. Goal

Establish a **blocking**, auditable software-supply-chain assurance pipeline so that: (a) no build ships with a known-vulnerable, yanked, or license-incompatible dependency; (b) a machine-readable SBOM is produced and published for every release; (c) the DSL parser — the highest-value untrusted-input surface in an authorization product — is continuously fuzzed; and (d) container images are scanned with a **blocking** severity threshold. The intent is that a bank's third-party-risk reviewer can be handed the SBOM, the `deny.toml`, the CI logs, and a documented vulnerability-response SLA, and pass Reaper without a finding.

---

## 2. Current state (evidence) — file:line

- **No `cargo audit` / `cargo deny` anywhere in CI** — the workflow set is `ci.yml`, `docker.yml`, `benchmark.yml`, `perf-tracking.yml`, `mutation.yml`, `release.yml` (`.github/workflows/`). Greps for `cargo audit`/`cargo-deny`/`cargo deny` across `.github/` return nothing (confirmed independently by Security P2-2, Code API-9, and the repo map §CI). `ci.yml`'s security-adjacent job is only `lint-and-analyze` (`ci.yml:37-65`: `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings`).
- **No SBOM generation** — no `cyclonedx`/`syft`/`cargo-cyclonedx` invocation anywhere; `release.yml` builds and uploads only the CLI tarball (`release.yml:112-166`, `Build Binaries` → `Upload Release Asset`) and the Helm chart (`release.yml:173-195`). No SBOM artifact is attached to the GitHub Release.
- **No fuzzing** — no `fuzz/` directory or `cargo-fuzz` targets (repo map §risk-hotspots; Code API absence check notes `proptest` differential harnesses exist — `differential_parity_tests.rs`, `check_mode_differential_tests.rs`, `delta_sync_differential_tests.rs`, run at `ci.yml:604-608` — but these are *property* tests over structured inputs, **not** coverage-guided fuzzing over raw bytes on the pest parser). The parser entry point that needs fuzzing is `ReapParser::parse` (`crates/policy-engine/src/reap/parser/mod.rs:35`), feeding `compile_policy` (`crates/policy-engine/src/reap/compiler/mod.rs:36`).
- **Trivy image scan is non-blocking** — `.github/workflows/docker.yml:101-113`: the `Run Trivy vulnerability scanner` step and the `Upload Trivy scan results` step are **both** `continue-on-error: true` (`docker.yml:107,113`), with no `severity`/`exit-code` threshold set, and the whole `scan-images` job is `if: github.event_name != 'pull_request'` (`docker.yml:90`) — so PRs are never scanned and pushes never fail on a finding.
- **Git dependencies unpinned/unverified** — no `[patch]`/git-rev discipline documented; `cargo-deny`'s `sources`/`bans` sections are the missing control here (to be added).
- **No documented vulnerability-response SLA** — nothing in `docs/` commits to a triage/patch window for a disclosed CVE in a dependency.

---

## 3. Definition of Done — testable checkboxes

- [ ] A committed `deny.toml` at the repo root configures `cargo-deny` with all four checks: `advisories` (deny vulnerabilities + unmaintained), `licenses` (allowlist), `bans` (deny duplicate/banned crates), and `sources` (only allowed registries/git hosts).
- [ ] A new CI job runs `cargo deny check` as a **required, blocking** gate on every PR and push (not `continue-on-error`), wired into `ci.yml` alongside `lint-and-analyze`.
- [ ] `cargo audit` runs in CI against the committed `Cargo.lock` and fails on any RUSTSEC advisory not explicitly acknowledged (advisory-DB kept fresh); a scheduled (nightly/weekly) run catches newly-disclosed CVEs against already-shipped commits.
- [ ] A CycloneDX SBOM (`bom.json` / `bom.xml`) is generated for the workspace and **attached to every GitHub Release** as an asset (and, ideally, for each container image).
- [ ] At least one `cargo-fuzz` target exists for the DSL: a `fuzz/` crate with a `fuzz_targets/parse_reap.rs` harness driving `ReapParser::parse` (and a second driving `compile_policy`), building in CI and running a bounded smoke iteration on every PR plus a longer scheduled run.
- [ ] The Trivy image scan is **blocking**: `continue-on-error` removed, `severity: CRITICAL,HIGH` and `exit-code: 1` (with `ignore-unfixed` as chosen), and it runs on PRs (the `if: != pull_request` guard removed or replaced by a PR-appropriate path).
- [ ] Any git dependency in `Cargo.toml`/`Cargo.lock` is pinned to a specific rev and allow-listed in `deny.toml`'s `[sources]`; `cargo deny check sources` passes.
- [ ] A written vulnerability-response SLA (triage window, severity→patch-window mapping, who owns it) is committed under `docs/security/` (e.g. `SECURITY.md` / `VULN_RESPONSE.md`) and referenced from the repo `SECURITY.md`.
- [ ] All new gates are documented in `CLAUDE.md`/CONTRIBUTING so a red build is understood as a supply-chain stop, not flakiness.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

**Step 1 — Add `cargo-deny` with a committed `deny.toml` as a blocking gate.**
- *What:* Author `deny.toml` covering `[advisories]` (`vulnerability = "deny"`, `unmaintained = "warn"→"deny"` after triage, `yanked = "deny"`), `[licenses]` (explicit allow-list — e.g. MIT/Apache-2.0/BSD/ISC/Unicode; `cedar-policy` and `git2`/OpenSSL license terms verified), `[bans]` (deny multiple-versions where feasible, ban known-bad crates), and `[sources]` (allow crates.io + any explicit git host). Add a CI job.
- *Where:* new `/home/user/reaper/deny.toml`; new job in `.github/workflows/ci.yml` (mirror the `lint-and-analyze` job shape at `ci.yml:37-65`) running `cargo deny check advisories licenses bans sources`. Do **not** set `continue-on-error`.
- *Verify:* CI job fails if a banned license or advisory is introduced (test by temporarily adding a GPL-only dev-dep in a scratch branch). `cargo deny check` passes clean on `main` after license allow-list is tuned to the current tree.

**Step 2 — Add `cargo audit` (PR + scheduled).**
- *What:* Run `cargo audit` against the committed `Cargo.lock`. Two triggers: on PR/push (fast fail on known advisories in the current lock), and a `schedule:` cron run so a CVE disclosed *after* merge still raises a failing build against `main`.
- *Where:* extend `ci.yml` with a `cargo-audit` step (can share the `cargo-deny` job or be its own), plus a `schedule:` trigger (there is precedent — `mutation.yml` runs nightly). Cache the advisory DB.
- *Verify:* Introduce a pinned old version of a crate with a known RUSTSEC ID in a scratch branch → audit fails. Confirm the scheduled run appears in the Actions tab.
- *Note:* `cargo-deny`'s `advisories` check overlaps `cargo-audit`; keep both intentionally — `deny` is the blocking PR gate, `audit`'s scheduled run is the continuous-watch on shipped code. Document that they are complementary, not redundant, to avoid a future "dedupe" that removes the scheduled watch.

**Step 3 — Generate and publish a CycloneDX SBOM as a release artifact.**
- *What:* Add SBOM generation (`cargo-cyclonedx` for the Rust workspace; optionally `syft` for the container images) and attach the output to the GitHub Release.
- *Where:* `.github/workflows/release.yml` — add an SBOM step in the `Build Binaries`/`Create Release` flow (`release.yml:62-166`) that runs `cargo cyclonedx --format json` and uploads `bom.json` via the same `upload-release-asset` mechanism already used at `release.yml:159-166`. Optionally also generate a per-image SBOM in `docker.yml` next to the Trivy step.
- *Verify:* A tagged release has a `bom.json` asset; validate it parses as CycloneDX and lists the workspace crates + transitive deps.

**Step 4 — Add `cargo-fuzz` targets on the DSL parser/compiler.**
- *What:* Create a `fuzz/` crate with libFuzzer targets that feed arbitrary bytes/strings to `ReapParser::parse` and to `parse → compile_policy`, asserting no panic/abort (this directly validates the depth-bound work in Plan 05). Seed the corpus with the existing `.reap` test policies and `test-data/`.
- *Where:* new `/home/user/reaper/fuzz/` (`cargo fuzz init`), targets `fuzz/fuzz_targets/parse_reap.rs` and `fuzz/fuzz_targets/compile_reap.rs` calling into `policy_engine::reap::parser::ReapParser::parse` (`reap/parser/mod.rs:35`) and `compiler::compile_policy` (`reap/compiler/mod.rs:36`). CI: a short bounded run (`-max_total_time=60`) on PRs, a longer scheduled run; store/restore corpus via `actions/cache` or an artifact. `cargo-fuzz` needs nightly — pin a nightly toolchain for that job only.
- *Verify:* The target builds and runs in CI; deliberately reverting the Plan-05 depth guard makes the fuzzer find a crash (proving the harness is live). No `fuzz/artifacts/` crash on `main`.

**Step 5 — Make the Trivy image scan blocking with a severity threshold.**
- *What:* Remove the two `continue-on-error: true` flags, set `severity: CRITICAL,HIGH`, `exit-code: 1`, `ignore-unfixed: true` (policy choice), and run on PRs so a vulnerable base image is caught before merge.
- *Where:* `.github/workflows/docker.yml:86-113` — edit the `scan-images` job: drop `continue-on-error` at `:107` and `:113`, add the `severity`/`exit-code`/`ignore-unfixed` inputs to the Trivy step (`:101-107`), and remove/relax the `if: github.event_name != 'pull_request'` guard at `:90` (or add a PR-time filesystem/image scan). Keep the SARIF upload but let the scan step's exit code fail the job.
- *Verify:* A PR that bumps a base image to one with a known CRITICAL CVE fails the `scan-images` job; a clean image passes. SARIF still appears in the Security tab.

**Step 6 — Pin/verify git dependencies and document the vuln-response SLA.**
- *What:* Ensure every git dependency is pinned to an exact rev and allow-listed in `deny.toml [sources]`; write the vulnerability-response SLA.
- *Where:* audit `Cargo.toml`/`Cargo.lock` for `git = ` entries; pin `rev = "..."`; add allowed git hosts to `deny.toml`. Add `docs/security/VULN_RESPONSE.md` (triage ≤ N business days, CRITICAL patch ≤ X days, HIGH ≤ Y, etc.) and link it from the root `SECURITY.md`.
- *Verify:* `cargo deny check sources` passes; the SLA doc exists and is referenced. `grep 'git = ' Cargo.lock` shows only pinned revs.

---

## 5. Dependencies

- CI tooling: `cargo-deny`, `cargo-audit`, `cargo-cyclonedx` (installable via `taiki-e/install-action` or `cargo install`), `cargo-fuzz` (+ a pinned nightly toolchain for the fuzz job only), `aquasecurity/trivy-action` (already present).
- **Plan 05 (Availability & Resilience)** is the natural co-delivery: Step 4's fuzz targets are the acceptance test for Plan 05's DSL depth bound. Sequence Plan 05 Step 2 and this plan's Step 4 together.
- License allow-list requires a one-time legal/eng decision on acceptable licenses (esp. any copyleft transitive deps).
- No product-code change is required for Steps 1–3, 5, 6; Step 4 adds a `fuzz/` crate (excluded from the default workspace build to keep nightly out of the normal path).

---

## 6. Testing & verification

- **cargo-deny/audit:** scratch-branch negative tests (inject a banned license and a known-RUSTSEC crate) must turn the gate red; `main` stays green after license tuning. Scheduled `cargo audit` run visible in Actions.
- **SBOM:** the release asset `bom.json` validates against the CycloneDX schema and enumerates the workspace + transitive crates; diff two releases to prove it tracks dependency changes.
- **Fuzz:** harness builds and runs; reverting the Plan-05 guard produces a reproducible crash artifact (proves the target actually exercises the parser); corpus is cached across runs.
- **Trivy:** a known-vulnerable image bump fails `scan-images`; a clean image passes; SARIF uploads unchanged.
- **Falsifiable acceptance:** `test -f deny.toml`; `grep -R "cargo deny\|cargo-deny" .github/workflows/` non-empty; `grep -R "continue-on-error" .github/workflows/docker.yml` no longer covers the Trivy scan step; `test -d fuzz/fuzz_targets`; a Release page shows a `bom.json` asset; `docs/security/VULN_RESPONSE.md` exists.

---

## 7. Effort & phasing — S/M/L

| Phase | Scope | Size |
|-------|-------|------|
| **Quick blocking gates** | Step 1 (`deny.toml` + job), Step 2 (`cargo audit`), Step 5 (Trivy blocking) — config-only, high assurance-per-hour | **S** |
| **Release provenance** | Step 3 (SBOM in `release.yml`), Step 6 (git pinning + SLA doc) | **S–M** |
| **Fuzzing** | Step 4 (`fuzz/` crate, nightly job, corpus, CI wiring) | **M** |

The whole plan is small relative to its assurance value: the S phase (deny + audit + Trivy) is roughly a day and closes the 3-way-converged finding's core. Fuzzing is the only M item and pairs with Plan 05.

---

## 8. Key decisions (ADR-style)

- **ADR-1: `cargo-deny` is the single blocking PR gate; `cargo-audit` is the scheduled watch.** *Context:* their `advisories` coverage overlaps. *Decision:* keep both with distinct roles — `deny` blocks PRs across advisories+licenses+bans+sources; `audit`'s cron catches CVEs disclosed after merge. *Consequence:* a future dedupe must not delete the scheduled `audit` run (documented in Step 2).
- **ADR-2: SBOM in CycloneDX JSON, attached per release.** *Decision:* CycloneDX over SPDX for tooling ubiquity and Trivy/Grype compatibility; publish as a Release asset (and per-image) so a buyer can ingest it. *Alternative rejected:* generating SBOM only locally — not auditable, not evidence.
- **ADR-3: Trivy blocks on CRITICAL+HIGH with `ignore-unfixed`.** *Decision:* fail builds on fixable CRITICAL/HIGH; ignore unfixed to avoid unactionable red builds, revisited as the base image matures. *Consequence:* PRs now scan (guard removed), trading a little CI time for pre-merge coverage.
- **ADR-4: Fuzz the parser first, as table stakes for a language implementation.** *Decision:* the DSL parser is the largest untrusted-input attack surface; a coverage-guided fuzzer complements (does not replace) the existing proptest differential suites. *Consequence:* a nightly toolchain is introduced but confined to the isolated `fuzz/` crate.
- **ADR-5: Git deps must be rev-pinned and source-allow-listed.** *Decision:* unpinned git deps are an unauditable supply-chain hole; `deny.toml [sources]` enforces it. *Consequence:* dependency bumps become explicit, reviewed rev changes.

---

## 9. Risks & rollback

- **Risk: license allow-list initially too strict → `main` build red on a legitimate transitive dep.** *Mitigation:* run `cargo deny check licenses` locally first and seed the allow-list from the actual tree before making the job required; start advisories/licenses as `deny` but `unmaintained` as `warn`. *Rollback:* relax a specific license/advisory ID in `deny.toml` (targeted, not blanket disable).
- **Risk: blocking Trivy causes red builds on unfixable base-image CVEs.** *Mitigation:* `ignore-unfixed: true` and a CRITICAL/HIGH-only threshold; escalate to a tracked exception list rather than reverting to `continue-on-error`. *Rollback:* re-add `continue-on-error` as a temporary measure with an issue tracking the debt (explicitly discouraged).
- **Risk: fuzz job flakiness / nightly toolchain churn.** *Mitigation:* pin the nightly date; keep the PR-time fuzz run short (`-max_total_time`) and the long run on schedule; the `fuzz/` crate is out of the default workspace so it never blocks normal builds if it fails to compile on a nightly bump. *Rollback:* mark the fuzz job non-required (keep it informational) while retaining every other gate.
- **Risk: `cargo audit` cron fails against `main` for a CVE with no available patch.** *Mitigation:* that is the intended signal — triage per the SLA doc; use `--ignore RUSTSEC-XXXX` with a dated justification, not a blanket mute. *Rollback:* none needed; the failure is actionable by design.
- **General rollback:** every change is additive CI config plus one new crate and a config file — no product-code, wire-format, or data changes. Any gate can be made non-required in one line without affecting the others, so a bad gate never blocks the whole pipeline irreversibly.
