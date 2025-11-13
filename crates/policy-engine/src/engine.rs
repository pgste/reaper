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
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::evaluators::{PolicyEvaluator, SimplePolicyEvaluator, CedarPolicyEvaluator};

/// Policy action types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
    Log,
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

    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// Cached evaluator (not serialized)
    #[serde(skip)]
    evaluator: Option<Arc<dyn PolicyEvaluator>>,
}

impl EnhancedPolicy {
    /// Create a new policy with Simple language (backward compatible)
    pub fn new(name: String, description: String, rules: Vec<PolicyRule>) -> Self {
        let content = serde_json::to_string(&rules).unwrap_or_default();
        let now = chrono::Utc::now();

        let evaluator = Arc::new(SimplePolicyEvaluator::new(rules.clone())) as Arc<dyn PolicyEvaluator>;

        Self {
            id: Uuid::new_v4(),
            version: 1,
            name,
            description,
            language: PolicyLanguage::Simple,
            content,
            rules,
            created_at: now,
            updated_at: now,
            evaluator: Some(evaluator),
        }
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
            created_at: now,
            updated_at: now,
            evaluator: None,
        };

        // Build and validate evaluator
        policy.build_evaluator()?;

        Ok(policy)
    }

    /// Build the evaluator from content and language
    fn build_evaluator(&mut self) -> Result<()> {
        let evaluator: Arc<dyn PolicyEvaluator> = match &self.language {
            PolicyLanguage::Simple => {
                let rules: Vec<PolicyRule> = serde_json::from_str(&self.content)
                    .map_err(|e| ReaperError::InvalidPolicy {
                        reason: format!("Failed to parse simple policy rules: {}", e),
                    })?;

                // Update rules for backward compatibility
                self.rules = rules.clone();

                Arc::new(SimplePolicyEvaluator::new(rules))
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

        self.evaluator.clone().ok_or_else(|| ReaperError::EvaluationError {
            reason: "Failed to build evaluator".to_string(),
        })
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

        // Get or build the evaluator
        let evaluator = policy_mut.get_evaluator()?;

        // Evaluate using the language-specific evaluator
        let decision = evaluator.evaluate(request)?;

        let evaluation_time_ns = start_time.elapsed().as_nanos() as u64;

        Ok(PolicyDecision {
            decision,
            policy_id: policy_mut.id,
            policy_version: policy_mut.version,
            evaluation_time_ns,
            matched_rule: None, // Matched rule index only available for Simple policies
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
}
