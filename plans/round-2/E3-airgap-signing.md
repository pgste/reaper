# Workstream E3 — Signed Air-Gap Bundle Export/Import

Round-2 remediation (`reviews/round-2/`, backlog `plans/round-2/00-NEXT-BACKLOG.md`).
*Closes PROD R2-4 (and the air-gap half of round-1 SEC R2-10).* The managed pull
path already signs bundles at compile and verifies them fail-closed on agents
(`docs/security/BUNDLE_SIGNING.md`), but the **CLI/air-gap path had a hole**:
`compile` produced an unsigned `.rbb` and `bundle deploy` sent no signature, so a
bundle authored offline and carried into an isolated network could not be
authenticated before an agent loaded it.

---

## STATUS (2026-07-15) — COMPLETE

**Landed:**
- **`reaper bundle export <input> -o <out.rbb>`** (`tools/reaper-cli`): compiles a
  source policy (or passes through an `.rbb`), signs the bytes into a **v2
  envelope** (lineage `bundle_id`, monotonic `version`, `not_before`/`expires_at`
  window) via `reaper_core::bundle_signing::sign_bundle_v2`, and writes a detached
  `<out.rbb>.sig` sidecar — byte-identical to the control plane's S3 sidecar. Key
  from `--key` / `REAPER_BUNDLE_SIGNING_KEY` (+ `--key-id`, `--algorithm`,
  `--bundle-id`, `--version`, `--validity-days`).
- **`reaper bundle import <file> [--deploy]`**: reads `<file>.sig`, verifies it
  **offline** against the pinned public key (`--public-key` /
  `REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY`), requiring v2 unless `--allow-v1` and
  failing closed on tamper / wrong-key / expiry / key-id mismatch. With `--deploy`
  it optionally `--data`-loads entities, sends the bundle **and** signature to the
  agent (which re-verifies), then **attests**: the agent-reported `bundle_hash` is
  compared to the signed SHA-256, aborting on `ATTESTATION MISMATCH`.
  `--insecure-skip-verify` is the documented escape hatch.
- **`bundle deploy` auto-attaches** a `<file>.sig` sidecar when present, so the
  normal connected deploy also carries a signature.
- **Agent checksum report**: `list_policies` (`GET /api/v1/policies`) now includes
  each active policy's `bundle_hash` (SHA-256 of the loaded bundle bytes), and
  **`reaper bundle attest [--expect-hash <sha>]`** prints the loaded-bundle
  checksums so an operator can confirm an air-gapped agent loaded exactly the
  signed bytes.
- `tools/reaper-cli/src/airgap.rs` — CLI-facing helper (env/flag key resolution,
  v2 claim construction, sidecar path) with 4 unit tests (round-trip, tamper,
  key-id pin, sidecar path). `docs/security/AIRGAP_BUNDLES.md` documents the flow.

**Verified end-to-end** against the built binary: `export` → offline `import`
verify passes; a flipped bundle byte, a wrong public key, and a key-id-pin
mismatch each fail closed.

**Reused (not rebuilt):** all crypto is `reaper_core::bundle_signing`
(`sign_bundle_v2` / `verify_bundle_at`, Ed25519 + ECDSA-P256, the v2 anti-replay
envelope); the agent already consumed `DeployBundleRequest.signature` and
verified via `BundleVerifier` — the CLI simply stopped leaving it null.
