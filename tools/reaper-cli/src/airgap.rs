//! Signed air-gap bundle helpers (round-2 E3).
//!
//! For air-gapped deployments the control plane is unreachable when a bundle is
//! applied, so the bundle must carry its own proof of authenticity across the
//! gap. These helpers let the CLI **sign** a compiled `.rbb` into a detached
//! `<file>.sig` sidecar (the same envelope the managed/S3 path uses) and
//! **verify** it offline against a pinned public key before deploying — so a
//! bundle physically carried into an isolated network is still only applied if
//! the control plane actually produced it.
//!
//! The crypto lives in `reaper_core::bundle_signing`; this module is the thin
//! CLI-facing layer: env/flag key resolution, v2 claim construction, and the
//! sidecar path convention.

use anyhow::anyhow;
use reaper_core::bundle_signing::{
    self, sign_bundle_v2, verify_bundle_at, BundleSignature, EnvelopeClaims, SigAlgorithm,
    SigningKey, VerifiedEnvelope, VerifyingKey,
};

/// Sidecar suffix for a detached signature (matches the management/S3 convention).
pub const SIGNATURE_SUFFIX: &str = ".sig";

/// The detached-signature path for a bundle file: `<path>.sig`.
pub fn sidecar_path(bundle_path: &str) -> String {
    format!("{bundle_path}{SIGNATURE_SUFFIX}")
}

/// Unix milliseconds — the default monotonic lineage `version` (mirrors the
/// control plane's `now.timestamp_millis()`), so two exports never collide.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A resolved private signing key + its id, for `export`.
pub struct SigningIdentity {
    pub key: SigningKey,
    pub key_id: String,
}

/// Resolve the signing key from an explicit hex or `REAPER_BUNDLE_SIGNING_KEY`,
/// the algorithm from `--algorithm` / `REAPER_BUNDLE_SIGNING_ALGORITHM` (default
/// ed25519), and the key id from `--key-id` / `REAPER_BUNDLE_SIGNING_KEY_ID`
/// (default `default`). Fails closed when no key is available.
pub fn resolve_signing_identity(
    key_hex: Option<&str>,
    algorithm: Option<&str>,
    key_id: Option<&str>,
) -> anyhow::Result<SigningIdentity> {
    let alg = resolve_algorithm(algorithm, "REAPER_BUNDLE_SIGNING_ALGORITHM")?;
    let hex = key_hex
        .map(str::to_string)
        .or_else(|| std::env::var("REAPER_BUNDLE_SIGNING_KEY").ok())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "no signing key: pass --key or set REAPER_BUNDLE_SIGNING_KEY \
                 (generate one with `reaper keygen`)"
            )
        })?;
    let key = SigningKey::from_hex(alg, hex.trim())
        .map_err(|e| anyhow!("invalid signing key for {}: {e}", alg.as_str()))?;
    let key_id = key_id
        .map(str::to_string)
        .or_else(|| std::env::var("REAPER_BUNDLE_SIGNING_KEY_ID").ok())
        .unwrap_or_else(|| "default".to_string());
    Ok(SigningIdentity { key, key_id })
}

/// Signing parameters for one exported bundle.
pub struct ExportParams {
    /// Bundle lineage UUID the envelope binds to.
    pub bundle_id: String,
    /// Monotonic version within the lineage (default: [`now_millis`]).
    pub version: u64,
    /// How long the envelope stays valid, in days.
    pub validity_days: u64,
}

/// Sign bundle bytes into a v2 envelope with a validity window (`not_before` is
/// backdated 5 min for clock skew, as the control plane does).
pub fn sign_export(
    bytes: &[u8],
    identity: &SigningIdentity,
    params: &ExportParams,
) -> BundleSignature {
    let now = bundle_signing::unix_now();
    let claims = EnvelopeClaims {
        bundle_id: params.bundle_id.clone(),
        version: params.version,
        not_before: now - 300,
        expires_at: now + (params.validity_days as i64) * 86_400,
    };
    sign_bundle_v2(bytes, &identity.key, &identity.key_id, &claims)
}

/// A resolved public verifying key + optional pinned key id, for `import`.
pub struct VerifyingIdentity {
    pub key: VerifyingKey,
    pub key_id: Option<String>,
}

/// Resolve the verifying key from an explicit hex or
/// `REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY`, algorithm from `--algorithm` /
/// `REAPER_MANAGEMENT_BUNDLE_SIGNATURE_ALGORITHM` (default ed25519), and an
/// optional pinned key id from `--key-id` / `REAPER_MANAGEMENT_BUNDLE_KEY_ID`.
pub fn resolve_verifying_identity(
    public_key_hex: Option<&str>,
    algorithm: Option<&str>,
    key_id: Option<&str>,
) -> anyhow::Result<VerifyingIdentity> {
    let alg = resolve_algorithm(algorithm, "REAPER_MANAGEMENT_BUNDLE_SIGNATURE_ALGORITHM")?;
    let hex = public_key_hex
        .map(str::to_string)
        .or_else(|| std::env::var("REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY").ok())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "no verifying key: pass --public-key or set \
                 REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY (or --insecure-skip-verify to skip)"
            )
        })?;
    let key = VerifyingKey::from_hex(alg, hex.trim())
        .map_err(|e| anyhow!("invalid public key for {}: {e}", alg.as_str()))?;
    let key_id = key_id
        .map(str::to_string)
        .or_else(|| std::env::var("REAPER_MANAGEMENT_BUNDLE_KEY_ID").ok())
        .filter(|s| !s.trim().is_empty());
    Ok(VerifyingIdentity { key, key_id })
}

/// Verify bundle bytes against a detached signature, offline. `require_v2`
/// rejects legacy v1 envelopes (anti-replay). Returns the authenticated claims.
pub fn verify_export(
    bytes: &[u8],
    sig: &BundleSignature,
    identity: &VerifyingIdentity,
    require_v2: bool,
) -> anyhow::Result<VerifiedEnvelope> {
    verify_bundle_at(
        bytes,
        sig,
        &identity.key,
        identity.key_id.as_deref(),
        bundle_signing::unix_now(),
        require_v2,
    )
    .map_err(|e| anyhow!("signature verification failed: {e}"))
}

fn resolve_algorithm(flag: Option<&str>, env_key: &str) -> anyhow::Result<SigAlgorithm> {
    let s = flag
        .map(str::to_string)
        .or_else(|| std::env::var(env_key).ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| bundle_signing::ALGORITHM.to_string());
    SigAlgorithm::parse(s.trim())
        .map_err(|e| anyhow!("{e} (use ed25519-sha256 or ecdsa-p256-sha256)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> SigningIdentity {
        let key = SigningKey::generate(SigAlgorithm::Ed25519Sha256);
        SigningIdentity {
            key,
            key_id: "airgap-test".to_string(),
        }
    }

    fn verifying_of(id: &SigningIdentity) -> VerifyingIdentity {
        VerifyingIdentity {
            key: VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, &id.key.public_key_hex())
                .unwrap(),
            key_id: Some(id.key_id.clone()),
        }
    }

    #[test]
    fn sidecar_path_appends_suffix() {
        assert_eq!(sidecar_path("policy.rbb"), "policy.rbb.sig");
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let id = identity();
        let bytes = b"REAP compiled bundle bytes";
        let params = ExportParams {
            bundle_id: "3f2b0e26-0000-4000-8000-000000000001".to_string(),
            version: now_millis(),
            validity_days: 365,
        };
        let sig = sign_export(bytes, &id, &params);
        assert_eq!(sig.envelope_version, bundle_signing::ENVELOPE_V2);
        assert_eq!(sig.bundle_id, params.bundle_id);

        // Offline verify against the public key, strict v2, succeeds.
        let verified = verify_export(bytes, &sig, &verifying_of(&id), true).unwrap();
        assert_eq!(verified.version, params.version);
        assert_eq!(verified.bundle_id, params.bundle_id);
    }

    #[test]
    fn tampered_bytes_fail_verification() {
        let id = identity();
        let params = ExportParams {
            bundle_id: "3f2b0e26-0000-4000-8000-000000000002".to_string(),
            version: 1,
            validity_days: 365,
        };
        let sig = sign_export(b"original bundle", &id, &params);
        let err = verify_export(b"tampered bundle", &sig, &verifying_of(&id), true).unwrap_err();
        assert!(err.to_string().contains("verification failed"), "{err}");
    }

    #[test]
    fn wrong_key_id_pin_is_rejected() {
        let id = identity();
        let params = ExportParams {
            bundle_id: "3f2b0e26-0000-4000-8000-000000000003".to_string(),
            version: 1,
            validity_days: 365,
        };
        let sig = sign_export(b"bundle", &id, &params);
        let mut vk = verifying_of(&id);
        vk.key_id = Some("some-other-key".to_string());
        assert!(verify_export(b"bundle", &sig, &vk, true).is_err());
    }
}
