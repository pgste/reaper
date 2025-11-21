//! Policy Evaluator Trait and Implementations
//!
//! This module provides a pluggable architecture for different policy languages.
//! Each evaluator implements the PolicyEvaluator trait, allowing Reaper to support
//! multiple policy languages while maintaining consistent performance guarantees.

use crate::{PolicyAction, PolicyRequest};
use reaper_core::ReaperError;
use std::fmt::Debug;

pub mod simple;
pub mod cedar;
pub mod cedar_integration;

pub use simple::SimplePolicyEvaluator;
pub use cedar::CedarPolicyEvaluator;
pub use cedar_integration::datastore_to_cedar_entities;

/// Core trait for policy evaluation across different languages
///
/// This trait enables pluggable policy languages while maintaining
/// Reaper's performance guarantees. Implementations should:
/// - Be thread-safe (Send + Sync)
/// - Optimize for sub-microsecond evaluation where possible
/// - Provide validation before deployment
/// - Handle errors gracefully
pub trait PolicyEvaluator: Send + Sync + Debug {
    /// Evaluate a policy request and return a decision
    ///
    /// # Performance
    /// Implementations should target sub-microsecond latency for hot paths.
    /// Complex policies may take longer but should still be optimized.
    ///
    /// # Arguments
    /// * `request` - The policy request containing resource, action, and context
    ///
    /// # Returns
    /// * `Ok(PolicyAction)` - The authorization decision
    /// * `Err(ReaperError)` - If evaluation fails
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError>;

    /// Validate the policy before deployment
    ///
    /// This is called during policy deployment to catch errors early.
    /// Implementations should check syntax, semantics, and any other
    /// constraints specific to the policy language.
    ///
    /// # Returns
    /// * `Ok(())` - Policy is valid
    /// * `Err(ReaperError)` - Validation failed with details
    fn validate(&self) -> Result<(), ReaperError>;

    /// Get a human-readable name for this evaluator type
    ///
    /// Used for logging, metrics, and debugging.
    fn evaluator_type(&self) -> &str;

    /// Optional: Get metadata about the policy for monitoring
    ///
    /// Returns information like rule count, complexity metrics, etc.
    /// Default implementation returns None.
    fn metadata(&self) -> Option<EvaluatorMetadata> {
        None
    }
}

/// Metadata about a policy evaluator for monitoring and debugging
#[derive(Debug, Clone)]
pub struct EvaluatorMetadata {
    /// Number of rules/statements in the policy
    pub rule_count: usize,
    /// Estimated complexity (0-100 scale)
    pub complexity: u8,
    /// Additional context-specific metadata
    pub extra: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // Test helper: mock evaluator
    #[derive(Debug)]
    struct MockEvaluator {
        decision: PolicyAction,
    }

    impl PolicyEvaluator for MockEvaluator {
        fn evaluate(&self, _request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
            Ok(self.decision.clone())
        }

        fn validate(&self) -> Result<(), ReaperError> {
            Ok(())
        }

        fn evaluator_type(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn test_mock_evaluator() {
        let evaluator = MockEvaluator {
            decision: PolicyAction::Allow,
        };

        let request = PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let result = evaluator.evaluate(&request).unwrap();
        assert!(matches!(result, PolicyAction::Allow));
    }
}
