//! Reaper DSL - Native Policy Language
//!
//! A Rust-native policy language optimized for sub-microsecond evaluation.
//! Leverages DataStore directly for zero-copy, interned-string-based policies.

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::data::{AttributeValue, DataStore, Entity, InternedString};
use crate::{PolicyAction, PolicyRequest};
use reaper_core::ReaperError;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Reaper DSL Policy Evaluator
///
/// Performance characteristics:
/// - Simple rules: < 500 ns
/// - Complex ABAC: < 10 µs
/// - Entity lookups: 20-50 ns (DataStore direct)
/// - Comparisons: 5-10 ns (interned string IDs)
/// - Regex matches: ~100-500 ns (pre-compiled patterns)
///
/// Security characteristics:
/// - Deny-precedence evaluation: All deny rules evaluated before any allow rules
/// - Explicit denies cannot be bypassed by subsequent allows
/// - Rules are pre-partitioned at construction for zero-overhead evaluation
///
/// **Expected: 1,000-20,000x faster than Cedar**
#[derive(Debug, Clone)]
pub struct ReaperDSLEvaluator {
    /// Reference to the data store
    store: Arc<DataStore>,
    /// Deny rules (evaluated first for security)
    deny_rules: Vec<Rule>,
    /// Allow rules (evaluated after deny rules)
    allow_rules: Vec<Rule>,
    /// Default decision if no rules match
    default_decision: PolicyAction,
    /// Pre-compiled regex patterns for O(1) lookup during evaluation
    /// Uses FxHashMap for faster hashing (no DoS resistance needed for static patterns)
    regex_cache: Arc<FxHashMap<String, regex::Regex>>,
    /// Pre-interned strings cache for O(1) lookup during evaluation
    /// Caches attribute names and string literals to avoid repeated interning
    interned_cache: Arc<FxHashMap<String, InternedString>>,
    /// Pre-computed AttributeValue objects for membership tests
    /// Avoids allocating AttributeValue::String on every membership check
    membership_cache: Arc<FxHashMap<String, AttributeValue>>,
}

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Condition to evaluate
    pub condition: Condition,
    /// Decision if condition is true
    pub decision: PolicyAction,
}

/// Comparison operators for same-entity attribute comparisons
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttrCompareOp {
    /// ==
    Equal,
    /// !=
    NotEqual,
    /// <=
    LessEqual,
    /// >=
    GreaterEqual,
    /// <
    Less,
    /// >
    Greater,
}

/// Policy condition (compiled from YAML/DSL)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// Always true
    Always,
    /// Compare action to literal value
    ActionEquals { value: String },
    /// Compare resource ID to literal value (for simple resource matching)
    ResourceIdEquals { value: String },
    /// Compare user attribute to literal value
    UserEquals { attribute: String, value: String },
    /// Compare user int/float attribute >= literal value
    UserGreaterEqualLiteral { attribute: String, value: f64 },
    /// Compare user int/float attribute > literal value
    UserGreaterLiteral { attribute: String, value: f64 },
    /// Compare user int/float attribute <= literal value
    UserLessEqualLiteral { attribute: String, value: f64 },
    /// Compare user int/float attribute < literal value
    UserLessLiteral { attribute: String, value: f64 },
    /// Compare resource attribute to literal value
    ResourceEquals { attribute: String, value: String },
    /// Compare resource int/float attribute >= literal value
    ResourceGreaterEqualLiteral { attribute: String, value: f64 },
    /// Compare resource int/float attribute > literal value
    ResourceGreaterLiteral { attribute: String, value: f64 },
    /// Compare resource int/float attribute <= literal value
    ResourceLessEqualLiteral { attribute: String, value: f64 },
    /// Compare resource int/float attribute < literal value
    ResourceLessLiteral { attribute: String, value: f64 },
    /// Compare user attribute to resource attribute
    UserEqualsResource {
        user_attr: String,
        resource_attr: String,
    },
    /// Compare user int attribute > resource int attribute
    UserIntGreater {
        user_attr: String,
        resource_attr: String,
    },
    /// Compare resource int attribute > user int attribute
    ResourceIntGreater {
        resource_attr: String,
        user_attr: String,
    },
    /// Compare two attributes of the same entity: entity.attr1 op entity.attr2
    /// Works for User, Resource, or Context entities
    SameEntityAttrCompare {
        entity_type: EntityType,
        left_attr: String,
        right_attr: String,
        op: AttrCompareOp,
    },
    /// Variable assignment: x := user.role
    /// Stores the value in evaluation context for later use
    Assignment {
        variable: String,
        entity_type: EntityType,
        attribute: String,
        index: Option<IndexExpr>,
    },
    /// Check membership in array/set: "admin" in user.roles
    /// Optimized with HashSet lookup for O(1) performance
    MembershipTest {
        value: LiteralValue,
        entity_type: EntityType,
        attribute: String,
        index: Option<IndexExpr>,
    },
    /// Compare with bracket notation: user.roles[0] == "admin"
    IndexedEquals {
        entity_type: EntityType,
        attribute: String,
        index: IndexExpr,
        value: String,
    },
    /// Compare attribute with variable: user.role == role_var
    EqualsVariable {
        entity_type: EntityType,
        attribute: String,
        variable: String,
    },

    // ============ Function Call Support ============

    /// Regex match: regex::matches(user.email, "pattern")
    RegexMatches {
        entity_type: EntityType,
        attribute: String,
        pattern: String,
    },

    /// String contains: user.email.contains("@company.com")
    StringContains {
        entity_type: EntityType,
        attribute: String,
        substring: String,
    },

    /// String starts with: user.username.startswith("admin_")
    StringStartsWith {
        entity_type: EntityType,
        attribute: String,
        prefix: String,
    },

    /// String ends with: user.email.endswith(".gov")
    StringEndsWith {
        entity_type: EntityType,
        attribute: String,
        suffix: String,
    },

    /// Time is after: time::is_after(user.token_expires_at, threshold)
    TimeIsAfter {
        entity_type: EntityType,
        attribute: String,
        threshold: i64,
    },

    /// Time is before: time::is_before(user.expires_at, threshold)
    TimeIsBefore {
        entity_type: EntityType,
        attribute: String,
        threshold: i64,
    },

    /// Array/Set count comparison: user.skills.count() >= 5
    CountGreaterEqual {
        entity_type: EntityType,
        attribute: String,
        threshold: usize,
    },

    /// Array/Set count comparison: user.items.count() > 0
    CountGreater {
        entity_type: EntityType,
        attribute: String,
        threshold: usize,
    },

    /// Array/Set count comparison: user.items.count() == 5
    CountEqual {
        entity_type: EntityType,
        attribute: String,
        threshold: usize,
    },

    // ============ String Case Methods ============

    /// String lowercase comparison: user.name.lower() == "admin"
    StringLowerEquals {
        entity_type: EntityType,
        attribute: String,
        value: String,
    },

    /// String uppercase comparison: user.code.upper() == "ADMIN123"
    StringUpperEquals {
        entity_type: EntityType,
        attribute: String,
        value: String,
    },

    // ============ Type Check Functions ============

    /// Type check: is_string(entity.attr)
    IsString {
        entity_type: EntityType,
        attribute: String,
    },

    /// Type check: is_number(entity.attr)
    IsNumber {
        entity_type: EntityType,
        attribute: String,
    },

    /// Type check: is_bool(entity.attr)
    IsBool {
        entity_type: EntityType,
        attribute: String,
    },

    // ============ Set Operations ============

    /// Set intersection count: groups.intersection(["a", "b"]).count() > 0
    SetIntersectionCountGreater {
        entity_type: EntityType,
        attribute: String,
        values: Vec<String>,
        threshold: usize,
    },

    /// Map keys membership: "key" in metadata.keys()
    MapKeyExists {
        entity_type: EntityType,
        attribute: String,
        key: String,
    },

    // ============ Comprehension Support ============

    /// Array comprehension with filter and count: [x | x := arr[_]; x.active == true].count() >= N
    ComprehensionCountGreaterEqual {
        entity_type: EntityType,
        attribute: String,
        filter_attr: String,
        filter_value: LiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },

    /// Array comprehension count equals zero: [x | x := arr[_]; x.active == false].count() == 0
    ComprehensionCountEqual {
        entity_type: EntityType,
        attribute: String,
        filter_attr: String,
        filter_value: LiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },

    /// AND of multiple conditions
    And(Vec<Condition>),
    /// OR of multiple conditions
    Or(Vec<Condition>),
    /// NOT of a condition
    Not(Box<Condition>),
}

/// Filter operation for comprehensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComprehensionFilterOp {
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterEqual,
    LessEqual,
    Contains,
}

/// Entity type for condition evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntityType {
    User,
    Resource,
    Context,
}

/// Index expression for bracket notation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexExpr {
    /// Numeric index: [0], [1], [42]
    Number(i64),
    /// String key: ["department"], ["role"]
    String(String),
    /// Wildcard for iteration: [_] - iterates over all elements (existential quantification)
    Wildcard,
}

/// Literal value for comparisons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Bool(bool),
}

impl ReaperDSLEvaluator {
    /// Create a new Reaper DSL evaluator
    ///
    /// Rules are automatically partitioned into deny and allow lists for optimal performance.
    /// Deny rules are evaluated first to ensure security-first evaluation.
    pub fn new(store: Arc<DataStore>, rules: Vec<Rule>, default_decision: PolicyAction) -> Self {
        // Partition rules into deny and allow for zero-overhead evaluation
        let mut deny_rules = Vec::new();
        let mut allow_rules = Vec::new();

        for rule in rules {
            match rule.decision {
                PolicyAction::Deny => deny_rules.push(rule),
                PolicyAction::Allow => allow_rules.push(rule),
                PolicyAction::Log => allow_rules.push(rule), // Log actions treated as allow
            }
        }

        // Pre-compile all regex patterns for O(1) lookup during evaluation
        // Uses FxHashMap for faster lookups (no DoS resistance needed for static patterns)
        let mut regex_cache = FxHashMap::default();
        for rule in deny_rules.iter().chain(allow_rules.iter()) {
            Self::collect_regex_patterns(&rule.condition, &mut regex_cache);
        }

        // Pre-intern all attribute names and string literals for O(1) lookup during evaluation
        let interner = store.interner();
        let mut interned_cache = FxHashMap::default();
        for rule in deny_rules.iter().chain(allow_rules.iter()) {
            Self::collect_strings_for_interning(&rule.condition, &mut interned_cache, interner);
        }

        // Pre-compute AttributeValue objects for membership tests
        // This avoids allocating AttributeValue::String on every membership check
        let mut membership_cache = FxHashMap::default();
        for rule in deny_rules.iter().chain(allow_rules.iter()) {
            Self::collect_membership_values(&rule.condition, &mut membership_cache, interner);
        }

        Self {
            store,
            deny_rules,
            allow_rules,
            default_decision,
            regex_cache: Arc::new(regex_cache),
            interned_cache: Arc::new(interned_cache),
            membership_cache: Arc::new(membership_cache),
        }
    }

    /// Recursively collect and compile regex patterns from a condition
    fn collect_regex_patterns(
        condition: &Condition,
        cache: &mut FxHashMap<String, regex::Regex>,
    ) {
        match condition {
            Condition::RegexMatches { pattern, .. } => {
                if !cache.contains_key(pattern) {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        cache.insert(pattern.clone(), re);
                    }
                }
            }
            Condition::And(conditions) | Condition::Or(conditions) => {
                for c in conditions {
                    Self::collect_regex_patterns(c, cache);
                }
            }
            Condition::Not(inner) => {
                Self::collect_regex_patterns(inner, cache);
            }
            _ => {} // Other conditions don't have regex patterns
        }
    }

    /// Recursively collect and pre-intern all strings from a condition
    /// This includes attribute names and string literals for O(1) lookup during evaluation
    fn collect_strings_for_interning(
        condition: &Condition,
        cache: &mut FxHashMap<String, InternedString>,
        interner: &crate::data::StringInterner,
    ) {
        // Helper to intern and cache a string
        let mut intern = |s: &String| {
            if !cache.contains_key(s) {
                cache.insert(s.clone(), interner.intern(s));
            }
        };

        match condition {
            Condition::ActionEquals { value } => intern(value),
            Condition::ResourceIdEquals { value } => intern(value),
            Condition::UserEquals { attribute, value } => {
                intern(attribute);
                intern(value);
            }
            Condition::UserGreaterEqualLiteral { attribute, .. }
            | Condition::UserGreaterLiteral { attribute, .. }
            | Condition::UserLessEqualLiteral { attribute, .. }
            | Condition::UserLessLiteral { attribute, .. } => {
                intern(attribute);
            }
            Condition::ResourceEquals { attribute, value } => {
                intern(attribute);
                intern(value);
            }
            Condition::ResourceGreaterEqualLiteral { attribute, .. }
            | Condition::ResourceGreaterLiteral { attribute, .. }
            | Condition::ResourceLessEqualLiteral { attribute, .. }
            | Condition::ResourceLessLiteral { attribute, .. } => {
                intern(attribute);
            }
            Condition::UserEqualsResource { user_attr, resource_attr } => {
                intern(user_attr);
                intern(resource_attr);
            }
            Condition::UserIntGreater { user_attr, resource_attr }
            | Condition::ResourceIntGreater { resource_attr, user_attr } => {
                intern(user_attr);
                intern(resource_attr);
            }
            Condition::Assignment { variable, attribute, .. } => {
                intern(variable);
                intern(attribute);
            }
            Condition::MembershipTest { attribute, value, .. } => {
                intern(attribute);
                // Also pre-intern the literal value for membership test
                if let LiteralValue::String(s) = value {
                    intern(s);
                }
            }
            Condition::IndexedEquals { attribute, value, .. } => {
                intern(attribute);
                intern(value);
            }
            Condition::EqualsVariable { attribute, variable, .. } => {
                intern(attribute);
                intern(variable);
            }
            Condition::RegexMatches { attribute, .. } => {
                intern(attribute);
            }
            Condition::StringContains { attribute, substring, .. } => {
                intern(attribute);
                intern(substring);
            }
            Condition::StringStartsWith { attribute, prefix, .. } => {
                intern(attribute);
                intern(prefix);
            }
            Condition::StringEndsWith { attribute, suffix, .. } => {
                intern(attribute);
                intern(suffix);
            }
            Condition::TimeIsAfter { attribute, .. }
            | Condition::TimeIsBefore { attribute, .. } => {
                intern(attribute);
            }
            Condition::CountGreaterEqual { attribute, .. }
            | Condition::CountGreater { attribute, .. }
            | Condition::CountEqual { attribute, .. } => {
                intern(attribute);
            }
            // String case methods
            Condition::StringLowerEquals { attribute, value, .. }
            | Condition::StringUpperEquals { attribute, value, .. } => {
                intern(attribute);
                intern(value);
            }
            // Type check functions
            Condition::IsString { attribute, .. }
            | Condition::IsNumber { attribute, .. }
            | Condition::IsBool { attribute, .. } => {
                intern(attribute);
            }
            // Set operations
            Condition::SetIntersectionCountGreater { attribute, values, .. } => {
                intern(attribute);
                for v in values {
                    intern(v);
                }
            }
            Condition::MapKeyExists { attribute, key, .. } => {
                intern(attribute);
                intern(key);
            }
            // Comprehensions
            Condition::ComprehensionCountGreaterEqual { attribute, filter_attr, .. }
            | Condition::ComprehensionCountEqual { attribute, filter_attr, .. } => {
                intern(attribute);
                intern(filter_attr);
            }
            // Same-entity attribute comparisons
            Condition::SameEntityAttrCompare { left_attr, right_attr, .. } => {
                intern(left_attr);
                intern(right_attr);
            }
            Condition::And(conditions) | Condition::Or(conditions) => {
                for c in conditions {
                    Self::collect_strings_for_interning(c, cache, interner);
                }
            }
            Condition::Not(inner) => {
                Self::collect_strings_for_interning(inner, cache, interner);
            }
            Condition::Always => {}
        }
    }

    /// Recursively collect and pre-compute AttributeValue objects for membership tests
    /// This avoids allocating AttributeValue::String during evaluation
    /// Only caches String values since Int/Bool are Copy types (no allocation)
    fn collect_membership_values(
        condition: &Condition,
        cache: &mut FxHashMap<String, AttributeValue>,
        interner: &crate::data::StringInterner,
    ) {
        match condition {
            Condition::MembershipTest { value: LiteralValue::String(s), .. } => {
                // Only pre-compute String values (Int/Bool are Copy types)
                if !cache.contains_key(s) {
                    let interned = interner.intern(s);
                    cache.insert(s.clone(), AttributeValue::String(interned));
                }
            }
            Condition::MembershipTest { .. } => {
                // Int/Bool are Copy types, no pre-computation needed
            }
            Condition::And(conditions) | Condition::Or(conditions) => {
                for c in conditions {
                    Self::collect_membership_values(c, cache, interner);
                }
            }
            Condition::Not(inner) => {
                Self::collect_membership_values(inner, cache, interner);
            }
            _ => {} // Other conditions don't have membership tests
        }
    }

    /// Get a pre-interned string from the cache, falling back to interning if not found
    /// This provides O(1) lookup for strings that were pre-interned at construction time
    #[inline(always)]
    fn get_interned(&self, s: &str, interner: &crate::data::StringInterner) -> InternedString {
        self.interned_cache
            .get(s)
            .copied()
            .unwrap_or_else(|| interner.intern(s))
    }

    /// Evaluate a condition against entities
    ///
    /// This is where the performance magic happens:
    /// - Direct DataStore access (no conversion)
    /// - Interned string comparisons (5ns vs 100ns)
    /// - Zero-copy entity access (Arc)
    /// - Pre-interned attribute names for O(1) lookup
    /// - Variable context for local bindings
    fn evaluate_condition(
        &self,
        condition: &Condition,
        user: &Entity,
        resource: &Entity,
        _context: &std::collections::HashMap<String, String>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
    ) -> bool {
        let interner = self.store.interner();

        match condition {
            Condition::Always => true,

            Condition::ActionEquals { value } => {
                // Action comes from context["action"]
                _context.get("action").map(|a| a == value).unwrap_or(false)
            }

            Condition::ResourceIdEquals { value } => {
                // Resource ID comes from context["resource"]
                _context.get("resource").map(|r| r == value).unwrap_or(false)
            }

            Condition::UserEquals { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);

                let result = match user.get_attribute(attr_key) {
                    Some(AttributeValue::String(actual)) => {
                        let expected_value = self.get_interned(value, interner);
                        let matched = *actual == expected_value;
                        tracing::debug!(
                            attribute = %attribute,
                            expected = %value,
                            actual = ?interner.resolve(*actual),
                            matched = %matched,
                            "UserEquals: String comparison"
                        );
                        matched
                    }
                    Some(AttributeValue::Bool(actual)) => {
                        // Handle boolean comparisons: "true" == true, "false" == false
                        let matched = value == &actual.to_string();
                        tracing::debug!(
                            attribute = %attribute,
                            expected = %value,
                            actual = %actual,
                            matched = %matched,
                            "UserEquals: Bool comparison"
                        );
                        matched
                    }
                    Some(AttributeValue::Int(actual)) => {
                        // Handle integer comparisons: "42" == 42
                        let matched = value == &actual.to_string();
                        tracing::debug!(
                            attribute = %attribute,
                            expected = %value,
                            actual = %actual,
                            matched = %matched,
                            "UserEquals: Int comparison"
                        );
                        matched
                    }
                    None => {
                        tracing::debug!(
                            attribute = %attribute,
                            expected = %value,
                            "UserEquals: Attribute not found"
                        );
                        false
                    }
                    _ => {
                        tracing::debug!(
                            attribute = %attribute,
                            expected = %value,
                            "UserEquals: Type mismatch"
                        );
                        false
                    }
                };
                result
            }

            Condition::UserGreaterEqualLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match user.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) >= *value,
                    Some(AttributeValue::Float(actual)) => *actual >= *value,
                    _ => false,
                }
            }

            Condition::UserGreaterLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match user.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) > *value,
                    Some(AttributeValue::Float(actual)) => *actual > *value,
                    _ => false,
                }
            }

            Condition::UserLessEqualLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match user.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) <= *value,
                    Some(AttributeValue::Float(actual)) => *actual <= *value,
                    _ => false,
                }
            }

            Condition::UserLessLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match user.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) < *value,
                    Some(AttributeValue::Float(actual)) => *actual < *value,
                    _ => false,
                }
            }

            Condition::ResourceEquals { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);

                match resource.get_attribute(attr_key) {
                    Some(AttributeValue::String(actual)) => {
                        let expected_value = self.get_interned(value, interner);
                        *actual == expected_value
                    }
                    Some(AttributeValue::Bool(actual)) => {
                        // Handle boolean comparisons: "true" == true, "false" == false
                        value == &actual.to_string()
                    }
                    Some(AttributeValue::Int(actual)) => {
                        // Handle integer comparisons: "42" == 42
                        value == &actual.to_string()
                    }
                    _ => false,
                }
            }

            Condition::ResourceGreaterEqualLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match resource.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) >= *value,
                    Some(AttributeValue::Float(actual)) => *actual >= *value,
                    _ => false,
                }
            }

            Condition::ResourceGreaterLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match resource.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) > *value,
                    Some(AttributeValue::Float(actual)) => *actual > *value,
                    _ => false,
                }
            }

            Condition::ResourceLessEqualLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match resource.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) <= *value,
                    Some(AttributeValue::Float(actual)) => *actual <= *value,
                    _ => false,
                }
            }

            Condition::ResourceLessLiteral { attribute, value } => {
                let attr_key = self.get_interned(attribute, interner);
                match resource.get_attribute(attr_key) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) < *value,
                    Some(AttributeValue::Float(actual)) => *actual < *value,
                    _ => false,
                }
            }

            Condition::UserEqualsResource {
                user_attr,
                resource_attr,
            } => {
                let user_key = self.get_interned(user_attr, interner);
                let resource_key = self.get_interned(resource_attr, interner);

                let result = match (
                    user.get_attribute(user_key),
                    resource.get_attribute(resource_key),
                ) {
                    (Some(AttributeValue::String(u)), Some(AttributeValue::String(r))) => {
                        let matched = u == r;
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            user_val = ?interner.resolve(*u),
                            resource_val = ?interner.resolve(*r),
                            matched = %matched,
                            "UserEqualsResource: String comparison"
                        );
                        matched
                    }
                    (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => {
                        let matched = u == r;
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            user_val = %u,
                            resource_val = %r,
                            matched = %matched,
                            "UserEqualsResource: Int comparison"
                        );
                        matched
                    }
                    (Some(AttributeValue::Bool(u)), Some(AttributeValue::Bool(r))) => {
                        let matched = u == r;
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            user_val = %u,
                            resource_val = %r,
                            matched = %matched,
                            "UserEqualsResource: Bool comparison"
                        );
                        matched
                    }
                    (None, _) => {
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            "UserEqualsResource: User attribute not found"
                        );
                        false
                    }
                    (_, None) => {
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            "UserEqualsResource: Resource attribute not found"
                        );
                        false
                    }
                    _ => {
                        tracing::debug!(
                            user_attr = %user_attr,
                            resource_attr = %resource_attr,
                            "UserEqualsResource: Type mismatch or unsupported types"
                        );
                        false
                    }
                };
                result
            }

            Condition::UserIntGreater {
                user_attr,
                resource_attr,
            } => {
                let user_key = self.get_interned(user_attr, interner);
                let resource_key = self.get_interned(resource_attr, interner);

                match (
                    user.get_attribute(user_key),
                    resource.get_attribute(resource_key),
                ) {
                    (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => u > r,
                    _ => false,
                }
            }

            Condition::ResourceIntGreater {
                resource_attr,
                user_attr,
            } => {
                let user_key = self.get_interned(user_attr, interner);
                let resource_key = self.get_interned(resource_attr, interner);

                match (
                    resource.get_attribute(resource_key),
                    user.get_attribute(user_key),
                ) {
                    (Some(AttributeValue::Int(r)), Some(AttributeValue::Int(u))) => r > u,
                    _ => false,
                }
            }

            // Same-entity attribute comparisons (entity.attr1 op entity.attr2)
            Condition::SameEntityAttrCompare {
                entity_type,
                left_attr,
                right_attr,
                op,
            } => {
                // Get the entity based on type
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => {
                        // TODO: Support context entity from DataStore
                        return false;
                    }
                };

                let left_key = self.get_interned(left_attr, interner);
                let right_key = self.get_interned(right_attr, interner);
                let left_val = entity.get_attribute(left_key);
                let right_val = entity.get_attribute(right_key);

                match (left_val, right_val, op) {
                    // Numeric comparisons (int vs int)
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::LessEqual) => *l <= *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::GreaterEqual) => *l >= *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Less) => *l < *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Greater) => *l > *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Equal) => *l == *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::NotEqual) => *l != *r,

                    // Numeric comparisons (float vs float)
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::LessEqual) => *l <= *r,
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::GreaterEqual) => *l >= *r,
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Less) => *l < *r,
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Greater) => *l > *r,
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Equal) => *l == *r,
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::NotEqual) => *l != *r,

                    // Numeric comparisons (int vs float)
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::LessEqual) => (*l as f64) <= *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::GreaterEqual) => (*l as f64) >= *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Less) => (*l as f64) < *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Greater) => (*l as f64) > *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Equal) => (*l as f64) == *r,
                    (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), AttrCompareOp::NotEqual) => (*l as f64) != *r,

                    // Numeric comparisons (float vs int)
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::LessEqual) => *l <= (*r as f64),
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::GreaterEqual) => *l >= (*r as f64),
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Less) => *l < (*r as f64),
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Greater) => *l > (*r as f64),
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Equal) => *l == (*r as f64),
                    (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), AttrCompareOp::NotEqual) => *l != (*r as f64),

                    // String equality comparisons
                    (Some(AttributeValue::String(l)), Some(AttributeValue::String(r)), AttrCompareOp::Equal) => l == r,
                    (Some(AttributeValue::String(l)), Some(AttributeValue::String(r)), AttrCompareOp::NotEqual) => l != r,

                    // Boolean equality comparisons
                    (Some(AttributeValue::Bool(l)), Some(AttributeValue::Bool(r)), AttrCompareOp::Equal) => *l == *r,
                    (Some(AttributeValue::Bool(l)), Some(AttributeValue::Bool(r)), AttrCompareOp::NotEqual) => *l != *r,

                    _ => false,
                }
            }

            Condition::Assignment {
                variable,
                entity_type,
                attribute,
                index,
            } => {
                // Get the attribute value based on entity type
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => {
                        // TODO: Support context entity from DataStore
                        return false;
                    }
                };

                let attr_key = self.get_interned(attribute, interner);

                let value = if let Some(idx) = index {
                    // Check for wildcard: role := user.roles[_]
                    if matches!(idx, IndexExpr::Wildcard) {
                        // For wildcards, assign first element from collection
                        // TODO: Full iteration semantics require And block restructuring
                        if let Some(collection) = entity.get_attribute(attr_key) {
                            match collection {
                                AttributeValue::List(items) => items.first().cloned(),
                                AttributeValue::Set(items) => items.iter().next().cloned(),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    } else {
                        // Normal indexed access: user.roles[0]
                        self.get_indexed_value(entity, attr_key, idx, interner)
                    }
                } else {
                    // Direct access: user.role
                    entity.get_attribute(attr_key).cloned()
                };

                // Store in variable context
                if let Some(val) = value {
                    variables.insert(variable.clone(), val);
                    true // Assignment always succeeds if attribute exists
                } else {
                    false // Assignment fails if attribute doesn't exist
                }
            }

            Condition::MembershipTest {
                value,
                entity_type,
                attribute,
                index,
            } => {
                // Get the collection based on entity type
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_key = self.get_interned(attribute, interner);
                let collection = if let Some(idx) = index {
                    self.get_indexed_value(entity, attr_key, idx, interner)
                } else {
                    entity.get_attribute(attr_key).cloned()
                };

                // Check membership based on collection type
                if let Some(coll) = collection {
                    match &coll {
                        AttributeValue::List(items) => {
                            // Linear search in list (could optimize with HashSet conversion)
                            self.value_in_list(value, items, interner)
                        }
                        AttributeValue::Set(items) => {
                            // O(1) lookup in HashSet
                            self.value_in_set(value, items, interner)
                        }
                        _ => false, // Not a collection
                    }
                } else {
                    false
                }
            }

            Condition::IndexedEquals {
                entity_type,
                attribute,
                index,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_key = self.get_interned(attribute, interner);

                // Handle wildcard iteration: user.roles[_] == "admin"
                if matches!(index, IndexExpr::Wildcard) {
                    // Existential quantification: check if ANY element equals the value
                    if let Some(collection) = entity.get_attribute(attr_key) {
                        let expected = self.get_interned(value, interner);
                        match collection {
                            AttributeValue::List(items) => {
                                // O(n) iteration over list
                                items.iter().any(|item| {
                                    matches!(item, AttributeValue::String(s) if *s == expected)
                                })
                            }
                            AttributeValue::Set(items) => {
                                // O(1) hash lookup in set
                                let expected_val = AttributeValue::String(expected);
                                items.contains(&expected_val)
                            }
                            _ => false, // Not a collection
                        }
                    } else {
                        false
                    }
                } else {
                    // Normal indexed access: user.roles[0] == "admin"
                    let indexed_val = self.get_indexed_value(entity, attr_key, index, interner);

                    if let Some(AttributeValue::String(actual)) = indexed_val {
                        let expected = self.get_interned(value, interner);
                        actual == expected
                    } else {
                        false
                    }
                }
            }

            Condition::EqualsVariable {
                entity_type,
                attribute,
                variable,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_key = self.get_interned(attribute, interner);
                let attr_val = entity.get_attribute(attr_key);
                let var_val = variables.get(variable);

                match (attr_val, var_val) {
                    (Some(a), Some(v)) => a == v,
                    _ => false,
                }
            }

            Condition::And(conditions) => {
                tracing::debug!(
                    conditions_count = %conditions.len(),
                    "Evaluating AND condition"
                );
                let mut all_match = true;
                for (i, c) in conditions.iter().enumerate() {
                    let matches = self.evaluate_condition(c, user, resource, _context, variables);
                    tracing::debug!(
                        index = %i,
                        matched = %matches,
                        "AND sub-condition result"
                    );
                    if !matches {
                        all_match = false;
                        break;
                    }
                }
                tracing::debug!(
                    result = %all_match,
                    "AND condition final result"
                );
                all_match
            }

            Condition::Or(conditions) => conditions
                .iter()
                .any(|c| self.evaluate_condition(c, user, resource, _context, variables)),

            Condition::Not(condition) => {
                !self.evaluate_condition(condition, user, resource, _context, variables)
            }

            // ============ Function Call Evaluation ============

            Condition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        // Resolve interned string to &str for regex matching
                        if let Some(resolved) = interner.resolve(*s) {
                            // Use pre-compiled regex from cache for O(1) lookup
                            self.regex_cache
                                .get(pattern)
                                .map(|re| re.is_match(&resolved))
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            Condition::StringContains {
                entity_type,
                attribute,
                substring,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                // Use pre-interned attribute key for O(1) lookup
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        // Resolve interned string to &str for string operations
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.contains(substring.as_str())
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            Condition::StringStartsWith {
                entity_type,
                attribute,
                prefix,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                // Use pre-interned attribute key for O(1) lookup
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        // Resolve interned string to &str for string operations
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.starts_with(prefix.as_str())
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            Condition::StringEndsWith {
                entity_type,
                attribute,
                suffix,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                // Use pre-interned attribute key for O(1) lookup
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        // Resolve interned string to &str for string operations
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.ends_with(suffix.as_str())
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            Condition::TimeIsAfter {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::Int(ts)) => *ts > *threshold,
                    Some(AttributeValue::Float(ts)) => (*ts as i64) > *threshold,
                    _ => false,
                }
            }

            Condition::TimeIsBefore {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::Int(ts)) => *ts < *threshold,
                    Some(AttributeValue::Float(ts)) => (*ts as i64) < *threshold,
                    _ => false,
                }
            }

            // ============ Collection Count Evaluation ============

            Condition::CountGreaterEqual {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::List(arr)) => arr.len() >= *threshold,
                    Some(AttributeValue::Set(set)) => set.len() >= *threshold,
                    _ => false,
                }
            }

            Condition::CountGreater {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::List(arr)) => arr.len() > *threshold,
                    Some(AttributeValue::Set(set)) => set.len() > *threshold,
                    _ => false,
                }
            }

            Condition::CountEqual {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::List(arr)) => arr.len() == *threshold,
                    Some(AttributeValue::Set(set)) => set.len() == *threshold,
                    _ => false,
                }
            }

            // ============ String Case Methods ============

            Condition::StringLowerEquals {
                entity_type,
                attribute,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.to_lowercase() == *value
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            Condition::StringUpperEquals {
                entity_type,
                attribute,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.to_uppercase() == *value
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            // ============ Type Check Functions ============

            Condition::IsString {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                matches!(entity.get_attribute(attr_key), Some(AttributeValue::String(_)))
            }

            Condition::IsNumber {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                matches!(
                    entity.get_attribute(attr_key),
                    Some(AttributeValue::Int(_)) | Some(AttributeValue::Float(_))
                )
            }

            Condition::IsBool {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                matches!(entity.get_attribute(attr_key), Some(AttributeValue::Bool(_)))
            }

            // ============ Set Operations ============

            Condition::SetIntersectionCountGreater {
                entity_type,
                attribute,
                values,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::Set(set)) => {
                        // Count how many of `values` are in the entity's set
                        let count = values.iter().filter(|v| {
                            let interned = interner.intern(v);
                            set.contains(&AttributeValue::String(interned))
                        }).count();
                        count > *threshold
                    }
                    Some(AttributeValue::List(list)) => {
                        // Convert list to set for intersection
                        let count = values.iter().filter(|v| {
                            let interned = interner.intern(v);
                            list.iter().any(|item| matches!(item, AttributeValue::String(s) if *s == interned))
                        }).count();
                        count > *threshold
                    }
                    _ => false,
                }
            }

            Condition::MapKeyExists {
                entity_type,
                attribute,
                key,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::Object(map)) => {
                        let key_interned = interner.intern(key);
                        map.contains_key(&key_interned)
                    }
                    _ => false,
                }
            }

            // ============ Comprehension Support ============

            Condition::ComprehensionCountGreaterEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                let filter_attr_key = self.get_interned(filter_attr, interner);

                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::List(items)) => {
                        let count = items.iter().filter(|item| {
                            if let AttributeValue::Object(obj) = item {
                                if let Some(field_val) = obj.get(&filter_attr_key) {
                                    self.compare_values(field_val, filter_value, filter_op, interner)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }).count();
                        count >= *threshold
                    }
                    _ => false,
                }
            }

            Condition::ComprehensionCountEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                let attr_key = self.get_interned(attribute, interner);
                let filter_attr_key = self.get_interned(filter_attr, interner);

                match entity.get_attribute(attr_key) {
                    Some(AttributeValue::List(items)) => {
                        let count = items.iter().filter(|item| {
                            if let AttributeValue::Object(obj) = item {
                                if let Some(field_val) = obj.get(&filter_attr_key) {
                                    self.compare_values(field_val, filter_value, filter_op, interner)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }).count();
                        count == *threshold
                    }
                    _ => false,
                }
            }
        }
    }

    /// Get indexed value from attribute (bracket notation)
    /// Performance: ~10-50ns depending on collection size
    fn get_indexed_value(
        &self,
        entity: &Entity,
        attr_key: crate::data::InternedString,
        index: &IndexExpr,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        let attr_val = entity.get_attribute(attr_key)?;

        match (attr_val, index) {
            // Numeric index on List: user.roles[0]
            (AttributeValue::List(items), IndexExpr::Number(idx)) => {
                let idx_usize = if *idx < 0 {
                    // Negative indexing: -1 = last element
                    items.len().checked_sub(idx.unsigned_abs() as usize)?
                } else {
                    *idx as usize
                };
                items.get(idx_usize).cloned()
            }

            // String key on Object: user.data["department"]
            (AttributeValue::Object(map), IndexExpr::String(key)) => {
                let key_interned = interner.intern(key);
                map.get(&key_interned).cloned()
            }

            _ => None, // Type mismatch
        }
    }

    /// Check if literal value exists in list
    /// Performance: O(n) linear search (rare case - most collections are Sets now)
    fn value_in_list(
        &self,
        value: &LiteralValue,
        items: &[AttributeValue],
        interner: &crate::data::StringInterner,
    ) -> bool {
        match value {
            LiteralValue::String(s) => {
                // Use pre-interned cache for O(1) lookup
                let s_interned = self.get_interned(s, interner);
                items
                    .iter()
                    .any(|item| matches!(item, AttributeValue::String(x) if *x == s_interned))
            }
            LiteralValue::Int(i) => items
                .iter()
                .any(|item| matches!(item, AttributeValue::Int(x) if *x == *i)),
            LiteralValue::Bool(b) => items
                .iter()
                .any(|item| matches!(item, AttributeValue::Bool(x) if *x == *b)),
        }
    }

    /// Check if literal value exists in set
    /// Performance: O(1) FxHashSet lookup with pre-computed AttributeValue - blazing fast!
    #[inline(always)]
    fn value_in_set(
        &self,
        value: &LiteralValue,
        items: &rustc_hash::FxHashSet<AttributeValue>,
        _interner: &crate::data::StringInterner,
    ) -> bool {
        match value {
            LiteralValue::String(s) => {
                // Use pre-computed AttributeValue from membership cache
                // This avoids allocating a new AttributeValue::String on every check
                if let Some(attr_value) = self.membership_cache.get(s) {
                    items.contains(attr_value)
                } else {
                    // Fallback: compute the value (should not happen if cache is populated)
                    let s_interned = _interner.intern(s);
                    items.contains(&AttributeValue::String(s_interned))
                }
            }
            // Int and Bool are Copy types - no allocation needed
            LiteralValue::Int(i) => items.contains(&AttributeValue::Int(*i)),
            LiteralValue::Bool(b) => items.contains(&AttributeValue::Bool(*b)),
        }
    }

    /// Compare an AttributeValue against a LiteralValue using the given operation
    /// Used for comprehension filter evaluation
    #[inline(always)]
    fn compare_values(
        &self,
        attr_val: &AttributeValue,
        literal: &LiteralValue,
        op: &ComprehensionFilterOp,
        interner: &crate::data::StringInterner,
    ) -> bool {
        match (attr_val, literal) {
            (AttributeValue::String(s), LiteralValue::String(ls)) => {
                if let Some(resolved) = interner.resolve(*s) {
                    match op {
                        ComprehensionFilterOp::Equal => &*resolved == ls,
                        ComprehensionFilterOp::NotEqual => &*resolved != ls,
                        ComprehensionFilterOp::Contains => resolved.contains(ls.as_str()),
                        _ => false, // Other ops not valid for strings
                    }
                } else {
                    false
                }
            }
            (AttributeValue::Int(i), LiteralValue::Int(li)) => match op {
                ComprehensionFilterOp::Equal => *i == *li,
                ComprehensionFilterOp::NotEqual => *i != *li,
                ComprehensionFilterOp::GreaterThan => *i > *li,
                ComprehensionFilterOp::LessThan => *i < *li,
                ComprehensionFilterOp::GreaterEqual => *i >= *li,
                ComprehensionFilterOp::LessEqual => *i <= *li,
                _ => false,
            },
            (AttributeValue::Float(f), LiteralValue::Int(li)) => {
                let lf = *li as f64;
                match op {
                    ComprehensionFilterOp::Equal => (*f - lf).abs() < f64::EPSILON,
                    ComprehensionFilterOp::NotEqual => (*f - lf).abs() >= f64::EPSILON,
                    ComprehensionFilterOp::GreaterThan => *f > lf,
                    ComprehensionFilterOp::LessThan => *f < lf,
                    ComprehensionFilterOp::GreaterEqual => *f >= lf,
                    ComprehensionFilterOp::LessEqual => *f <= lf,
                    _ => false,
                }
            }
            (AttributeValue::Bool(b), LiteralValue::Bool(lb)) => match op {
                ComprehensionFilterOp::Equal => *b == *lb,
                ComprehensionFilterOp::NotEqual => *b != *lb,
                _ => false,
            },
            _ => false, // Type mismatch
        }
    }
}

impl PolicyEvaluator for ReaperDSLEvaluator {
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
        let interner = self.store.interner();

        // Parse entity IDs from request
        // In production, these would be passed directly as InternedString
        let user_id = interner.intern(request.context.get("principal").ok_or_else(|| {
            ReaperError::EvaluationError {
                reason: "Missing principal in context".to_string(),
            }
        })?);

        let resource_id = interner.intern(&request.resource);

        // Fast DataStore lookups (~20-50ns each)
        let user = self
            .store
            .get(user_id)
            .ok_or_else(|| ReaperError::EvaluationError {
                reason: format!("User entity not found: {:?}", user_id),
            })?;

        // Resource lookup - if entity doesn't exist, create a temporary entity
        // This allows simple `resource == "value"` checks to work even without resource entities
        let resource = self.store.get(resource_id).unwrap_or_else(|| {
            // Create a minimal entity with just the resource ID for simple resource matching
            let resource_type = interner.intern("resource");
            Arc::new(Entity::new(resource_id, resource_type, std::collections::HashMap::new()))
        });

        // Create evaluation context with action and resource included
        let mut eval_context = request.context.clone();
        eval_context.insert("action".to_string(), request.action.clone());
        eval_context.insert("resource".to_string(), request.resource.clone());

        // Variable context for local bindings (scoped to policy evaluation)
        // Performance: HashMap with pre-allocated capacity for common case
        let mut variables = std::collections::HashMap::with_capacity(4);

        // Security-first evaluation: Deny rules ALWAYS take precedence over Allow rules
        // Rules are pre-partitioned at construction time for optimal performance

        // Phase 1: Evaluate all DENY rules first (pre-partitioned, no type checking needed)
        for rule in &self.deny_rules {
            if self.evaluate_condition(
                &rule.condition,
                &user,
                &resource,
                &eval_context,
                &mut variables,
            ) {
                // Explicit deny - return immediately, no allow can override this
                return Ok(PolicyAction::Deny);
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 2: No deny matched, evaluate ALLOW rules (pre-partitioned, no type checking needed)
        for rule in &self.allow_rules {
            let matches = self.evaluate_condition(
                &rule.condition,
                &user,
                &resource,
                &eval_context,
                &mut variables,
            );

            // Debug logging
            tracing::debug!(
                rule_name = %rule.name,
                matches = %matches,
                action = %request.action,
                user_id = ?user_id,
                "Evaluating allow rule"
            );

            if matches {
                tracing::info!(
                    rule_name = %rule.name,
                    action = %request.action,
                    "Rule matched - returning Allow"
                );
                return Ok(PolicyAction::Allow);
            }
            // Clear variables between rules (each rule has independent scope)
            variables.clear();
        }

        // Phase 3: No rule matched - return default decision
        tracing::debug!(
            default_decision = ?self.default_decision,
            action = %request.action,
            user_id = ?user_id,
            "No rules matched - returning default decision"
        );
        Ok(self.default_decision.clone())
    }

    fn validate(&self) -> Result<(), ReaperError> {
        // Validate that rules are well-formed
        if self.deny_rules.is_empty() && self.allow_rules.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "Policy must have at least one rule".to_string(),
            });
        }

        // Validate deny rules
        for (index, rule) in self.deny_rules.iter().enumerate() {
            if rule.name.is_empty() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Deny rule {} has empty name", index),
                });
            }
        }

        // Validate allow rules
        for (index, rule) in self.allow_rules.iter().enumerate() {
            if rule.name.is_empty() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Allow rule {} has empty name", index),
                });
            }
        }

        Ok(())
    }

    fn evaluator_type(&self) -> &str {
        "reaper-dsl"
    }

    fn metadata(&self) -> Option<EvaluatorMetadata> {
        let total_rules = self.deny_rules.len() + self.allow_rules.len();
        let mut extra = std::collections::HashMap::new();
        extra.insert("rule_count".to_string(), total_rules.to_string());
        extra.insert("deny_rules".to_string(), self.deny_rules.len().to_string());
        extra.insert(
            "allow_rules".to_string(),
            self.allow_rules.len().to_string(),
        );
        extra.insert(
            "default_decision".to_string(),
            format!("{:?}", self.default_decision),
        );

        Some(EvaluatorMetadata {
            rule_count: total_rules,
            complexity: (total_rules.min(50) as u8) * 2, // Rough complexity estimate
            extra,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityBuilder;
    use std::collections::HashMap;

    #[test]
    fn test_reaper_dsl_simple_rule() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create test entities
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_string(role_key, admin_value)
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: admin can do anything
        let rules = vec![Rule {
            name: "admin_access".to_string(),
            condition: Condition::UserEquals {
                attribute: "role".to_string(),
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        // Test evaluation
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_reaper_dsl_complex_rule() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user
        let bob_id = interner.intern("bob");
        let user_type = interner.intern("User");
        let dept_key = interner.intern("department");
        let eng_value = interner.intern("engineering");

        let bob = EntityBuilder::new(bob_id, user_type)
            .with_string(dept_key, eng_value)
            .build();

        // Create resource
        let doc_id = interner.intern("doc2");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type)
            .with_string(dept_key, eng_value)
            .build();

        store.insert(bob);
        store.insert(doc);

        // Create policy: same department access
        let rules = vec![Rule {
            name: "department_access".to_string(),
            condition: Condition::UserEqualsResource {
                user_attr: "department".to_string(),
                resource_attr: "department".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "bob".to_string());

        let request = PolicyRequest {
            resource: "doc2".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_membership_test_with_set() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with Set of roles
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let roles_key = interner.intern("roles");

        let admin_role = interner.intern("admin");
        let user_role = interner.intern("user");

        let mut roles_set = rustc_hash::FxHashSet::default();
        roles_set.insert(AttributeValue::String(admin_role));
        roles_set.insert(AttributeValue::String(user_role));

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(roles_key, AttributeValue::Set(roles_set))
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: check if "admin" in user.roles (Set)
        let rules = vec![Rule {
            name: "admin_access".to_string(),
            condition: Condition::MembershipTest {
                value: LiteralValue::String("admin".to_string()),
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: None,
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_membership_test_with_list() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with List of permissions
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let perms_key = interner.intern("permissions");

        let read_perm = interner.intern("read");
        let write_perm = interner.intern("write");

        let perms_list = vec![
            AttributeValue::String(read_perm),
            AttributeValue::String(write_perm),
        ];

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(perms_key, AttributeValue::List(perms_list))
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: check if "write" in user.permissions (List)
        let rules = vec![Rule {
            name: "write_access".to_string(),
            condition: Condition::MembershipTest {
                value: LiteralValue::String("write".to_string()),
                entity_type: EntityType::User,
                attribute: "permissions".to_string(),
                index: None,
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "write".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_indexed_access_numeric() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with List of roles
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let roles_key = interner.intern("roles");

        let admin_role = interner.intern("admin");
        let viewer_role = interner.intern("viewer");

        let roles_list = vec![
            AttributeValue::String(admin_role),
            AttributeValue::String(viewer_role),
        ];

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(roles_key, AttributeValue::List(roles_list))
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: check if user.roles[0] == "admin"
        let rules = vec![Rule {
            name: "first_role_admin".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: IndexExpr::Number(0),
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_indexed_access_string_key() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with Object (HashMap)
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let data_key = interner.intern("data");

        let dept_key = interner.intern("department");
        let eng_value = interner.intern("engineering");

        let mut data_map = std::collections::HashMap::new();
        data_map.insert(dept_key, AttributeValue::String(eng_value));

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(data_key, AttributeValue::Object(data_map))
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: check if user.data["department"] == "engineering"
        let rules = vec![Rule {
            name: "eng_access".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "data".to_string(),
                index: IndexExpr::String("department".to_string()),
                value: "engineering".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_variable_assignment_and_comparison() {
        let store = Arc::new(DataStore::new());
        let interner = store.interner();

        // Create user with role
        let alice_id = interner.intern("alice");
        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let admin_value = interner.intern("admin");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_string(role_key, admin_value)
            .build();

        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("Document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Create policy: role_var := user.role, then user.role == role_var
        let rules = vec![Rule {
            name: "var_test".to_string(),
            condition: Condition::And(vec![
                Condition::Assignment {
                    variable: "role_var".to_string(),
                    entity_type: EntityType::User,
                    attribute: "role".to_string(),
                    index: None,
                },
                Condition::EqualsVariable {
                    entity_type: EntityType::User,
                    attribute: "role".to_string(),
                    variable: "role_var".to_string(),
                },
            ]),
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_wildcard_iteration_list() {
        let store = Arc::new(DataStore::new());

        let interner = store.interner();

        // Create user with roles list
        let alice_id = interner.intern("user-alice");
        let user_type = interner.intern("user");
        let roles_key = interner.intern("roles");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(
                roles_key,
                AttributeValue::List(vec![
                    AttributeValue::String(interner.intern("developer")),
                    AttributeValue::String(interner.intern("admin")),
                    AttributeValue::String(interner.intern("manager")),
                ]),
            )
            .build();

        // Create resource entity (required by evaluator)
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Test: user.roles[_] == "admin" (existential: does user have admin role?)
        let rules = vec![Rule {
            name: "wildcard_in_list".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: IndexExpr::Wildcard,
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user-alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_wildcard_iteration_list_not_found() {
        let store = Arc::new(DataStore::new());

        let interner = store.interner();

        // Create user with roles list that doesn't contain "superadmin"
        let alice_id = interner.intern("user-alice");
        let user_type = interner.intern("user");
        let roles_key = interner.intern("roles");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(
                roles_key,
                AttributeValue::List(vec![
                    AttributeValue::String(interner.intern("developer")),
                    AttributeValue::String(interner.intern("manager")),
                ]),
            )
            .build();

        // Create resource entity (required by evaluator)
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Test: user.roles[_] == "superadmin" (should fail)
        let rules = vec![Rule {
            name: "wildcard_not_found".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: IndexExpr::Wildcard,
                value: "superadmin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user-alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny)); // Should deny
    }

    #[test]
    fn test_wildcard_iteration_set() {
        let store = Arc::new(DataStore::new());

        let interner = store.interner();

        // Create resource with allowed_roles as a Set (O(1) lookup!)
        let doc_id = interner.intern("resource-doc1");
        let doc_type = interner.intern("document");
        let roles_key = interner.intern("allowed_roles");

        let mut roles_set = rustc_hash::FxHashSet::default();
        roles_set.insert(AttributeValue::String(interner.intern("admin")));
        roles_set.insert(AttributeValue::String(interner.intern("manager")));
        roles_set.insert(AttributeValue::String(interner.intern("developer")));

        let resource = EntityBuilder::new(doc_id, doc_type)
            .with_attribute(roles_key, AttributeValue::Set(roles_set))
            .build();

        // Create user entity (required by evaluator)
        let user_id = interner.intern("user-alice");
        let user_type = interner.intern("user");
        let user = EntityBuilder::new(user_id, user_type).build();

        store.insert(resource);
        store.insert(user);

        // Test: resource.allowed_roles[_] == "admin" (O(1) set lookup)
        let rules = vec![Rule {
            name: "wildcard_in_set".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::Resource,
                attribute: "allowed_roles".to_string(),
                index: IndexExpr::Wildcard,
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user-alice".to_string());
        context.insert("resource".to_string(), "resource-doc1".to_string());

        let request = PolicyRequest {
            resource: "resource-doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow));
    }

    #[test]
    fn test_wildcard_assignment() {
        let store = Arc::new(DataStore::new());

        let interner = store.interner();

        // Create user with permissions list
        let alice_id = interner.intern("user-alice");
        let user_type = interner.intern("user");
        let perm_key = interner.intern("permissions");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(
                perm_key,
                AttributeValue::List(vec![
                    AttributeValue::String(interner.intern("read")),
                    AttributeValue::String(interner.intern("write")),
                ]),
            )
            .build();

        // Create resource entity (required by evaluator)
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Test: perm := user.permissions[_] (assigns first element for now)
        let rules = vec![Rule {
            name: "wildcard_assignment".to_string(),
            condition: Condition::Assignment {
                variable: "perm".to_string(),
                entity_type: EntityType::User,
                attribute: "permissions".to_string(),
                index: Some(IndexExpr::Wildcard),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user-alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Allow)); // Assignment succeeds
    }

    #[test]
    fn test_wildcard_empty_list() {
        let store = Arc::new(DataStore::new());

        let interner = store.interner();

        // Create user with empty roles list
        let alice_id = interner.intern("user-alice");
        let user_type = interner.intern("user");
        let roles_key = interner.intern("roles");

        let alice = EntityBuilder::new(alice_id, user_type)
            .with_attribute(roles_key, AttributeValue::List(vec![]))
            .build();

        // Create resource entity (required by evaluator)
        let doc_id = interner.intern("doc1");
        let doc_type = interner.intern("document");
        let doc = EntityBuilder::new(doc_id, doc_type).build();

        store.insert(alice);
        store.insert(doc);

        // Test: user.roles[_] == "admin" (should fail on empty list)
        let rules = vec![Rule {
            name: "wildcard_empty".to_string(),
            condition: Condition::IndexedEquals {
                entity_type: EntityType::User,
                attribute: "roles".to_string(),
                index: IndexExpr::Wildcard,
                value: "admin".to_string(),
            },
            decision: PolicyAction::Allow,
        }];

        let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user-alice".to_string());

        let request = PolicyRequest {
            resource: "doc1".to_string(),
            action: "read".to_string(),
            context,
        };

        let decision = evaluator.evaluate(&request).unwrap();
        assert!(matches!(decision, PolicyAction::Deny)); // Empty list = no match
    }
}
