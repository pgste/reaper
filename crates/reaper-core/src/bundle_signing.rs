//! Detached signing + SHA-256 integrity for policy bundles.
//!
//! The control plane signs every bundle **at creation time**; the signature
//! envelope is stored and travels with the bundle, so it can be served by the
//! management plane, an object store (S3), or a CDN without re-signing. Agents
//! verify the signature against a **pinned public key** and the SHA-256 digest
//! before hot-swapping a policy. This makes distribution trustworthy
//! *independently of the transport*: even a compromised bundle store, CDN, or a
//! proxy past TLS termination cannot get an agent to load a policy the control
//! plane did not sign.
//!
//! Two independent checks, both must pass (fail closed):
//! 1. **Integrity** — recompute SHA-256 of the bundle bytes and compare to the
//!    signed digest.
//! 2. **Authenticity** — verify the signature over the bundle bytes with the
//!    pinned public key.
//!
//! Two algorithms are supported, selected by the `algorithm` field so we stay
//! crypto-agile (e.g. for FIPS-validated-module requirements):
//! - `ed25519-sha256` — Ed25519 (default; fast, small keys).
//! - `ecdsa-p256-sha256` — ECDSA over NIST P-256 (for shops that require a
//!   FIPS 186-approved curve / validated module).
//!
//! Keys are carried as lowercase hex in config (copy-paste friendly, log-safe):
//! - Ed25519: 32-byte seed (signing) / 32-byte public key (verifying).
//! - P-256: 32-byte scalar (signing) / SEC1 point, compressed 33-byte or
//!   uncompressed 65-byte (verifying).

// `Signer`/`Verifier` come from the `signature` crate (re-exported here via
// p256); the same trait covers both Ed25519 and P-256 keys.
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Ed25519 algorithm identifier.
pub const ALG_ED25519: &str = "ed25519-sha256";
/// ECDSA P-256 algorithm identifier.
pub const ALG_ECDSA_P256: &str = "ecdsa-p256-sha256";
/// Default algorithm when none is configured.
pub const ALGORITHM: &str = ALG_ED25519;

/// HTTP header the control plane uses to ship the [`BundleSignature`] (as JSON)
/// alongside a bundle download. Agents parse and verify it before hot-swap.
pub const SIGNATURE_HEADER: &str = "x-reaper-bundle-signature";

/// Supported signature algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigAlgorithm {
    Ed25519Sha256,
    EcdsaP256Sha256,
}

impl SigAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            SigAlgorithm::Ed25519Sha256 => ALG_ED25519,
            SigAlgorithm::EcdsaP256Sha256 => ALG_ECDSA_P256,
        }
    }

    pub fn parse(s: &str) -> Result<Self, SignatureError> {
        match s {
            ALG_ED25519 => Ok(SigAlgorithm::Ed25519Sha256),
            ALG_ECDSA_P256 => Ok(SigAlgorithm::EcdsaP256Sha256),
            other => Err(SignatureError::UnsupportedAlgorithm(other.to_string())),
        }
    }
}

/// Current envelope schema version. v1 (legacy) envelopes carry only
/// `algorithm/key_id/sha256/signature`; v2 adds authenticated anti-replay
/// metadata (`bundle_id`, monotonic `version`, validity window).
pub const ENVELOPE_V2: u8 = 2;

fn default_envelope_version() -> u8 {
    1 // a serialized envelope without the field is, by definition, legacy v1
}

/// A detached signature over a bundle's bytes plus its SHA-256 digest.
///
/// Shipped alongside (not inside) the bundle so the bundle format is unchanged
/// and the same envelope works over HTTP headers, S3 object metadata, or a
/// sidecar file.
///
/// **v2 envelopes** additionally authenticate `bundle_id`, a monotonic
/// `version`, and a `not_before`/`expires_at` validity window: these fields
/// are folded into the signed message (see [`sign_bundle_v2`]), so tampering
/// with any of them breaks authenticity — they are not advisory JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleSignature {
    /// Envelope schema version (1 = legacy, 2 = current). Serialized v1
    /// envelopes lack the field and deserialize as 1.
    #[serde(default = "default_envelope_version")]
    pub envelope_version: u8,
    /// Signature scheme: `ed25519-sha256` or `ecdsa-p256-sha256`.
    pub algorithm: String,
    /// Identifier of the key that signed this bundle (for key rotation). The
    /// verifier can require a specific `key_id` to pin to one key.
    pub key_id: String,
    /// v2: UUID of the bundle lineage this envelope is bound to — prevents
    /// replaying a valid envelope against a different bundle's slot.
    #[serde(default)]
    pub bundle_id: String,
    /// v2: monotonic version within the bundle lineage; verifiers refuse
    /// non-increasing versions (anti-rollback).
    #[serde(default)]
    pub version: u64,
    /// v2: unix seconds; the envelope is not valid before this instant.
    #[serde(default)]
    pub not_before: i64,
    /// v2: unix seconds; the envelope is not valid after this instant.
    #[serde(default)]
    pub expires_at: i64,
    /// Lowercase-hex SHA-256 of the signed bundle bytes (64 hex chars).
    pub sha256: String,
    /// Lowercase-hex signature. v1: over the bundle bytes. v2: over the
    /// canonical metadata preamble followed by the bundle bytes.
    pub signature: String,
}

/// The authenticated claims a v2 envelope binds to the bundle bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeClaims {
    /// Bundle lineage UUID.
    pub bundle_id: String,
    /// Monotonic version within the lineage.
    pub version: u64,
    /// Unix seconds; envelope invalid before this instant.
    pub not_before: i64,
    /// Unix seconds; envelope invalid after this instant.
    pub expires_at: i64,
}

/// Verified metadata returned by [`verify_bundle_at`] so callers can run
/// anti-rollback / lineage checks outside the crypto core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEnvelope {
    pub envelope_version: u8,
    pub bundle_id: String,
    pub version: u64,
}

/// Errors from signing-key handling and bundle verification.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("unsupported signature algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("algorithm mismatch: signature is {sig}, key is {key}")]
    AlgorithmMismatch { sig: String, key: String },
    #[error("integrity check failed: SHA-256 mismatch")]
    IntegrityMismatch,
    #[error("signature verification failed")]
    BadSignature,
    #[error("key id mismatch: expected {expected}, got {got}")]
    KeyIdMismatch { expected: String, got: String },
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("malformed signature envelope: {0}")]
    Malformed(String),
    #[error("envelope expired at {expires_at} (now {now})")]
    Expired { expires_at: i64, now: i64 },
    #[error("envelope not valid before {not_before} (now {now})")]
    NotYetValid { not_before: i64, now: i64 },
    #[error("unsupported envelope version {got} (require >= {required})")]
    EnvelopeVersionUnsupported { got: u8, required: u8 },
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// A private signing key for one of the supported algorithms.
pub enum SigningKey {
    Ed25519(Box<ed25519_dalek::SigningKey>),
    EcdsaP256(Box<p256::ecdsa::SigningKey>),
}

impl SigningKey {
    pub fn algorithm(&self) -> SigAlgorithm {
        match self {
            SigningKey::Ed25519(_) => SigAlgorithm::Ed25519Sha256,
            SigningKey::EcdsaP256(_) => SigAlgorithm::EcdsaP256Sha256,
        }
    }

    /// Load a signing key from lowercase hex for the given algorithm.
    /// Ed25519: 32-byte seed. P-256: 32-byte scalar.
    pub fn from_hex(alg: SigAlgorithm, hex: &str) -> Result<Self, SignatureError> {
        let bytes = from_hex(hex.trim()).map_err(|e| SignatureError::InvalidKey(e.to_string()))?;
        match alg {
            SigAlgorithm::Ed25519Sha256 => {
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    SignatureError::InvalidKey("ed25519 seed must be 32 bytes".to_string())
                })?;
                Ok(SigningKey::Ed25519(Box::new(
                    ed25519_dalek::SigningKey::from_bytes(&arr),
                )))
            }
            SigAlgorithm::EcdsaP256Sha256 => {
                let key = p256::ecdsa::SigningKey::from_slice(&bytes)
                    .map_err(|e| SignatureError::InvalidKey(format!("p256 scalar: {e}")))?;
                Ok(SigningKey::EcdsaP256(Box::new(key)))
            }
        }
    }

    /// Generate a fresh random signing key for `alg` from the OS RNG.
    pub fn generate(alg: SigAlgorithm) -> Self {
        loop {
            let mut seed = [0u8; 32];
            // OS RNG failure is unrecoverable and must never silently mint a
            // weak key; `generate` returns `Self`, so we deliberately abort key
            // generation rather than propagate. Justified panic (Plan 05 gate).
            #[allow(clippy::expect_used)]
            {
                getrandom::fill(&mut seed).expect("OS RNG unavailable");
            }
            // For Ed25519 any 32 bytes is a valid seed; for P-256 a random
            // 32-byte scalar is valid with overwhelming probability (retry the
            // astronomically rare invalid case).
            if let Ok(key) = SigningKey::from_hex(alg, &to_hex(&seed)) {
                return key;
            }
        }
    }

    /// Hex-encode the private key (Ed25519 seed / P-256 scalar) for storage in
    /// the control-plane config. Keep this secret.
    pub fn private_key_hex(&self) -> String {
        match self {
            SigningKey::Ed25519(k) => to_hex(k.as_bytes()),
            SigningKey::EcdsaP256(k) => to_hex(&k.to_bytes()),
        }
    }

    /// Hex-encode the matching public key (for config/distribution).
    /// Ed25519: 32-byte key. P-256: compressed SEC1 point (33 bytes).
    pub fn public_key_hex(&self) -> String {
        match self {
            SigningKey::Ed25519(k) => to_hex(k.verifying_key().as_bytes()),
            SigningKey::EcdsaP256(k) => {
                let vk = k.verifying_key();
                to_hex(vk.to_encoded_point(true).as_bytes())
            }
        }
    }

    pub(crate) fn sign_raw(&self, msg: &[u8]) -> Vec<u8> {
        match self {
            SigningKey::Ed25519(k) => k.sign(msg).to_bytes().to_vec(),
            SigningKey::EcdsaP256(k) => {
                let sig: p256::ecdsa::Signature = k.sign(msg);
                sig.to_bytes().to_vec()
            }
        }
    }
}

/// A public verifying key for one of the supported algorithms.
pub enum VerifyingKey {
    Ed25519(Box<ed25519_dalek::VerifyingKey>),
    EcdsaP256(Box<p256::ecdsa::VerifyingKey>),
}

impl VerifyingKey {
    pub fn algorithm(&self) -> SigAlgorithm {
        match self {
            VerifyingKey::Ed25519(_) => SigAlgorithm::Ed25519Sha256,
            VerifyingKey::EcdsaP256(_) => SigAlgorithm::EcdsaP256Sha256,
        }
    }

    /// Load a public key from lowercase hex for the given algorithm.
    /// Ed25519: 32-byte key. P-256: SEC1 point (compressed 33 or uncompressed 65).
    pub fn from_hex(alg: SigAlgorithm, hex: &str) -> Result<Self, SignatureError> {
        let bytes = from_hex(hex.trim()).map_err(|e| SignatureError::InvalidKey(e.to_string()))?;
        match alg {
            SigAlgorithm::Ed25519Sha256 => {
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    SignatureError::InvalidKey("ed25519 public key must be 32 bytes".to_string())
                })?;
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&arr)
                    .map_err(|e| SignatureError::InvalidKey(e.to_string()))?;
                Ok(VerifyingKey::Ed25519(Box::new(vk)))
            }
            SigAlgorithm::EcdsaP256Sha256 => {
                let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&bytes)
                    .map_err(|e| SignatureError::InvalidKey(format!("p256 point: {e}")))?;
                Ok(VerifyingKey::EcdsaP256(Box::new(vk)))
            }
        }
    }

    pub(crate) fn verify_raw(&self, msg: &[u8], sig: &[u8]) -> Result<(), SignatureError> {
        match self {
            VerifyingKey::Ed25519(k) => {
                let arr: [u8; 64] = sig.try_into().map_err(|_| {
                    SignatureError::Malformed("ed25519 sig must be 64 bytes".into())
                })?;
                let signature = ed25519_dalek::Signature::from_bytes(&arr);
                k.verify(msg, &signature)
                    .map_err(|_| SignatureError::BadSignature)
            }
            VerifyingKey::EcdsaP256(k) => {
                let signature = p256::ecdsa::Signature::from_slice(sig)
                    .map_err(|_| SignatureError::Malformed("invalid p256 signature".into()))?;
                k.verify(msg, &signature)
                    .map_err(|_| SignatureError::BadSignature)
            }
        }
    }
}

/// Canonical v2 signed message: a domain-separated, NUL-delimited metadata
/// preamble followed by the bundle bytes. Every field a v2 envelope promises
/// is inside this message, so tampering with any of them breaks authenticity.
/// (No field can contain NUL: UUIDs, decimal integers, and hex digests.)
fn v2_message(
    bundle_id: &str,
    version: u64,
    not_before: i64,
    expires_at: i64,
    sha256_hex: &str,
    bundle_bytes: &[u8],
) -> Vec<u8> {
    let preamble = format!(
        "reaper-bundle-envelope-v2\0{bundle_id}\0{version}\0{not_before}\0{expires_at}\0{sha256_hex}\0"
    );
    let mut msg = Vec::with_capacity(preamble.len() + bundle_bytes.len());
    msg.extend_from_slice(preamble.as_bytes());
    msg.extend_from_slice(bundle_bytes);
    msg
}

/// Sign `bytes` with `key`, tagging the envelope with `key_id`.
///
/// Produces a **legacy v1** envelope (no anti-replay metadata). New code
/// should use [`sign_bundle_v2`]; this remains for compatibility and tests.
pub fn sign_bundle(bytes: &[u8], key: &SigningKey, key_id: &str) -> BundleSignature {
    BundleSignature {
        envelope_version: 1,
        algorithm: key.algorithm().as_str().to_string(),
        key_id: key_id.to_string(),
        bundle_id: String::new(),
        version: 0,
        not_before: 0,
        expires_at: 0,
        sha256: to_hex(&sha256(bytes)),
        signature: to_hex(&key.sign_raw(bytes)),
    }
}

/// Sign `bytes` with `key` into a **v2** envelope binding the given claims.
pub fn sign_bundle_v2(
    bytes: &[u8],
    key: &SigningKey,
    key_id: &str,
    claims: &EnvelopeClaims,
) -> BundleSignature {
    let sha256_hex = to_hex(&sha256(bytes));
    let msg = v2_message(
        &claims.bundle_id,
        claims.version,
        claims.not_before,
        claims.expires_at,
        &sha256_hex,
        bytes,
    );
    BundleSignature {
        envelope_version: ENVELOPE_V2,
        algorithm: key.algorithm().as_str().to_string(),
        key_id: key_id.to_string(),
        bundle_id: claims.bundle_id.clone(),
        version: claims.version,
        not_before: claims.not_before,
        expires_at: claims.expires_at,
        sha256: sha256_hex,
        signature: to_hex(&key.sign_raw(&msg)),
    }
}

/// Verify `bytes` against `sig` using `verifying_key` (compatibility form).
///
/// Accepts both v1 and v2 envelopes; enforces the v2 validity window against
/// the system clock. Callers that must **require** v2 (strict anti-replay) or
/// need the verified claims should use [`verify_bundle_at`].
pub fn verify_bundle(
    bytes: &[u8],
    sig: &BundleSignature,
    verifying_key: &VerifyingKey,
    expected_key_id: Option<&str>,
) -> Result<(), SignatureError> {
    verify_bundle_at(
        bytes,
        sig,
        verifying_key,
        expected_key_id,
        unix_now(),
        false,
    )
    .map(|_| ())
}

/// Unix seconds from the system clock.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Verify `bytes` against `sig` using `verifying_key`, at time `now`.
///
/// All of the following must pass (fail closed): the envelope algorithm must
/// be supported and match the key; if `expected_key_id` is `Some`, the
/// envelope's `key_id` must match it; for v2 envelopes the validity window
/// must contain `now`; the SHA-256 must match; the signature must verify over
/// the version-appropriate message. When `require_v2` is true, v1 envelopes
/// are rejected outright (`EnvelopeVersionUnsupported`).
///
/// Returns the authenticated claims so callers can run anti-rollback and
/// lineage checks (v1 envelopes yield empty `bundle_id` / version 0).
pub fn verify_bundle_at(
    bytes: &[u8],
    sig: &BundleSignature,
    verifying_key: &VerifyingKey,
    expected_key_id: Option<&str>,
    now: i64,
    require_v2: bool,
) -> Result<VerifiedEnvelope, SignatureError> {
    let alg = SigAlgorithm::parse(&sig.algorithm)?;
    if alg != verifying_key.algorithm() {
        return Err(SignatureError::AlgorithmMismatch {
            sig: sig.algorithm.clone(),
            key: verifying_key.algorithm().as_str().to_string(),
        });
    }
    if let Some(expected) = expected_key_id {
        if sig.key_id != expected {
            return Err(SignatureError::KeyIdMismatch {
                expected: expected.to_string(),
                got: sig.key_id.clone(),
            });
        }
    }

    match sig.envelope_version {
        1 if require_v2 => {
            return Err(SignatureError::EnvelopeVersionUnsupported {
                got: 1,
                required: ENVELOPE_V2,
            })
        }
        1 | 2 => {}
        other => {
            // Unknown future schema: fail closed rather than guess.
            return Err(SignatureError::EnvelopeVersionUnsupported {
                got: other,
                required: ENVELOPE_V2,
            });
        }
    }

    // v2 validity window — checked before the (cheaper to skip) crypto so an
    // expired envelope never reports "bad signature".
    if sig.envelope_version == ENVELOPE_V2 {
        if now < sig.not_before {
            return Err(SignatureError::NotYetValid {
                not_before: sig.not_before,
                now,
            });
        }
        if now > sig.expires_at {
            return Err(SignatureError::Expired {
                expires_at: sig.expires_at,
                now,
            });
        }
    }

    // 1. Integrity.
    let expected_digest = to_hex(&sha256(bytes));
    if !constant_time_eq(sig.sha256.as_bytes(), expected_digest.as_bytes()) {
        return Err(SignatureError::IntegrityMismatch);
    }

    // 2. Authenticity over the version-appropriate message.
    let sig_bytes = from_hex(&sig.signature)
        .map_err(|e| SignatureError::Malformed(format!("signature hex: {e}")))?;
    if sig.envelope_version == ENVELOPE_V2 {
        let msg = v2_message(
            &sig.bundle_id,
            sig.version,
            sig.not_before,
            sig.expires_at,
            &sig.sha256,
            bytes,
        );
        verifying_key.verify_raw(&msg, &sig_bytes)?;
    } else {
        verifying_key.verify_raw(bytes, &sig_bytes)?;
    }

    Ok(VerifiedEnvelope {
        envelope_version: sig.envelope_version,
        bundle_id: sig.bundle_id.clone(),
        version: sig.version,
    })
}

// -- small hex + constant-time helpers (no extra deps) ------------------------

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Both nibbles are 0..=15, always valid indices into HEX — no panic.
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Length-independent equality for the (public) digest comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed25519_key() -> SigningKey {
        SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])))
    }

    fn p256_key() -> SigningKey {
        SigningKey::EcdsaP256(Box::new(
            p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap(),
        ))
    }

    fn verifying_of(key: &SigningKey) -> VerifyingKey {
        VerifyingKey::from_hex(key.algorithm(), &key.public_key_hex()).unwrap()
    }

    #[test]
    fn ed25519_roundtrips() {
        let key = ed25519_key();
        let bundle = b"policy bundle bytes v1";
        let sig = sign_bundle(bundle, &key, "k1");
        assert_eq!(sig.algorithm, ALG_ED25519);
        verify_bundle(bundle, &sig, &verifying_of(&key), Some("k1")).unwrap();
    }

    #[test]
    fn p256_roundtrips() {
        let key = p256_key();
        let bundle = b"policy bundle bytes v1";
        let sig = sign_bundle(bundle, &key, "fips-key");
        assert_eq!(sig.algorithm, ALG_ECDSA_P256);
        verify_bundle(bundle, &sig, &verifying_of(&key), Some("fips-key")).unwrap();
    }

    #[test]
    fn tampered_bytes_fail_integrity_both_algs() {
        for key in [ed25519_key(), p256_key()] {
            let sig = sign_bundle(b"original bundle", &key, "k1");
            let err =
                verify_bundle(b"originbl bundle", &sig, &verifying_of(&key), None).unwrap_err();
            assert_eq!(err, SignatureError::IntegrityMismatch, "{}", sig.algorithm);
        }
    }

    #[test]
    fn tampered_signature_fails_authenticity_both_algs() {
        for key in [ed25519_key(), p256_key()] {
            let bundle = b"bundle";
            let mut sig = sign_bundle(bundle, &key, "k1");
            let mut raw = from_hex(&sig.signature).unwrap();
            raw[0] ^= 0xff;
            sig.signature = to_hex(&raw);
            let err = verify_bundle(bundle, &sig, &verifying_of(&key), None).unwrap_err();
            // Either BadSignature or Malformed depending on how the corruption
            // parses; both are rejections.
            assert!(
                matches!(
                    err,
                    SignatureError::BadSignature | SignatureError::Malformed(_)
                ),
                "{}: {err:?}",
                sig.algorithm
            );
        }
    }

    #[test]
    fn wrong_key_is_rejected_both_algs() {
        // Ed25519
        let signer = ed25519_key();
        let other =
            SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])));
        let sig = sign_bundle(b"bundle", &signer, "k1");
        assert_eq!(
            verify_bundle(b"bundle", &sig, &verifying_of(&other), None).unwrap_err(),
            SignatureError::BadSignature
        );
        // P-256
        let signer = p256_key();
        let other = SigningKey::EcdsaP256(Box::new(
            p256::ecdsa::SigningKey::from_slice(&[9u8; 32]).unwrap(),
        ));
        let sig = sign_bundle(b"bundle", &signer, "k1");
        assert_eq!(
            verify_bundle(b"bundle", &sig, &verifying_of(&other), None).unwrap_err(),
            SignatureError::BadSignature
        );
    }

    #[test]
    fn algorithm_mismatch_is_rejected() {
        // Signature says ed25519 but the pinned key is p256.
        let ed = ed25519_key();
        let sig = sign_bundle(b"bundle", &ed, "k1");
        let p256_vk = verifying_of(&p256_key());
        let err = verify_bundle(b"bundle", &sig, &p256_vk, None).unwrap_err();
        assert!(matches!(err, SignatureError::AlgorithmMismatch { .. }));
    }

    #[test]
    fn key_id_pinning_is_enforced() {
        let key = ed25519_key();
        let sig = sign_bundle(b"bundle", &key, "k1");
        let err = verify_bundle(b"bundle", &sig, &verifying_of(&key), Some("k2")).unwrap_err();
        assert_eq!(
            err,
            SignatureError::KeyIdMismatch {
                expected: "k2".into(),
                got: "k1".into()
            }
        );
    }

    #[test]
    fn unknown_algorithm_is_rejected() {
        let key = ed25519_key();
        let mut sig = sign_bundle(b"bundle", &key, "k1");
        sig.algorithm = "rsa-sha1".to_string();
        let err = verify_bundle(b"bundle", &sig, &verifying_of(&key), None).unwrap_err();
        assert_eq!(
            err,
            SignatureError::UnsupportedAlgorithm("rsa-sha1".to_string())
        );
    }

    #[test]
    fn key_hex_roundtrip_both_algs() {
        for key in [ed25519_key(), p256_key()] {
            let pub_hex = key.public_key_hex();
            let vk = VerifyingKey::from_hex(key.algorithm(), &pub_hex).unwrap();
            assert_eq!(vk.algorithm(), key.algorithm());
        }
    }

    #[test]
    fn bad_key_hex_is_rejected() {
        assert!(matches!(
            VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, "zz"),
            Err(SignatureError::InvalidKey(_))
        ));
        assert!(matches!(
            VerifyingKey::from_hex(SigAlgorithm::Ed25519Sha256, "00"),
            Err(SignatureError::InvalidKey(_))
        ));
    }

    #[test]
    fn envelope_serde_roundtrips() {
        let key = p256_key();
        let sig = sign_bundle(b"bundle", &key, "k1");
        let json = serde_json::to_string(&sig).unwrap();
        let back: BundleSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(sig, back);
    }

    fn claims(version: u64, now: i64) -> EnvelopeClaims {
        EnvelopeClaims {
            bundle_id: "3f2b0e26-0000-4000-8000-000000000001".to_string(),
            version,
            not_before: now - 60,
            expires_at: now + 3600,
        }
    }

    #[test]
    fn v2_roundtrips_and_returns_claims() {
        let key = ed25519_key();
        let now = 1_700_000_000;
        let sig = sign_bundle_v2(b"bundle v7", &key, "k1", &claims(7, now));
        assert_eq!(sig.envelope_version, ENVELOPE_V2);
        let verified = verify_bundle_at(
            b"bundle v7",
            &sig,
            &verifying_of(&key),
            Some("k1"),
            now,
            true,
        )
        .unwrap();
        assert_eq!(verified.version, 7);
        assert_eq!(verified.bundle_id, sig.bundle_id);
    }

    #[test]
    fn v2_expired_and_not_yet_valid_are_rejected() {
        let key = ed25519_key();
        let now = 1_700_000_000;
        let sig = sign_bundle_v2(b"bundle", &key, "k1", &claims(1, now));

        let err = verify_bundle_at(
            b"bundle",
            &sig,
            &verifying_of(&key),
            None,
            now + 4000, // past expires_at
            true,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Expired { .. }));

        let err = verify_bundle_at(
            b"bundle",
            &sig,
            &verifying_of(&key),
            None,
            now - 3600, // before not_before
            true,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::NotYetValid { .. }));
    }

    #[test]
    fn v2_metadata_tampering_breaks_authenticity() {
        let key = ed25519_key();
        let now = 1_700_000_000;
        let base = sign_bundle_v2(b"bundle", &key, "k1", &claims(5, now));

        // Bump the version field without re-signing → authenticity failure
        // (the version is inside the signed message).
        let mut tampered = base.clone();
        tampered.version = 6;
        let err = verify_bundle_at(b"bundle", &tampered, &verifying_of(&key), None, now, true)
            .unwrap_err();
        assert_eq!(err, SignatureError::BadSignature);

        // Swap the bundle_id → same failure.
        let mut tampered = base.clone();
        tampered.bundle_id = "3f2b0e26-0000-4000-8000-00000000dead".to_string();
        let err = verify_bundle_at(b"bundle", &tampered, &verifying_of(&key), None, now, true)
            .unwrap_err();
        assert_eq!(err, SignatureError::BadSignature);

        // Extend the expiry window → same failure.
        let mut tampered = base.clone();
        tampered.expires_at += 999_999;
        let err = verify_bundle_at(b"bundle", &tampered, &verifying_of(&key), None, now, true)
            .unwrap_err();
        assert_eq!(err, SignatureError::BadSignature);
    }

    #[test]
    fn legacy_v1_rejected_when_strict_accepted_otherwise() {
        let key = ed25519_key();
        let sig = sign_bundle(b"bundle", &key, "k1"); // v1
        let now = unix_now();

        let err =
            verify_bundle_at(b"bundle", &sig, &verifying_of(&key), None, now, true).unwrap_err();
        assert_eq!(
            err,
            SignatureError::EnvelopeVersionUnsupported {
                got: 1,
                required: ENVELOPE_V2
            }
        );

        // Non-strict (compat wrapper) still accepts v1.
        verify_bundle(b"bundle", &sig, &verifying_of(&key), Some("k1")).unwrap();
    }

    #[test]
    fn unknown_future_envelope_version_fails_closed() {
        let key = ed25519_key();
        let now = 1_700_000_000;
        let mut sig = sign_bundle_v2(b"bundle", &key, "k1", &claims(1, now));
        sig.envelope_version = 9;
        let err =
            verify_bundle_at(b"bundle", &sig, &verifying_of(&key), None, now, false).unwrap_err();
        assert!(matches!(
            err,
            SignatureError::EnvelopeVersionUnsupported { got: 9, .. }
        ));
    }

    #[test]
    fn v1_json_deserializes_as_envelope_version_1() {
        // A stored/legacy envelope predating the schema tag.
        let json = r#"{"algorithm":"ed25519-sha256","key_id":"k1",
            "sha256":"00","signature":"00"}"#;
        let sig: BundleSignature = serde_json::from_str(json).unwrap();
        assert_eq!(sig.envelope_version, 1);
        assert_eq!(sig.version, 0);
        assert!(sig.bundle_id.is_empty());
    }

    #[test]
    fn generate_produces_working_keypair_both_algs() {
        for alg in [SigAlgorithm::Ed25519Sha256, SigAlgorithm::EcdsaP256Sha256] {
            let key = SigningKey::generate(alg);
            // Private key hex reloads to the same public key.
            let reloaded = SigningKey::from_hex(alg, &key.private_key_hex()).unwrap();
            assert_eq!(reloaded.public_key_hex(), key.public_key_hex());
            // The generated key signs and its public key verifies.
            let sig = sign_bundle(b"data", &key, "k");
            let vk = VerifyingKey::from_hex(alg, &key.public_key_hex()).unwrap();
            verify_bundle(b"data", &sig, &vk, Some("k")).unwrap();
        }
    }
}
