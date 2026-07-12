//! Commit-signature verification (Plan 09 Step 5, Security P1-3 part 2).
//!
//! With `require_signed_commits` set on a source, a sync of an unsigned or
//! untrusted-key HEAD commit must **fail closed** — whatever the branch tip
//! carries would otherwise become policy. We verify the commit's SSH signature
//! (the `SSHSIG` format `git config gpg.format ssh` produces) against the
//! source's configured trusted signer keys.
//!
//! Git signs commits under the `git` SSHSIG namespace; the signed message is
//! the commit object with its `gpgsig` header removed — exactly the
//! `(signature, content)` pair `git2::Repository::extract_signature` returns.
//!
//! GPG (`gpg.format openpgp`) signatures are **not** verified here: rather than
//! accept a signature we can't check, a source that requires signing but whose
//! HEAD is GPG-signed fails closed with a clear reason.

use ssh_key::{HashAlg, PublicKey, SshSig};
use thiserror::Error;

/// The SSHSIG namespace git uses for commit/tag signatures.
const GIT_NAMESPACE: &str = "git";

#[derive(Debug, Error, PartialEq)]
pub enum CommitVerifyError {
    #[error("commit is not signed")]
    Unsigned,
    #[error("commit signature is not a supported SSH signature (GPG is not supported)")]
    UnsupportedSignature,
    #[error("no trusted signing keys are configured for this source")]
    NoTrustedKeys,
    #[error("a configured trusted key could not be parsed: {0}")]
    BadTrustedKey(String),
    #[error("commit signature did not verify against any trusted key")]
    Untrusted,
}

/// Verify a commit's SSH signature against a set of trusted `authorized_keys`-
/// style public keys (`ssh-ed25519 AAAA… comment`).
///
/// `signature` and `signed_content` are the two buffers from
/// `git2::Repository::extract_signature`. Returns `Ok(fingerprint)` of the key
/// that verified, or a fail-closed error.
pub fn verify_commit_signature(
    signature: &[u8],
    signed_content: &[u8],
    trusted_keys: &[String],
) -> Result<String, CommitVerifyError> {
    if signature.is_empty() {
        return Err(CommitVerifyError::Unsigned);
    }
    if trusted_keys.is_empty() {
        return Err(CommitVerifyError::NoTrustedKeys);
    }

    // git stores the SSH signature as a PEM-armored SSHSIG blob
    // (-----BEGIN SSH SIGNATURE-----). A GPG signature ("-----BEGIN PGP…")
    // parses as neither and lands in UnsupportedSignature.
    let sig = SshSig::from_pem(signature).map_err(|_| CommitVerifyError::UnsupportedSignature)?;

    // At least one configured key must parse, or we can't make a trust
    // decision — treat an all-unparseable key set as fail-closed.
    let mut any_parsed = false;
    let mut last_parse_err: Option<String> = None;
    for key_str in trusted_keys {
        let key = match PublicKey::from_openssh(key_str.trim()) {
            Ok(k) => k,
            Err(e) => {
                last_parse_err = Some(e.to_string());
                continue;
            }
        };
        any_parsed = true;

        // git signs with sha512; verify() checks the namespace and message.
        if key
            .verify(GIT_NAMESPACE, signed_content, &sig)
            .or_else(|_| verify_with_alg(&key, signed_content, &sig))
            .is_ok()
        {
            return Ok(key.fingerprint(HashAlg::Sha256).to_string());
        }
    }

    if !any_parsed {
        return Err(CommitVerifyError::BadTrustedKey(
            last_parse_err.unwrap_or_else(|| "no parseable trusted keys".to_string()),
        ));
    }
    Err(CommitVerifyError::Untrusted)
}

/// `PublicKey::verify` already derives the hash from the signature, but be
/// explicit for older signatures: retry the message as-is (no-op hook kept so
/// the call site reads clearly and future hash quirks have a home).
fn verify_with_alg(key: &PublicKey, msg: &[u8], sig: &SshSig) -> Result<(), ssh_key::Error> {
    key.verify(GIT_NAMESPACE, msg, sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

    /// Sign `msg` under the git namespace with a fresh ed25519 key; return
    /// (armored SSHSIG, authorized_keys public line).
    fn sign(msg: &[u8]) -> (String, String) {
        // Deterministic key from a fixed seed so the test needs no RNG.
        let key = PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[7u8; 32]));
        let sig = key
            .sign(GIT_NAMESPACE, HashAlg::Sha512, msg)
            .unwrap()
            .to_pem(LineEnding::LF)
            .unwrap();
        let pubkey = key.public_key().to_openssh().unwrap();
        (sig, pubkey)
    }

    #[test]
    fn valid_signature_by_trusted_key_verifies() {
        let msg = b"tree abc\nauthor a\n\ncommit message\n";
        let (sig, pubkey) = sign(msg);
        let fp = verify_commit_signature(sig.as_bytes(), msg, &[pubkey]).unwrap();
        assert!(fp.starts_with("SHA256:"));
    }

    #[test]
    fn unsigned_commit_fails_closed() {
        let err = verify_commit_signature(b"", b"content", &["ssh-ed25519 AAAA".to_string()])
            .unwrap_err();
        assert_eq!(err, CommitVerifyError::Unsigned);
    }

    #[test]
    fn no_trusted_keys_fails_closed() {
        let msg = b"content";
        let (sig, _pubkey) = sign(msg);
        let err = verify_commit_signature(sig.as_bytes(), msg, &[]).unwrap_err();
        assert_eq!(err, CommitVerifyError::NoTrustedKeys);
    }

    #[test]
    fn signature_by_untrusted_key_is_rejected() {
        let msg = b"content";
        let (sig, _signer_pub) = sign(msg);
        // A DIFFERENT key is the only trusted one.
        let other = PrivateKey::random(&mut rand::thread_rng(), Algorithm::Ed25519).unwrap();
        let other_pub = other.public_key().to_openssh().unwrap();
        let err = verify_commit_signature(sig.as_bytes(), msg, &[other_pub]).unwrap_err();
        assert_eq!(err, CommitVerifyError::Untrusted);
    }

    #[test]
    fn tampered_message_is_rejected() {
        let msg = b"original content";
        let (sig, pubkey) = sign(msg);
        let err =
            verify_commit_signature(sig.as_bytes(), b"tampered content", &[pubkey]).unwrap_err();
        assert_eq!(err, CommitVerifyError::Untrusted);
    }

    #[test]
    fn gpg_signature_is_unsupported_not_accepted() {
        let gpg = b"-----BEGIN PGP SIGNATURE-----\n\nfake\n-----END PGP SIGNATURE-----\n";
        let err = verify_commit_signature(gpg, b"content", &["ssh-ed25519 AAAA".to_string()])
            .unwrap_err();
        assert_eq!(err, CommitVerifyError::UnsupportedSignature);
    }

    #[test]
    fn unparseable_trusted_keys_fail_closed() {
        let msg = b"content";
        let (sig, _pubkey) = sign(msg);
        let err =
            verify_commit_signature(sig.as_bytes(), msg, &["not-a-key".to_string()]).unwrap_err();
        assert!(matches!(err, CommitVerifyError::BadTrustedKey(_)));
    }
}
