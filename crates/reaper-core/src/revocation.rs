//! Signed bundle revocation list (Plan 02, Phase B, step 4).
//!
//! A revocation list distrusts already-signed bundles after the fact — a
//! leaked signing key, or a specific bad bundle that must never load again.
//! The signature envelope's validity window bounds *how long* a bundle is
//! accepted; revocation is the *immediate* kill switch within that window.
//!
//! Distribution is **list-pull, not per-load online check** (ADR-2): agents
//! fetch the list from the control plane on the same sync cadence they already
//! use for bundle promotions and cache it, so revocation adds no dependency to
//! the hot load path. The list is itself signed with the bundle signing key,
//! so a compromised CDN/store/proxy cannot forge or strip it, and it carries a
//! monotonic `serial` so an old list cannot be replayed over a newer one.
//!
//! A bundle is refused if its bundle-bytes SHA-256 is in `revoked_bundle_hashes`
//! **or** its signing `key_id` is in `revoked_key_ids`.

use serde::{Deserialize, Serialize};

use crate::bundle_signing::{
    self, sha256, to_hex, BundleSignature, SignatureError, SigningKey, VerifyingKey,
};

/// The revocation list body (the bytes that get signed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationList {
    /// RFC3339 issue time (informational).
    pub issued_at: String,
    /// Monotonic list version. An agent refuses a list whose serial is below
    /// the last one it accepted (anti-rollback for the list itself).
    pub serial: u64,
    /// Unix seconds after which the list is considered *stale*; the agent's
    /// staleness policy then decides fail-open (Monitor) vs fail-closed
    /// (Enforce). 0 = never goes stale.
    pub next_update: i64,
    /// Lowercase-hex SHA-256 digests of revoked bundle bytes.
    pub revoked_bundle_hashes: Vec<String>,
    /// Signing key ids whose every bundle is distrusted (leaked key).
    pub revoked_key_ids: Vec<String>,
    /// Revoked capability ids (F1 agentic authz). Revoking an id kills that
    /// capability AND every capability derived from it (ancestry check in
    /// `Capability::verify_at`). Rides the same signed list-pull channel as
    /// bundle revocation — no per-request online check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_capability_ids: Vec<String>,
}

impl RevocationList {
    /// Canonical bytes for signing/verification: fields in a fixed order with
    /// the lists sorted+deduped, so the same logical list always produces
    /// the same signed message regardless of insertion order.
    ///
    /// COMPATIBILITY: the capability segment is appended ONLY when non-empty.
    /// A list without capability revocations therefore canonicalizes to the
    /// exact pre-F1 bytes, so existing signed lists keep verifying. An agent
    /// running pre-F1 code that receives a list WITH capability revocations
    /// fails signature verification and keeps its last good list — fail
    /// closed, never silently dropping revocations it cannot see.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut hashes = self.revoked_bundle_hashes.clone();
        hashes.sort();
        hashes.dedup();
        let mut keys = self.revoked_key_ids.clone();
        keys.sort();
        keys.dedup();
        // Domain-separated, NUL-delimited; list elements joined by \x1f (unit
        // separator) which cannot appear in hex digests or key ids.
        let mut msg = format!(
            "reaper-revocation-list-v1\0{}\0{}\0{}\0{}\0{}",
            self.issued_at,
            self.serial,
            self.next_update,
            hashes.join("\x1f"),
            keys.join("\x1f"),
        );
        if !self.revoked_capability_ids.is_empty() {
            let mut caps = self.revoked_capability_ids.clone();
            caps.sort();
            caps.dedup();
            msg.push('\0');
            msg.push_str("caps:");
            msg.push_str(&caps.join("\x1f"));
        }
        msg.into_bytes()
    }

    /// Is this bundle (by bytes-digest and signing key id) revoked?
    pub fn is_revoked(&self, bundle_sha256_hex: &str, key_id: &str) -> bool {
        self.revoked_key_ids.iter().any(|k| k == key_id)
            || self
                .revoked_bundle_hashes
                .iter()
                .any(|h| h.eq_ignore_ascii_case(bundle_sha256_hex))
    }
}

/// A revocation list plus its detached signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRevocationList {
    pub list: RevocationList,
    pub signature: BundleSignature,
}

impl SignedRevocationList {
    /// Sign `list` with the control plane's bundle signing key.
    pub fn sign(list: RevocationList, key: &SigningKey, key_id: &str) -> Self {
        let signature = bundle_signing::sign_bundle(&list.canonical_bytes(), key, key_id);
        Self { list, signature }
    }

    /// Verify the list's own signature against the pinned verifying key
    /// (optionally pinning `key_id`). Returns the list on success.
    pub fn verify(
        &self,
        verifying_key: &VerifyingKey,
        expected_key_id: Option<&str>,
    ) -> Result<&RevocationList, SignatureError> {
        bundle_signing::verify_bundle(
            &self.list.canonical_bytes(),
            &self.signature,
            verifying_key,
            expected_key_id,
        )?;
        Ok(&self.list)
    }
}

/// Convenience: lowercase-hex SHA-256 of bundle bytes, matching the digest
/// form stored in `revoked_bundle_hashes`.
pub fn bundle_hash_hex(bundle_bytes: &[u8]) -> String {
    to_hex(&sha256(bundle_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[5u8; 32])))
    }

    fn vk(k: &SigningKey) -> VerifyingKey {
        VerifyingKey::from_hex(k.algorithm(), &k.public_key_hex()).unwrap()
    }

    fn list() -> RevocationList {
        RevocationList {
            issued_at: "2026-01-01T00:00:00Z".to_string(),
            serial: 3,
            next_update: 0,
            revoked_bundle_hashes: vec!["aa".to_string(), "bb".to_string()],
            revoked_key_ids: vec!["leaked-key".to_string()],
            revoked_capability_ids: Vec::new(),
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let k = key();
        let signed = SignedRevocationList::sign(list(), &k, "k1");
        let verified = signed.verify(&vk(&k), Some("k1")).unwrap();
        assert_eq!(verified.serial, 3);
    }

    #[test]
    fn tampering_the_list_breaks_the_signature() {
        let k = key();
        let mut signed = SignedRevocationList::sign(list(), &k, "k1");
        // Add a revocation without re-signing.
        signed.list.revoked_bundle_hashes.push("cc".to_string());
        assert!(signed.verify(&vk(&k), None).is_err());
    }

    #[test]
    fn canonical_bytes_are_order_independent() {
        let mut a = list();
        let mut b = list();
        a.revoked_bundle_hashes = vec!["bb".into(), "aa".into()];
        b.revoked_bundle_hashes = vec!["aa".into(), "bb".into(), "aa".into()];
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn revocation_matching() {
        let l = list();
        assert!(l.is_revoked("aa", "some-key"), "revoked hash");
        assert!(
            l.is_revoked("AA", "some-key"),
            "hash match is case-insensitive"
        );
        assert!(l.is_revoked("ff", "leaked-key"), "revoked key id");
        assert!(!l.is_revoked("ff", "good-key"), "neither revoked");
    }

    #[test]
    fn empty_capability_segment_preserves_pre_f1_canonical_bytes() {
        // The exact byte layout existing signed lists were produced over —
        // a list with no capability revocations MUST keep this encoding, or
        // every already-distributed signed list stops verifying.
        let expected =
            b"reaper-revocation-list-v1\x002026-01-01T00:00:00Z\x003\x000\x00aa\x1fbb\x00leaked-key";
        assert_eq!(list().canonical_bytes(), expected);
    }

    #[test]
    fn capability_revocations_sign_verify_and_tamper() {
        let k = key();
        let mut l = list();
        l.revoked_capability_ids = vec!["cap-2".to_string(), "cap-1".to_string()];
        let signed = SignedRevocationList::sign(l, &k, "k1");
        let verified = signed.verify(&vk(&k), Some("k1")).unwrap();
        assert_eq!(verified.revoked_capability_ids.len(), 2);

        // Adding a capability revocation without re-signing breaks the sig —
        // a middlebox cannot strip or extend the capability kill list.
        let mut tampered = signed.clone();
        tampered.list.revoked_capability_ids.push("cap-3".into());
        assert!(tampered.verify(&vk(&k), None).is_err());

        // Stripping the whole segment also breaks it.
        let mut stripped = signed;
        stripped.list.revoked_capability_ids.clear();
        assert!(stripped.verify(&vk(&k), None).is_err());
    }

    #[test]
    fn capability_ids_are_order_independent() {
        let mut a = list();
        let mut b = list();
        a.revoked_capability_ids = vec!["y".into(), "x".into()];
        b.revoked_capability_ids = vec!["x".into(), "y".into(), "x".into()];
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn legacy_json_without_capability_field_deserializes() {
        let json = r#"{"issued_at":"2026-01-01T00:00:00Z","serial":1,"next_update":0,
            "revoked_bundle_hashes":[],"revoked_key_ids":[]}"#;
        let l: RevocationList = serde_json::from_str(json).unwrap();
        assert!(l.revoked_capability_ids.is_empty());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let signed = SignedRevocationList::sign(list(), &key(), "k1");
        let other =
            SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])));
        assert!(signed.verify(&vk(&other), None).is_err());
    }
}
