//! Attenuated, short-lived capabilities for non-human / agentic actors
//! (Workstream F1 slice 1, `plans/round-2/F1-agentic-authz.md`).
//!
//! A capability is a signed, expiring grant: *"actor A may perform this
//! SUBSET of what subject S can do, until T"*. It is a derived, attenuated,
//! expiring principal — not a durable identity. Design decisions (locked):
//!
//! - **Homegrown envelope on the existing crypto**: signed with the same
//!   [`SigningKey`]/[`VerifyingKey`] machinery as bundle signatures
//!   (Ed25519 / ECDSA-P256, algorithm is a value not a hardcode). No new
//!   dependencies.
//! - **Attenuation is issuer-side re-issuance**: [`attenuate`] produces a
//!   NEW capability whose grants must be a strict subset of the parent's and
//!   whose validity window must nest inside it — enforced here, not by
//!   convention. Every attenuation records its ancestry, so revoking any
//!   ancestor kills the whole derivation chain.
//! - **Verification is pure and clock-explicit**: [`Capability::verify_at`]
//!   takes `now_unix` and the revoked-id set as inputs — no I/O, no ambient
//!   clock, wasm-safe. The engine never does crypto at eval time; the
//!   enforcing edge (agent / MCP gate) verifies pre-eval and injects
//!   verified facts.
//!
//! The signed message is a domain-separated, length-prefixed canonical
//! encoding of every claim (fields may contain arbitrary bytes, so
//! delimiter-based encodings are ambiguous; length prefixes are not).

use crate::bundle_signing::{SigAlgorithm, SignatureError, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

/// Capability envelope version.
pub const CAPABILITY_V1: u32 = 1;

/// One permitted (action, resource) pattern pair. Patterns are literal
/// strings, `*` (anything), or a prefix followed by a trailing `*`
/// (`"doc/*"`). Matching and subset semantics live in [`pattern_matches`]
/// and [`pattern_covers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub action: String,
    pub resource: String,
}

impl Grant {
    pub fn new(action: impl Into<String>, resource: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            resource: resource.into(),
        }
    }
}

/// A signed, expiring, attenuable capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Envelope version (currently [`CAPABILITY_V1`]).
    pub v: u32,
    /// Unique id (revocation handle).
    pub id: String,
    /// Signature algorithm (`SigAlgorithm::as_str`).
    pub algorithm: String,
    /// Identifies the issuing key (rotation / pinning).
    pub key_id: String,
    /// The human/durable principal this capability derives from.
    pub subject: String,
    /// The non-human actor allowed to wield it.
    pub actor: String,
    /// What the actor may do — a subset of the subject's authority, and on
    /// attenuation a subset of the parent capability's grants.
    pub grants: Vec<Grant>,
    /// Validity window (unix seconds, inclusive bounds).
    pub not_before: i64,
    pub expires_at: i64,
    /// Ancestor capability ids, root first. Revoking ANY ancestor revokes
    /// this capability (checked in [`Capability::verify_at`]).
    #[serde(default)]
    pub ancestry: Vec<String>,
    /// Hex signature over the canonical claims.
    pub signature: String,
}

/// Why a capability was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityError {
    Expired {
        expires_at: i64,
        now: i64,
    },
    NotYetValid {
        not_before: i64,
        now: i64,
    },
    InvalidWindow,
    BadSignature,
    Revoked {
        id: String,
    },
    UnknownAlgorithm(String),
    KeyMismatch {
        expected: String,
        got: String,
    },
    /// Attenuation attempted to grant something the parent does not cover.
    WidenedGrant {
        grant: String,
    },
    /// Attenuation attempted to extend the parent's validity window.
    WidenedWindow,
    /// Attenuation attempted to change subject or actor lineage rules.
    LineageViolation(String),
    Malformed(String),
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired { expires_at, now } => {
                write!(f, "capability expired at {expires_at} (now {now})")
            }
            Self::NotYetValid { not_before, now } => {
                write!(f, "capability not valid before {not_before} (now {now})")
            }
            Self::InvalidWindow => write!(f, "not_before must be <= expires_at"),
            Self::BadSignature => write!(f, "capability signature verification failed"),
            Self::Revoked { id } => write!(f, "capability (or ancestor) {id} is revoked"),
            Self::UnknownAlgorithm(a) => write!(f, "unknown signature algorithm '{a}'"),
            Self::KeyMismatch { expected, got } => {
                write!(
                    f,
                    "key_id mismatch: capability signed by '{got}', expected '{expected}'"
                )
            }
            Self::WidenedGrant { grant } => {
                write!(
                    f,
                    "attenuation widens authority: parent does not cover {grant}"
                )
            }
            Self::WidenedWindow => write!(f, "attenuation must nest inside the parent window"),
            Self::LineageViolation(s) => write!(f, "lineage violation: {s}"),
            Self::Malformed(s) => write!(f, "malformed capability: {s}"),
        }
    }
}

impl std::error::Error for CapabilityError {}

impl From<SignatureError> for CapabilityError {
    fn from(e: SignatureError) -> Self {
        match e {
            SignatureError::BadSignature => CapabilityError::BadSignature,
            other => CapabilityError::Malformed(other.to_string()),
        }
    }
}

/// Does `pattern` match the concrete `value`? Literal equality, `*`, or a
/// trailing-`*` prefix pattern. (Interior wildcards are NOT supported — a
/// capability grant is deliberately a narrow language.)
pub fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// Does `parent` cover EVERY value `child` could match? This is the
/// subset relation attenuation enforces:
/// - `*` covers everything;
/// - `"p*"` covers `"p<anything>"` and any `"p<suffix>*"`;
/// - a literal covers only the identical literal.
pub fn pattern_covers(parent: &str, child: &str) -> bool {
    if parent == "*" {
        return true;
    }
    match (parent.strip_suffix('*'), child.strip_suffix('*')) {
        (Some(pp), Some(cp)) => cp.starts_with(pp),
        (Some(pp), None) => child.starts_with(pp),
        // A literal parent cannot cover a wildcard child (the child would
        // match values the parent does not).
        (None, Some(_)) => false,
        (None, None) => parent == child,
    }
}

/// One grant covers another iff both its action and resource patterns cover.
fn grant_covered(parent: &[Grant], child: &Grant) -> bool {
    parent.iter().any(|p| {
        pattern_covers(&p.action, &child.action) && pattern_covers(&p.resource, &child.resource)
    })
}

/// Canonical signed message: domain tag + length-prefixed claims. Length
/// prefixes (not delimiters) because subjects/actors/patterns are arbitrary
/// caller-controlled strings; no concatenation of two different claim sets
/// can produce the same bytes.
fn canonical_message(cap: &Capability) -> Vec<u8> {
    let mut msg = Vec::with_capacity(256);
    msg.extend_from_slice(b"reaper-capability-v1\0");
    let mut push = |bytes: &[u8]| {
        msg.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        msg.extend_from_slice(bytes);
    };
    push(&cap.v.to_be_bytes());
    push(cap.id.as_bytes());
    push(cap.algorithm.as_bytes());
    push(cap.key_id.as_bytes());
    push(cap.subject.as_bytes());
    push(cap.actor.as_bytes());
    push(&(cap.grants.len() as u64).to_be_bytes());
    for g in &cap.grants {
        push(g.action.as_bytes());
        push(g.resource.as_bytes());
    }
    push(&cap.not_before.to_be_bytes());
    push(&cap.expires_at.to_be_bytes());
    push(&(cap.ancestry.len() as u64).to_be_bytes());
    for a in &cap.ancestry {
        push(a.as_bytes());
    }
    msg
}

/// Issue a ROOT capability: `actor` may exercise `grants` on behalf of
/// `subject` within `[not_before, expires_at]`.
pub fn issue(
    key: &SigningKey,
    key_id: &str,
    subject: &str,
    actor: &str,
    grants: Vec<Grant>,
    not_before: i64,
    expires_at: i64,
) -> Result<Capability, CapabilityError> {
    if not_before > expires_at {
        return Err(CapabilityError::InvalidWindow);
    }
    let mut cap = Capability {
        v: CAPABILITY_V1,
        id: uuid::Uuid::new_v4().to_string(),
        algorithm: key.algorithm().as_str().to_string(),
        key_id: key_id.to_string(),
        subject: subject.to_string(),
        actor: actor.to_string(),
        grants,
        not_before,
        expires_at,
        ancestry: Vec::new(),
        signature: String::new(),
    };
    cap.signature = hex_encode(&key.sign_raw(&canonical_message(&cap)));
    Ok(cap)
}

/// Attenuate `parent` into a strictly-narrower capability for (possibly) a
/// different actor — the "orchestrator hands a narrowed capability to a
/// sub-agent" flow, executed at the issuer (issuer-side re-issuance was the
/// locked design decision; holders cannot mint).
///
/// Enforced, not advisory:
/// - every new grant must be covered by some parent grant;
/// - the validity window must nest inside the parent's;
/// - the subject is inherited (a capability chain never changes WHO the
///   authority derives from);
/// - ancestry extends the parent's chain, so ancestor revocation cascades.
pub fn attenuate(
    parent: &Capability,
    key: &SigningKey,
    key_id: &str,
    actor: &str,
    grants: Vec<Grant>,
    not_before: i64,
    expires_at: i64,
) -> Result<Capability, CapabilityError> {
    if not_before > expires_at {
        return Err(CapabilityError::InvalidWindow);
    }
    if not_before < parent.not_before || expires_at > parent.expires_at {
        return Err(CapabilityError::WidenedWindow);
    }
    for g in &grants {
        if !grant_covered(&parent.grants, g) {
            return Err(CapabilityError::WidenedGrant {
                grant: format!("({}, {})", g.action, g.resource),
            });
        }
    }
    let mut ancestry = parent.ancestry.clone();
    ancestry.push(parent.id.clone());

    let mut cap = Capability {
        v: CAPABILITY_V1,
        id: uuid::Uuid::new_v4().to_string(),
        algorithm: key.algorithm().as_str().to_string(),
        key_id: key_id.to_string(),
        subject: parent.subject.clone(),
        actor: actor.to_string(),
        grants,
        not_before,
        expires_at,
        ancestry,
        signature: String::new(),
    };
    cap.signature = hex_encode(&key.sign_raw(&canonical_message(&cap)));
    Ok(cap)
}

impl Capability {
    /// Verify this capability at an explicit instant against an explicit
    /// revocation set. Pure (no clock, no I/O — wasm-safe); the enforcing
    /// edge supplies `now_unix` and the current revoked-id set.
    ///
    /// Checks, fail-closed and in order: envelope version, key-id pin,
    /// algorithm, signature over every claim, validity window, and
    /// revocation of this id or ANY ancestor.
    pub fn verify_at(
        &self,
        verifying_key: &VerifyingKey,
        expected_key_id: &str,
        now_unix: i64,
        revoked_ids: &HashSet<String>,
    ) -> Result<(), CapabilityError> {
        if self.v != CAPABILITY_V1 {
            return Err(CapabilityError::Malformed(format!(
                "unsupported capability version {}",
                self.v
            )));
        }
        if self.key_id != expected_key_id {
            return Err(CapabilityError::KeyMismatch {
                expected: expected_key_id.to_string(),
                got: self.key_id.clone(),
            });
        }
        let alg = SigAlgorithm::parse(&self.algorithm)
            .map_err(|_| CapabilityError::UnknownAlgorithm(self.algorithm.clone()))?;
        if alg != verifying_key.algorithm() {
            return Err(CapabilityError::UnknownAlgorithm(format!(
                "capability algorithm {} does not match verifying key {}",
                self.algorithm,
                verifying_key.algorithm().as_str()
            )));
        }
        let sig = hex_decode(&self.signature)
            .ok_or_else(|| CapabilityError::Malformed("signature is not valid hex".into()))?;
        verifying_key.verify_raw(&canonical_message(self), &sig)?;

        if self.not_before > self.expires_at {
            return Err(CapabilityError::InvalidWindow);
        }
        if now_unix < self.not_before {
            return Err(CapabilityError::NotYetValid {
                not_before: self.not_before,
                now: now_unix,
            });
        }
        if now_unix > self.expires_at {
            return Err(CapabilityError::Expired {
                expires_at: self.expires_at,
                now: now_unix,
            });
        }
        if revoked_ids.contains(&self.id) {
            return Err(CapabilityError::Revoked {
                id: self.id.clone(),
            });
        }
        for ancestor in &self.ancestry {
            if revoked_ids.contains(ancestor) {
                return Err(CapabilityError::Revoked {
                    id: ancestor.clone(),
                });
            }
        }
        Ok(())
    }

    /// Does this (already-verified) capability authorize `action` on
    /// `resource`? Pure pattern matching; an empty grant list authorizes
    /// nothing.
    pub fn authorizes(&self, action: &str, resource: &str) -> bool {
        self.grants
            .iter()
            .any(|g| pattern_matches(&g.action, action) && pattern_matches(&g.resource, resource))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use fmt::Write;
        // Writing to a String cannot fail.
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
