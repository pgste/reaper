//! Request and response types for the platform API.
//!
//! This module contains all the serializable types used in API endpoints.

use chrono::{DateTime, Utc};
use policy_engine::{EnhancedPolicy, PolicyAction};
use serde::{Deserialize, Serialize};

// ============================================================================
// Policy Types
// ============================================================================

/// Request to create a new policy
#[derive(Debug, Deserialize)]
pub struct CreatePolicyRequest {
    pub name: String,
    pub description: Option<String>,
    pub rules: Vec<CreatePolicyRule>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePolicyRule {
    pub action: String, // "allow", "deny", "log"
    pub resource: String,
    pub conditions: Option<Vec<String>>,
}

/// Request to update a policy
#[derive(Debug, Deserialize)]
pub struct UpdatePolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub rules: Option<Vec<CreatePolicyRule>>,
}

/// Policy response
#[derive(Debug, Serialize)]
pub struct PolicyResponse {
    pub id: String,
    pub version: u64,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRuleResponse>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PolicyRuleResponse {
    pub action: String,
    pub resource: String,
    pub conditions: Vec<String>,
}

impl From<EnhancedPolicy> for PolicyResponse {
    fn from(policy: EnhancedPolicy) -> Self {
        Self {
            id: policy.id.to_string(),
            version: policy.version,
            name: policy.name,
            description: policy.description,
            rules: policy
                .rules
                .into_iter()
                .map(|rule| PolicyRuleResponse {
                    action: match rule.action {
                        PolicyAction::Allow => "allow".to_string(),
                        PolicyAction::Deny => "deny".to_string(),
                        PolicyAction::Log => "log".to_string(),
                    },
                    resource: rule.resource,
                    conditions: rule.conditions,
                })
                .collect(),
            created_at: policy.created_at,
            updated_at: policy.updated_at,
        }
    }
}

// ============================================================================
// Bundle Types
// ============================================================================

/// Request to create a bundle from a policy
#[derive(Debug, Deserialize)]
pub struct CreateBundleRequest {
    pub policy_id: String,
    pub version: String,
    pub description: Option<String>,
}

/// Bundle response
#[derive(Debug, Serialize)]
pub struct BundleResponse {
    pub bundle_id: String,
    pub policy_id: String,
    pub version: String,
    pub size_bytes: usize,
    pub created_at: DateTime<Utc>,
}

/// Request to deploy bundle to agents
#[derive(Debug, Deserialize)]
pub struct DeployBundleToAgentsRequest {
    pub bundle_id: String, // Bundle ID to deploy
    #[allow(dead_code)]
    #[serde(default)]
    pub agent_ids: Vec<String>, // If empty, deploy to all agents
    #[allow(dead_code)]
    #[serde(default)]
    pub force: bool,
}

/// Deployment result per agent
#[derive(Debug, Serialize)]
pub struct AgentDeploymentResult {
    pub agent_id: String,
    pub agent_url: String,
    pub success: bool,
    pub message: String,
    pub deployed_version: Option<String>,
}

/// Bundle deployment response
#[derive(Debug, Serialize)]
pub struct DeployBundleToAgentsResponse {
    pub bundle_id: String,
    pub total_agents: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<AgentDeploymentResult>,
}
