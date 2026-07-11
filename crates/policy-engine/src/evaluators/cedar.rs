//! Cedar Policy Language Evaluator
//!
//! Integrates AWS Cedar policy language for expressive, schema-validated authorization.
//! Cedar provides rich policy expression while maintaining good performance.
//!
//! Learn more: https://www.cedarpolicy.com/

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::{PolicyAction, PolicyRequest};
use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityTypeName, EntityUid, PolicySet, Request,
    RestrictedExpression,
};
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Cedar policy evaluator providing AWS-compatible authorization
///
/// Cedar is a purpose-built authorization policy language with:
/// - Schema validation
/// - Rich conditions and operators
/// - Attribute-based access control (ABAC)
/// - Auditable policy decisions
///
/// # Performance
/// Cedar evaluation is typically 10-50 microseconds, slower than SimplePolicyEvaluator
/// but acceptable for most use cases. The tradeoff is much richer policy expression.
///
/// # Example Cedar Policy
/// ```cedar
/// permit(
///     principal == User::"alice",
///     action == Action::"read",
///     resource in Folder::"documents"
/// ) when {
///     context.ip_address.isInRange("10.0.0.0/8")
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CedarPolicyEvaluator {
    /// Raw Cedar policy text
    policy_text: String,

    /// Compiled Cedar policy set
    /// We store this as a String for serialization, but parse it on demand
    /// In production, you might cache the parsed PolicySet
    #[serde(skip)]
    #[allow(dead_code)]
    cached_policy_set: Option<PolicySet>,
}

impl CedarPolicyEvaluator {
    /// Create a new Cedar evaluator from policy text
    ///
    /// # Arguments
    /// * `policy_text` - Cedar policy syntax string
    ///
    /// # Example
    /// ```text
    /// let policy = r#"
    ///     permit(principal, action == Action::"read", resource);
    /// "#;
    /// let evaluator = CedarPolicyEvaluator::new(policy.to_string())?;
    /// ```
    pub fn new(policy_text: String) -> Result<Self, ReaperError> {
        let evaluator = Self {
            policy_text,
            cached_policy_set: None,
        };

        // Validate and cache on creation
        evaluator.validate()?;

        Ok(evaluator)
    }

    /// Get or create the cached policy set
    #[allow(dead_code)]
    fn get_policy_set(&self) -> Result<&PolicySet, ReaperError> {
        // In a real implementation, we'd use interior mutability (RefCell/Mutex)
        // For now, we'll re-parse each time
        // TODO: Add caching with lazy_static or once_cell
        Err(ReaperError::EvaluationError {
            reason: "Policy set access requires mutable reference for caching".to_string(),
        })
    }

    /// Parse the policy text into a Cedar PolicySet
    fn parse_policy_set(&self) -> Result<PolicySet, ReaperError> {
        PolicySet::from_str(&self.policy_text).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to parse Cedar policy: {}", e),
        })
    }

    /// Convert Reaper PolicyRequest to Cedar Request
    ///
    /// Maps our generic request structure to Cedar's typed request format.
    /// This is where we define the entity model for Cedar.
    fn convert_request(&self, request: &PolicyRequest) -> Result<Request, ReaperError> {
        // Create entity UIDs for principal, action, and resource
        // For now, we'll use a simple mapping:
        // - Principal: User::"<from context or anonymous>"
        // - Action: Action::"<action>"
        // - Resource: Resource::"<resource>"

        let principal_id = request
            .context
            .get("principal")
            .cloned()
            .unwrap_or_else(|| "anonymous".to_string());

        let principal = EntityUid::from_type_name_and_id(
            EntityTypeName::from_str("User").map_err(|e| ReaperError::EvaluationError {
                reason: format!("Invalid principal type: {}", e),
            })?,
            principal_id
                .parse()
                .map_err(|e| ReaperError::EvaluationError {
                    reason: format!("Invalid principal ID: {}", e),
                })?,
        );

        let action = EntityUid::from_type_name_and_id(
            EntityTypeName::from_str("Action").map_err(|e| ReaperError::EvaluationError {
                reason: format!("Invalid action type: {}", e),
            })?,
            request
                .action
                .parse()
                .map_err(|e| ReaperError::EvaluationError {
                    reason: format!("Invalid action ID: {}", e),
                })?,
        );

        let resource = EntityUid::from_type_name_and_id(
            EntityTypeName::from_str("Resource").map_err(|e| ReaperError::EvaluationError {
                reason: format!("Invalid resource type: {}", e),
            })?,
            request
                .resource
                .parse()
                .map_err(|e| ReaperError::EvaluationError {
                    reason: format!("Invalid resource ID: {}", e),
                })?,
        );

        // Build context from request.context
        let mut context_map = HashMap::new();
        for (key, value) in &request.context {
            if key != "principal" {
                // Skip principal as it's handled separately
                // Convert string value to RestrictedExpression
                // For now, we'll treat everything as strings
                // In production, you'd want proper type conversion
                let expr =
                    RestrictedExpression::from_str(&format!("\"{}\"", value)).map_err(|e| {
                        ReaperError::EvaluationError {
                            reason: format!("Failed to convert context value for {}: {}", key, e),
                        }
                    })?;
                context_map.insert(key.clone(), expr);
            }
        }

        let context =
            Context::from_pairs(context_map).map_err(|e| ReaperError::EvaluationError {
                reason: format!("Failed to build context: {}", e),
            })?;

        // Build the Cedar request
        Request::new(principal, action, resource, context, None).map_err(|e| {
            ReaperError::EvaluationError {
                reason: format!("Failed to build Cedar request: {}", e),
            }
        })
    }

    /// Convert Cedar Decision to PolicyAction
    fn convert_decision(decision: Decision) -> PolicyAction {
        match decision {
            Decision::Allow => PolicyAction::Allow,
            Decision::Deny => PolicyAction::Deny,
        }
    }
}

impl PolicyEvaluator for CedarPolicyEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        // Parse policy set (in production, use cached version)
        let policy_set = self.parse_policy_set()?;

        // Convert request to Cedar format
        let cedar_request = self.convert_request(request)?;

        // Create empty entity store (in production, you'd populate this with actual entities)
        let entities = Entities::empty();

        // Create authorizer and evaluate
        let authorizer = Authorizer::new();
        let response = authorizer.is_authorized(&cedar_request, &policy_set, &entities);

        // Convert decision
        Ok(Self::convert_decision(response.decision()))
    }

    fn evaluate_matched(
        &self,
        request: &PolicyRequest,
    ) -> Result<(PolicyAction, bool), ReaperError> {
        let policy_set = self.parse_policy_set()?;
        let cedar_request = self.convert_request(request)?;
        let entities = Entities::empty();
        let authorizer = Authorizer::new();
        let response = authorizer.is_authorized(&cedar_request, &policy_set, &entities);

        // Cedar reaches an *implicit* default deny when no policy determined the
        // outcome — `diagnostics().reason()` is then empty. Any determining
        // policy (allow or forbid) makes the decision decisive for set-level
        // combination (Plan 08 Phase A).
        let matched = response.diagnostics().reason().next().is_some();
        Ok((Self::convert_decision(response.decision()), matched))
    }

    fn validate(&self) -> Result<(), ReaperError> {
        // Attempt to parse the policy
        let policy_set = self.parse_policy_set()?;

        // Check that we have at least one policy
        if policy_set.policies().count() == 0 {
            return Err(ReaperError::InvalidPolicy {
                reason: "Cedar policy set is empty".to_string(),
            });
        }

        // Additional validation could go here:
        // - Schema validation
        // - Policy conflicts
        // - etc.

        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "cedar"
    }

    fn metadata(&self) -> Option<EvaluatorMetadata> {
        // Try to parse policy set to get metadata
        if let Ok(policy_set) = self.parse_policy_set() {
            let policy_count = policy_set.policies().count();

            let mut extra = std::collections::HashMap::new();
            extra.insert("policy_count".to_string(), policy_count.to_string());
            extra.insert(
                "policy_length".to_string(),
                self.policy_text.len().to_string(),
            );

            Some(EvaluatorMetadata {
                rule_count: policy_count,
                complexity: ((policy_count * 10).min(100) as u8), // Rough complexity estimate
                extra,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_cedar_evaluator_simple_allow() {
        let policy = r#"
            permit(principal, action, resource);
        "#;

        let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "document1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_cedar_evaluator_deny() {
        let policy = r#"
            forbid(principal, action, resource);
        "#;

        let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "bob".to_string());

        let request = PolicyRequest {
            resource: "document1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny));
    }

    #[test]
    fn test_cedar_validation_invalid_syntax() {
        let policy = r#"
            this is not valid cedar syntax
        "#;

        let result = CedarPolicyEvaluator::new(policy.to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_cedar_validation_empty() {
        let policy = "";
        let result = CedarPolicyEvaluator::new(policy.to_string());
        // Empty policy might parse but should fail validation
        if let Ok(evaluator) = result {
            assert!(evaluator.validate().is_err());
        }
    }

    #[test]
    fn test_cedar_metadata() {
        let policy = r#"
            permit(principal, action == Action::"read", resource);
        "#;

        let evaluator = CedarPolicyEvaluator::new(policy.to_string()).unwrap();
        let metadata = evaluator.metadata().unwrap();

        assert!(metadata.rule_count > 0);
        assert_eq!(evaluator.evaluator_type(), "cedar");
    }
}
