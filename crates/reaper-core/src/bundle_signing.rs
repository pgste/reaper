//! Detached Ed25519 signing + SHA-256 integrity for policy bundles.
//!
//! The control plane signs every bundle it serves; agents verify the signature
//! against a **pinned public key** and the SHA-256 digest before hot-swapping a
//! policy. This makes policy distribution trustworthy *independently of the
//! transport*: even a compromised bundle store, CDN, or a proxy sitting past TLS
//! termination cannot get an agent to load a policy the control plane did not
//! sign.
//!
//! Two independent checks, both must pass (fail closed):
//! 1. **Integrity** — recompute SHA-256 of the bundle bytes and compare to the
//!    signed digest. Fast, catches corruption/truncation.
//! 2. **Authenticity** — verify the Ed25519 signature over the bundle bytes with
//!    the pinned public key. Catches tampering/substitution.
//!
//! Keys are Ed25519 (32-byte seed / 32-byte public key), carried as lowercase
//! hex in config so they are copy-paste friendly and log-safe.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Algorithm identifier embedded in every signature envelope. Lets us rotate to
/// a new scheme later while rejecting anything we do not understand.
pub const ALGORITHM: &str = "ed25519-sha256";

/// A detached signature over a bundle's bytes plus its SHA-256 digest.
///
/// Shipped alongside (not inside) the bundle so the bundle format is unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleSignature {
    /// Signature scheme; currently always [`ALGORITHM`].
    pub algorithm: String,
    /// Identifier of the key that signed this bundle (for key rotation). The
    /// verifier can require a specific `key_id` to pin to one key.
    pub key_id: String,
    /// Lowercase-hex SHA-256 of the signed bundle bytes (64 hex chars).
    pub sha256: String,
    /// Lowercase-hex Ed25519 signature over the bundle bytes (128 hex chars).
    pub signature: String,
}

/// Errors from signing-key handling and bundle verification.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("unsupported signature algorithm: {0}")]
    UnsupportedAlgorithm(String),
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
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Sign `bytes` with `key`, tagging the envelope with `key_id`.
pub fn sign_bundle(bytes: &[u8], key: &SigningKey, key_id: &str) -> BundleSignature {
    let digest = sha256(bytes);
    // Ed25519 signs arbitrary-length messages; we sign the bundle bytes directly
    // (authenticity) and carry the SHA-256 separately (fast integrity pre-check
    // + stable content id).
    let signature = key.sign(bytes);
    BundleSignature {
        algorithm: ALGORITHM.to_string(),
        key_id: key_id.to_string(),
        sha256: to_hex(&digest),
        signature: to_hex(&signature.to_bytes()),
    }
}

/// Verify `bytes` against `sig` using `verifying_key`.
///
/// Both integrity (SHA-256) and authenticity (Ed25519) must pass. If
/// `expected_key_id` is `Some`, the envelope's `key_id` must match it (pinning).
pub fn verify_bundle(
    bytes: &[u8],
    sig: &BundleSignature,
    verifying_key: &VerifyingKey,
    expected_key_id: Option<&str>,
) -> Result<(), SignatureError> {
    if sig.algorithm != ALGORITHM {
        return Err(SignatureError::UnsupportedAlgorithm(sig.algorithm.clone()));
    }
    if let Some(expected) = expected_key_id {
        if sig.key_id != expected {
            return Err(SignatureError::KeyIdMismatch {
                expected: expected.to_string(),
                got: sig.key_id.clone(),
            });
        }
    }

    // 1. Integrity: recompute the digest and compare.
    let expected_digest = to_hex(&sha256(bytes));
    if !constant_time_eq(sig.sha256.as_bytes(), expected_digest.as_bytes()) {
        return Err(SignatureError::IntegrityMismatch);
    }

    // 2. Authenticity: verify the Ed25519 signature over the bytes.
    let sig_bytes = from_hex(&sig.signature)
        .map_err(|e| SignatureError::Malformed(format!("signature hex: {e}")))?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| SignatureError::Malformed("signature must be 64 bytes".to_string()))?;
    let signature = Signature::from_bytes(&sig_arr);
    verifying_key
        .verify(bytes, &signature)
        .map_err(|_| SignatureError::BadSignature)
}

/// Parse a 32-byte Ed25519 public key from lowercase hex.
pub fn verifying_key_from_hex(hex: &str) -> Result<VerifyingKey, SignatureError> {
    let bytes = from_hex(hex.trim()).map_err(|e| SignatureError::InvalidKey(e.to_string()))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SignatureError::InvalidKey("public key must be 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| SignatureError::InvalidKey(e.to_string()))
}

/// Parse a 32-byte Ed25519 signing-key seed from lowercase hex.
pub fn signing_key_from_hex(hex: &str) -> Result<SigningKey, SignatureError> {
    let bytes = from_hex(hex.trim()).map_err(|e| SignatureError::InvalidKey(e.to_string()))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SignatureError::InvalidKey("signing key seed must be 32 bytes".to_string()))?;
    Ok(SigningKey::from_bytes(&arr))
}

/// Hex-encode the public half of a signing key (for config/distribution).
pub fn public_key_hex(key: &SigningKey) -> String {
    to_hex(key.verifying_key().as_bytes())
}

// -- small hex + constant-time helpers (no extra deps) ------------------------

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("odd-length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Length-independent equality for the (public) digest comparison. Not strictly
/// required since the digest is public, but avoids leaking match position.
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

    fn test_key() -> SigningKey {
        // Deterministic seed — tests must not depend on an RNG.
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn sign_then_verify_roundtrips() {
        let key = test_key();
        let bundle = b"policy bundle bytes v1";
        let sig = sign_bundle(bundle, &key, "k1");
        assert_eq!(sig.algorithm, ALGORITHM);
        assert_eq!(sig.key_id, "k1");
        assert_eq!(sig.sha256.len(), 64);
        assert_eq!(sig.signature.len(), 128);
        verify_bundle(bundle, &sig, &key.verifying_key(), Some("k1")).unwrap();
    }

    #[test]
    fn tampered_bytes_fail_integrity() {
        let key = test_key();
        let sig = sign_bundle(b"original bundle", &key, "k1");
        // Flip one byte of the payload.
        let err = verify_bundle(b"originbl bundle", &sig, &key.verifying_key(), None).unwrap_err();
        assert_eq!(err, SignatureError::IntegrityMismatch);
    }

    #[test]
    fn tampered_signature_fails_authenticity() {
        let key = test_key();
        let bundle = b"bundle";
        let mut sig = sign_bundle(bundle, &key, "k1");
        // Corrupt the signature but keep a valid-length hex string so we reach
        // the Ed25519 check (not the malformed path).
        let mut raw = from_hex(&sig.signature).unwrap();
        raw[0] ^= 0xff;
        sig.signature = to_hex(&raw);
        let err = verify_bundle(bundle, &sig, &key.verifying_key(), None).unwrap_err();
        assert_eq!(err, SignatureError::BadSignature);
    }

    #[test]
    fn wrong_key_is_rejected() {
        let signer = test_key();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let bundle = b"bundle";
        let sig = sign_bundle(bundle, &signer, "k1");
        let err = verify_bundle(bundle, &sig, &other.verifying_key(), None).unwrap_err();
        assert_eq!(err, SignatureError::BadSignature);
    }

    #[test]
    fn key_id_pinning_is_enforced() {
        let key = test_key();
        let bundle = b"bundle";
        let sig = sign_bundle(bundle, &key, "k1");
        let err = verify_bundle(bundle, &sig, &key.verifying_key(), Some("k2")).unwrap_err();
        assert_eq!(
            err,
            SignatureError::KeyIdMismatch {
                expected: "k2".to_string(),
                got: "k1".to_string()
            }
        );
    }

    #[test]
    fn unknown_algorithm_is_rejected() {
        let key = test_key();
        let mut sig = sign_bundle(b"bundle", &key, "k1");
        sig.algorithm = "rsa-sha1".to_string();
        let err = verify_bundle(b"bundle", &sig, &key.verifying_key(), None).unwrap_err();
        assert_eq!(
            err,
            SignatureError::UnsupportedAlgorithm("rsa-sha1".to_string())
        );
    }

    #[test]
    fn key_hex_roundtrip() {
        let key = test_key();
        let pub_hex = public_key_hex(&key);
        let vk = verifying_key_from_hex(&pub_hex).unwrap();
        assert_eq!(vk.as_bytes(), key.verifying_key().as_bytes());
        // Signing key seed roundtrip.
        let seed_hex = to_hex(key.as_bytes());
        let sk = signing_key_from_hex(&seed_hex).unwrap();
        assert_eq!(sk.as_bytes(), key.as_bytes());
    }

    #[test]
    fn bad_key_hex_is_rejected() {
        assert!(matches!(
            verifying_key_from_hex("zz"),
            Err(SignatureError::InvalidKey(_))
        ));
        assert!(matches!(
            verifying_key_from_hex("00"),
            Err(SignatureError::InvalidKey(_))
        ));
    }

    #[test]
    fn envelope_serde_roundtrips() {
        let key = test_key();
        let sig = sign_bundle(b"bundle", &key, "k1");
        let json = serde_json::to_string(&sig).unwrap();
        let back: BundleSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(sig, back);
    }
}
