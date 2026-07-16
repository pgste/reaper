# Release Integrity & Provenance

> **STATUS: PLANNED (round-3)** — closes the single P0 and the four build/publish
> P1s from `reviews/round-3/07-cicd.md`. This is the round-3 critical path: it is
> the one finding that moves the CI/CD verdict from **NOT READY** back toward
> **CONDITIONAL**. Everything here is CI/workflow config plus a signing/attest
> flow — **no product code, wire-format, or data change.**

**Readiness gate:** Build-and-publish integrity (SLSA build track, SSDF / EO 14028
provenance expectation, SOC 2 CC8.1 change-integrity, DORA Art. 28 ICT third-party
attestation). A bank's third-party-risk process rejects unsigned, unattested
container images and binaries outright — there is no cryptographic chain from the
code that passed review to the artifact a customer pulls.
**Priority:** **P0** (CICD C1) + P1 (C2–C6). Moves gate: **NOT READY → CONDITIONAL.**
**Findings closed:** CICD C1 (P0), C2, C3, C4, C5, C6 (P1); secondary C8/C9/C10 (P2, efficiency).
**Threat model:** compromised-CI — a malicious PR, a poisoned cache, or a
swapped/force-pushed third-party action must not be able to ship a backdoored
image/binary/bundle, and must not obtain the signing identity.

---

## 1. Goal

Establish a **built-once, scanned-before-publish, signed, and attested** release
pipeline so that every shipped artifact class (service images, CLI binaries, Helm
chart, policy bundles) carries a verifiable cryptographic chain from reviewed
source to consumed bytes. Concretely: (a) the exact digest that ships is the exact
digest that was scanned, across **every** published architecture; (b) the
vulnerability gate runs **before** publish and fails closed; (c) every artifact is
cosign-signed via keyless OIDC (no long-lived key); (d) an SLSA build-provenance
attestation and an **image-bound** SBOM are attached to the published digest; and
(e) no third-party action floats on a mutable ref that could execute attacker code
inside a job holding `packages: write` / `contents: write`. The acceptance target
is that a verifier (`cosign verify` + `cosign verify-attestation`) run by a
customer against `ghcr.io/<repo>/reaper-*@<digest>` succeeds and binds the digest
to this repo's workflow identity.

---

## 2. Current state (evidence) — file:line

- **Nothing is signed or attested.** Grep across `.github/workflows/` for
  `cosign|sigstore|minisign|attest|provenance|slsa` → **none**; `build-push-action`
  carries **no** `provenance:`/`sbom:` inputs and no `id-token: write` permission
  (`docker.yml:67-81`, `build-images.permissions` at `:24-26` is only
  `contents: read` + `packages: write`). (CICD **C1, P0**.)
- **Scan runs after publish on releases.** `scan-images` is `needs: build-images`
  (`docker.yml:93`), and `build-images` pushes unconditionally on non-PR events
  (`push: ${{ github.event_name != 'pull_request' }}`, `docker.yml:72`) to
  `latest`/`{{version}}`/`{{major}}.{{minor}}`/sha (`docker.yml:60-65`). A CRITICAL
  first seen at tag time turns the run red **after** the image is already live.
  There is no promote-on-green step and no unpublish. (CICD **C1/C3**.)
- **The scanned bytes are not the shipped bytes.** `scan-images` does its *own*
  `build-push-action` with `load: true`, `platforms: linux/amd64`, tag
  `reaper-scan/${service}:ci` (`docker.yml:109-117`), and Trivy scans that local
  rebuild (`docker.yml:123-130`). The published artifact is a separate multi-arch
  build `platforms: linux/amd64,linux/arm64` (`docker.yml:81`). Two independent
  builds ⇒ no digest identity, and **arm64 is never scanned.** (CICD **C2, P1**.)
- **`build-binaries` is dead.** `release.yml:181` uses `dtolnay/rust-action@stable`
  — a **non-existent action** (correct name `rust-toolchain`, used correctly at
  `:24` and `:125`). The step errors at action-resolve time, so the whole
  `build-binaries` matrix fails on every tag and the advertised
  `reaper-cli-{linux,darwin}-{amd64,arm64}.tar.gz` **have never been produced** —
  while `create-release` (draft), `sbom`, and `publish-helm` succeed independently,
  so a maintainer can undraft a release with no binaries. Undetected because
  `release.yml` triggers only on `tags: ['v*']` (`release.yml:4-5`) and is never
  dry-run. (CICD **C4, P1**.)
- **Zero SHA-pinned actions; two on branch heads.** Grep `uses: .*@[0-9a-f]{40}`
  → **none**. Security-critical actions float on mutable refs:
  `aquasecurity/trivy-action@master` (`docker.yml:124,136`) and
  `dtolnay/rust-toolchain@master` (`fuzz.yml:57`) run with `packages: write` /
  `security-events: write`. The scanner asserting the image is clean is itself
  unpinned. (CICD **C5, P1**.)
- **SBOM is source-level and not artifact-bound.** `release.yml:132-138` runs
  `cargo cyclonedx --format json --all --all-features`. `--all-features` pulls
  backends the shipped binaries never compile; it is a *source-dependency* BOM
  (no base-image OS packages), attached as a **loose tarball** (`release.yml:141-149`)
  and attested to **no** digest. A consumer cannot answer "what is in the image I
  run?" from it. (CICD **C6, P1**.)
- **Policy bundles (.rbb/.rpp) are never built or published in CI** (grep across
  workflows → no bundle compile/publish job). The signed-bundle distribution path
  that Plan 02 specifies has **no** build/publish workflow, so it scores blank on
  every integrity column. (CICD scorecard, last row.)
- **Correctly done today (preserve, don't regress):** fork PRs are safe — the ghcr
  login is guarded `if: github.event_name != 'pull_request'` (`docker.yml:48`), CI
  uses `pull_request` (not `pull_request_target`) with no secrets in test jobs, and
  PR images are built-not-pushed. The signing work below must keep this property:
  **OIDC token minting and any registry write happen only on `push`/tag events,
  never on a fork PR.**

---

## 3. Definition of Done — the build/publish provenance scorecard

The gate is met when **every cell below is green** for each shipped artifact class.
This scorecard is the falsifiable exit criterion; the round-3 review's version of
it (all No/Partial) is the baseline we are inverting.

| Artifact class | Built once | Scanned (shipped digest, all arches) | Signed (cosign keyless) | SBOM (attested to digest) | Provenance (SLSA) | Reproducible | Right registry | Right identity |
|----------------|:---------:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **Service images** (agent/platform/management) | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ `ghcr.io` | ☐ repo OIDC |
| **CLI binaries** (reaper-cli ×4 targets) | ☐ | n/a | ☐ blob sig | ☐ `SHA256SUMS` + SBOM | ☐ | ☐ `--locked` | ☐ GH Release | ☐ repo OIDC |
| **Helm chart** | ☐ | n/a | ☐ OCI sign | ☐ | ☐ | n/a | ☐ `ghcr.io/charts` | ☐ repo OIDC |
| **Policy bundles** (.rbb/.rpp) | ☐ | n/a | ☐ blob sig | ☐ | ☐ | ☐ deterministic compile | ☐ GH Release / OCI | ☐ repo OIDC |

Per-cell acceptance:

- **Built once** — the digest referenced in the release body, scanned, signed, and
  attested is byte-identical to the one pushed. No second local rebuild anywhere.
  *Verify:* `docker buildx imagetools inspect` digest == the digest the sign/scan
  steps consumed (assert equality in-workflow).
- **Scanned (shipped digest, all arches)** — Trivy scans the pushed multi-arch
  manifest by digest, amd64 **and** arm64, and its non-zero exit blocks promotion.
  *Verify:* a seeded CRITICAL in the base image fails the release before any
  consumer-facing tag moves.
- **Signed** — `cosign verify ghcr.io/<repo>/reaper-<svc>@<digest>
  --certificate-identity-regexp '…/\.github/workflows/…' --certificate-oidc-issuer
  https://token.actions.githubusercontent.com` exits 0. CLI/bundle blobs verify via
  `cosign verify-blob`. *Verify:* a tampered byte makes verification fail.
- **SBOM (attested to digest)** — `cosign verify-attestation --type cyclonedx`
  against the digest returns an image-derived SBOM (base-image OS packages present,
  shipped feature set only). *Verify:* the SBOM lists `libssl`/distro packages the
  source-level `cargo cyclonedx` omitted.
- **Provenance (SLSA)** — `cosign verify-attestation --type slsaprovenance` (or GH
  `actions/attest-build-provenance`) binds `builder.id`, source commit, and
  workflow ref to the digest. *Verify:* provenance `subject.digest` == shipped
  digest; `buildType` is the SLSA GitHub generator.
- **Reproducible** — `cargo build --locked`; `SOURCE_DATE_EPOCH` set; apt pinned or
  distroless. *Verify:* two builds of the same tag on clean runners produce the same
  image digest (best-effort; documented if unattainable).
- **Right identity** — every signature/attestation certificate's SAN is this repo's
  workflow at the release ref; **no** signature is ever produced on a fork PR.

Plus these workflow-shape checkboxes:

- ☐ `release.yml` builds each artifact **exactly once** in a job whose output digest
  is consumed by scan/sign/attest downstream (no `dtolnay/rust-action@stable`;
  binaries actually produced — closes C4).
- ☐ Publish order is **build → scan → sign+attest → promote tag** (or push to a
  quarantine tag, promote only on green scan) — no publish precedes the gate
  (closes C1/C3).
- ☐ Every `uses:` in every workflow is pinned to a full 40-char commit SHA
  (first-party `actions/*` included); `trivy-action` and `rust-toolchain@master`
  pinned; Dependabot `github-actions` ecosystem enabled to bump pins (closes C5).
- ☐ The release body references images **by digest** (`@sha256:…`), not by mutable
  tag, and the draft cannot be undrafted until images + binaries + Helm all pass
  (closes C12 coupling; secondary).
- ☐ `grep -RE 'uses: .*@(master|main|stable)' .github/workflows/` returns nothing
  for third-party actions; `grep -R 'cosign' .github/workflows/` is non-empty.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

**Step 1 — SHA-pin every action; kill the two mutable-head refs first.**
- *What:* Replace every `uses: owner/action@<tag|branch>` with
  `uses: owner/action@<40-char-sha>  # <tag>`. Prioritize the two branch-head
  security-critical ones. Add `.github/dependabot.yml` with the `github-actions`
  ecosystem so pins are bumped by reviewed PRs, not left to rot.
- *Where:* all of `.github/workflows/*` — start with `docker.yml:124,136`
  (`trivy-action@master`) and `fuzz.yml:57` (`rust-toolchain@master`); then
  `release.yml`, `docker.yml`, `ci.yml`, `perf-gate.yml`, etc.
- *Verify:* `grep -RE 'uses: .*@[0-9a-f]{40}' .github/workflows | wc -l` == total
  `uses:` count; `grep -RE 'uses: .*@(master|main|v[0-9]+|stable)' .github/workflows`
  finds only the trailing `# vX` comments, not live refs.
- *Rationale:* do this **first** — every later step adds more `uses:` (cosign,
  attest); pinning is cheapest before the surface grows, and it is the compromised-CI
  root fix that makes the rest trustworthy.

**Step 2 — Reshape `docker.yml` to build-once → scan-by-digest → sign+attest → promote.**
- *What:* Collapse `build-images` + `scan-images` into one flow that (a) builds and
  pushes the multi-arch image **by digest only** to a quarantine reference (or pushes
  digests without moving `latest`/semver tags), (b) scans the **pushed digest** for
  every platform, (c) on green, cosign-signs the digest and attaches SLSA provenance
  + an image-derived CycloneDX SBOM, (d) **then** moves the consumer tags. Add
  `id-token: write` to the job and set `provenance: true` / `sbom: true` on
  `build-push-action` (buildx-native attestations), reinforced by cosign.
- *Where:* `.github/workflows/docker.yml` — replace the split at `:21-149`. Remove
  the second local rebuild (`:109-117`) entirely (kills C2). Keep the fork guard
  (`:48`) and gate the sign/attest/tag-promote steps on
  `if: github.event_name != 'pull_request'` so fork PRs never mint OIDC or write.
- *Verify:* on a tag, the digest scanned == the digest signed == the digest in the
  release body; arm64 appears in scan logs; `cosign verify …@<digest>` and
  `cosign verify-attestation --type cyclonedx` succeed; a seeded CRITICAL blocks the
  tag-promote step (image never reachable under `latest`/semver).
- *Sketch (illustrative, SHAs elided):*
  ```yaml
  build-scan-sign:
    permissions:
      contents: read
      packages: write
      id-token: write        # OIDC for cosign keyless — never granted on fork PRs
    steps:
      - uses: docker/build-push-action@<sha>   # v6
        id: build
        with:
          platforms: linux/amd64,linux/arm64
          push: ${{ github.event_name != 'pull_request' }}
          provenance: true                     # buildx SLSA provenance attestation
          sbom: true                           # buildx image-bound SBOM
          outputs: type=image,push-by-digest=true,name-canonical=true
      # scan the EXACT pushed digest, all platforms — before any tag moves
      - uses: aquasecurity/trivy-action@<sha>
        with:
          image-ref: ${{ env.REGISTRY }}/${{ env.IMAGE_PREFIX }}/${{ matrix.service }}@${{ steps.build.outputs.digest }}
          severity: 'CRITICAL,HIGH'
          exit-code: '1'
          ignore-unfixed: true
      - if: github.event_name != 'pull_request'
        uses: sigstore/cosign-installer@<sha>
      - if: github.event_name != 'pull_request'
        run: |
          cosign sign --yes "${IMAGE}@${DIGEST}"
          syft "${IMAGE}@${DIGEST}" -o cyclonedx-json > sbom.json
          cosign attest --yes --type cyclonedx --predicate sbom.json "${IMAGE}@${DIGEST}"
      # promote consumer tags ONLY after scan+sign are green
      - if: github.event_name != 'pull_request'
        run: docker buildx imagetools create --tag "${IMAGE}:${VERSION}" "${IMAGE}@${DIGEST}"
  ```

**Step 3 — Fix and harden `build-binaries`; sign blobs; ship checksums.**
- *What:* Correct the dead action ref, build each target **once** with `--locked`,
  emit a `SHA256SUMS`, and cosign-sign each tarball (`cosign sign-blob`) so the CLI
  has the same provenance story as the images. Migrate the archived release actions.
- *Where:* `release.yml:181` `dtolnay/rust-action@stable` → `dtolnay/rust-toolchain@<sha>`;
  `release.yml:192` add `--locked`; replace `actions/create-release@v1` (`:80`) and
  `actions/upload-release-asset@v1` (`:142`, `:203`) with a SHA-pinned
  `softprops/action-gh-release@<sha>` that uploads all tarballs + `SHA256SUMS` +
  `*.sig`/`*.pem` in one step (also addresses C13). Add `id-token: write` to the job.
- *Verify:* a tagged (or dry-run) build produces all four `reaper-cli-*.tar.gz`;
  `sha256sum -c SHA256SUMS` passes; `cosign verify-blob --signature … --certificate …`
  succeeds against repo OIDC identity.

**Step 4 — Replace the source SBOM with an image-/artifact-bound SBOM attested to the digest.**
- *What:* Stop attaching the over-broad `--all-features` source SBOM as the artifact
  BOM. Generate the SBOM **from the built image** (`syft <ref>@<digest> -o
  cyclonedx-json` or `trivy image --format cyclonedx`) with the shipped feature set,
  and attest it to the digest (Step 2). Keep a *source* SBOM only as a supplementary
  release asset clearly labeled `source-deps`, not the artifact BOM.
- *Where:* remove/relabel `release.yml:132-149`; the authoritative SBOM now comes
  from `docker.yml` Step 2's `cosign attest --type cyclonedx`. For the CLI, generate
  a per-binary CycloneDX from the built artifact's dependency set (compiled features
  only, not `--all-features`).
- *Verify:* `cosign verify-attestation --type cyclonedx …@<digest>` returns a BOM
  containing base-image OS packages; the BOM does **not** list mysql/AWS backends the
  shipped image omits (closes C6).

**Step 5 — Add a policy-bundle build/sign/publish job (closes the blank scorecard row).**
- *What:* Compile the canonical policy set to `.rbb`/`.rpp` in CI via the existing
  `reaper-cli compile`, sign the bundle bytes with cosign (reusing Plan 02's signing
  primitive semantics — see ADR-5), and publish signed bundles as release assets
  (and/or an OCI artifact). Deterministic compile so the bundle is reproducible.
- *Where:* new job in `release.yml` (or a dedicated `bundles.yml`) invoking
  `reaper-cli compile … --output` then `cosign sign-blob`; upload via the same
  `action-gh-release` step as Step 3.
- *Verify:* a released `.rbb` asset verifies with `cosign verify-blob`; `reaper-cli
  bundle validate` accepts it; two compiles of the same source produce identical bytes.

**Step 6 — Couple the release so a draft cannot ship half-built; reference images by digest.**
- *What:* Make `create-release` depend on images + binaries + bundles all passing,
  reference images **by digest** in the body, and add an explicit undraft/publish
  step gated on all upstream jobs green (removes the "human undrafts a release missing
  binaries" hazard, C12). Add a tag-shaped dry-run (`workflow_dispatch` on a fake ref)
  so `release.yml` breakage is caught off the tag path (closes the C4 detection gap).
- *Where:* `release.yml` job graph — add `needs:` edges from the publish/undraft step
  to `build-binaries`, the image flow, and the bundle job; template the body with
  `${digest}` outputs instead of `:${version}` tags at `:95-97`.
- *Verify:* a run where any artifact job fails leaves the release in `draft` with no
  undraft; the published body's `docker pull` lines carry `@sha256:…`.

---

## 5. Dependencies

- **CI tooling:** `sigstore/cosign-installer`, `syft` (or `trivy image --format
  cyclonedx`), `actions/attest-build-provenance` (or the SLSA generator),
  `softprops/action-gh-release`, buildx ≥ v0.11 for native `provenance:`/`sbom:`
  attestations. All SHA-pinned (Step 1).
- **Repository settings (out-of-band, must accompany):** enable OIDC (`id-token:
  write`) — already grantable per-job; confirm ghcr allows attestation/signature
  push (cosign writes `.sig`/`.att` tags alongside the image). No org secret is
  required for keyless signing — that is the point (ADR-2).
- **Plan 02 (Policy Integrity & Distribution)** — Step 5's bundle signing must not
  fork a second signer. Reuse Plan 02's `bundle_signing` verification semantics so a
  cosign-signed *distribution* wrapper and Plan 02's *content* signature compose
  rather than conflict (ADR-5). This is the same "build the signer once" linkage the
  roadmap calls out (04 ↔ 02).
- **No product-code change** for any step; Step 5 invokes the already-shipped
  `reaper-cli compile` path. Fork-PR safety (§2 last bullet) is a hard constraint on
  Steps 2–5, not a follow-up.

---

## 6. Testing & verification

- **Digest identity (C1/C2):** in-workflow assertion that the scanned/signed/attested
  digest equals the pushed digest; fail the job on mismatch. Manual: `cosign verify
  …@<digest>` from a clean machine with no repo access.
- **Scan-before-publish (C1/C3):** a scratch branch seeds a base image with a known
  fixable CRITICAL → the tag-promote step is never reached; `latest`/semver do not
  move; the run is red. Revert → green, tags move.
- **arm64 coverage (C2):** scan logs show both `linux/amd64` and `linux/arm64`
  scanned for the same manifest digest.
- **Binaries exist (C4):** a `workflow_dispatch` dry-run of `release.yml` produces all
  four tarballs + `SHA256SUMS`; `sha256sum -c` passes; each verifies with
  `cosign verify-blob`.
- **Action pinning (C5):** `grep -RE 'uses: .*@[0-9a-f]{40}'` covers every `uses:`;
  no live `@master`/`@main`/`@stable`. A red-team check: point a workflow at a
  throwaway action tag, force-push the tag to malicious code — the pinned SHA must
  keep running the reviewed code.
- **Image-bound SBOM (C6):** `cosign verify-attestation --type cyclonedx` returns a
  BOM with base-image OS packages present and unshipped backends absent.
- **Bundles (scorecard):** released `.rbb` verifies with `cosign verify-blob` and
  `reaper-cli bundle validate`; deterministic re-compile matches bytes.
- **Fork safety:** open a PR from a fork → confirm no `id-token` mint, no registry
  write, no signature produced (image built-not-pushed as today).
- **Falsifiable acceptance:** `grep -R 'cosign' .github/workflows/` non-empty;
  `grep -RE 'uses: .*@(master|main|stable)[^0-9a-f]' .github/workflows/` empty (third-party);
  `grep -n 'dtolnay/rust-action' .github/workflows/release.yml` empty; a release page
  shows `reaper-cli-*.tar.gz`, `SHA256SUMS`, `*.sig`, and a `bom.json` attested to a
  digest; `cosign verify ghcr.io/<repo>/reaper-agent@<digest>` exits 0.

---

## 7. Effort & phasing — S/M/L

| Phase | Scope | Size |
|-------|-------|------|
| **Pin + unblock** | Step 1 (SHA-pin all actions, Dependabot), Step 3's one-line C4 fix (`rust-action`→`rust-toolchain` + `--locked`) — config-only, removes the compromised-CI vector and revives binaries | **S** |
| **Scan-then-publish + sign images** | Step 2 (build-once, scan-by-digest, cosign sign + SLSA provenance + image SBOM, tag-promote on green) — the P0 core | **M** |
| **Blob signing + SBOM fidelity + coupling** | Step 3 (blob sig + checksums + gh-release migration), Step 4 (image-bound SBOM), Step 6 (release coupling + dry-run) | **S–M** |
| **Bundle provenance** | Step 5 (compile/sign/publish policy bundles) | **S–M** |

The P0 (C1) is fully closed by Phase 1 + Phase 2. Phases 3–4 close the remaining P1s
(C4 fully, C6, and the blank bundle row) and the C12/C13 P2 coupling. Total is small
relative to assurance value — no product code, and each phase is independently
shippable and revertible.

---

## 8. Key decisions (ADR-style)

- **ADR-1: Build once, scan the pushed digest, promote tags only on green.**
  *Context:* today two independent builds exist and publish precedes the scan
  (C1/C2/C3). *Decision:* a single multi-arch build pushed by digest, scanned by that
  digest across all platforms, with consumer tags moved only after scan+sign pass.
  *Consequence:* the scanned bytes **are** the shipped bytes; a release-time CRITICAL
  can no longer reach `latest`/semver. *Rejected:* keeping a separate amd64 scan
  rebuild "for speed" — it is precisely the gap a bank flags.
- **ADR-2: Keyless cosign (OIDC), no long-lived signing key.** *Decision:* sign via
  the workflow's short-lived OIDC identity; the certificate SAN binds each signature
  to this repo's workflow at the release ref. *Consequence:* no key to store, rotate,
  or leak; verification is identity-based (`--certificate-identity-regexp` +
  `--certificate-oidc-issuer`). *Rejected:* a minisign/long-lived cosign keypair —
  introduces a secret that a compromised runner could exfiltrate (the very threat
  model), and offline air-gapped signing is tracked separately by round-2 E3.
- **ADR-3: SLSA build provenance via both buildx-native attestation and cosign
  attest.** *Decision:* set `provenance: true` on `build-push-action` and additionally
  `cosign attest` so provenance is verifiable through the sigstore path a customer
  already uses for signatures. *Consequence:* one verification toolchain
  (`cosign verify`/`verify-attestation`) covers signature, SBOM, and provenance.
- **ADR-4: SBOM is generated from the built artifact, not `--all-features` source.**
  *Decision:* the authoritative BOM is `syft`/`trivy` over the image digest (shipped
  feature set + base-image packages), attested to the digest; the source-level
  `cargo cyclonedx` is kept only as a labeled supplement. *Consequence:* the BOM
  answers "what is in the image I run?" and stops over-reporting unshipped backends
  (closes C6).
- **ADR-5: Bundle distribution signing reuses Plan 02's signing primitive, layered
  not duplicated.** *Decision:* Plan 02 signs bundle *content* for load-time
  verification; this plan's cosign `sign-blob` signs the *distributed file* for
  supply-chain provenance. They compose — do not build a third signer. *Consequence:*
  a released bundle is verifiable both as "authentic distribution" (cosign) and
  "loadable, unrevoked content" (Plan 02) — matching the roadmap's build-the-signer-once
  linkage.
- **ADR-6: SHA-pin all actions; fork PRs never touch the signing identity.**
  *Decision:* every `uses:` pinned to a full commit SHA (Dependabot bumps); OIDC mint
  and registry writes gated to non-PR events, preserving the existing fork-safe login
  (`docker.yml:48`) and the `pull_request` (not `pull_request_target`) posture.
  *Consequence:* neither a swapped action tag nor a fork PR can obtain
  `packages: write` or the signing OIDC token.

---

## 9. Risks & rollback

- **Risk: keyless signing/attestation misconfigured → release job red on a real tag.**
  *Mitigation:* land Step 2 behind a `workflow_dispatch` dry-run against a scratch
  digest first; verify `cosign verify`/`verify-attestation` succeed before the change
  is on the tag path. *Rollback:* the sign/attest/promote steps are additive and
  gated on `!= pull_request`; disabling them reverts to build+push while leaving the
  scan-before-publish reorder intact.
- **Risk: scan-then-promote adds latency / an unfixable base-image CVE blocks a
  release.** *Mitigation:* `ignore-unfixed: true` + CRITICAL/HIGH threshold (unchanged
  policy); escalate an unfixable finding to a tracked, dated exception per
  `docs/security/VULN_RESPONSE.md`, never back to publish-before-scan. *Rollback:*
  none preferred — the ordering is the fix; a temporary exception list is the safety
  valve.
- **Risk: SHA-pinning breaks a workflow when an action's SHA lags a needed fix.**
  *Mitigation:* Dependabot `github-actions` opens bump PRs; pins carry a `# vX`
  comment for readability. *Rollback:* bump a single pin in one line; never revert to
  a floating ref.
- **Risk: image-bound SBOM tooling (`syft`/`trivy cyclonedx`) output differs from the
  prior source SBOM, surprising a consumer.** *Mitigation:* keep the source SBOM as a
  labeled supplementary asset during transition; document the switch in the release
  notes and `CLAUDE.md` supply-chain section. *Rollback:* re-attach the source SBOM as
  primary (discouraged — it is the C6 finding).
- **Risk: reproducible-build parity (`SOURCE_DATE_EPOCH`, pinned apt) unattainable on
  a given base image.** *Mitigation:* treat "reproducible" as best-effort; if digests
  can't be made bit-identical, document why and rely on signing+provenance for the
  integrity guarantee (reproducibility is defense-in-depth, not the gate). *Rollback:*
  n/a — non-blocking column.
- **General rollback:** every change is additive CI config plus SHA pins — no
  product-code, wire-format, or data change. Any single gate (sign, attest, scan,
  bundle publish) can be disabled in one line without affecting the others, so a
  misconfigured gate never blocks the pipeline irreversibly.

---

## Secondary — CI efficiency (P2: C8/C9/C10)

Not gate-moving, but cheap wall-clock/cost wins the review flags; fold in while the
workflows are already open. Keep these **separate commits** from the integrity work
so a revert of one never touches the other.

- **Move heavy suites off the every-PR path (C9, highest-leverage).**
  memory-scale-100k (`ci.yml:778`), the 5×10k volume matrix (`ci.yml:702`),
  scale-tests (`ci.yml:865`), and the eval micro-bench (`ci.yml:1283`) run per-PR as
  `continue-on-error` (non-blocking) — ~7 heavy release-mode jobs burning runner-hours
  while gating nothing. *Fix:* move to `schedule:` + an opt-in `perf`/`scale` label
  (`if: contains(github.event.pull_request.labels.*.name, 'scale')`).
- **Drop the global `jobs = 2` compile cap in CI (C8).** `.cargo/config.toml:2` caps
  *every* compile in *every* fan-out job to 2 parallel rustc on 4-vCPU runners.
  *Fix:* leave the local dev cap but override in CI via `CARGO_BUILD_JOBS=0` (or
  `nproc`) in the workflow `env:`; ~1.5–2× compile speedup fleet-wide. Do **not**
  edit the committed `.cargo/config.toml` value (dev machines rely on it) — override
  by env in CI only.
- **Fix `save-if` cache thrash (C10).** The main/develop `save-if` guard is present on
  lint/api/mgmt-pg/wasm but **absent** on unit/ebpf/volume/memory/scale/integration/
  bdd/eval, all sharing `shared-key: reaper-ci` — so many PR jobs race to write one
  key, evicting each other. *Fix:* let one canonical job own the `shared-key` write and
  apply the `save-if: github.ref == 'refs/heads/main'` guard consistently to the rest.
- **Generate data fixtures once (C14).** `generate_{rbac,abac,multilayer}_data` are
  recompiled+run separately in integration, bdd, and volume jobs. *Fix:* generate once,
  pass as an artifact between jobs.

*These are advisory efficiency items — none block the CONDITIONAL gate; they reduce
CI cost and wall-clock and remove PR cache flakiness.*

---

*Planning artifact only — no product code modified. Anchors verified against current
`main`: `docker.yml` (build/scan split, `trivy-action@master`), `release.yml:181`
(`dtolnay/rust-action@stable`), `release.yml:132-138` (source SBOM), `fuzz.yml:57`
(`rust-toolchain@master`), `.cargo/config.toml:2` (`jobs = 2`).*
