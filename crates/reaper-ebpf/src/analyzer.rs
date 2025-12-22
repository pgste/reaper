//! Policy condition analyzer for eBPF promotability
//!
//! Analyzes policy conditions to determine if they can be promoted to eBPF kernel space
//! and estimates their performance characteristics.

use policy_engine::reap::{
    ComparisonLeft, ComparisonRight, EntityAttr, Operator, ReapCondition, ReapValue,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Result of analyzing a policy condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Whether this condition can be promoted to eBPF
    pub promotable: bool,

    /// Complexity score (0-10, higher = more complex)
    pub complexity: u8,

    /// Estimated latency in nanoseconds
    pub estimated_latency_ns: u32,

    /// Required entity lookups
    pub entity_lookups: Vec<String>,

    /// Reasons why it cannot be promoted (if promotable = false)
    pub blocking_reasons: Vec<String>,

    /// Warnings (non-blocking issues)
    pub warnings: Vec<String>,

    /// Recommendations for optimization
    pub recommendations: Vec<String>,

    /// Detected patterns (JWT, RBAC, ABAC, ReBAC)
    pub patterns: HashSet<AccessPattern>,
}

/// Access control patterns detected in policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessPattern {
    /// JWT claim validation
    Jwt,
    /// Role-based access control
    Rbac,
    /// Attribute-based access control
    Abac,
    /// Relationship-based access control
    Rebac,
}

impl AnalysisResult {
    /// Create a new analysis result (default: not promotable)
    pub fn new() -> Self {
        Self {
            promotable: false,
            complexity: 0,
            estimated_latency_ns: 0,
            entity_lookups: Vec::new(),
            blocking_reasons: Vec::new(),
            warnings: Vec::new(),
            recommendations: Vec::new(),
            patterns: HashSet::new(),
        }
    }

    /// Mark as promotable
    pub fn set_promotable(&mut self, promotable: bool) {
        self.promotable = promotable;
    }

    /// Add a blocking reason
    pub fn add_blocking_reason(&mut self, reason: String) {
        self.blocking_reasons.push(reason);
        self.promotable = false;
    }

    /// Add a warning
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// Add a recommendation
    pub fn add_recommendation(&mut self, recommendation: String) {
        self.recommendations.push(recommendation);
    }

    /// Add an entity lookup
    pub fn add_entity_lookup(&mut self, lookup: String) {
        if !self.entity_lookups.contains(&lookup) {
            self.entity_lookups.push(lookup);
        }
    }

    /// Add a detected pattern
    pub fn add_pattern(&mut self, pattern: AccessPattern) {
        self.patterns.insert(pattern);
    }
}

impl Default for AnalysisResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Policy condition analyzer
pub struct ConditionAnalyzer {
    /// Maximum complexity allowed for eBPF promotion (default: 5)
    max_complexity: u8,

    /// Maximum entity lookups allowed (default: 3)
    max_lookups: usize,
}

impl ConditionAnalyzer {
    /// Create a new analyzer with default settings
    pub fn new() -> Self {
        Self {
            max_complexity: 5,
            max_lookups: 3,
        }
    }

    /// Set maximum complexity threshold
    pub fn with_max_complexity(mut self, max_complexity: u8) -> Self {
        self.max_complexity = max_complexity;
        self
    }

    /// Set maximum entity lookups
    pub fn with_max_lookups(mut self, max_lookups: usize) -> Self {
        self.max_lookups = max_lookups;
        self
    }

    /// Analyze a policy condition
    pub fn analyze(&self, condition: &ReapCondition) -> AnalysisResult {
        let mut result = AnalysisResult::new();

        match condition {
            ReapCondition::True => {
                // Always allow - trivial case
                result.set_promotable(true);
                result.complexity = 0;
                result.estimated_latency_ns = 10; // ~10ns for constant
            }
            ReapCondition::False => {
                // Always deny - trivial case
                result.set_promotable(true);
                result.complexity = 0;
                result.estimated_latency_ns = 10;
            }
            ReapCondition::Comparison { left, op, right } => {
                self.analyze_comparison(left, op, right, &mut result);
            }
            ReapCondition::And(conditions) => {
                self.analyze_and(conditions, &mut result);
            }
            ReapCondition::Or(conditions) => {
                self.analyze_or(conditions, &mut result);
            }
            ReapCondition::Not(inner) => {
                let inner_result = self.analyze(inner);
                result.promotable = inner_result.promotable;
                result.complexity = inner_result.complexity + 1;
                result.estimated_latency_ns = inner_result.estimated_latency_ns + 5;
                result.entity_lookups = inner_result.entity_lookups.clone();
                result.blocking_reasons = inner_result.blocking_reasons.clone();
                result.patterns = inner_result.patterns.clone();
            }
            ReapCondition::Assignment { .. } => {
                result
                    .add_blocking_reason("Variable assignments not supported in eBPF".to_string());
            }
            ReapCondition::Expr(_) => {
                result.add_blocking_reason(
                    "Expression conditions not yet supported in eBPF".to_string(),
                );
            }
        }

        result
    }

    /// Analyze AND conditions
    fn analyze_and(&self, conditions: &[ReapCondition], result: &mut AnalysisResult) {
        // Analyze each condition
        let mut total_latency = 0u32;
        let mut max_complexity = 0u8;
        let mut all_promotable = true;

        for cond in conditions {
            let cond_result = self.analyze(cond);

            if !cond_result.promotable {
                all_promotable = false;
            }

            max_complexity = max_complexity.max(cond_result.complexity);
            total_latency += cond_result.estimated_latency_ns;

            // Merge entity lookups
            for lookup in &cond_result.entity_lookups {
                result.add_entity_lookup(lookup.clone());
            }

            // Merge blocking reasons
            result.blocking_reasons.extend(cond_result.blocking_reasons);

            // Merge patterns
            result.patterns.extend(cond_result.patterns);
        }

        result.promotable = all_promotable;
        result.complexity = max_complexity + 1;
        result.estimated_latency_ns = total_latency;

        // Check limits
        if result.complexity > self.max_complexity {
            result.add_blocking_reason(format!(
                "Complexity {} exceeds max {}",
                result.complexity, self.max_complexity
            ));
        }

        if result.entity_lookups.len() > self.max_lookups {
            result.add_blocking_reason(format!(
                "{} entity lookups exceeds max {}",
                result.entity_lookups.len(),
                self.max_lookups
            ));
        }
    }

    /// Analyze OR conditions
    fn analyze_or(&self, conditions: &[ReapCondition], result: &mut AnalysisResult) {
        let mut max_latency = 0u32;
        let mut max_complexity = 0u8;
        let mut all_promotable = true;

        for cond in conditions {
            let cond_result = self.analyze(cond);

            if !cond_result.promotable {
                all_promotable = false;
            }

            max_complexity = max_complexity.max(cond_result.complexity);
            max_latency = max_latency.max(cond_result.estimated_latency_ns);

            for lookup in &cond_result.entity_lookups {
                result.add_entity_lookup(lookup.clone());
            }

            result.blocking_reasons.extend(cond_result.blocking_reasons);
            result.patterns.extend(cond_result.patterns);
        }

        result.promotable = all_promotable;
        result.complexity = max_complexity + 2; // OR is more complex
        result.estimated_latency_ns = max_latency + 20; // Branch overhead
    }

    /// Analyze a comparison
    fn analyze_comparison(
        &self,
        left: &ComparisonLeft,
        op: &Operator,
        right: &ComparisonRight,
        result: &mut AnalysisResult,
    ) {
        // Count entity accesses
        let mut lookups = 0;

        // Extract from left side
        match left {
            ComparisonLeft::EntityAttr(entity_attr) => {
                lookups += 1;
                let lookup_key = format!("{:?}.{}", entity_attr.entity, entity_attr.attribute);
                result.add_entity_lookup(lookup_key);
                self.detect_pattern_from_attr(entity_attr, result);
            }
            ComparisonLeft::VarAttr(_) => {
                result.add_blocking_reason("Variable attributes not supported in eBPF".to_string());
                return;
            }
            ComparisonLeft::Expr(_) => {
                result.add_blocking_reason("Expressions not supported in eBPF".to_string());
                return;
            }
        }

        // Extract from right side
        match right {
            ComparisonRight::Value(value) => {
                // Check for IN operator with array
                if matches!(op, Operator::In) {
                    if let ReapValue::Array(arr) = value {
                        if arr.len() <= 64 {
                            result.add_pattern(AccessPattern::Rbac);
                        } else {
                            result.add_blocking_reason(format!(
                                "IN array too large ({} items, max 64)",
                                arr.len()
                            ));
                            return;
                        }
                    }
                }
            }
            ComparisonRight::EntityAttr(entity_attr) => {
                lookups += 1;
                let lookup_key = format!("{:?}.{}", entity_attr.entity, entity_attr.attribute);
                result.add_entity_lookup(lookup_key);
                self.detect_pattern_from_attr(entity_attr, result);
            }
            ComparisonRight::Variable(_) => {
                result.add_blocking_reason("Variables not supported in eBPF".to_string());
                return;
            }
            ComparisonRight::VarAttr(_) => {
                result.add_blocking_reason("Variable attributes not supported in eBPF".to_string());
                return;
            }
            ComparisonRight::Expr(_) => {
                result.add_blocking_reason("Expressions not supported in eBPF".to_string());
                return;
            }
        }

        // Detect ABAC patterns from operator
        if matches!(
            op,
            Operator::GreaterThan
                | Operator::LessThan
                | Operator::GreaterEqual
                | Operator::LessEqual
        ) {
            result.add_pattern(AccessPattern::Abac);
        }

        // Simple comparison: 50ns per lookup
        result.estimated_latency_ns = 50 * lookups as u32;
        result.complexity = lookups.min(10) as u8;

        // Allow up to 3 entity accesses
        if lookups <= 3 {
            result.set_promotable(true);
        } else {
            result.add_blocking_reason(format!(
                "Too many entity accesses ({}) in comparison",
                lookups
            ));
        }
    }

    /// Detect access control patterns from entity attribute
    fn detect_pattern_from_attr(&self, attr: &EntityAttr, result: &mut AnalysisResult) {
        // JWT pattern detection
        if attr.attribute == "exp"
            || attr.attribute == "iat"
            || attr.attribute == "nbf"
            || attr.attribute == "sub"
        {
            result.add_pattern(AccessPattern::Jwt);
        }

        // RBAC pattern detection
        if attr.attribute == "role" || attr.attribute == "roles" {
            result.add_pattern(AccessPattern::Rbac);
        }

        // ReBAC pattern detection
        if attr.attribute == "owner" || attr.attribute == "parent" || attr.attribute == "group" {
            result.add_pattern(AccessPattern::Rebac);
        }
    }
}

impl Default for ConditionAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy_engine::reap::{AssignmentValue, Entity};

    #[test]
    fn test_trivial_conditions() {
        let analyzer = ConditionAnalyzer::new();

        // Always allow
        let result = analyzer.analyze(&ReapCondition::True);
        assert!(result.promotable);
        assert_eq!(result.complexity, 0);
        assert_eq!(result.estimated_latency_ns, 10);

        // Always deny
        let result = analyzer.analyze(&ReapCondition::False);
        assert!(result.promotable);
        assert_eq!(result.complexity, 0);
    }

    #[test]
    fn test_simple_equality() {
        let analyzer = ConditionAnalyzer::new();

        // user.role == "admin"
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::Equal,
            right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
        };

        let result = analyzer.analyze(&condition);
        assert!(result.promotable, "Simple equality should be promotable");
        assert_eq!(result.complexity, 1);
        assert_eq!(result.estimated_latency_ns, 50);
        assert!(result.patterns.contains(&AccessPattern::Rbac));
        assert_eq!(result.entity_lookups.len(), 1);
    }

    #[test]
    fn test_jwt_expiration_check() {
        let analyzer = ConditionAnalyzer::new();

        // user.exp >= 1735689600
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "exp".to_string(),
                index: None,
            }),
            op: Operator::GreaterEqual,
            right: ComparisonRight::Value(ReapValue::Integer(1735689600)),
        };

        let result = analyzer.analyze(&condition);
        assert!(result.promotable);
        assert!(result.patterns.contains(&AccessPattern::Jwt));
        assert!(result.patterns.contains(&AccessPattern::Abac));
    }

    #[test]
    fn test_abac_comparison() {
        let analyzer = ConditionAnalyzer::new();

        // user.clearance >= resource.min_clearance
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "clearance".to_string(),
                index: None,
            }),
            op: Operator::GreaterEqual,
            right: ComparisonRight::EntityAttr(EntityAttr {
                entity: Entity::Resource,
                attribute: "min_clearance".to_string(),
                index: None,
            }),
        };

        let result = analyzer.analyze(&condition);
        assert!(result.promotable);
        assert_eq!(result.complexity, 2);
        assert_eq!(result.estimated_latency_ns, 100); // 2 lookups
        assert!(result.patterns.contains(&AccessPattern::Abac));
        assert_eq!(result.entity_lookups.len(), 2);
    }

    #[test]
    fn test_in_expression_small_list() {
        let analyzer = ConditionAnalyzer::new();

        // user.role IN ["admin", "manager"]
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::In,
            right: ComparisonRight::Value(ReapValue::Array(vec![
                ReapValue::String("admin".to_string()),
                ReapValue::String("manager".to_string()),
            ])),
        };

        let result = analyzer.analyze(&condition);
        assert!(result.promotable);
        assert!(result.patterns.contains(&AccessPattern::Rbac));
    }

    #[test]
    fn test_in_expression_large_list() {
        let analyzer = ConditionAnalyzer::new();

        // Create list with > 64 items
        let mut list = Vec::new();
        for i in 0..100 {
            list.push(ReapValue::String(format!("role{}", i)));
        }

        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::In,
            right: ComparisonRight::Value(ReapValue::Array(list)),
        };

        let result = analyzer.analyze(&condition);
        assert!(!result.promotable);
        assert!(!result.blocking_reasons.is_empty());
    }

    #[test]
    fn test_and_condition() {
        let analyzer = ConditionAnalyzer::new();

        // user.role == "admin" AND user.dept == "engineering"
        let condition = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "dept".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
        ]);

        let result = analyzer.analyze(&condition);
        assert!(result.promotable);
        assert_eq!(result.complexity, 2);
        assert_eq!(result.estimated_latency_ns, 100);
        assert_eq!(result.entity_lookups.len(), 2);
    }

    #[test]
    fn test_complexity_limit() {
        let analyzer = ConditionAnalyzer::new().with_max_complexity(2);

        // Create nested AND of AND conditions to exceed complexity limit
        let inner_and = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "dept".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("eng".to_string())),
            },
        ]); // complexity: 1 (comparisons) + 1 (AND) = 2

        // Wrap in another AND, which will push complexity to 3 (exceeds limit of 2)
        let condition = ReapCondition::And(vec![inner_and, ReapCondition::True]);

        let result = analyzer.analyze(&condition);
        assert!(!result.promotable, "Complexity 3 should exceed limit of 2");
        assert!(!result.blocking_reasons.is_empty());
        assert!(result
            .blocking_reasons
            .iter()
            .any(|r| r.contains("Complexity")));
    }

    #[test]
    fn test_assignment_not_supported() {
        let analyzer = ConditionAnalyzer::new();

        let condition = ReapCondition::Assignment {
            variable: "x".to_string(),
            value: AssignmentValue::Value(ReapValue::String("test".to_string())),
        };

        let result = analyzer.analyze(&condition);
        assert!(!result.promotable);
        assert!(result
            .blocking_reasons
            .iter()
            .any(|r| r.contains("assignments")));
    }
}
