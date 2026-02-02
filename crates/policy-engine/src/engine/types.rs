//! Type definitions for the Policy Engine.
//!
//! This module contains all the core types used throughout the policy engine:
//! - PolicyAction, PolicyLanguage, PolicySource
//! - PolicyRequest, PolicyDecision, PolicyVersion
//! - Package evaluation result types
//! - Staging types for two-phase commit

use reaper_core::PolicyId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

/// Policy action types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
    Log,
}

/// Policy version tracking for bundle deployments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersion {
    /// Semantic version string (e.g., "1.2.3")
    pub version: String,
    /// When this version was deployed
    pub deployed_at: SystemTime,
    /// SHA-256 hash of the bundle for integrity verification
    pub bundle_hash: [u8; 32],
    /// Policy identifier this version belongs to
    pub policy_id: String,
}

/// Supported policy languages
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyLanguage {
    /// Simple rule-based policies (sub-microsecond evaluation)
    Simple,
    /// AWS Cedar policy language (rich ABAC, schema validation)
    Cedar,
    /// Future: Custom Reaper DSL (compile-time optimization)
    #[serde(rename = "reaper")]
    Custom,
}

impl std::fmt::Display for PolicyLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyLanguage::Simple => write!(f, "simple"),
            PolicyLanguage::Cedar => write!(f, "cedar"),
            PolicyLanguage::Custom => write!(f, "custom"),
        }
    }
}

/// Policy source - where the policy was loaded from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicySource {
    /// Loaded from local file on startup
    File { path: String },
    /// Deployed via direct API call
    Api { client_id: Option<String> },
    /// Synchronized from management server
    SyncClient {
        server_url: String,
        server_version: String,
        team: Option<String>,
    },
    /// Default policy created by system
    Default,
}

impl std::fmt::Display for PolicySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicySource::File { path } => write!(f, "file:{}", path),
            PolicySource::Api { client_id } => {
                if let Some(id) = client_id {
                    write!(f, "api:{}", id)
                } else {
                    write!(f, "api")
                }
            }
            PolicySource::SyncClient { server_url, .. } => write!(f, "sync:{}", server_url),
            PolicySource::Default => write!(f, "default"),
        }
    }
}

/// Metadata about how/when a policy was deployed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySourceMetadata {
    /// Where the policy came from
    pub source: PolicySource,
    /// When the policy was deployed to this agent
    pub deployed_at: chrono::DateTime<chrono::Utc>,
    /// Who/what deployed the policy
    pub deployed_by: Option<String>,
    /// Version from the source (server version, file mtime, etc.)
    pub source_version: Option<String>,
    /// SHA-256 checksum of the policy content
    pub checksum: Option<String>,
}

impl PolicySourceMetadata {
    /// Create metadata for a file-based policy
    pub fn from_file(path: impl Into<String>) -> Self {
        Self {
            source: PolicySource::File { path: path.into() },
            deployed_at: chrono::Utc::now(),
            deployed_by: None,
            source_version: None,
            checksum: None,
        }
    }

    /// Create metadata for an API-deployed policy
    pub fn from_api(client_id: Option<String>) -> Self {
        Self {
            source: PolicySource::Api { client_id },
            deployed_at: chrono::Utc::now(),
            deployed_by: None,
            source_version: None,
            checksum: None,
        }
    }

    /// Create metadata for a sync client deployment
    pub fn from_sync_client(
        server_url: impl Into<String>,
        server_version: impl Into<String>,
        team: Option<String>,
    ) -> Self {
        Self {
            source: PolicySource::SyncClient {
                server_url: server_url.into(),
                server_version: server_version.into(),
                team,
            },
            deployed_at: chrono::Utc::now(),
            deployed_by: Some("sync-client".to_string()),
            source_version: None,
            checksum: None,
        }
    }

    /// Create metadata for a default policy
    pub fn default_policy() -> Self {
        Self {
            source: PolicySource::Default,
            deployed_at: chrono::Utc::now(),
            deployed_by: Some("system".to_string()),
            source_version: None,
            checksum: None,
        }
    }

    /// Set the deployed_by field
    pub fn with_deployed_by(mut self, deployed_by: impl Into<String>) -> Self {
        self.deployed_by = Some(deployed_by.into());
        self
    }

    /// Set the source version
    pub fn with_source_version(mut self, version: impl Into<String>) -> Self {
        self.source_version = Some(version.into());
        self
    }

    /// Set the checksum
    pub fn with_checksum(mut self, checksum: impl Into<String>) -> Self {
        self.checksum = Some(checksum.into());
        self
    }

    /// Calculate and set checksum from content
    pub fn compute_checksum(&mut self, content: &str) {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let result = hasher.finalize();
        self.checksum = Some(hex::encode(result));
    }
}

impl Default for PolicySourceMetadata {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// Default priority for policies (lower = higher priority)
pub fn default_priority() -> u32 {
    1000
}

/// Policy rule definition - used for Simple language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub action: PolicyAction,
    pub resource: String,
    pub conditions: Vec<String>,
}

/// Policy evaluation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub resource: String,
    pub action: String,
    pub context: std::collections::HashMap<String, String>,
}

/// Policy evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision: PolicyAction,
    pub policy_id: PolicyId,
    pub policy_version: u64,
    pub evaluation_time_ns: u64,
    pub matched_rule: Option<usize>,
}

/// Information about a denial (which policy denied the request)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenyInfo {
    /// ID of the policy that denied the request
    pub policy_id: PolicyId,
    /// Name of the policy that denied the request
    pub policy_name: String,
    /// Package the policy belongs to
    pub package: String,
    /// Which rule matched (if available)
    pub matched_rule: Option<String>,
}

/// Result of evaluating all policies in a package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEvaluationResult {
    /// Package that was evaluated
    pub package: String,
    /// Overall decision (DENY if any policy denies)
    pub decision: PolicyAction,
    /// Details about which policy denied (if denied)
    pub denied_by: Option<DenyInfo>,
    /// Number of policies evaluated before a decision was reached
    pub policies_evaluated: usize,
    /// Total evaluation time in nanoseconds
    pub total_evaluation_time_ns: u64,
    /// Individual policy results (only for allowed - stops at first deny)
    pub results: Vec<PolicyDecision>,
}

/// Result of evaluating ALL policies across ALL packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllPoliciesEvaluationResult {
    /// Overall decision (DENY if any policy denies)
    pub decision: PolicyAction,
    /// Details about which policy denied (if denied)
    pub denied_by: Option<DenyInfo>,
    /// Number of policies evaluated
    pub policies_evaluated: usize,
    /// Number of packages evaluated
    pub packages_evaluated: usize,
    /// Total evaluation time in nanoseconds
    pub total_evaluation_time_ns: u64,
}

/// Result of a staged package operation
#[derive(Debug, Clone)]
pub struct StagedPackage {
    /// Unique ID for this staged package
    pub staging_id: uuid::Uuid,
    /// Policy IDs that were successfully staged
    pub staged_policy_ids: Vec<PolicyId>,
    /// Policy names that were staged
    pub staged_policy_names: Vec<String>,
    /// Validation errors (if any) - empty means all valid
    pub validation_errors: Vec<String>,
    /// Timestamp when staging started
    pub staged_at: chrono::DateTime<chrono::Utc>,
}

impl StagedPackage {
    /// Check if the staged package is valid (no validation errors)
    pub fn is_valid(&self) -> bool {
        self.validation_errors.is_empty()
    }
}

/// Information about a policy package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Package name
    pub name: String,
    /// Number of policies in the package
    pub policy_count: usize,
    /// List of policy names in this package
    pub policy_names: Vec<String>,
}

/// Engine statistics for monitoring
#[derive(Debug, Serialize, Deserialize)]
pub struct PolicyEngineStats {
    pub total_policies: usize,
    pub has_default_policy: bool,
}

// Legacy simple types for backward compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimpleAction {
    Allow,
    Deny,
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleRule {
    pub action: SimpleAction,
    pub resource: String,
    pub conditions: Vec<String>,
}
