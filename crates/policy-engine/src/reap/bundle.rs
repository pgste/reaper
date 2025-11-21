//! Binary Bundle Format (.rbb)
//!
//! Compiles .reap policies into optimized binary bundles for instant loading.

use super::ast::Policy;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Bundle format metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFormat {
    /// Format version
    pub version: u32,
    /// Compilation timestamp
    pub compiled_at: u64,
    /// Original policy name
    pub policy_name: String,
    /// Policy version (if any)
    pub policy_version: Option<String>,
    /// Checksum of source
    pub source_checksum: u64,
}

/// Complete policy bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub metadata: BundleFormat,
    pub policy: Policy,
}

impl PolicyBundle {
    const MAGIC_BYTES: &'static [u8; 4] = b"REAP";
    const FORMAT_VERSION: u32 = 1;

    /// Create a new bundle from a policy
    pub fn new(policy: Policy) -> Self {
        let policy_version = policy.metadata.get("version").cloned();
        let source_checksum = calculate_checksum(&policy);

        let metadata = BundleFormat {
            version: Self::FORMAT_VERSION,
            compiled_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            policy_name: policy.name.clone(),
            policy_version,
            source_checksum,
        };

        Self { metadata, policy }
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();

        // Magic bytes
        bytes.extend_from_slice(Self::MAGIC_BYTES);

        // Serialize bundle
        let bundle_bytes = bincode::serialize(self).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;

        bytes.extend_from_slice(&bundle_bytes);

        Ok(bytes)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        // Check magic bytes
        if bytes.len() < 4 || &bytes[0..4] != Self::MAGIC_BYTES {
            return Err(ReaperError::InvalidPolicy {
                reason: "Invalid bundle format: magic bytes mismatch".to_string(),
            });
        }

        // Deserialize
        let bundle: Self = bincode::deserialize(&bytes[4..]).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize bundle: {}", e),
            }
        })?;

        // Version check
        if bundle.metadata.version > Self::FORMAT_VERSION {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Bundle version {} is newer than supported version {}",
                    bundle.metadata.version,
                    Self::FORMAT_VERSION
                ),
            });
        }

        Ok(bundle)
    }
}

/// Compile a policy AST to a binary bundle
pub fn compile_to_bundle(policy: &Policy) -> Result<Vec<u8>, ReaperError> {
    let bundle = PolicyBundle::new(policy.clone());
    bundle.to_bytes()
}

/// Load a policy AST from a binary bundle
pub fn load_from_bundle(bytes: &[u8]) -> Result<Policy, ReaperError> {
    let bundle = PolicyBundle::from_bytes(bytes)?;
    Ok(bundle.policy)
}

/// Calculate a simple checksum for a policy
fn calculate_checksum(policy: &Policy) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    policy.name.hash(&mut hasher);
    policy.rules.len().hash(&mut hasher);

    for rule in &policy.rules {
        rule.name.hash(&mut hasher);
    }

    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ast::*;
    use std::collections::HashMap;

    #[test]
    fn test_bundle_roundtrip() {
        let policy = Policy {
            name: "test".to_string(),
            metadata: HashMap::new(),
            default_decision: Decision::Deny,
            rules: vec![Rule {
                name: "admin".to_string(),
                decision: Decision::Allow,
                condition: Condition::True,
            }],
        };

        // Compile to bundle
        let bytes = compile_to_bundle(&policy).unwrap();

        // Load from bundle
        let loaded = load_from_bundle(&bytes).unwrap();

        assert_eq!(loaded.name, policy.name);
        assert_eq!(loaded.rules.len(), policy.rules.len());
    }

    #[test]
    fn test_bundle_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("version".to_string(), "1.0.0".to_string());

        let policy = Policy {
            name: "versioned".to_string(),
            metadata,
            default_decision: Decision::Allow,
            rules: vec![],
        };

        let bundle = PolicyBundle::new(policy);

        assert_eq!(bundle.metadata.policy_version, Some("1.0.0".to_string()));
        assert_eq!(bundle.metadata.policy_name, "versioned");
    }
}
