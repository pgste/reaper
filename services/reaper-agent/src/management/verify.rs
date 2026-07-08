//! The single bundle-verification policy for the agent (Plan 02).
//!
//! Both bundle ingestion paths — the management **pull** (SyncService) and the
//! HTTP **push** endpoints (`/api/v1/bundles/deploy`, `/api/v1/bundles/load`)
//! — route through [`BundleVerifier`], so there is exactly one implementation
//! of the fail-closed policy matrix and one place the v2 envelope rules
//! (validity window, strict schema) are enforced. Before this, the push path
//! parsed and hot-swapped bundles with **no signature check at all**
//! (Security P0-2).
//!
//! Policy matrix (`require` = `require_signed_bundles`):
//! - key set, signature present -> verify (integrity + authenticity + v2
//!   window); reject on failure.
//! - key set, signature absent  -> reject if `require`, else warn+allow.
//! - no key, managed agent      -> reject if `require` (misconfiguration must
//!   not fail open), else warn+allow.
//! - no key, standalone agent (push only) -> warn+allow: a standalone agent
//!   has no control plane and no key to verify against; refusing everything
//!   would make the product unusable without a management plane. Protecting
//!   the push surface in that mode is the job of inbound auth + the loopback
//!   default (Plan 01 Phase C).

use std::path::PathBuf;

use reaper_core::bundle_signing::{
    self, BundleSignature, SigAlgorithm, VerifiedEnvelope, VerifyingKey,
};
use reaper_core::config::ManagementSettings;
use tracing::{error, info, warn};

use super::anti_rollback::AntiRollbackStore;

/// Outcome of a successful verification decision.
#[derive(Debug)]
pub enum VerifyOutcome {
    /// A signature was present and fully verified (and passed anti-rollback).
    Verified(#[allow(dead_code)] VerifiedEnvelope),
    /// No signature was verified, but policy allows the load (non-strict or
    /// standalone mode). Callers may log/annotate but proceed.
    UnsignedAllowed,
}

/// Pre-parsed verification policy — built once from config, shared by the
/// sync service and the push handlers.
pub struct BundleVerifier {
    require_signed: bool,
    require_v2: bool,
    key: Option<VerifyingKey>,
    key_id_pin: Option<String>,
    /// Whether this agent is management-connected. Affects only the
    /// no-key push case (standalone agents must keep working).
    managed: bool,
    /// Persisted per-lineage anti-rollback floor (Plan 02 Phase B).
    anti_rollback: AntiRollbackStore,
}

impl BundleVerifier {
    /// Build from config with an in-memory anti-rollback floor (no
    /// persistence). Used by tests and standalone agents.
    pub fn from_config(config: &ManagementSettings) -> Self {
        Self::from_config_with_store(config, AntiRollbackStore::in_memory())
    }

    /// Build from config, persisting the anti-rollback floor to `path` so a
    /// downgrade is still refused after a process restart.
    pub fn from_config_persistent(config: &ManagementSettings, path: PathBuf) -> Self {
        Self::from_config_with_store(config, AntiRollbackStore::persistent(path))
    }

    fn from_config_with_store(
        config: &ManagementSettings,
        anti_rollback: AntiRollbackStore,
    ) -> Self {
        let key = match &config.bundle_public_key {
            Some(hex) => {
                let alg_str = config
                    .bundle_signature_algorithm
                    .as_deref()
                    .unwrap_or(bundle_signing::ALGORITHM);
                match SigAlgorithm::parse(alg_str).and_then(|alg| VerifyingKey::from_hex(alg, hex))
                {
                    Ok(k) => {
                        info!(algorithm = %alg_str, "Bundle signature verification enabled");
                        Some(k)
                    }
                    Err(e) => {
                        error!(error = %e, "Invalid management.bundle_public_key/algorithm; \
                            bundle verification will FAIL CLOSED until fixed");
                        None
                    }
                }
            }
            None => {
                if config.require_signed_bundles && config.enabled {
                    warn!(
                        "require_signed_bundles is true but no bundle_public_key is set — \
                         managed bundles will be REJECTED until a key is configured"
                    );
                }
                None
            }
        };

        Self {
            require_signed: config.require_signed_bundles,
            require_v2: config.require_envelope_v2,
            key,
            key_id_pin: config.bundle_key_id.clone(),
            managed: config.enabled,
            anti_rollback,
        }
    }

    /// Verify a **managed** (pulled) bundle before apply. Fail closed.
    pub fn verify_managed(
        &self,
        data: &[u8],
        sig: Option<&BundleSignature>,
        label: &str,
    ) -> Result<VerifyOutcome, String> {
        self.verify_inner(data, sig, label, /* standalone_open= */ false, false)
    }

    /// Verify a **pushed** bundle (HTTP deploy/load endpoints) before apply.
    /// Identical policy, except a standalone agent (no management, no key)
    /// accepts unsigned pushes — see module docs. `force` overrides only the
    /// anti-rollback floor (an authorized emergency downgrade), never the
    /// signature, window, or revocation checks.
    pub fn verify_push(
        &self,
        data: &[u8],
        sig: Option<&BundleSignature>,
        label: &str,
        force: bool,
    ) -> Result<VerifyOutcome, String> {
        let standalone_open = !self.managed && self.key.is_none();
        self.verify_inner(data, sig, label, standalone_open, force)
    }

    fn verify_inner(
        &self,
        data: &[u8],
        sig: Option<&BundleSignature>,
        label: &str,
        standalone_open: bool,
        force: bool,
    ) -> Result<VerifyOutcome, String> {
        match (&self.key, sig) {
            (Some(key), Some(sig)) => {
                let verified = bundle_signing::verify_bundle_at(
                    data,
                    sig,
                    key,
                    self.key_id_pin.as_deref(),
                    bundle_signing::unix_now(),
                    self.require_v2,
                )
                .map_err(|e| e.to_string())?;
                // Anti-rollback: reject a genuinely-signed but superseded
                // version, and raise the persisted floor on success.
                self.anti_rollback
                    .admit(&verified.bundle_id, verified.version, force)
                    .map_err(|e| e.to_string())?;
                info!(bundle = %label, key_id = %sig.key_id,
                    envelope_version = verified.envelope_version,
                    version = verified.version,
                    "Bundle signature verified");
                Ok(VerifyOutcome::Verified(verified))
            }
            (Some(_), None) => {
                if self.require_signed {
                    Err(
                        "bundle is unsigned but a verification key is configured and \
                         require_signed_bundles is true"
                            .to_string(),
                    )
                } else {
                    warn!(bundle = %label,
                        "Applying UNSIGNED bundle (require_signed_bundles=false)");
                    Ok(VerifyOutcome::UnsignedAllowed)
                }
            }
            (None, _) => {
                if standalone_open {
                    warn!(bundle = %label,
                        "Standalone agent accepting push without signature verification \
                         (no bundle_public_key configured; protect this surface with \
                         inbound auth / loopback bind)");
                    return Ok(VerifyOutcome::UnsignedAllowed);
                }
                if self.require_signed {
                    Err(
                        "require_signed_bundles is true but no bundle_public_key is configured"
                            .to_string(),
                    )
                } else {
                    warn!(bundle = %label,
                        "Bundle signature verification DISABLED (no key, require_signed_bundles=false)");
                    Ok(VerifyOutcome::UnsignedAllowed)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaper_core::bundle_signing::{sign_bundle, sign_bundle_v2, EnvelopeClaims, SigningKey};

    fn test_key() -> SigningKey {
        SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[3u8; 32])))
    }

    fn verifier(
        require: bool,
        require_v2: bool,
        key: Option<&SigningKey>,
        pin: Option<&str>,
        managed: bool,
    ) -> BundleVerifier {
        BundleVerifier {
            require_signed: require,
            require_v2,
            key: key.map(|k| VerifyingKey::from_hex(k.algorithm(), &k.public_key_hex()).unwrap()),
            key_id_pin: pin.map(str::to_string),
            managed,
            anti_rollback: AntiRollbackStore::in_memory(),
        }
    }

    fn v2_sig(key: &SigningKey, data: &[u8], version: u64) -> BundleSignature {
        let now = bundle_signing::unix_now();
        sign_bundle_v2(
            data,
            key,
            "k1",
            &EnvelopeClaims {
                bundle_id: "b-1".to_string(),
                version,
                not_before: now - 60,
                expires_at: now + 3600,
            },
        )
    }

    #[test]
    fn signed_v2_bundle_verifies_and_returns_claims() {
        let key = test_key();
        let v = verifier(true, true, Some(&key), None, true);
        let sig = v2_sig(&key, b"data", 42);
        match v.verify_managed(b"data", Some(&sig), "b").unwrap() {
            VerifyOutcome::Verified(env) => assert_eq!(env.version, 42),
            other => panic!("expected Verified, got {other:?}"),
        }
    }

    #[test]
    fn tampered_bundle_is_rejected() {
        let key = test_key();
        let v = verifier(true, true, Some(&key), None, true);
        let sig = v2_sig(&key, b"data", 1);
        assert!(v.verify_managed(b"DATA", Some(&sig), "b").is_err());
    }

    #[test]
    fn unsigned_rejected_when_required() {
        let key = test_key();
        let v = verifier(true, true, Some(&key), None, true);
        assert!(v.verify_managed(b"data", None, "b").is_err());
        // Push path with a key configured is just as strict.
        assert!(v.verify_push(b"data", None, "b", false).is_err());
    }

    #[test]
    fn unsigned_allowed_when_not_required() {
        let key = test_key();
        let v = verifier(false, true, Some(&key), None, true);
        assert!(matches!(
            v.verify_managed(b"data", None, "b").unwrap(),
            VerifyOutcome::UnsignedAllowed
        ));
    }

    #[test]
    fn no_key_managed_fails_closed() {
        let v = verifier(true, true, None, None, true);
        assert!(v.verify_managed(b"data", None, "b").is_err());
        // A managed agent's PUSH surface fails closed too.
        assert!(v.verify_push(b"data", None, "b", false).is_err());
    }

    #[test]
    fn no_key_standalone_push_is_allowed() {
        // Standalone (management disabled, no key): pushes keep working —
        // inbound auth / loopback bind protect this surface, not signatures
        // the agent has no key to check.
        let v = verifier(true, true, None, None, false);
        assert!(matches!(
            v.verify_push(b"data", None, "b", false).unwrap(),
            VerifyOutcome::UnsignedAllowed
        ));
        // ... but the managed pull path in the same config still fails closed.
        assert!(v.verify_managed(b"data", None, "b").is_err());
    }

    #[test]
    fn legacy_v1_envelope_rejected_when_strict() {
        let key = test_key();
        let sig = sign_bundle(b"data", &key, "k1"); // v1
        let strict = verifier(true, true, Some(&key), None, true);
        assert!(strict.verify_managed(b"data", Some(&sig), "b").is_err());

        // Migration window: require_envelope_v2=false accepts v1.
        let relaxed = verifier(true, false, Some(&key), None, true);
        assert!(matches!(
            relaxed.verify_managed(b"data", Some(&sig), "b").unwrap(),
            VerifyOutcome::Verified(env) if env.envelope_version == 1
        ));
    }

    #[test]
    fn wrong_key_id_pin_is_rejected() {
        let key = test_key();
        let v = verifier(true, true, Some(&key), Some("k2"), true);
        let sig = v2_sig(&key, b"data", 1);
        let err = v.verify_managed(b"data", Some(&sig), "b").unwrap_err();
        assert!(err.contains("key id mismatch"), "{err}");
    }
}
