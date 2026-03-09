//! Decision Tree Optimization for O(log r) Policy Evaluation
//!
//! Compiles policy rules into an optimized decision tree structure that enables
//! logarithmic-time evaluation regardless of the number of rules.
//!
//! # Algorithm
//!
//! 1. **Build Phase:**
//!    - Analyze all policy rules
//!    - Determine optimal attribute ordering (most selective first)
//!    - Build binary decision tree
//!    - Optimize tree structure (prune, compress paths)
//!
//! 2. **Evaluation Phase:**
//!    - Start at tree root
//!    - Navigate based on request attributes: O(log r)
//!    - Return decision at leaf node
//!
//! # Performance
//!
//! - Tree construction: O(r * log r) where r = rule count
//! - Evaluation: O(log r) - logarithmic in rule count
//! - Memory: O(r) - linear in rule count
//!
//! # Example
//!
//! ```text
//! let builder = DecisionTreeBuilder::new();
//! let tree = builder.build_from_rules(&policy_rules)?;
//!
//! // Evaluate in O(log r) time
//! let decision = tree.evaluate(&request, &store)?;
//! ```

use crate::data::DataStore;
use crate::engine::{PolicyAction, PolicyDecision, PolicyRequest, PolicyRule};
use reaper_core::{PolicyId, ReaperError};
use std::collections::HashMap;
use std::sync::Arc;

/// Decision tree for optimized policy evaluation
///
/// Provides O(log r) evaluation time regardless of policy size.
#[derive(Debug, Clone)]
pub struct DecisionTree {
    root: Arc<TreeNode>,
    stats: TreeStats,
    rule_count: usize,
}

/// Statistics about the decision tree
#[derive(Debug, Clone)]
pub struct TreeStats {
    /// Total number of nodes in the tree
    pub node_count: usize,
    /// Maximum depth of the tree
    pub max_depth: usize,
    /// Number of decision (leaf) nodes
    pub decision_count: usize,
    /// Number of attribute check (branch) nodes
    pub branch_count: usize,
}

/// A node in the decision tree
#[derive(Debug, Clone)]
pub enum TreeNode {
    /// Leaf node containing a decision
    Decision {
        action: PolicyAction,
        rule_name: Option<String>,
    },

    /// Branch node that checks an attribute
    AttributeCheck {
        /// Attribute to check (e.g., "role", "resource")
        attribute: String,

        /// Branches for specific values
        branches: HashMap<String, Arc<TreeNode>>,

        /// Default branch if no match
        default: Arc<TreeNode>,

        /// Selectivity score (0.0 - 1.0, higher = more selective)
        selectivity: f64,
    },
}

impl DecisionTree {
    /// Create a new decision tree from a root node with rule count
    pub fn new(root: Arc<TreeNode>, rule_count: usize) -> Self {
        let stats = Self::calculate_stats(&root);
        Self {
            root,
            stats,
            rule_count,
        }
    }

    /// Evaluate a request against the decision tree (full metadata version)
    ///
    /// # Performance
    /// O(log r) where r = number of rules
    pub fn evaluate(
        &self,
        request: &PolicyRequest,
        policy_id: PolicyId,
        policy_version: u64,
        _store: &DataStore,
    ) -> Result<PolicyDecision, ReaperError> {
        let start = std::time::Instant::now();
        let result = self.traverse(&self.root, request, &mut HashMap::new());
        let evaluation_time_ns = start.elapsed().as_nanos() as u64;

        result.map(|(decision, matched_rule)| PolicyDecision {
            decision,
            policy_id,
            policy_name: String::new(),
            policy_version,
            evaluation_time_ns,
            matched_rule,
        })
    }

    /// Evaluate a request and return just the action and matched rule
    ///
    /// Simpler API for use in evaluators that don't track policy metadata.
    ///
    /// # Performance
    /// O(log r) where r = number of rules
    pub fn evaluate_simple(
        &self,
        request: &PolicyRequest,
        _store: &DataStore,
    ) -> Result<(PolicyAction, Option<usize>), ReaperError> {
        self.traverse(&self.root, request, &mut HashMap::new())
    }

    /// Traverse the tree to find the decision
    fn traverse(
        &self,
        node: &TreeNode,
        request: &PolicyRequest,
        context: &mut HashMap<String, String>,
    ) -> Result<(PolicyAction, Option<usize>), ReaperError> {
        match node {
            TreeNode::Decision { action, .. } => {
                // Return the action and None for matched_rule (can be enhanced later)
                Ok((action.clone(), None))
            }

            TreeNode::AttributeCheck {
                attribute,
                branches,
                default,
                ..
            } => {
                // Extract attribute value from request or context
                let attr_value = self.extract_attribute(request, attribute, context);

                // Try to find matching branch
                if let Some(value) = attr_value {
                    if let Some(branch) = branches.get(&value) {
                        return self.traverse(branch, request, context);
                    }
                }

                // Fall back to default branch
                self.traverse(default, request, context)
            }
        }
    }

    /// Extract attribute value from request
    fn extract_attribute(
        &self,
        request: &PolicyRequest,
        attribute: &str,
        context: &HashMap<String, String>,
    ) -> Option<String> {
        // Check request fields
        match attribute {
            "action" => Some(request.action.clone()),
            "resource" => Some(request.resource.clone()),
            "principal" => request.context.get("principal").cloned(),
            _ => {
                // Check context
                context
                    .get(attribute)
                    .cloned()
                    .or_else(|| request.context.get(attribute).cloned())
            }
        }
    }

    /// Calculate tree statistics
    fn calculate_stats(node: &TreeNode) -> TreeStats {
        let mut stats = TreeStats {
            node_count: 0,
            max_depth: 0,
            decision_count: 0,
            branch_count: 0,
        };

        Self::calculate_stats_recursive(node, &mut stats, 0);
        stats
    }

    fn calculate_stats_recursive(node: &TreeNode, stats: &mut TreeStats, depth: usize) {
        stats.node_count += 1;
        stats.max_depth = stats.max_depth.max(depth);

        match node {
            TreeNode::Decision { .. } => {
                stats.decision_count += 1;
            }
            TreeNode::AttributeCheck {
                branches, default, ..
            } => {
                stats.branch_count += 1;
                for branch in branches.values() {
                    Self::calculate_stats_recursive(branch, stats, depth + 1);
                }
                Self::calculate_stats_recursive(default, stats, depth + 1);
            }
        }
    }

    /// Get tree statistics with rule count
    pub fn stats(&self) -> TreeStats {
        TreeStats {
            node_count: self.stats.node_count,
            max_depth: self.stats.max_depth,
            decision_count: self.stats.decision_count,
            branch_count: self.stats.branch_count,
        }
    }

    /// Get the number of rules compiled
    pub fn rule_count(&self) -> usize {
        self.rule_count
    }
}

/// Builder for constructing optimized decision trees
pub struct DecisionTreeBuilder {
    /// Minimum rules required to create a branch (otherwise use decision)
    min_split_size: usize,
}

impl DecisionTreeBuilder {
    /// Create a new decision tree builder
    pub fn new() -> Self {
        Self { min_split_size: 2 }
    }

    /// Build a decision tree from policy rules
    pub fn build_from_rules(&self, rules: &[PolicyRule]) -> Result<DecisionTree, ReaperError> {
        if rules.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "Cannot build decision tree from empty rule set".to_string(),
            });
        }

        let rule_count = rules.len();

        // Analyze rules to determine attribute selectivity
        let selectivity = self.analyze_selectivity(rules);

        // Build tree recursively
        let root = self.build_node(rules, &selectivity, 0)?;

        Ok(DecisionTree::new(Arc::new(root), rule_count))
    }

    /// Build a tree node recursively
    fn build_node(
        &self,
        rules: &[PolicyRule],
        selectivity: &HashMap<String, f64>,
        depth: usize,
    ) -> Result<TreeNode, ReaperError> {
        // Base case: single rule or too small to split
        if rules.len() <= self.min_split_size || depth > 20 {
            return Ok(self.create_decision_node(rules));
        }

        // Find best attribute to split on
        if let Some((best_attr, splits)) = self.find_best_split(rules, selectivity) {
            // Create branches for each value
            let mut branches = HashMap::new();

            for (value, subset) in splits {
                if !subset.is_empty() {
                    let branch = self.build_node(&subset, selectivity, depth + 1)?;
                    branches.insert(value, Arc::new(branch));
                }
            }

            // Create default branch with all rules
            let default = Arc::new(self.create_decision_node(rules));

            let attr_selectivity = selectivity.get(&best_attr).copied().unwrap_or(0.0);

            Ok(TreeNode::AttributeCheck {
                attribute: best_attr,
                branches,
                default,
                selectivity: attr_selectivity,
            })
        } else {
            // No good split found, create decision node
            Ok(self.create_decision_node(rules))
        }
    }

    /// Create a decision node from rules (first match wins)
    fn create_decision_node(&self, rules: &[PolicyRule]) -> TreeNode {
        if let Some(first_rule) = rules.first() {
            TreeNode::Decision {
                action: first_rule.action.clone(),
                rule_name: Some(format!("rule_{}", first_rule.resource)),
            }
        } else {
            TreeNode::Decision {
                action: PolicyAction::Deny,
                rule_name: Some("default_deny".to_string()),
            }
        }
    }

    /// Find the best attribute to split on
    fn find_best_split(
        &self,
        rules: &[PolicyRule],
        selectivity: &HashMap<String, f64>,
    ) -> Option<(String, HashMap<String, Vec<PolicyRule>>)> {
        let attributes = vec!["action", "resource", "principal"];

        let mut best_attr: Option<String> = None;
        let mut best_score = 0.0;
        let mut best_splits: Option<HashMap<String, Vec<PolicyRule>>> = None;

        for attr in attributes {
            let splits = self.partition_by_attribute(rules, attr);

            // Calculate split quality (entropy-based)
            let score = self.calculate_split_score(&splits, rules.len())
                * selectivity.get(attr).copied().unwrap_or(1.0);

            if score > best_score && splits.len() > 1 {
                best_score = score;
                best_attr = Some(attr.to_string());
                best_splits = Some(splits);
            }
        }

        best_attr.and_then(|attr| best_splits.map(|splits| (attr, splits)))
    }

    /// Partition rules by attribute value
    fn partition_by_attribute(
        &self,
        rules: &[PolicyRule],
        attribute: &str,
    ) -> HashMap<String, Vec<PolicyRule>> {
        let mut partitions: HashMap<String, Vec<PolicyRule>> = HashMap::new();

        for rule in rules {
            let value = match attribute {
                "action" => match &rule.action {
                    PolicyAction::Allow => "allow".to_string(),
                    PolicyAction::Deny => "deny".to_string(),
                    PolicyAction::Log => "log".to_string(),
                },
                "resource" => rule.resource.clone(),
                "principal" => "*".to_string(), // Simplified for now
                _ => continue,
            };

            partitions.entry(value).or_default().push(rule.clone());
        }

        partitions
    }

    /// Calculate split score (information gain)
    fn calculate_split_score(
        &self,
        splits: &HashMap<String, Vec<PolicyRule>>,
        total: usize,
    ) -> f64 {
        if total == 0 {
            return 0.0;
        }

        let mut score = 0.0;
        for subset in splits.values() {
            let proportion = subset.len() as f64 / total as f64;
            if proportion > 0.0 {
                score -= proportion * proportion.log2();
            }
        }

        score
    }

    /// Analyze attribute selectivity across rules
    fn analyze_selectivity(&self, rules: &[PolicyRule]) -> HashMap<String, f64> {
        let mut selectivity = HashMap::new();

        // Analyze each attribute
        for attr in &["action", "resource", "principal"] {
            let partitions = self.partition_by_attribute(rules, attr);
            let unique_values = partitions.len();

            // Higher selectivity = more unique values = better for splitting
            let score = if unique_values > 1 {
                (unique_values as f64).log2() / (rules.len() as f64).log2()
            } else {
                0.0
            };

            selectivity.insert(attr.to_string(), score);
        }

        selectivity
    }
}

impl Default for DecisionTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_rules() -> Vec<PolicyRule> {
        vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "admin_resource".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "user_resource".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Deny,
                resource: "*".to_string(),
                conditions: vec![],
            },
        ]
    }

    #[test]
    fn test_tree_builder_creation() {
        let builder = DecisionTreeBuilder::new();
        assert_eq!(builder.min_split_size, 2);
    }

    #[test]
    fn test_build_tree_from_rules() {
        let rules = create_test_rules();
        let builder = DecisionTreeBuilder::new();

        let tree = builder.build_from_rules(&rules).unwrap();

        assert!(tree.stats().node_count > 0);
        // max_depth is always >= 0 for a usize, so just verify tree was built
        assert_eq!(tree.rule_count(), 3);
    }

    #[test]
    fn test_tree_stats() {
        let rules = create_test_rules();
        let builder = DecisionTreeBuilder::new();
        let tree = builder.build_from_rules(&rules).unwrap();

        assert_eq!(tree.rule_count(), 3);
        assert!(tree.stats().node_count >= 3);
    }

    #[test]
    fn test_simple_evaluation() {
        let rules = vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "resource1".to_string(),
            conditions: vec![],
        }];

        let builder = DecisionTreeBuilder::new();
        let tree = builder.build_from_rules(&rules).unwrap();

        let request = PolicyRequest {
            resource: "resource1".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        let store = DataStore::new();
        let policy_id = Uuid::new_v4();
        let decision = tree.evaluate(&request, policy_id, 1, &store).unwrap();

        assert!(matches!(decision.decision, PolicyAction::Allow));
    }

    #[test]
    fn test_selectivity_analysis() {
        let rules = create_test_rules();
        let builder = DecisionTreeBuilder::new();

        let selectivity = builder.analyze_selectivity(&rules);

        // Should have selectivity scores for attributes
        assert!(selectivity.contains_key("action"));
        assert!(selectivity.contains_key("resource"));
    }

    #[test]
    fn test_partition_by_attribute() {
        let rules = create_test_rules();
        let builder = DecisionTreeBuilder::new();

        let partitions = builder.partition_by_attribute(&rules, "resource");

        // Should have multiple partitions
        assert!(partitions.len() > 1);
        assert!(partitions.contains_key("admin_resource"));
        assert!(partitions.contains_key("user_resource"));
    }

    #[test]
    fn test_empty_rules_error() {
        let builder = DecisionTreeBuilder::new();
        let result = builder.build_from_rules(&[]);

        assert!(result.is_err());
    }

    #[test]
    fn test_single_rule() {
        let rules = vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "test".to_string(),
            conditions: vec![],
        }];

        let builder = DecisionTreeBuilder::new();
        let tree = builder.build_from_rules(&rules).unwrap();

        assert_eq!(tree.rule_count(), 1);
    }

    #[test]
    fn test_deep_tree() {
        // Create many rules with different resources
        let mut rules = Vec::new();
        for i in 0..20 {
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

        let builder = DecisionTreeBuilder::new();
        let tree = builder.build_from_rules(&rules).unwrap();

        assert!(tree.stats().max_depth > 0);
        assert_eq!(tree.rule_count(), 20);
    }
}
