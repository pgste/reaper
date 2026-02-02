//! Enhanced Policy with multi-language support.
//!
//! The EnhancedPolicy struct is the core policy representation that supports
//! multiple policy languages (Simple, Cedar, Custom) and provides methods
//! for building evaluators, managing compilation, and tracking source metadata.

use super::types::{
    default_priority, PolicyLanguage, PolicyRule, PolicySource, PolicySourceMetadata,
};
use crate::evaluators::{CedarPolicyEvaluator, PolicyEvaluator, SimplePolicyEvaluator};
use reaper_core::{PolicyId, ReaperError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Enhanced Policy with multi-language support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedPolicy {
    pub id: PolicyId,
    pub version: u64,
    pub name: String,
    pub description: String,
    pub language: PolicyLanguage,

    /// Policy content based on language
    /// For Simple: serialized rules
    /// For Cedar: policy text
    /// For Custom: future custom format
    pub content: String,

    /// Legacy field for backward compatibility with Simple policies
    #[serde(default)]
    pub rules: Vec<PolicyRule>,

    /// Optional metadata for optimization hints
    #[serde(default)]
    pub metadata: HashMap<String, String>,

    /// Policy priority (lower number = higher priority, default = 1000)
    #[serde(default = "default_priority")]
    pub priority: u32,

    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// Source tracking - where/how the policy was deployed
    #[serde(default)]
    pub source_metadata: Option<PolicySourceMetadata>,

    /// Cached evaluator (not serialized)
    #[serde(skip)]
    pub evaluator: Option<Arc<dyn PolicyEvaluator>>,
}

impl EnhancedPolicy {
    /// Create a new policy with Simple language (backward compatible)
    pub fn new(name: String, description: String, rules: Vec<PolicyRule>) -> Self {
        let content = serde_json::to_string(&rules).unwrap_or_default();
        let now = chrono::Utc::now();

        let evaluator =
            Arc::new(SimplePolicyEvaluator::new(rules.clone())) as Arc<dyn PolicyEvaluator>;

        Self {
            id: Uuid::new_v4(),
            version: 1,
            name,
            description,
            language: PolicyLanguage::Simple,
            content,
            rules,
            metadata: HashMap::new(),
            priority: default_priority(),
            created_at: now,
            updated_at: now,
            evaluator: Some(evaluator),
            source_metadata: None,
        }
    }

    /// Create a new policy with tree optimization enabled
    ///
    /// Recommended for policies with 100+ rules.
    /// Provides 10-600x faster evaluation at the cost of 1-10ms compilation time.
    pub fn new_with_tree_optimization(
        name: String,
        description: String,
        rules: Vec<PolicyRule>,
    ) -> Result<Self> {
        let content = serde_json::to_string(&rules).unwrap_or_default();
        let now = chrono::Utc::now();

        let evaluator = Arc::new(SimplePolicyEvaluator::with_tree_optimization(
            rules.clone(),
        )?) as Arc<dyn PolicyEvaluator>;

        let mut metadata = HashMap::new();
        metadata.insert("optimization".to_string(), "tree".to_string());

        Ok(Self {
            id: Uuid::new_v4(),
            version: 1,
            name,
            description,
            language: PolicyLanguage::Simple,
            content,
            rules,
            metadata,
            priority: default_priority(),
            created_at: now,
            updated_at: now,
            evaluator: Some(evaluator),
            source_metadata: None,
        })
    }

    /// Create a new policy with specified language
    pub fn new_with_language(
        name: String,
        description: String,
        language: PolicyLanguage,
        content: String,
    ) -> Result<Self> {
        let now = chrono::Utc::now();

        let mut policy = Self {
            id: Uuid::new_v4(),
            version: 1,
            name,
            description,
            language: language.clone(),
            content: content.clone(),
            rules: Vec::new(),
            metadata: HashMap::new(),
            priority: default_priority(),
            created_at: now,
            updated_at: now,
            evaluator: None,
            source_metadata: None,
        };

        // Build and validate evaluator
        policy.build_evaluator()?;

        Ok(policy)
    }

    /// Build the evaluator from content and language
    pub fn build_evaluator(&mut self) -> Result<()> {
        let evaluator: Arc<dyn PolicyEvaluator> = match &self.language {
            PolicyLanguage::Simple => {
                let rules: Vec<PolicyRule> = serde_json::from_str(&self.content).map_err(|e| {
                    ReaperError::InvalidPolicy {
                        reason: format!("Failed to parse simple policy rules: {}", e),
                    }
                })?;

                // Update rules for backward compatibility
                self.rules = rules.clone();

                // Check if tree optimization is requested in metadata
                let use_tree = self
                    .metadata
                    .get("optimization")
                    .map(|v| v == "tree")
                    .unwrap_or(false);

                if use_tree {
                    Arc::new(SimplePolicyEvaluator::with_tree_optimization(rules)?)
                } else {
                    Arc::new(SimplePolicyEvaluator::new(rules))
                }
            }
            PolicyLanguage::Cedar => {
                let evaluator = CedarPolicyEvaluator::new(self.content.clone())?;
                Arc::new(evaluator)
            }
            PolicyLanguage::Custom => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "Custom policy language not yet implemented".to_string(),
                });
            }
        };

        // Validate before storing
        evaluator.validate()?;
        self.evaluator = Some(evaluator);

        Ok(())
    }

    /// Get the evaluator, building it if necessary
    pub fn get_evaluator(&mut self) -> Result<Arc<dyn PolicyEvaluator>> {
        if self.evaluator.is_none() {
            self.build_evaluator()?;
        }

        self.evaluator
            .clone()
            .ok_or_else(|| ReaperError::EvaluationError {
                reason: "Failed to build evaluator".to_string(),
            })
    }

    /// Build an optimized compiled evaluator
    ///
    /// This applies optimizations:
    /// - Partial evaluation (if static_context provided)
    /// - Policy compilation (flattened, pre-parsed rules)
    /// - Inline simple checks
    ///
    /// Expected performance: 2-10x faster than standard evaluator
    pub fn build_compiled(
        &self,
        static_context: Option<&std::collections::HashMap<String, String>>,
    ) -> Result<Arc<dyn PolicyEvaluator>> {
        use crate::compiled_evaluator::CompiledPolicyEvaluator;

        let evaluator = CompiledPolicyEvaluator::compile(self, static_context)?;
        Ok(Arc::new(evaluator))
    }

    /// Update policy rules (for Simple language - backward compatible)
    pub fn update_rules(&mut self, rules: Vec<PolicyRule>) {
        self.content = serde_json::to_string(&rules).unwrap_or_default();
        self.rules = rules.clone();
        self.version += 1;
        self.updated_at = chrono::Utc::now();

        // Rebuild evaluator
        let _ = self.build_evaluator();
    }

    /// Update policy content (for any language)
    pub fn update_content(&mut self, content: String) -> Result<()> {
        self.content = content;
        self.version += 1;
        self.updated_at = chrono::Utc::now();

        // Rebuild and validate evaluator
        self.build_evaluator()?;

        Ok(())
    }

    /// Enable compilation for this policy
    ///
    /// When enabled, the policy will be compiled to native Rust code for
    /// maximum performance (10-500x speedup). This adds a one-time compilation
    /// cost at deploy time but dramatically improves runtime performance.
    ///
    /// # Note
    /// Compilation requires the policy to remain stable. Frequent policy updates
    /// will incur recompilation overhead.
    pub fn enable_compilation(&mut self) {
        self.metadata
            .insert("compile".to_string(), "true".to_string());
    }

    /// Disable compilation for this policy
    pub fn disable_compilation(&mut self) {
        self.metadata.remove("compile");
    }

    /// Check if compilation is enabled for this policy
    pub fn is_compilation_enabled(&self) -> bool {
        self.metadata
            .get("compile")
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    /// Set compilation flag
    pub fn set_compilation(&mut self, enabled: bool) {
        if enabled {
            self.enable_compilation();
        } else {
            self.disable_compilation();
        }
    }

    /// Set the source metadata for this policy
    pub fn set_source_metadata(&mut self, metadata: PolicySourceMetadata) {
        self.source_metadata = Some(metadata);
    }

    /// Get the source metadata for this policy
    pub fn get_source_metadata(&self) -> Option<&PolicySourceMetadata> {
        self.source_metadata.as_ref()
    }

    /// Create source metadata for a file-based policy
    pub fn set_file_source(&mut self, path: &str, deployed_by: Option<String>) {
        let checksum = hex::encode(Sha256::digest(self.content.as_bytes()));
        self.source_metadata = Some(PolicySourceMetadata {
            source: PolicySource::File {
                path: path.to_string(),
            },
            deployed_at: chrono::Utc::now(),
            deployed_by,
            source_version: Some(format!("v{}", self.version)),
            checksum: Some(checksum),
        });
    }

    /// Create source metadata for an API-deployed policy
    pub fn set_api_source(&mut self, client_id: Option<String>, deployed_by: Option<String>) {
        let checksum = hex::encode(Sha256::digest(self.content.as_bytes()));
        self.source_metadata = Some(PolicySourceMetadata {
            source: PolicySource::Api { client_id },
            deployed_at: chrono::Utc::now(),
            deployed_by,
            source_version: Some(format!("v{}", self.version)),
            checksum: Some(checksum),
        });
    }

    /// Create source metadata for a sync-client deployed policy
    pub fn set_sync_source(
        &mut self,
        server_url: &str,
        server_version: &str,
        team: Option<String>,
        deployed_by: Option<String>,
    ) {
        let checksum = hex::encode(Sha256::digest(self.content.as_bytes()));
        self.source_metadata = Some(PolicySourceMetadata {
            source: PolicySource::SyncClient {
                server_url: server_url.to_string(),
                server_version: server_version.to_string(),
                team,
            },
            deployed_at: chrono::Utc::now(),
            deployed_by,
            source_version: Some(format!("v{}", self.version)),
            checksum: Some(checksum),
        });
    }

    /// Compute checksum of policy content
    pub fn compute_checksum(&self) -> String {
        hex::encode(Sha256::digest(self.content.as_bytes()))
    }

    /// Get the package name for this policy
    ///
    /// Policies can specify a package in their metadata with the "package" key.
    /// Policies without a package specification default to "default".
    pub fn package(&self) -> &str {
        self.metadata
            .get("package")
            .map(|s| s.as_str())
            .unwrap_or("default")
    }

    /// Set the package name for this policy
    pub fn set_package(&mut self, package: impl Into<String>) {
        self.metadata.insert("package".to_string(), package.into());
    }
}
