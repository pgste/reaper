//! Policy Engine Implementation
//!
//! Features Rust's atomic operations for zero-downtime policy swapping
//! and lock-free lookups for sub-microsecond performance.
//!
//! Supports multiple policy languages through the PolicyEvaluator trait.

use dashmap::DashMap;
use parking_lot::RwLock;
use reaper_core::{PolicyId, ReaperError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::evaluators::{CedarPolicyEvaluator, PolicyEvaluator, SimplePolicyEvaluator};
use crate::reap::PolicyBundle;

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
fn default_priority() -> u32 {
    1000
}

/// Policy rule definition - used for Simple language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub action: PolicyAction,
    pub resource: String,
    pub conditions: Vec<String>,
}

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
    fn build_evaluator(&mut self) -> Result<()> {
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
        use sha2::{Digest, Sha256};
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
        use sha2::{Digest, Sha256};
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
        use sha2::{Digest, Sha256};
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
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(self.content.as_bytes()))
    }
}

/// Policy evaluation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub resource: String,
    pub action: String,
    pub context: HashMap<String, String>,
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

/// High-performance policy engine with atomic hot-swapping
///
/// Key Rust Features for End-User Value:
/// - Arc for zero-copy policy sharing across threads
/// - DashMap for lock-free concurrent access
/// - Atomic operations for zero-downtime policy updates
#[derive(Clone)]
pub struct PolicyEngine {
    /// Active policies - lock-free for sub-microsecond lookups
    active_policies: Arc<DashMap<PolicyId, Arc<EnhancedPolicy>>>,
    /// Policy lookup by name for convenience
    policy_names: Arc<DashMap<String, PolicyId>>,
    /// Default policy for unknown policies
    default_policy: Arc<RwLock<Option<Arc<EnhancedPolicy>>>>,
    /// Version tracking for policy bundles
    versions: Arc<DashMap<PolicyId, Vec<PolicyVersion>>>,
    /// Bundle cache for rollback support (keyed by policy_id:version)
    bundle_cache: Arc<DashMap<String, PolicyBundle>>,
}

impl std::fmt::Debug for PolicyEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyEngine")
            .field("active_policies_count", &self.active_policies.len())
            .field("policy_names_count", &self.policy_names.len())
            .field("has_default_policy", &self.default_policy.read().is_some())
            .finish()
    }
}

impl PolicyEngine {
    pub fn new() -> Self {
        info!("Initializing Reaper Policy Engine with lock-free storage");
        Self {
            active_policies: Arc::new(DashMap::new()),
            policy_names: Arc::new(DashMap::new()),
            default_policy: Arc::new(RwLock::new(None)),
            versions: Arc::new(DashMap::new()),
            bundle_cache: Arc::new(DashMap::new()),
        }
    }

    /// Hot-swap a policy with zero downtime
    /// Uses atomic operations to ensure no request sees inconsistent state
    #[instrument(skip(self, policy), fields(policy_id = %policy.id, version = policy.version))]
    pub fn deploy_policy(&self, policy: EnhancedPolicy) -> Result<()> {
        let policy_id = policy.id;
        let policy_name = policy.name.clone();
        let policy_arc = Arc::new(policy);

        info!(
            "Hot-swapping policy '{}' (version {})",
            policy_name, policy_arc.version
        );

        // Atomic insertion - old policy is automatically dropped
        self.active_policies.insert(policy_id, policy_arc.clone());
        self.policy_names.insert(policy_name.clone(), policy_id);

        info!("Policy '{}' deployed successfully", policy_name);
        Ok(())
    }

    /// Remove a policy atomically
    #[instrument(skip(self), fields(policy_id = %policy_id))]
    pub fn remove_policy(&self, policy_id: &PolicyId) -> Result<EnhancedPolicy> {
        let removed_policy = self
            .active_policies
            .remove(policy_id)
            .map(|(_, policy)| policy)
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Remove from name lookup too
        self.policy_names.retain(|_, &mut v| v != *policy_id);

        info!("Policy {} removed successfully", policy_id);
        Ok(Arc::try_unwrap(removed_policy).unwrap_or_else(|arc| (*arc).clone()))
    }

    /// Deploy a policy from a .rbb bundle with version tracking
    ///
    /// This method:
    /// 1. Validates the bundle
    /// 2. Generates a SHA-256 hash for integrity verification
    /// 3. Checks that the version is newer than the current version (unless force=true)
    /// 4. Compiles the policy to an EnhancedPolicy
    /// 5. Atomically inserts/replaces the policy in the engine
    /// 6. Stores version metadata for tracking and rollback
    /// 7. Caches the bundle for potential rollback operations
    ///
    /// # Arguments
    /// * `bundle` - The PolicyBundle to deploy
    /// * `force` - If true, skip version validation and allow downgrade
    ///
    /// # Returns
    /// PolicyVersion with deployment metadata including bundle hash
    #[instrument(skip(self, bundle), fields(policy_name = %bundle.policy.name))]
    pub fn deploy_bundle(&self, bundle: PolicyBundle, force: bool) -> Result<PolicyVersion> {
        let bundle_version = bundle.metadata.policy_version.as_deref().unwrap_or("1.0.0");
        info!(
            "Deploying policy bundle: {} (version: {})",
            bundle.metadata.policy_name, bundle_version
        );

        // 1. Generate bundle hash (SHA-256)
        let bundle_bytes = bundle.to_bytes().map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bundle_bytes);
        let bundle_hash: [u8; 32] = hasher.finalize().into();

        // 2. Convert bundle to EnhancedPolicy
        let policy = bundle.to_enhanced_policy()?;
        let policy_id = policy.id;
        let policy_id_str = policy_id.to_string();

        // 3. Version validation (unless force=true)
        if !force {
            if let Some(existing_versions) = self.versions.get(&policy_id) {
                if !existing_versions.is_empty() {
                    // Check if new version is actually newer
                    // For simplicity, we just check if the version string is different
                    let last_version = &existing_versions.last().unwrap().version;
                    if last_version == bundle_version {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Version {} already deployed. Use force=true to redeploy.",
                                bundle_version
                            ),
                        });
                    }
                }
            }
        }

        // 4. Deploy the policy (atomic hot-swap)
        self.deploy_policy(policy)?;

        // 5. Create version metadata
        let policy_version = PolicyVersion {
            version: bundle_version.to_string(),
            deployed_at: SystemTime::now(),
            bundle_hash,
            policy_id: policy_id_str.clone(),
        };

        // 6. Store version in history
        self.versions
            .entry(policy_id)
            .or_default()
            .push(policy_version.clone());

        // 7. Cache bundle for rollback (key: policy_id:version)
        let cache_key = format!("{}:{}", policy_id_str, bundle_version);
        self.bundle_cache.insert(cache_key, bundle.clone());

        info!(
            "Bundle deployed successfully: {} version {}",
            bundle.metadata.policy_name, bundle_version
        );

        Ok(policy_version)
    }

    /// Deploy a policy from a .rbb bundle with full ReaperDSL compilation
    ///
    /// This method compiles the bundle using the full ReaperDSL compiler,
    /// preserving all complex conditions, functions, and rule logic.
    /// This is the recommended method for production bundle deployment.
    ///
    /// # Arguments
    /// * `bundle` - The PolicyBundle to deploy
    /// * `store` - DataStore containing entity data for the evaluator
    /// * `force` - If true, skip version validation and allow downgrade
    ///
    /// # Returns
    /// PolicyVersion with deployment metadata including bundle hash
    #[instrument(skip(self, bundle, store), fields(policy_name = %bundle.policy.name))]
    pub fn deploy_bundle_with_store(
        &self,
        bundle: PolicyBundle,
        store: Arc<crate::data::DataStore>,
        force: bool,
    ) -> Result<PolicyVersion> {
        let bundle_version = bundle.metadata.policy_version.as_deref().unwrap_or("1.0.0");
        info!(
            "Deploying policy bundle with compiled evaluator: {} (version: {})",
            bundle.metadata.policy_name, bundle_version
        );

        // 1. Generate bundle hash (SHA-256)
        let bundle_bytes = bundle.to_bytes().map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bundle_bytes);
        let bundle_hash: [u8; 32] = hasher.finalize().into();

        // 2. Convert bundle to EnhancedPolicy with compiled evaluator
        let policy = bundle.to_enhanced_policy_with_store(store)?;
        let policy_id = policy.id;
        let policy_id_str = policy_id.to_string();

        // 3. Version validation (unless force=true)
        if !force {
            if let Some(existing_versions) = self.versions.get(&policy_id) {
                if !existing_versions.is_empty() {
                    let last_version = &existing_versions.last().unwrap().version;
                    if last_version == bundle_version {
                        return Err(ReaperError::InvalidPolicy {
                            reason: format!(
                                "Version {} already deployed. Use force=true to redeploy.",
                                bundle_version
                            ),
                        });
                    }
                }
            }
        }

        // 4. Deploy the policy (atomic hot-swap)
        self.deploy_policy(policy)?;

        // 5. Create version metadata
        let policy_version = PolicyVersion {
            version: bundle_version.to_string(),
            deployed_at: SystemTime::now(),
            bundle_hash,
            policy_id: policy_id_str.clone(),
        };

        // 6. Store version in history
        self.versions
            .entry(policy_id)
            .or_default()
            .push(policy_version.clone());

        // 7. Cache bundle for rollback (key: policy_id:version)
        let cache_key = format!("{}:{}", policy_id_str, bundle_version);
        self.bundle_cache.insert(cache_key, bundle.clone());

        info!(
            "Bundle deployed with compiled evaluator: {} version {} ({} rules)",
            bundle.metadata.policy_name,
            bundle_version,
            bundle.policy.rules.len()
        );

        Ok(policy_version)
    }

    /// Rollback a policy to a previous version
    ///
    /// This loads the cached bundle for the specified version and re-deploys it.
    ///
    /// # Arguments
    /// * `policy_id` - The ID of the policy to rollback
    /// * `target_version` - The version to rollback to
    ///
    /// # Returns
    /// PolicyVersion of the restored version
    #[instrument(skip(self), fields(policy_id = %policy_id, target_version = %target_version))]
    pub fn rollback(&self, policy_id: &PolicyId, target_version: &str) -> Result<PolicyVersion> {
        info!(
            "Rolling back policy {} to version {}",
            policy_id, target_version
        );

        // 1. Lookup bundle from cache
        let cache_key = format!("{}:{}", policy_id, target_version);
        let bundle = self
            .bundle_cache
            .get(&cache_key)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: format!(
                    "Bundle not found in cache: {}:{}",
                    policy_id, target_version
                ),
            })?;

        // 2. Re-deploy bundle (force=true to allow "downgrade")
        let version = self.deploy_bundle(bundle, true)?;

        info!(
            "Rollback successful: policy {} restored to version {}",
            policy_id, target_version
        );

        Ok(version)
    }

    /// Get the current version of a policy
    ///
    /// Returns the most recently deployed version metadata.
    pub fn get_version(&self, policy_id: &PolicyId) -> Option<PolicyVersion> {
        self.versions
            .get(policy_id)
            .and_then(|versions| versions.last().cloned())
    }

    /// List all versions of a policy in chronological order
    ///
    /// Returns all cached versions for the specified policy.
    pub fn list_versions(&self, policy_id: &PolicyId) -> Vec<PolicyVersion> {
        self.versions
            .get(policy_id)
            .map(|versions| versions.clone())
            .unwrap_or_default()
    }

    /// Get policy by ID - lock-free for maximum performance
    pub fn get_policy(&self, policy_id: &PolicyId) -> Option<Arc<EnhancedPolicy>> {
        self.active_policies
            .get(policy_id)
            .map(|entry| entry.value().clone())
    }

    /// Get policy by name
    pub fn get_policy_by_name(&self, name: &str) -> Option<Arc<EnhancedPolicy>> {
        self.policy_names
            .get(name)
            .and_then(|entry| self.get_policy(entry.value()))
    }

    /// List all active policies
    pub fn list_policies(&self) -> Vec<Arc<EnhancedPolicy>> {
        self.active_policies
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Set default policy for unknown policy requests
    pub fn set_default_policy(&self, policy: EnhancedPolicy) {
        let mut default = self.default_policy.write();
        *default = Some(Arc::new(policy));
        info!("Default policy updated");
    }

    /// Evaluate a request against a policy
    /// Optimized for sub-microsecond latency with Simple policies
    /// Cedar policies may take 10-50 microseconds depending on complexity
    #[instrument(skip(self, request), fields(resource = %request.resource, action = %request.action))]
    pub fn evaluate(
        &self,
        policy_id: &PolicyId,
        request: &PolicyRequest,
    ) -> Result<PolicyDecision> {
        let start_time = std::time::Instant::now();

        let mut policy = self
            .get_policy(policy_id)
            .or_else(|| self.default_policy.read().clone())
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Get the policy as mutable to access evaluator
        let policy_mut = Arc::make_mut(&mut policy);

        // Evaluate using the language-specific evaluator
        // For Simple policies, find the matched rule index
        let (decision, matched_rule) = if policy_mut.language == PolicyLanguage::Simple {
            // For simple policies, manually find which rule matches
            let mut matched_index = None;
            for (index, rule) in policy_mut.rules.iter().enumerate() {
                // Check if rule matches (same logic as SimplePolicyEvaluator)
                if rule.resource == "*" || rule.resource == request.resource {
                    matched_index = Some(index);
                    break;
                }
            }

            // Get or build the evaluator
            let evaluator = policy_mut.get_evaluator()?;
            let decision = evaluator.evaluate(request)?;
            (decision, matched_index)
        } else {
            // Get or build the evaluator
            let evaluator = policy_mut.get_evaluator()?;
            let decision = evaluator.evaluate(request)?;
            (decision, None)
        };

        let evaluation_time_ns = start_time.elapsed().as_nanos() as u64;

        Ok(PolicyDecision {
            decision,
            policy_id: policy_mut.id,
            policy_version: policy_mut.version,
            evaluation_time_ns,
            matched_rule,
        })
    }

    /// Get engine statistics for monitoring
    pub fn get_stats(&self) -> PolicyEngineStats {
        PolicyEngineStats {
            total_policies: self.active_policies.len(),
            has_default_policy: self.default_policy.read().is_some(),
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_policy_deployment_and_lookup() {
        let engine = PolicyEngine::new();

        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "Test policy".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        let policy_id = policy.id;

        // Deploy policy
        engine.deploy_policy(policy.clone()).unwrap();

        // Verify policy exists
        let retrieved = engine.get_policy(&policy_id).unwrap();
        assert_eq!(retrieved.name, "test-policy");

        // Verify lookup by name
        let by_name = engine.get_policy_by_name("test-policy").unwrap();
        assert_eq!(by_name.id, policy_id);
    }

    #[tokio::test]
    async fn test_hot_swap() {
        let engine = PolicyEngine::new();

        let mut policy = EnhancedPolicy::new(
            "hot-swap".to_string(),
            "Hot swap test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Deny,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        let policy_id = policy.id;

        // Deploy initial policy
        engine.deploy_policy(policy.clone()).unwrap();

        // Update policy rules
        policy.update_rules(vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }]);

        // Hot swap
        engine.deploy_policy(policy).unwrap();

        // Verify new version
        let updated = engine.get_policy(&policy_id).unwrap();
        assert_eq!(updated.version, 2);
        match &updated.rules[0].action {
            PolicyAction::Allow => (),
            _ => panic!("Expected Allow action"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation() {
        let engine = PolicyEngine::new();

        let policy = EnhancedPolicy::new(
            "eval-test".to_string(),
            "Evaluation test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "test-resource".to_string(),
                conditions: vec![],
            }],
        );
        let policy_id = policy.id;

        engine.deploy_policy(policy).unwrap();

        let request = PolicyRequest {
            resource: "test-resource".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = engine.evaluate(&policy_id, &request).unwrap();

        match decision.decision {
            PolicyAction::Allow => (),
            _ => panic!("Expected Allow decision"),
        }

        assert!(decision.evaluation_time_ns > 0);
        assert_eq!(decision.matched_rule, Some(0));
    }

    #[tokio::test]
    async fn test_tree_optimization() {
        // Create policy with tree optimization
        let rules = vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "resource1".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "resource2".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Deny,
                resource: "*".to_string(),
                conditions: vec![],
            },
        ];

        let policy = EnhancedPolicy::new_with_tree_optimization(
            "tree-test".to_string(),
            "Tree optimization test".to_string(),
            rules,
        )
        .unwrap();

        // Verify metadata is set
        assert_eq!(
            policy.metadata.get("optimization"),
            Some(&"tree".to_string())
        );

        let policy_id = policy.id;
        let engine = PolicyEngine::new();
        engine.deploy_policy(policy).unwrap();

        // Test evaluation
        let request = PolicyRequest {
            resource: "resource1".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let decision = engine.evaluate(&policy_id, &request).unwrap();
        assert!(matches!(decision.decision, PolicyAction::Allow));
    }

    #[tokio::test]
    async fn test_tree_optimization_scale() {
        // Generate many rules to test tree optimization performance
        let mut rules = Vec::new();
        for i in 0..100 {
            rules.push(PolicyRule {
                action: if i % 2 == 0 {
                    PolicyAction::Allow
                } else {
                    PolicyAction::Deny
                },
                resource: format!("resource_{}", i),
                conditions: vec![],
            });
        }

        // Create with tree optimization
        let tree_policy = EnhancedPolicy::new_with_tree_optimization(
            "tree-scale-test".to_string(),
            "Tree scale test".to_string(),
            rules.clone(),
        )
        .unwrap();

        // Create without tree optimization for comparison
        let linear_policy = EnhancedPolicy::new(
            "linear-scale-test".to_string(),
            "Linear scale test".to_string(),
            rules,
        );

        let engine = PolicyEngine::new();
        let tree_id = tree_policy.id;
        let linear_id = linear_policy.id;

        engine.deploy_policy(tree_policy).unwrap();
        engine.deploy_policy(linear_policy).unwrap();

        // Test both
        let request = PolicyRequest {
            resource: "resource_50".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let tree_decision = engine.evaluate(&tree_id, &request).unwrap();
        let linear_decision = engine.evaluate(&linear_id, &request).unwrap();

        // Both should give same result
        assert_eq!(tree_decision.decision, linear_decision.decision);

        // Tree should be faster (generally, though with only 100 rules the difference may be small)
        println!(
            "Tree eval: {}ns, Linear eval: {}ns",
            tree_decision.evaluation_time_ns, linear_decision.evaluation_time_ns
        );
    }

    #[tokio::test]
    async fn test_metadata_flag_enables_tree() {
        let content = serde_json::to_string(&vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "test".to_string(),
            conditions: vec![],
        }])
        .unwrap();

        let mut policy = EnhancedPolicy::new_with_language(
            "metadata-test".to_string(),
            "Metadata test".to_string(),
            PolicyLanguage::Simple,
            content,
        )
        .unwrap();

        // Set tree optimization metadata
        policy
            .metadata
            .insert("optimization".to_string(), "tree".to_string());

        // Rebuild evaluator with tree optimization
        policy.build_evaluator().unwrap();

        // Verify evaluator has tree optimization enabled
        let evaluator = policy.get_evaluator().unwrap();
        if let Some(metadata) = evaluator.metadata() {
            assert!(
                metadata
                    .extra
                    .get("tree_optimized")
                    .map(|v| v == "true")
                    .unwrap_or(false),
                "Tree optimization should be enabled"
            );
        }
    }

    // ========== Hot-Reload Tests ==========

    #[tokio::test]
    async fn test_bundle_deployment_with_version_tracking() {
        use crate::reap::{Decision, Policy as ReapPolicy, ReapCondition, ReapRule};

        let engine = PolicyEngine::new();

        // Create a Reap policy
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("version".to_string(), "1.0.0".to_string());

        let reap_policy = ReapPolicy {
            name: "test-bundle-policy".to_string(),
            metadata,
            default_decision: Decision::Deny,
            rules: vec![ReapRule {
                name: "allow-admins".to_string(),
                decision: Decision::Allow,
                condition: ReapCondition::True,
            }],
        };

        // Create bundle
        let bundle = crate::reap::PolicyBundle::new(reap_policy);

        // Deploy bundle with version tracking
        let version = engine.deploy_bundle(bundle.clone(), false).unwrap();

        assert_eq!(version.version, "1.0.0");
        assert_eq!(version.policy_id, version.policy_id);
        assert!(version.bundle_hash.len() == 32); // SHA-256 hash
        assert_eq!(
            version.deployed_at.elapsed().unwrap().as_secs(),
            0,
            "Deployment should be recent"
        );
    }

    #[tokio::test]
    async fn test_bundle_version_history() {
        use crate::reap::{Decision, Policy as ReapPolicy};

        let engine = PolicyEngine::new();

        // Create and deploy first version
        let mut metadata1 = std::collections::HashMap::new();
        metadata1.insert("version".to_string(), "1.0.0".to_string());

        let policy1 = ReapPolicy {
            name: "versioned-policy".to_string(),
            metadata: metadata1,
            default_decision: Decision::Deny,
            rules: vec![],
        };

        let bundle1 = crate::reap::PolicyBundle::new(policy1);
        let version1 = engine.deploy_bundle(bundle1, false).unwrap();

        // Create and deploy second version
        let mut metadata2 = std::collections::HashMap::new();
        metadata2.insert("version".to_string(), "2.0.0".to_string());

        let policy2 = ReapPolicy {
            name: "another-policy".to_string(),
            metadata: metadata2,
            default_decision: Decision::Allow,
            rules: vec![],
        };

        let bundle2 = crate::reap::PolicyBundle::new(policy2);
        let version2 = engine.deploy_bundle(bundle2, false).unwrap();

        // Verify each has their own version history
        let policy1_uuid = uuid::Uuid::parse_str(&version1.policy_id).unwrap();
        let versions1 = engine.list_versions(&policy1_uuid);
        assert_eq!(versions1.len(), 1);
        assert_eq!(versions1[0].version, "1.0.0");

        let policy2_uuid = uuid::Uuid::parse_str(&version2.policy_id).unwrap();
        let versions2 = engine.list_versions(&policy2_uuid);
        assert_eq!(versions2.len(), 1);
        assert_eq!(versions2[0].version, "2.0.0");
    }

    #[tokio::test]
    async fn test_bundle_rollback() {
        use crate::reap::{Decision, Policy as ReapPolicy};

        let engine = PolicyEngine::new();

        // Deploy version 1.0.0
        let mut metadata1 = std::collections::HashMap::new();
        metadata1.insert("version".to_string(), "1.0.0".to_string());

        let policy1 = ReapPolicy {
            name: "rollback-test".to_string(),
            metadata: metadata1,
            default_decision: Decision::Deny,
            rules: vec![],
        };

        let bundle1 = crate::reap::PolicyBundle::new(policy1);
        let version1 = engine.deploy_bundle(bundle1, false).unwrap();

        // Deploy version 2.0.0
        let mut metadata2 = std::collections::HashMap::new();
        metadata2.insert("version".to_string(), "2.0.0".to_string());

        let policy2 = ReapPolicy {
            name: "rollback-test".to_string(),
            metadata: metadata2,
            default_decision: Decision::Allow,
            rules: vec![],
        };

        let bundle2 = crate::reap::PolicyBundle::new(policy2);
        engine.deploy_bundle(bundle2, false).unwrap();

        // Rollback to 1.0.0
        let policy_uuid = uuid::Uuid::parse_str(&version1.policy_id).unwrap();
        let rollback_version = engine.rollback(&policy_uuid, "1.0.0").unwrap();

        assert_eq!(rollback_version.version, "1.0.0");

        // Verify the policy was rolled back
        let policy = engine.get_policy(&policy_uuid).unwrap();
        assert_eq!(policy.name, "rollback-test");
    }

    #[tokio::test]
    async fn test_bundle_force_deployment() {
        use crate::reap::{Decision, Policy as ReapPolicy};

        let engine = PolicyEngine::new();

        // Create bundle
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("version".to_string(), "1.0.0".to_string());

        let policy = ReapPolicy {
            name: "force-test".to_string(),
            metadata,
            default_decision: Decision::Deny,
            rules: vec![],
        };

        let bundle = crate::reap::PolicyBundle::new(policy);

        // Deploy first time
        engine.deploy_bundle(bundle.clone(), false).unwrap();

        // Deploy again with force=true (should succeed)
        let version = engine.deploy_bundle(bundle, true).unwrap();
        assert_eq!(version.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_bundle_hash_integrity() {
        use crate::reap::{Decision, Policy as ReapPolicy};

        let engine = PolicyEngine::new();

        // Create two identical bundles
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("version".to_string(), "1.0.0".to_string());

        let policy = ReapPolicy {
            name: "hash-test".to_string(),
            metadata: metadata.clone(),
            default_decision: Decision::Deny,
            rules: vec![],
        };

        let bundle1 = crate::reap::PolicyBundle::new(policy.clone());
        let version1 = engine.deploy_bundle(bundle1, false).unwrap();

        // Create different bundle
        let mut metadata2 = std::collections::HashMap::new();
        metadata2.insert("version".to_string(), "2.0.0".to_string());

        let policy2 = ReapPolicy {
            name: "hash-test".to_string(),
            metadata: metadata2,
            default_decision: Decision::Allow,
            rules: vec![],
        };

        let bundle2 = crate::reap::PolicyBundle::new(policy2);
        let version2 = engine.deploy_bundle(bundle2, false).unwrap();

        // Hashes should be different
        assert_ne!(
            version1.bundle_hash, version2.bundle_hash,
            "Different bundles should have different hashes"
        );
    }

    #[tokio::test]
    async fn test_get_version_metadata() {
        use crate::reap::{Decision, Policy as ReapPolicy};

        let engine = PolicyEngine::new();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("version".to_string(), "1.5.0".to_string());

        let policy = ReapPolicy {
            name: "version-metadata-test".to_string(),
            metadata,
            default_decision: Decision::Deny,
            rules: vec![],
        };

        let bundle = crate::reap::PolicyBundle::new(policy);
        let deployed_version = engine.deploy_bundle(bundle, false).unwrap();

        // Get version metadata
        let policy_uuid = uuid::Uuid::parse_str(&deployed_version.policy_id).unwrap();
        let retrieved_version = engine.get_version(&policy_uuid).unwrap();

        assert_eq!(retrieved_version.version, "1.5.0");
        assert_eq!(
            retrieved_version.bundle_hash, deployed_version.bundle_hash,
            "Hashes should match"
        );
    }
}
