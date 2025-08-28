//! Policy Engine Implementation
//!
//! Features Rust's atomic operations for zero-downtime policy swapping
//! and lock-free lookups for sub-microsecond performance.

use dashmap::DashMap;
use parking_lot::RwLock;
use reaper_core::{PolicyId, ReaperError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, instrument};
use uuid::Uuid;

/// Policy action types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyAction {
    Allow,
    Deny,
    Log,
}

/// Policy rule definition - extensible for complex policies later
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub action: PolicyAction,
    pub resource: String,
    pub conditions: Vec<String>,
}

/// Enhanced Policy with rules and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedPolicy {
    pub id: PolicyId,
    pub version: u64,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl EnhancedPolicy {
    pub fn new(name: String, description: String, rules: Vec<PolicyRule>) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: Uuid::new_v4(),
            version: 1,
            name,
            description,
            rules,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn update_rules(&mut self, rules: Vec<PolicyRule>) {
        self.rules = rules;
        self.version += 1;
        self.updated_at = chrono::Utc::now();
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
    /// Optimized for sub-microsecond latency
    #[instrument(skip(self, request), fields(resource = %request.resource, action = %request.action))]
    pub fn evaluate(
        &self,
        policy_id: &PolicyId,
        request: &PolicyRequest,
    ) -> Result<PolicyDecision> {
        let start_time = std::time::Instant::now();

        let policy = self
            .get_policy(policy_id)
            .or_else(|| self.default_policy.read().clone())
            .ok_or_else(|| ReaperError::PolicyNotFound {
                policy_id: policy_id.to_string(),
            })?;

        // Simple rule matching - optimized for performance
        let mut matched_rule = None;
        let mut decision = PolicyAction::Deny; // Default deny

        for (index, rule) in policy.rules.iter().enumerate() {
            if self.matches_rule(rule, request) {
                decision = rule.action.clone();
                matched_rule = Some(index);
                break; // First match wins
            }
        }

        let evaluation_time_ns = start_time.elapsed().as_nanos() as u64;

        Ok(PolicyDecision {
            decision,
            policy_id: policy.id,
            policy_version: policy.version,
            evaluation_time_ns,
            matched_rule,
        })
    }

    /// Rule matching logic - will be enhanced in future iterations
    fn matches_rule(&self, rule: &PolicyRule, request: &PolicyRequest) -> bool {
        // Simple wildcard matching for MVP
        rule.resource == "*" || rule.resource == request.resource
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
