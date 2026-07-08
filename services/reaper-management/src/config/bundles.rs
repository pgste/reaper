//! Bundle compilation configuration

use serde::{Deserialize, Serialize};

/// Bundle compilation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundlesConfig {
    #[serde(default)]
    pub auto_compile_on_source_sync: bool,
    #[serde(default = "default_require_staged")]
    pub require_staged_before_promote: bool,

    /// Private signing key (lowercase hex) used to sign every compiled bundle at
    /// creation time. When set, the signature is stored next to the bundle (a
    /// `<key>.sig` sidecar object) so it travels with the bundle to any store
    /// (S3, filesystem) and is served to agents for verification. Ed25519:
    /// 32-byte seed; ECDSA P-256: 32-byte scalar.
    #[serde(default)]
    pub signing_key: Option<String>,

    /// Identifier advertised in each signature envelope (for key rotation).
    #[serde(default = "default_signing_key_id")]
    pub signing_key_id: String,

    /// Signature algorithm: `ed25519-sha256` (default) or `ecdsa-p256-sha256`.
    #[serde(default = "default_signing_algorithm")]
    pub signing_algorithm: String,

    /// How long a signature envelope stays valid after signing, in days
    /// (v2 envelopes carry an authenticated `expires_at`). A bundle older
    /// than this must be recompiled (re-signed) before agents will load it.
    #[serde(default = "default_signature_validity_days")]
    pub signature_validity_days: u64,
}

impl Default for BundlesConfig {
    fn default() -> Self {
        Self {
            auto_compile_on_source_sync: false,
            require_staged_before_promote: true,
            signing_key: None,
            signing_key_id: default_signing_key_id(),
            signing_algorithm: default_signing_algorithm(),
            signature_validity_days: default_signature_validity_days(),
        }
    }
}

fn default_signature_validity_days() -> u64 {
    365
}

fn default_require_staged() -> bool {
    true
}

fn default_signing_key_id() -> String {
    "default".to_string()
}

fn default_signing_algorithm() -> String {
    reaper_core::bundle_signing::ALGORITHM.to_string()
}
