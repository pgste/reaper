# Subagent 7 — CI/CD & Release Engineering Review (Round 3)

**Persona:** Staff release/CI engineer, regulated-fintech delivery. Hostile external audit.
**Scope:** `.github/workflows/*`, `Makefile`, `deny.toml`, `clippy.toml`, `.cargo/config.toml`,
`scripts/release.sh`, `services/*/Dockerfile`, `docker-compose*.yml`. UI out of scope.
**Threat model applied:** compromised-CI — a malicious PR, a poisoned cache, or a swapped
third-party action must not be able to ship a backdoored image/binary/bundle.

---

## VERDICT: NOT READY

One P0 (shipped container images and CLI binaries are unsigned and unattested, and the
release-time image scan runs *after* the image is already published), plus five P1s. The
*testing placement* of this pipeline is genuinely strong — the paired-A/B perf gate, the
blocking supply-chain job, mutation/fuzz/SLO on the right cadence — but the **build-and-publish
integrity** axis is where a bank rejects it: nothing cryptographically binds "what was
reviewed/scanned" to "what a customer pulls," and the one release artifact job (`build-binaries`)
is dead on a typo'd action reference and has never produced a binary.

---

## Exec summary (≤10 lines)

1. **P0** — No signing (cosign/minisign), no SLSA/build provenance, no image-bound SBOM on *any*
   shipped artifact; Trivy scan on release runs in a job that `needs: build-images`, i.e. *after*
   `build-images` has already `push:true`'d the image to ghcr. Scan gates PRs, not releases.
2. **P1** — The scanned image is a *separate, amd64-only local rebuild* (`reaper-scan/*:ci`); the
   published image is a separate multi-arch (amd64+arm64) build+push. No digest binding; arm64 is
   never scanned.
3. **P1** — `release.yml:181` uses `dtolnay/rust-action@stable` — that action does not exist
   (correct: `rust-toolchain`). `build-binaries` fails on every tag; the CLI tarballs it claims to
   ship have never been produced. The draft release still gets SBOM + Helm.
4. **P1** — Zero SHA-pinning of third-party actions; the *scanner itself* (`aquasecurity/trivy-action@master`)
   and `dtolnay/rust-toolchain@master` (fuzz) float on mutable refs that execute with `packages: write`.
5. **P1** — Docker/release SBOM is source-level (`cargo cyclonedx --all-features`), over-broad vs the
   shipped binary and not attached to the image digest; release images referenced in the release body
   are built by a *different* workflow with no correctness-gate coordination.
6. **Done well:** paired A/B perf gate w/ self-test, blocking cargo-deny+audit with dated ignores,
   PR image scan built from *this* commit, fail-open sccache probe, clippy pipefail guard.
7. Efficiency: global `jobs=2` compile cap throttles every job; heavy 100k/10k suites run
   (non-blocking) on every PR; `save-if` cache guard applied inconsistently → PR cache thrash.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| C1 | **P0** | `docker.yml:67-81` (push), `:90-93` (scan needs build), `release.yml` (no cosign) | Images/binaries shipped **unsigned, no provenance, no image-bound SBOM**; release scan runs after publish | No cryptographic binding of reviewed→published; a poisoned build or registry tamper is undetectable by consumers | Add cosign keyless (OIDC) sign + `attest` provenance & SBOM; scan-then-sign-then-push in one job; buildx `provenance:true`/`sbom:true` |
| C2 | **P1** | `docker.yml:90-130` vs `:67-81` | Scanned image (`reaper-scan/*:ci`, amd64-only local `load`) ≠ published multi-arch digest; arm64 never scanned | The scanned bytes are not the shipped bytes; arm64 CVEs ship unscanned | Scan the exact pushed digest (scan after push in same job, by digest) incl. every platform |
| C3 | **P1** | `docker.yml:48,67-72` | On `push`/tag the image is pushed in `build-images` *before* `scan-images` runs; Trivy failure does not unpublish | A CRITICAL/HIGH first seen at release ships to `latest`/semver tags anyway | Gate: build→scan→push (or push to a quarantine tag, promote only on green scan) |
| C4 | **P1** | `release.yml:181` | `uses: dtolnay/rust-action@stable` — non-existent action (typo for `rust-toolchain`) | `build-binaries` fails every release; advertised CLI tarballs never produced; drift undetected because release only runs on tags | Fix to `dtolnay/rust-toolchain@<sha>`; add a tag-dry-run smoke |
| C5 | **P1** | all workflows; `docker.yml:124,136`, `fuzz.yml:57` | No third-party action SHA-pinned; `trivy-action@master`, `rust-toolchain@master` on mutable refs | Compromised-CI: a swapped action tag runs with `packages:/contents: write` → backdoored image | SHA-pin every `uses:` (incl. `actions/*`); Dependabot for action bumps |
| C6 | **P1** | `release.yml:132-138` | SBOM via `cargo cyclonedx --all-features`, source-level, `--all-features` includes unshipped backends (mysql, all storage); not attached to any image digest | SBOM over-reports vs shipped binary and is not the artifact's BOM; fails third-party-risk ingest fidelity | Generate per-artifact SBOM from the built image (syft/trivy) with the *shipped* feature set; attest to digest |
| C7 | **P2** | `services/*/Dockerfile:31`, `:16-19` | `cargo build --release` without `--locked`; apt packages unpinned; no `SOURCE_DATE_EPOCH` | Non-reproducible images; Cargo.lock drift possible at build | Add `--locked`; pin apt versions or use distroless; set reproducible-build flags |
| C8 | **P2** | `.cargo/config.toml:2` (`jobs = 2`) | Global cap of 2 compile jobs applies to *every* CI job on 4-vCPU runners | Every compile in every fan-out job is throttled ~2x | Remove the cap in CI (env `CARGO_BUILD_JOBS`) or raise to nproc |
| C9 | **P2** | `ci.yml:702,778,865,1283` | 100k memory-scale, 5×10k volume matrix, scale-tests, eval-microbench run on *every PR* as `continue-on-error` (non-blocking) | Cost + wall-clock on the PR path for jobs that never gate | Move heavy/advisory suites to `schedule` + a `perf`/`scale` label |
| C10 | **P2** | `ci.yml:280-285,461-465,740-744,812-816,900-904,1046-1050,1203-1207,1313-1314` | `save-if` main/develop guard present on lint/api/mgmt-pg/wasm but *absent* on unit/ebpf/volume/memory/scale/integration/bdd/eval, all sharing `shared-key: reaper-ci` | Many jobs race to write one cache key on PRs → cache thrash + wasted uploads | Apply `save-if` consistently; let one canonical job own the shared-key write |
| C11 | **P2** | repo root (no `.github/CODEOWNERS`, no branch-protection-as-code) | Which checks actually *block merge* is invisible in the repo | The entire "blocking gate" story rests on unseen GitHub settings | Commit required-checks/branch-protection as code (or Terraform); add CODEOWNERS |
| C12 | **P2** | `release.yml:78-98` vs `docker.yml:4-6` | Release body links `ghcr.io/.../reaper-*:<ver>` images built by a *separate* workflow with no `needs`/correctness-gate coupling; release stays `draft:true` with no undraft step | Release can reference images that failed scan or never finished; human may publish a draft missing binaries | Coordinate: one release pipeline that gates images on correctness + scan before the release is publishable |
| C13 | **P3** | `release.yml:80,142,203` | `actions/create-release@v1` + `upload-release-asset@v1` are archived/unmaintained | Dead-end action surface; no checksums/`SHA256SUMS` asset | Migrate to `softprops/action-gh-release` (SHA-pinned); publish a signed checksums file |
| C14 | **P3** | `ci.yml:1052-1056,1209-1214,746-748` | `generate_rbac/abac/multilayer_data` examples recompiled+run separately in integration, bdd, and volume jobs | Redundant release-mode compile+run of the same generators | Generate once, pass as an artifact between jobs |

---

## Detailed findings (P0/P1)

### C1 (P0) — Shipped artifacts are unsigned, unattested, and the release scan is post-publish
Evidence:
- `docker.yml:67-72` `build-push-action` with `push: ${{ github.event_name != 'pull_request' }}`
  and **no** `provenance:`/`sbom:` inputs; grep across `docker.yml` for
  `provenance:|sbom:|attestations|id-token` → **NONE**.
- Grep across all of `.github/workflows/` for `cosign|sigstore|minisign|attest|provenance|slsa`
  → **NONE** (only unrelated `--locked` hits).
- `scan-images` `needs: build-images` (`docker.yml:93`) — so on `push`/tag the publish in
  `build-images` completes *before* Trivy ever runs.
- `release.yml` ships CLI tarballs (`:196-210`) and an SBOM tarball (`:141-149`) with no signature
  and no checksums asset.

Why P0 for a regulated buyer: there is no cryptographic chain from the code that passed review to
the image/binary a customer pulls. A tampered registry object, or a build produced off a poisoned
cache/action (see C5), is indistinguishable from the real one at the consumer end. Combined with the
publish-before-scan ordering, the release-time vulnerability gate is advisory in practice. A bank's
third-party-risk process rejects unsigned, unattested container images outright.

Fix shape: single publish job that (a) builds once, (b) scans the built digest, (c) cosign-signs
(keyless via `id-token: write` OIDC), (d) attaches an image-derived SBOM + SLSA provenance attestation,
(e) only then tags `latest`/semver. Do the equivalent (minisign/cosign blob + `SHA256SUMS`) for CLI
tarballs and Helm charts.

### C2 (P1) — Scanned image is not the published image
`scan-images` (`docker.yml:109-130`) does its *own* `build-push-action` with `load: true`,
`platforms: linux/amd64`, tag `reaper-scan/${service}:ci`, and Trivy scans *that*. The published
artifact from `build-images` is `platforms: linux/amd64,linux/arm64` pushed to ghcr
(`docker.yml:81`). Two independent builds ⇒ no guarantee of identical digest, and the arm64 half of
the shipped manifest is **never scanned**. The "always covers THIS commit's image" comment
(`docker.yml:87-89`) is true for amd64 provenance but false for "the published digest." Scan the
pushed digest itself, across all platforms.

### C3 (P1) — Publish precedes the vulnerability gate on releases
Because `build-images` pushes unconditionally on non-PR events and `scan-images` is a downstream
job, a fixable CRITICAL/HIGH that first appears at tag time turns the workflow red *after* the image
is already live under `latest`/`{{version}}`/`{{major}}.{{minor}}`/sha tags (`docker.yml:60-65`).
There is no promote-on-green step and no unpublish. For PRs the gate is real (images built, not
pushed); for the artifacts that actually ship, it is not. Reorder to build→scan→(sign)→push, or push
to a quarantine tag promoted only on a green scan.

### C4 (P1) — `build-binaries` is dead: typo'd action
`release.yml:181` `uses: dtolnay/rust-action@stable`. The action is `dtolnay/rust-toolchain`
(correctly used at `:24` and `:125` in the same file). `dtolnay/rust-action` does not exist, so the
step errors at "resolve action" and the whole `build-binaries` matrix fails on every tag. Net effect:
the release advertises `reaper-cli-{linux,darwin}-{amd64,arm64}.tar.gz` that have never been built,
while `create-release` (draft), `sbom`, and `publish-helm` succeed independently — so a maintainer
undrafting the release ships one with no binaries. This has gone undetected because `release.yml`
only triggers on `tags: ['v*']` and is never dry-run. Fix the ref (SHA-pin it) and add a tag-shaped
smoke run.

### C5 (P1) — No action pinning; mutable refs on security-critical actions
Grep for `uses: .*@[0-9a-f]{40}` → **NO SHA-PINNED ACTIONS** anywhere. Everything floats on tags
(`@v4`, `@v5`, `@stable`), and worse, `aquasecurity/trivy-action@master` (`docker.yml:124,136`) and
`dtolnay/rust-toolchain@master` (`fuzz.yml:57`) float on branch heads. Under the compromised-CI
threat model this is the classic backdoor vector: a tag/branch force-push on a third-party action
executes attacker code inside jobs holding `packages: write` (docker) and `contents: write`
(perf-tracking gh-pages). The scanner floating on `@master` is especially perverse — the thing
asserting the image is clean is itself unpinned. SHA-pin every `uses:` (including first-party
`actions/*`) and drive bumps via Dependabot.

### C6 (P1) — SBOM is source-level, over-broad, and not artifact-bound
`release.yml:132-138` runs `cargo cyclonedx --format json --all --all-features`. `--all-features`
pulls features the shipped binaries never compile (e.g. the AWS/mysql/storage backends the
`deny.toml` ignores at `:23-48` explicitly note are *off* by default), so the BOM over-reports the
attack surface, and it is a *source-dependency* BOM, not a BOM of the produced image/binary
(base-image OS packages are absent). It is attached to the release as a loose tarball, not attested
to any digest. A CycloneDX consumer cannot answer "what is actually in the image I run?" from it.
Generate the SBOM *from the built image* (syft/`trivy image --format cyclonedx`) with the shipped
feature set, and attest it to the digest.

---

## Right-test-at-right-layer matrix

| Check | Current trigger | Blocking? | Correct layer? | Recommendation |
|-------|-----------------|-----------|----------------|----------------|
| `cargo fmt` / `clippy -D warnings` | PR/push (`ci.yml:52`) | Yes (pipefail guard `:98-102`) | ✅ cheapest, earliest | keep |
| cargo-deny (adv/lic/bans/sources) | PR/push (`ci.yml:118`) | Yes | ✅ | keep; the dated ignores in `deny.toml` are exemplary |
| cargo-audit (Cargo.lock) | PR/push (`ci.yml:148`) + weekly (`supply-chain-nightly.yml`) | Yes / Yes | ✅ overlap is deliberate & correct | keep |
| dependency-freshness | PR/push (`ci.yml:156`) | No (informational) | ✅ advisory | keep |
| API-contract (OpenAPI parity) | PR/push (`ci.yml:189`) | Yes | ✅ | keep |
| Unit + mgmt-integration (SQLite) | PR/push (`ci.yml:247`) | Yes | ✅ | keep |
| Management tests on **Postgres** | PR/push (`ci.yml:362`) | Yes | ✅ prod engine in CI — good | keep |
| Compiled≡AST / interner-leak / pruning | PR/push (`ci.yml:299-320`) | Yes | ✅ correctness at unit layer | keep |
| Differential parity (500 cases) | PR/push (`ci.yml:1087`) | Yes | ✅ | keep |
| Process-level data-plane E2E | PR/push (`ci.yml:1077`) | Yes | ✅ real binaries over HTTP | keep |
| Docker compose E2E | PR + push + dispatch (`docker.yml:154`) | Yes | ✅ (moved off `build-images` — good) | keep |
| wasm build + browser smoke | PR/push (`ci.yml:600`) | Yes | ✅ regression firewall | keep |
| eBPF build | PR/push after unit (`ci.yml:417`) | Yes (except demo step `:524`) | ⚠️ experimental, on every PR | consider label-gating |
| Volume 10k (5-matrix) | every PR/push (`ci.yml:702`) | No (`continue-on-error`) | ❌ heavy, non-gating, on PR path | → schedule/label |
| Memory-scale 100k | every PR/push (`ci.yml:778`) | No | ❌ heavy, non-gating, on PR path | → schedule/label |
| Scale tests | PR-only (`ci.yml:865`) | No | ❌ advisory cost on PR | → label/nightly |
| Eval micro-bench (criterion) | every PR/push (`ci.yml:1283`) | No (`|| true`) | ⚠️ advisory | fold into perf-tracking |
| **Perf gate (paired A/B criterion + HTTP)** | PR (`perf-gate.yml`) | **Yes** | ✅ variance-cancelled, self-tested | keep — best-in-class |
| Perf-tracking (trend) | push main/`claude/**` | No (comment) | ✅ correctly advisory | narrow `claude/**` trigger |
| Benchmark (vs OPA) | PR + push + dispatch | No | ✅ separate | keep |
| SLO harness | nightly/dispatch | No | ✅ | keep |
| Fuzz (parse/compile) | PR-smoke 60s + nightly 15m | PR: yes (crash fails) | ✅ right cadence | SHA-pin toolchain (C5) |
| Mutation | nightly | No (report) | ✅ | keep |
| Trivy image scan | PR + push + tag | PR: yes; **release: post-publish** | ❌ see C3 | build→scan→sign→push |

---

## Build/publish provenance scorecard

| Artifact class | Built once | Scanned | Signed | SBOM (artifact-bound) | Reproducible | Right registry | Right identity |
|----------------|-----------|---------|--------|----------------------|--------------|----------------|----------------|
| **Service images** (agent/platform/management) | **No** — pushed build + separate scan build (`docker.yml:67` vs `:109`) | Partial — amd64 local rebuild only, post-publish on release (C2/C3) | **No** (C1) | **No** — source SBOM only, not image-bound (C6) | **No** — no `--locked`, unpinned apt (C7) | Yes — `ghcr.io` (`docker.yml:14`) | Yes — `GITHUB_TOKEN` + `packages:write`, PR-gated login (`docker.yml:48`) |
| **CLI binary** (reaper-cli tarballs) | N/A — **never built** (typo, C4) | No | **No** (C1) | No | No | GH Release asset | `GITHUB_TOKEN` |
| **Helm chart** | Yes (`release.yml:234`) | No | **No** (OCI push unsigned, `release.yml:238`) | No | n/a | `ghcr.io/.../charts` | `GITHUB_TOKEN` + `packages:write` |
| **SBOM tarball** | Yes (`release.yml:132`) | n/a | **No** | — over-broad `--all-features`, source-only (C6) | n/a | GH Release asset | `GITHUB_TOKEN` |
| **Policy bundles** (.rbb/.rpp) | **Not built/published in CI at all** (grep: no bundle compile/publish job) | — | — | — | — | — | — |

Legend: images are the primary shipped artifact and score No/Partial on every integrity column that
matters. Nothing is signed; nothing carries provenance.

---

## Efficiency hit-list (ranked by wall-clock/cost saved)

1. **Heavy suites on the PR path (largest waste).** memory-scale-100k (`ci.yml:778`), volume 5×10k
   matrix (`:702`), scale-tests (`:865`), eval-microbench (`:1283`) all run per-PR as non-blocking.
   That's ~7 heavy release-mode jobs (matrix expands volume to 5) burning runner-hours on every push
   while gating nothing. Move to `schedule` + opt-in label. **Highest-leverage single change.**
2. **Global `jobs = 2` compile cap (`.cargo/config.toml:2`)** throttles *every* compile in *every*
   one of ~15 fan-out jobs to 2 parallel rustc on 4-vCPU runners. Remove/override in CI
   (`CARGO_BUILD_JOBS=0`); ~1.5-2x compile speedup fleet-wide.
3. **Cache write thrash (`save-if` inconsistency, C10).** 8 heavy jobs write `shared-key: reaper-ci`
   on PRs without the main/develop guard the other jobs use → racing uploads, evictions, misses. Let
   one job own the shared-key write; guard the rest.
4. **Redundant data-generator recompiles (C14).** `generate_{rbac,abac,multilayer}_data` compiled+run
   separately in integration, bdd, volume. Generate once → artifact.
5. **`perf-tracking` + `benchmark` on every `claude/**` push** — full criterion / OPA runs on feature
   branches. Restrict to `main` + dispatch.

Note the pipeline already does several efficiency things *right*: shared sccache with a fail-open
probe (`ci.yml:64-84`), `GIT_LFS_SKIP_SMUDGE` on all perf workflows (saves ~500MB/run), concurrency
`cancel-in-progress`, per-service buildx cache scopes (`docker.yml:79`), and pushing CI to
`pull_request`-only on feature branches to avoid duplicate push+PR runs (`ci.yml:11-16`).

---

## Absence checks (what I looked for and did not find)

- **Signing:** grep `cosign|sigstore|minisign` across `.github/workflows/` → none.
- **Provenance/attestation:** grep `attest|provenance|slsa`, and `provenance:`/`sbom:` inputs on
  `build-push-action` → none.
- **Image-bound SBOM:** only `cargo cyclonedx` (source) in `release.yml:132`; no `syft`/
  `trivy … --format cyclonedx` against a built image.
- **Checksums:** no `SHA256SUMS`/`sha256sum` asset on release binaries.
- **SHA-pinned actions:** grep `uses: .*@[0-9a-f]{40}` → none; two actions on `@master`.
- **Bundle (.rbb/.rpp) build/publish job:** grep across workflows → none (bundles only referenced in
  memory-profile output text; the signed-bundle distribution path is not exercised by any
  build/publish workflow).
- **Branch-protection / required-checks as code, CODEOWNERS:** none in repo — the blocking-vs-advisory
  distinction is only enforceable via unseen GitHub settings (C11).
- **`pull_request_target` misuse / secret exposure to fork code:** none found — `ci.yml` uses
  `pull_request` with no secrets in test jobs; `docker.yml` login is `if: github.event_name != 'pull_request'`
  (`:48`); this is correctly done.

## What's done well (≤5)

1. **Paired A/B perf gate** (`perf-gate.yml`) — benchmarks merge-base and head on the *same* runner,
   compares CIs, and runs a `--self-test` proving a synthetic +15% regression fails before trusting
   the gate. This is how you make nanosecond benches blocking; genuinely excellent.
2. **Supply-chain gate** — cargo-deny (advisories+licenses+bans+sources) blocking on PR
   (`ci.yml:131`) plus a weekly re-audit against a fresh DB (`supply-chain-nightly.yml`); `deny.toml`
   ignores are dated, per-ID, feature-graph-justified, not blanket mutes.
3. **Correct advisory/blocking split** — perf-gate blocks, perf-tracking only comments (variance
   rationale documented `perf-tracking.yml:16-26`); fuzz is PR-smoke + nightly campaign; mutation/SLO
   nightly. The cadence placement is right.
4. **Prod DB in CI** — management suite runs against real Postgres 16 (`ci.yml:362`), not just the
   SQLite dev path, catching dialect/migration regressions.
5. **Robust green-signal accounting** — `generate-report` (`ci.yml:1390-1407`) treats a *skipped*
   required suite as failure (cascade detection) and fails on zero-tests-ran, and the clippy step has
   an explicit `set -o pipefail` guard so `tee` can't mask `-D warnings`.

---

## Not covered
`benchmark.yml` internals beyond triggers/permissions (only the trigger surface reviewed); the
`scripts/perf_ab_gate.py` gate logic line-by-line (assessed via its workflow contract, not audited as
code); Helm chart contents (`deploy/helm/**`) and K8s manifests; the `docker-compose.trading.yml`/
`.benchmark.yml` variants; and non-published-image services (reaper-sync/-mcp/-bench/-cli have no
image in the `docker.yml` matrix — noted as intentional, not audited).
