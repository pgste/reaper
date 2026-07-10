//! Bundle compilation logic
//!
//! Compiles policy collections into binary bundle format (.rbb).

use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, info};

use crate::domain::bundle::BundlePolicy;
use crate::domain::policy::PolicyVersion;

/// Compilation errors
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("No policies to compile")]
    NoPolicies,
    #[error("Policy validation failed: {0}")]
    ValidationFailed(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Policy not found: {0}")]
    PolicyNotFound(String),
}

/// Compiled bundle output
#[derive(Debug, Clone)]
pub struct CompiledBundle {
    /// Bundle binary data
    pub data: Vec<u8>,
    /// SHA-256 checksum
    pub checksum: String,
    /// Number of policies included
    pub policy_count: i32,
    /// Compilation warnings
    pub warnings: Vec<String>,
}

/// Bundle compiler
pub struct BundleCompiler {
    /// Whether to include debug info
    include_debug: bool,
}

impl BundleCompiler {
    /// Create a new bundle compiler
    pub fn new() -> Self {
        Self {
            include_debug: false,
        }
    }

    /// Create a compiler with debug info enabled
    pub fn with_debug(mut self, include_debug: bool) -> Self {
        self.include_debug = include_debug;
        self
    }

    /// Compile policies into a bundle
    pub fn compile(
        &self,
        bundle_policies: &[BundlePolicy],
        policy_versions: &[PolicyVersion],
    ) -> Result<CompiledBundle, CompileError> {
        if bundle_policies.is_empty() {
            return Err(CompileError::NoPolicies);
        }

        let mut warnings = Vec::new();

        // Build the bundle content
        let mut bundle_content = BundleContent {
            version: 1,
            format: "rbb".to_string(),
            policies: Vec::new(),
            metadata: BundleMetadata {
                created_at: chrono::Utc::now().to_rfc3339(),
                policy_count: bundle_policies.len() as i32,
                include_debug: self.include_debug,
            },
        };

        // Process each policy in priority order
        let mut sorted_policies = bundle_policies.to_vec();
        sorted_policies.sort_by_key(|p| p.priority);

        for bp in &sorted_policies {
            // Find the corresponding policy version
            let policy_version = policy_versions
                .iter()
                .find(|pv| pv.policy_id == bp.policy_id && pv.version == bp.policy_version)
                .ok_or_else(|| {
                    CompileError::PolicyNotFound(format!(
                        "Policy {} version {}",
                        bp.policy_id, bp.policy_version
                    ))
                })?;

            // Validate the policy content
            if let Err(validation_error) = self.validate_policy(&policy_version.content) {
                warnings.push(format!(
                    "Policy {} validation warning: {}",
                    bp.policy_id, validation_error
                ));
            }

            bundle_content.policies.push(BundledPolicy {
                id: bp.policy_id.to_string(),
                version: bp.policy_version,
                priority: bp.priority,
                content: policy_version.content.clone(),
                content_hash: policy_version.content_hash.clone(),
                language: "reaper".to_string(), // Could be enhanced to detect language
            });
        }

        // Serialize to binary format (JSON for now, could be MessagePack/CBOR for production)
        let data = serde_json::to_vec(&bundle_content)
            .map_err(|e| CompileError::Serialization(e.to_string()))?;

        // Calculate checksum
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let checksum = format!("{:x}", hasher.finalize());

        info!(
            policies = bundle_content.policies.len(),
            size_bytes = data.len(),
            checksum = %checksum,
            "Bundle compiled successfully"
        );

        Ok(CompiledBundle {
            data,
            checksum,
            policy_count: bundle_content.policies.len() as i32,
            warnings,
        })
    }

    /// Validate policy content
    fn validate_policy(&self, content: &str) -> Result<(), String> {
        // Basic validation - check for non-empty content
        if content.trim().is_empty() {
            return Err("Empty policy content".to_string());
        }

        // Could add more sophisticated validation here:
        // - Syntax checking for policy language
        // - Security analysis
        // - Dependency resolution

        debug!(content_len = content.len(), "Policy validated");
        Ok(())
    }
}

impl Default for BundleCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// The compiled `.rbb` artifact schema — what `compile()` serializes and what
/// agents (and the control plane's replay engine) parse back. Public because
/// it IS the wire format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleContent {
    pub version: i32,
    pub format: String,
    pub policies: Vec<BundledPolicy>,
    pub metadata: BundleMetadata,
}

/// One policy inside a compiled bundle: raw policy text + provenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundledPolicy {
    pub id: String,
    pub version: i32,
    pub priority: i32,
    pub content: String,
    pub content_hash: String,
    pub language: String,
}

/// Bundle metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleMetadata {
    pub created_at: String,
    pub policy_count: i32,
    pub include_debug: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_policy_version(policy_id: Uuid, version: i32, content: &str) -> PolicyVersion {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        PolicyVersion {
            id: Uuid::new_v4(),
            policy_id,
            version,
            content: content.to_string(),
            content_hash,
            source_commit: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_compile_empty_policies() {
        let compiler = BundleCompiler::new();
        let result = compiler.compile(&[], &[]);
        assert!(matches!(result, Err(CompileError::NoPolicies)));
    }

    #[test]
    fn test_compile_single_policy() {
        let compiler = BundleCompiler::new();
        let policy_id = Uuid::new_v4();
        let content = "allow admin to access /admin";

        let bundle_policy = BundlePolicy {
            bundle_id: Uuid::new_v4(),
            policy_id,
            policy_version: 1,
            priority: 0,
        };

        let policy_version = create_test_policy_version(policy_id, 1, content);

        let result = compiler.compile(&[bundle_policy], &[policy_version]);
        assert!(result.is_ok());

        let compiled = result.unwrap();
        assert_eq!(compiled.policy_count, 1);
        assert!(!compiled.checksum.is_empty());
        assert!(!compiled.data.is_empty());
    }

    #[test]
    fn test_compile_multiple_policies() {
        let compiler = BundleCompiler::new();

        let policy1_id = Uuid::new_v4();
        let policy2_id = Uuid::new_v4();

        let bundle_policies = vec![
            BundlePolicy {
                bundle_id: Uuid::new_v4(),
                policy_id: policy1_id,
                policy_version: 1,
                priority: 1,
            },
            BundlePolicy {
                bundle_id: Uuid::new_v4(),
                policy_id: policy2_id,
                policy_version: 1,
                priority: 0, // Higher priority (lower number)
            },
        ];

        let policy_versions = vec![
            create_test_policy_version(policy1_id, 1, "policy 1 content"),
            create_test_policy_version(policy2_id, 1, "policy 2 content"),
        ];

        let result = compiler.compile(&bundle_policies, &policy_versions);
        assert!(result.is_ok());

        let compiled = result.unwrap();
        assert_eq!(compiled.policy_count, 2);
    }

    #[test]
    fn test_compile_missing_policy_version() {
        let compiler = BundleCompiler::new();
        let policy_id = Uuid::new_v4();

        let bundle_policy = BundlePolicy {
            bundle_id: Uuid::new_v4(),
            policy_id,
            policy_version: 1,
            priority: 0,
        };

        // No policy versions provided
        let result = compiler.compile(&[bundle_policy], &[]);
        assert!(matches!(result, Err(CompileError::PolicyNotFound(_))));
    }
}
