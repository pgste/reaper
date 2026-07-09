//! Bundle compilation configuration

use serde::{Deserialize, Serialize};

/// How promotions (and rollbacks) are governed.
///
/// The *code* default is single-control, so a lone operator, a CI service
/// account, and the OPA-style sidecar/engine deployments keep working out of
/// the box (a promote requires only `bundle:promote`). The posture is chosen
/// per **deployment profile**: the managed control plane (Helm chart / docker
/// `management` profile) ships `dual_control` on, so enterprises get four-eyes
/// by default and opt *out*, while lightweight deployments opt *in*. Either
/// way every promotion is written to an immutable change record + the audit
/// log — only the approval gate varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionApproval {
    /// Single-control (default): the caller with `bundle:promote` promotes
    /// immediately. A change record is still written for the audit trail.
    #[default]
    Disabled,
    /// Dual-control (two-person / four-eyes): promote/rollback open a *pending*
    /// change request that a different principal must approve to execute.
    DualControl,
}

impl PromotionApproval {
    /// Parse a config/env string (accepts a couple of common spellings).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disabled" | "single" | "single_control" | "off" | "none" => Some(Self::Disabled),
            "dual_control" | "dual" | "two_person" | "four_eyes" | "on" => Some(Self::DualControl),
            _ => None,
        }
    }

    pub fn is_dual_control(self) -> bool {
        matches!(self, Self::DualControl)
    }
}

/// Bundle compilation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundlesConfig {
    #[serde(default)]
    pub auto_compile_on_source_sync: bool,
    #[serde(default = "default_require_staged")]
    pub require_staged_before_promote: bool,

    /// Promotion governance. Single-control by default; set to `dual_control`
    /// to require two-person approval before a bundle is promoted/rolled back.
    #[serde(default)]
    pub promotion_approval: PromotionApproval,

    /// In dual-control, whether a principal may approve its **own** request.
    ///
    /// Default `false` — strict separation of duties (the requester and the
    /// approver must be different identities). Set `true` only for fully
    /// automated pipelines where the real approval gate lives outside Reaper
    /// (a CI/CD promotion job, a ServiceNow change record) and a single service
    /// account both opens and executes the change. Leaving this off is what
    /// makes the four-eyes guarantee meaningful.
    #[serde(default)]
    pub allow_self_approval: bool,

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
            promotion_approval: PromotionApproval::default(),
            allow_self_approval: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_defaults_to_single_control() {
        // The shipping default is single-control; dual-control is turned on per
        // deployment profile (Helm / docker `management` profile).
        let cfg = BundlesConfig::default();
        assert_eq!(cfg.promotion_approval, PromotionApproval::Disabled);
        assert!(!cfg.promotion_approval.is_dual_control());
        assert!(!cfg.allow_self_approval);
    }

    #[test]
    fn promotion_approval_parses_common_spellings() {
        for s in [
            "dual_control",
            "dual",
            "two_person",
            "four_eyes",
            "on",
            "  DUAL  ",
        ] {
            assert_eq!(
                PromotionApproval::parse(s),
                Some(PromotionApproval::DualControl),
                "{s:?} should parse to dual-control"
            );
        }
        for s in ["disabled", "single", "off", "none"] {
            assert_eq!(
                PromotionApproval::parse(s),
                Some(PromotionApproval::Disabled)
            );
        }
        assert_eq!(PromotionApproval::parse("nonsense"), None);
    }
}
