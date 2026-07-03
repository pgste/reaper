# Bundle Signing

Reaper signs every policy bundle so agents only ever apply policy the control
plane actually produced. The signature is created **when the bundle is compiled**
(not at serve time) and stored next to the bundle, so it stays valid however the
bundle reaches an agent — served by the management plane, pulled from S3, or via
a CDN. Agents **fail closed**: an unsigned or invalid bundle is rejected before
hot-swap.

This makes distribution trustworthy independent of the transport: even a
compromised bundle store, CDN, or a proxy past TLS termination cannot get an
agent to load a policy the control plane did not sign.

## How it works

```
compile ──sign(bundle bytes)──▶ store bundle + <key>.sig sidecar (S3/fs)
                                          │
agent ◀── download (X-Reaper-Bundle-Signature header)  OR  pull sidecar from S3
   └─ verify: SHA-256 integrity + signature authenticity ─▶ hot-swap (or reject)
```

- **Integrity**: SHA-256 of the bundle bytes.
- **Authenticity**: Ed25519 (default) or ECDSA P-256 signature over the bytes.
- The signature envelope (`algorithm`, `key_id`, `sha256`, `signature`) is stored
  as a `<storage_key>.sig` sidecar object in the same backend as the bundle and
  is served in the `X-Reaper-Bundle-Signature` response header.

## 1. Generate a keypair

```bash
reaper-cli keygen                                   # Ed25519 (default)
reaper-cli keygen --algorithm ecdsa-p256-sha256 --key-id fips-2026
```

It prints copy-paste env vars for both sides. Keep the **private** key secret
(control plane only); distribute the **public** key to agents.

## 2. Configure the control plane (reaper-management)

```bash
REAPER_BUNDLE_SIGNING_KEY=<private-key-hex>      # secret
REAPER_BUNDLE_SIGNING_KEY_ID=default
REAPER_BUNDLE_SIGNING_ALGORITHM=ed25519-sha256
```

With no key set, bundles compile **unsigned** (and agents that require signatures
will reject them). A warning is logged at startup and per compile.

## 3. Configure agents (reaper-agent)

```bash
REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY=<public-key-hex>
REAPER_MANAGEMENT_BUNDLE_SIGNATURE_ALGORITHM=ed25519-sha256
REAPER_MANAGEMENT_BUNDLE_KEY_ID=default            # optional: pin to one key
REAPER_MANAGEMENT_REQUIRE_SIGNED_BUNDLES=true      # default; fail closed
```

Verification matrix (`require` = `REAPER_MANAGEMENT_REQUIRE_SIGNED_BUNDLES`):

| Pinned key | Signature present | Result |
|------------|-------------------|--------|
| yes | yes | verify; reject on failure |
| yes | no  | reject if `require`, else warn+allow |
| no  | any | reject if `require`, else warn+allow |

**Secure default:** `require_signed_bundles` is `true`. If you upgrade agents
before configuring signing, managed mode fails closed (rejects bundles) until a
key is set — deploy signing on the control plane and distribute the public key
first, then enable enforcement.

## S3 / pull mode

Because the signature is a sidecar object next to the bundle
(`bundles/<org>/<id>.rbb.sig` beside `bundles/<org>/<id>.rbb`), an agent that
pulls the bundle **directly from S3** can fetch the sidecar from the same prefix
and verify it — no dependency on the management plane being reachable at pull
time.

## Algorithms & FIPS

- `ed25519-sha256` (default) — fast, small keys. Ed25519 is FIPS 186-5 approved.
- `ecdsa-p256-sha256` — ECDSA over NIST P-256, for shops that require a FIPS
  186 curve / a FIPS-140 validated module. SHA-256 is FIPS 180-4.

The `algorithm` field is carried in every signature, so a new scheme can be
added without changing the envelope or re-issuing existing bundles.

## Key rotation

1. Generate a new keypair with a new `key_id`.
2. Point the control plane at the new signing key/`key_id`; new bundles are
   signed with it.
3. Roll the new public key + `key_id` to agents.

To pin agents to exactly one key, set `REAPER_MANAGEMENT_BUNDLE_KEY_ID`; leave it
unset to accept any `key_id` that verifies against the configured public key
during a rollover.
