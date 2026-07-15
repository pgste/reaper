# Air-Gapped Signed Bundle Transfer

*Round-2 workstream E3. Closes PROD R2-4.*

In an air-gapped deployment the control plane is unreachable when a policy is
applied, so a bundle carried across the gap must prove its own authenticity. The
same Ed25519/ECDSA-P256 signing that protects the managed pull path
([`BUNDLE_SIGNING.md`](BUNDLE_SIGNING.md)) is exposed on the CLI so an operator
can **sign on the connected side** and **verify offline on the isolated side**
before an agent ever loads the policy.

```
connected side                    ✈ air gap ✈                 isolated side
  reaper bundle export  ──▶  policy.rbb + policy.rbb.sig  ──▶  reaper bundle import --verify --deploy
   (signs with the                (copy both files:               (verifies offline against the
    private key)                   USB / data diode)               pinned PUBLIC key, then the
                                                                   agent re-verifies before hot-swap)
```

Fail-closed at every step: a tampered bundle, a wrong key, an expired envelope,
or a key-id mismatch aborts the import — the agent never sees it.

## 1. Generate a keypair (once)

```bash
reaper keygen --key-id airgap-2026
```

Keep the **private** key on the connected/signing host
(`REAPER_BUNDLE_SIGNING_KEY`); distribute the **public** key to the isolated
host and the agents (`REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY`).

## 2. Export + sign (connected side)

```bash
export REAPER_BUNDLE_SIGNING_KEY=<private-hex>
export REAPER_BUNDLE_SIGNING_KEY_ID=airgap-2026

reaper bundle export policy.reap -o policy.rbb --validity-days 30
#   writes policy.rbb  and  policy.rbb.sig  (detached signature envelope)
```

`export` accepts either a source policy (`.reap`/`.yaml`/`.json`, compiled on the
fly) or an already-compiled `.rbb`. Flags: `--key`, `--key-id`, `--algorithm`,
`--bundle-id` (lineage UUID, default random), `--version` (monotonic, default
unix-ms), `--validity-days` (default 3650). The signature is a **v2 envelope**:
it binds the bundle id, a monotonic version, and a `not_before`/`expires_at`
window into the signed message, so none of them can be altered without breaking
authenticity.

Carry **both** `policy.rbb` and `policy.rbb.sig` across the gap.

## 3. Import + verify (isolated side)

```bash
export REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY=<public-hex>
export REAPER_MANAGEMENT_BUNDLE_KEY_ID=airgap-2026     # optional: pin to one key

# Verify only (offline authenticity check, no agent needed):
reaper bundle import policy.rbb

# Verify AND deploy to the local agent (the agent re-verifies too):
reaper bundle import policy.rbb --deploy --agent-url http://agent:8080
```

`import` reads `policy.rbb.sig` (or `--sig <path>`), verifies it against the
public key (`--public-key` or env), and refuses legacy v1 envelopes unless
`--allow-v1`. It aborts on any failure. `--insecure-skip-verify` bypasses the
check (not recommended). With `--deploy` it also `--data <file>` loads entities
first and sends the signature to the agent so the agent runs the **same**
fail-closed verification before hot-swapping.

## 4. Attestation — confirm the agent loaded exactly what was signed

The agent computes `bundle_hash = SHA-256(bundle bytes)` for what it actually
applied, and `import --deploy` compares it to the signed digest:

```
   ✅ Deployed
      • agent bundle hash: fc53b4561f11…
      • ✅ attested: agent hash matches the signed SHA-256
```

A mismatch aborts with `ATTESTATION MISMATCH` — proof the applied bundle is not
the one that was signed. For a standing "what's loaded across the fleet" view,
`reaper bundle attest --agent-url <url>` lists each active policy with the
agent-reported `bundle_hash`; compare it to the `SHA-256` printed by `export`.

## Notes

- `bundle deploy` (the normal, connected path) now also **auto-attaches** a
  `<file>.sig` sidecar if one sits next to the bundle, so a signed bundle is sent
  with its signature without any extra flag.
- The `.sig` sidecar is byte-identical to what the control plane writes next to
  an S3 bundle, so the same artifact works for managed pull, CLI deploy, and
  air-gapped import.
- Algorithms and key rotation are identical to the managed path — see
  [`BUNDLE_SIGNING.md`](BUNDLE_SIGNING.md).
