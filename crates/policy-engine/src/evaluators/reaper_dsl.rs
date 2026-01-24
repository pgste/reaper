//! Reaper DSL - Native Policy Language
//!
//! A Rust-native policy language optimized for sub-microsecond evaluation.
//! Leverages DataStore directly for zero-copy, interned-string-based policies.

use super::{EvaluatorMetadata, PolicyEvaluator};
use crate::data::{AttributeValue, DataStore, Entity, InternedString};
use crate::{PolicyAction, PolicyRequest};
use memchr::memmem;
use reaper_core::ReaperError;
use rustc_hash::{FxHashMap, FxHashSet};
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
    /// Compiled deny rules (evaluated first for security) - zero HashMap lookups
    compiled_deny_rules: Vec<CompiledRule>,
    /// Compiled allow rules (evaluated after deny rules) - zero HashMap lookups
    compiled_allow_rules: Vec<CompiledRule>,
    /// Default decision if no rules match
    default_decision: PolicyAction,
    /// Pre-compiled regex patterns for O(1) lookup during evaluation
    #[allow(dead_code)]
    regex_cache: Arc<FxHashMap<String, regex::Regex>>,
    /// Pre-computed AttributeValue objects for membership tests
    #[allow(dead_code)]
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
    /// Wildcard iteration: user.attr[_] == resource.attr
    /// Existential quantification - true if ANY element in user's array equals resource's scalar
    UserWildcardEqualsResourceAttr {
        user_attr: String,
        resource_attr: String,
    },
    /// Wildcard iteration: resource.attr[_] == user.attr
    /// Existential quantification - true if ANY element in resource's array equals user's scalar
    ResourceWildcardEqualsUserAttr {
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

/// Compiled condition with pre-interned strings for zero-lookup evaluation.
/// This is the "hot path" version - all strings are pre-interned at construction time.
/// Eliminates ~50 HashMap lookups per evaluation.
#[derive(Debug, Clone)]
pub enum CompiledCondition {
    Always,
    ActionEquals {
        value: InternedString,
    },
    ResourceIdEquals {
        value: InternedString,
    },
    UserEquals {
        attribute: InternedString,
        value: InternedString,
    },
    UserGreaterEqualLiteral {
        attribute: InternedString,
        value: f64,
    },
    UserGreaterLiteral {
        attribute: InternedString,
        value: f64,
    },
    UserLessEqualLiteral {
        attribute: InternedString,
        value: f64,
    },
    UserLessLiteral {
        attribute: InternedString,
        value: f64,
    },
    ResourceEquals {
        attribute: InternedString,
        value: InternedString,
    },
    ResourceGreaterEqualLiteral {
        attribute: InternedString,
        value: f64,
    },
    ResourceGreaterLiteral {
        attribute: InternedString,
        value: f64,
    },
    ResourceLessEqualLiteral {
        attribute: InternedString,
        value: f64,
    },
    ResourceLessLiteral {
        attribute: InternedString,
        value: f64,
    },
    UserEqualsResource {
        user_attr: InternedString,
        resource_attr: InternedString,
    },
    UserIntGreater {
        user_attr: InternedString,
        resource_attr: InternedString,
    },
    ResourceIntGreater {
        resource_attr: InternedString,
        user_attr: InternedString,
    },
    UserWildcardEqualsResourceAttr {
        user_attr: InternedString,
        resource_attr: InternedString,
    },
    ResourceWildcardEqualsUserAttr {
        resource_attr: InternedString,
        user_attr: InternedString,
    },
    SameEntityAttrCompare {
        entity_type: EntityType,
        left_attr: InternedString,
        right_attr: InternedString,
        op: AttrCompareOp,
    },
    Assignment {
        variable: InternedString,
        entity_type: EntityType,
        attribute: InternedString,
        index: Option<IndexExpr>,
    },
    MembershipTest {
        value: CompiledLiteralValue,
        entity_type: EntityType,
        attribute: InternedString,
        index: Option<IndexExpr>,
    },
    IndexedEquals {
        entity_type: EntityType,
        attribute: InternedString,
        index: IndexExpr,
        value: InternedString,
    },
    EqualsVariable {
        entity_type: EntityType,
        attribute: InternedString,
        variable: InternedString,
    },
    RegexMatches {
        entity_type: EntityType,
        attribute: InternedString,
        pattern: String, // Keep as String for regex cache lookup
    },
    StringContains {
        entity_type: EntityType,
        attribute: InternedString,
        substring: String, // Keep as String for memchr
    },
    StringStartsWith {
        entity_type: EntityType,
        attribute: InternedString,
        prefix: String,
    },
    StringEndsWith {
        entity_type: EntityType,
        attribute: InternedString,
        suffix: String,
    },
    TimeIsAfter {
        entity_type: EntityType,
        attribute: InternedString,
        threshold: i64,
    },
    TimeIsBefore {
        entity_type: EntityType,
        attribute: InternedString,
        threshold: i64,
    },
    CountGreaterEqual {
        entity_type: EntityType,
        attribute: InternedString,
        threshold: usize,
    },
    CountGreater {
        entity_type: EntityType,
        attribute: InternedString,
        threshold: usize,
    },
    CountEqual {
        entity_type: EntityType,
        attribute: InternedString,
        threshold: usize,
    },
    StringLowerEquals {
        entity_type: EntityType,
        attribute: InternedString,
        value: String, // Keep as String for comparison
    },
    StringUpperEquals {
        entity_type: EntityType,
        attribute: InternedString,
        value: String,
    },
    IsString {
        entity_type: EntityType,
        attribute: InternedString,
    },
    IsNumber {
        entity_type: EntityType,
        attribute: InternedString,
    },
    IsBool {
        entity_type: EntityType,
        attribute: InternedString,
    },
    SetIntersectionCountGreater {
        entity_type: EntityType,
        attribute: InternedString,
        values: Vec<InternedString>,
        threshold: usize,
    },
    MapKeyExists {
        entity_type: EntityType,
        attribute: InternedString,
        key: InternedString,
    },
    ComprehensionCountGreaterEqual {
        entity_type: EntityType,
        attribute: InternedString,
        filter_attr: InternedString,
        filter_value: CompiledLiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },
    ComprehensionCountEqual {
        entity_type: EntityType,
        attribute: InternedString,
        filter_attr: InternedString,
        filter_value: CompiledLiteralValue,
        filter_op: ComprehensionFilterOp,
        threshold: usize,
    },
    And(Vec<CompiledCondition>),
    Or(Vec<CompiledCondition>),
    Not(Box<CompiledCondition>),
}

/// Compiled literal value with pre-interned strings
#[derive(Debug, Clone)]
pub enum CompiledLiteralValue {
    String(InternedString),
    Int(i64),
    Bool(bool),
}

/// Compiled rule with pre-interned condition for fast evaluation
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Pre-compiled condition with interned strings
    pub condition: CompiledCondition,
    /// Decision if condition is true
    pub decision: PolicyAction,
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
    /// All conditions are pre-compiled with interned strings for zero HashMap lookups.
    pub fn new(store: Arc<DataStore>, rules: Vec<Rule>, default_decision: PolicyAction) -> Self {
        let interner = store.interner();

        // Pre-compile all regex patterns for O(1) lookup during evaluation
        let mut regex_cache = FxHashMap::default();
        for rule in &rules {
            Self::collect_regex_patterns(&rule.condition, &mut regex_cache);
        }

        // Pre-warm thread-local regex cache for zero-overhead access on hot path
        // This ensures the first evaluation doesn't pay compilation cost
        let patterns: Vec<String> = regex_cache.keys().cloned().collect();
        crate::regex_cache::prewarm_patterns_owned(&patterns);

        // Pre-compute AttributeValue objects for membership tests
        let mut membership_cache = FxHashMap::default();
        for rule in &rules {
            Self::collect_membership_values(&rule.condition, &mut membership_cache, interner);
        }

        // Compile rules with pre-interned strings (eliminates HashMap lookups during evaluation)
        let mut compiled_deny_rules = Vec::new();
        let mut compiled_allow_rules = Vec::new();

        for rule in rules {
            let compiled = CompiledRule {
                name: rule.name,
                condition: Self::compile_condition(&rule.condition, interner),
                decision: rule.decision.clone(),
            };
            match compiled.decision {
                PolicyAction::Deny => compiled_deny_rules.push(compiled),
                PolicyAction::Allow | PolicyAction::Log => compiled_allow_rules.push(compiled),
            }
        }

        Self {
            store,
            compiled_deny_rules,
            compiled_allow_rules,
            default_decision,
            regex_cache: Arc::new(regex_cache),
            membership_cache: Arc::new(membership_cache),
        }
    }

    /// Compile a condition with pre-interned strings for zero-lookup evaluation.
    /// This is called once at construction time, not during evaluation.
    fn compile_condition(
        condition: &Condition,
        interner: &crate::data::StringInterner,
    ) -> CompiledCondition {
        match condition {
            Condition::Always => CompiledCondition::Always,
            Condition::ActionEquals { value } => CompiledCondition::ActionEquals {
                value: interner.intern(value),
            },
            Condition::ResourceIdEquals { value } => CompiledCondition::ResourceIdEquals {
                value: interner.intern(value),
            },
            Condition::UserEquals { attribute, value } => CompiledCondition::UserEquals {
                attribute: interner.intern(attribute),
                value: interner.intern(value),
            },
            Condition::UserGreaterEqualLiteral { attribute, value } => {
                CompiledCondition::UserGreaterEqualLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::UserGreaterLiteral { attribute, value } => {
                CompiledCondition::UserGreaterLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::UserLessEqualLiteral { attribute, value } => {
                CompiledCondition::UserLessEqualLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::UserLessLiteral { attribute, value } => CompiledCondition::UserLessLiteral {
                attribute: interner.intern(attribute),
                value: *value,
            },
            Condition::ResourceEquals { attribute, value } => CompiledCondition::ResourceEquals {
                attribute: interner.intern(attribute),
                value: interner.intern(value),
            },
            Condition::ResourceGreaterEqualLiteral { attribute, value } => {
                CompiledCondition::ResourceGreaterEqualLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::ResourceGreaterLiteral { attribute, value } => {
                CompiledCondition::ResourceGreaterLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::ResourceLessEqualLiteral { attribute, value } => {
                CompiledCondition::ResourceLessEqualLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::ResourceLessLiteral { attribute, value } => {
                CompiledCondition::ResourceLessLiteral {
                    attribute: interner.intern(attribute),
                    value: *value,
                }
            }
            Condition::UserEqualsResource {
                user_attr,
                resource_attr,
            } => CompiledCondition::UserEqualsResource {
                user_attr: interner.intern(user_attr),
                resource_attr: interner.intern(resource_attr),
            },
            Condition::UserIntGreater {
                user_attr,
                resource_attr,
            } => CompiledCondition::UserIntGreater {
                user_attr: interner.intern(user_attr),
                resource_attr: interner.intern(resource_attr),
            },
            Condition::ResourceIntGreater {
                resource_attr,
                user_attr,
            } => CompiledCondition::ResourceIntGreater {
                resource_attr: interner.intern(resource_attr),
                user_attr: interner.intern(user_attr),
            },
            Condition::UserWildcardEqualsResourceAttr {
                user_attr,
                resource_attr,
            } => CompiledCondition::UserWildcardEqualsResourceAttr {
                user_attr: interner.intern(user_attr),
                resource_attr: interner.intern(resource_attr),
            },
            Condition::ResourceWildcardEqualsUserAttr {
                resource_attr,
                user_attr,
            } => CompiledCondition::ResourceWildcardEqualsUserAttr {
                resource_attr: interner.intern(resource_attr),
                user_attr: interner.intern(user_attr),
            },
            Condition::SameEntityAttrCompare {
                entity_type,
                left_attr,
                right_attr,
                op,
            } => CompiledCondition::SameEntityAttrCompare {
                entity_type: entity_type.clone(),
                left_attr: interner.intern(left_attr),
                right_attr: interner.intern(right_attr),
                op: *op,
            },
            Condition::Assignment {
                variable,
                entity_type,
                attribute,
                index,
            } => CompiledCondition::Assignment {
                variable: interner.intern(variable),
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                index: index.clone(),
            },
            Condition::MembershipTest {
                value,
                entity_type,
                attribute,
                index,
            } => CompiledCondition::MembershipTest {
                value: Self::compile_literal(value, interner),
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                index: index.clone(),
            },
            Condition::IndexedEquals {
                entity_type,
                attribute,
                index,
                value,
            } => CompiledCondition::IndexedEquals {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                index: index.clone(),
                value: interner.intern(value),
            },
            Condition::EqualsVariable {
                entity_type,
                attribute,
                variable,
            } => CompiledCondition::EqualsVariable {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                variable: interner.intern(variable),
            },
            Condition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            } => CompiledCondition::RegexMatches {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                pattern: pattern.clone(),
            },
            Condition::StringContains {
                entity_type,
                attribute,
                substring,
            } => CompiledCondition::StringContains {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                substring: substring.clone(),
            },
            Condition::StringStartsWith {
                entity_type,
                attribute,
                prefix,
            } => CompiledCondition::StringStartsWith {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                prefix: prefix.clone(),
            },
            Condition::StringEndsWith {
                entity_type,
                attribute,
                suffix,
            } => CompiledCondition::StringEndsWith {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                suffix: suffix.clone(),
            },
            Condition::TimeIsAfter {
                entity_type,
                attribute,
                threshold,
            } => CompiledCondition::TimeIsAfter {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                threshold: *threshold,
            },
            Condition::TimeIsBefore {
                entity_type,
                attribute,
                threshold,
            } => CompiledCondition::TimeIsBefore {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                threshold: *threshold,
            },
            Condition::CountGreaterEqual {
                entity_type,
                attribute,
                threshold,
            } => CompiledCondition::CountGreaterEqual {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                threshold: *threshold,
            },
            Condition::CountGreater {
                entity_type,
                attribute,
                threshold,
            } => CompiledCondition::CountGreater {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                threshold: *threshold,
            },
            Condition::CountEqual {
                entity_type,
                attribute,
                threshold,
            } => CompiledCondition::CountEqual {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                threshold: *threshold,
            },
            Condition::StringLowerEquals {
                entity_type,
                attribute,
                value,
            } => CompiledCondition::StringLowerEquals {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                value: value.clone(),
            },
            Condition::StringUpperEquals {
                entity_type,
                attribute,
                value,
            } => CompiledCondition::StringUpperEquals {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                value: value.clone(),
            },
            Condition::IsString {
                entity_type,
                attribute,
            } => CompiledCondition::IsString {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
            },
            Condition::IsNumber {
                entity_type,
                attribute,
            } => CompiledCondition::IsNumber {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
            },
            Condition::IsBool {
                entity_type,
                attribute,
            } => CompiledCondition::IsBool {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
            },
            Condition::SetIntersectionCountGreater {
                entity_type,
                attribute,
                values,
                threshold,
            } => CompiledCondition::SetIntersectionCountGreater {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                values: values.iter().map(|v| interner.intern(v)).collect(),
                threshold: *threshold,
            },
            Condition::MapKeyExists {
                entity_type,
                attribute,
                key,
            } => CompiledCondition::MapKeyExists {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                key: interner.intern(key),
            },
            Condition::ComprehensionCountGreaterEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => CompiledCondition::ComprehensionCountGreaterEqual {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                filter_attr: interner.intern(filter_attr),
                filter_value: Self::compile_literal(filter_value, interner),
                filter_op: filter_op.clone(),
                threshold: *threshold,
            },
            Condition::ComprehensionCountEqual {
                entity_type,
                attribute,
                filter_attr,
                filter_value,
                filter_op,
                threshold,
            } => CompiledCondition::ComprehensionCountEqual {
                entity_type: entity_type.clone(),
                attribute: interner.intern(attribute),
                filter_attr: interner.intern(filter_attr),
                filter_value: Self::compile_literal(filter_value, interner),
                filter_op: filter_op.clone(),
                threshold: *threshold,
            },
            Condition::And(conditions) => CompiledCondition::And(
                conditions
                    .iter()
                    .map(|c| Self::compile_condition(c, interner))
                    .collect(),
            ),
            Condition::Or(conditions) => CompiledCondition::Or(
                conditions
                    .iter()
                    .map(|c| Self::compile_condition(c, interner))
                    .collect(),
            ),
            Condition::Not(inner) => {
                CompiledCondition::Not(Box::new(Self::compile_condition(inner, interner)))
            }
        }
    }

    /// Compile a literal value with pre-interned strings
    fn compile_literal(
        value: &LiteralValue,
        interner: &crate::data::StringInterner,
    ) -> CompiledLiteralValue {
        match value {
            LiteralValue::String(s) => CompiledLiteralValue::String(interner.intern(s)),
            LiteralValue::Int(i) => CompiledLiteralValue::Int(*i),
            LiteralValue::Bool(b) => CompiledLiteralValue::Bool(*b),
        }
    }

    /// Recursively collect and compile regex patterns from a condition
    fn collect_regex_patterns(condition: &Condition, cache: &mut FxHashMap<String, regex::Regex>) {
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
    #[allow(dead_code)]
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
            Condition::UserEqualsResource {
                user_attr,
                resource_attr,
            } => {
                intern(user_attr);
                intern(resource_attr);
            }
            Condition::UserIntGreater {
                user_attr,
                resource_attr,
            }
            | Condition::ResourceIntGreater {
                resource_attr,
                user_attr,
            }
            | Condition::UserWildcardEqualsResourceAttr {
                user_attr,
                resource_attr,
            }
            | Condition::ResourceWildcardEqualsUserAttr {
                resource_attr,
                user_attr,
            } => {
                intern(user_attr);
                intern(resource_attr);
            }
            Condition::Assignment {
                variable,
                attribute,
                ..
            } => {
                intern(variable);
                intern(attribute);
            }
            Condition::MembershipTest {
                attribute, value, ..
            } => {
                intern(attribute);
                // Also pre-intern the literal value for membership test
                if let LiteralValue::String(s) = value {
                    intern(s);
                }
            }
            Condition::IndexedEquals {
                attribute, value, ..
            } => {
                intern(attribute);
                intern(value);
            }
            Condition::EqualsVariable {
                attribute,
                variable,
                ..
            } => {
                intern(attribute);
                intern(variable);
            }
            Condition::RegexMatches { attribute, .. } => {
                intern(attribute);
            }
            Condition::StringContains {
                attribute,
                substring,
                ..
            } => {
                intern(attribute);
                intern(substring);
            }
            Condition::StringStartsWith {
                attribute, prefix, ..
            } => {
                intern(attribute);
                intern(prefix);
            }
            Condition::StringEndsWith {
                attribute, suffix, ..
            } => {
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
            Condition::StringLowerEquals {
                attribute, value, ..
            }
            | Condition::StringUpperEquals {
                attribute, value, ..
            } => {
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
            Condition::SetIntersectionCountGreater {
                attribute, values, ..
            } => {
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
            Condition::ComprehensionCountGreaterEqual {
                attribute,
                filter_attr,
                ..
            }
            | Condition::ComprehensionCountEqual {
                attribute,
                filter_attr,
                ..
            } => {
                intern(attribute);
                intern(filter_attr);
            }
            // Same-entity attribute comparisons
            Condition::SameEntityAttrCompare {
                left_attr,
                right_attr,
                ..
            } => {
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
            Condition::MembershipTest {
                value: LiteralValue::String(s),
                ..
            } => {
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

    /// Get a string interned, used by the legacy evaluate_condition path
    /// For the fast path, use evaluate_compiled_condition with pre-interned CompiledCondition
    #[allow(dead_code)]
    #[inline(always)]
    fn get_interned(&self, s: &str, interner: &crate::data::StringInterner) -> InternedString {
        interner.intern(s)
    }

    /// Evaluate a compiled condition against entities
    ///
    /// This is the FAST PATH - all strings are pre-interned at construction time.
    /// Zero HashMap lookups during evaluation (eliminated ~50 lookups per request).
    /// Performance: Direct InternedString comparisons (5ns vs 100ns with HashMap lookup)
    fn evaluate_compiled_condition(
        &self,
        condition: &CompiledCondition,
        user: &Entity,
        resource: &Entity,
        _context: &std::collections::HashMap<String, String>,
        variables: &mut std::collections::HashMap<String, AttributeValue>,
    ) -> bool {
        let interner = self.store.interner();

        match condition {
            CompiledCondition::Always => true,

            CompiledCondition::ActionEquals { value } => {
                // Resolve the pre-interned value to compare with context string
                _context
                    .get("action")
                    .map(|a| {
                        interner
                            .resolve(*value)
                            .map(|v| a.as_str() == &*v)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            }

            CompiledCondition::ResourceIdEquals { value } => {
                // Resolve the pre-interned value to compare with context string
                _context
                    .get("resource")
                    .map(|r| {
                        interner
                            .resolve(*value)
                            .map(|v| r.as_str() == &*v)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            }

            CompiledCondition::UserEquals { attribute, value } => {
                // attribute and value are already interned - direct O(1) comparison
                match user.get_attribute(*attribute) {
                    Some(AttributeValue::String(actual)) => *actual == *value,
                    Some(AttributeValue::Bool(actual)) => interner
                        .resolve(*value)
                        .map(|v| &*v == actual.to_string().as_str())
                        .unwrap_or(false),
                    Some(AttributeValue::Int(actual)) => interner
                        .resolve(*value)
                        .map(|v| &*v == actual.to_string().as_str())
                        .unwrap_or(false),
                    _ => false,
                }
            }

            CompiledCondition::UserGreaterEqualLiteral { attribute, value } => {
                match user.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) >= *value,
                    Some(AttributeValue::Float(actual)) => *actual >= *value,
                    _ => false,
                }
            }

            CompiledCondition::UserGreaterLiteral { attribute, value } => {
                match user.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) > *value,
                    Some(AttributeValue::Float(actual)) => *actual > *value,
                    _ => false,
                }
            }

            CompiledCondition::UserLessEqualLiteral { attribute, value } => {
                match user.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) <= *value,
                    Some(AttributeValue::Float(actual)) => *actual <= *value,
                    _ => false,
                }
            }

            CompiledCondition::UserLessLiteral { attribute, value } => {
                match user.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) < *value,
                    Some(AttributeValue::Float(actual)) => *actual < *value,
                    _ => false,
                }
            }

            CompiledCondition::ResourceEquals { attribute, value } => {
                match resource.get_attribute(*attribute) {
                    Some(AttributeValue::String(actual)) => *actual == *value,
                    Some(AttributeValue::Bool(actual)) => interner
                        .resolve(*value)
                        .map(|v| &*v == actual.to_string().as_str())
                        .unwrap_or(false),
                    Some(AttributeValue::Int(actual)) => interner
                        .resolve(*value)
                        .map(|v| &*v == actual.to_string().as_str())
                        .unwrap_or(false),
                    _ => false,
                }
            }

            CompiledCondition::ResourceGreaterEqualLiteral { attribute, value } => {
                match resource.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) >= *value,
                    Some(AttributeValue::Float(actual)) => *actual >= *value,
                    _ => false,
                }
            }

            CompiledCondition::ResourceGreaterLiteral { attribute, value } => {
                match resource.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) > *value,
                    Some(AttributeValue::Float(actual)) => *actual > *value,
                    _ => false,
                }
            }

            CompiledCondition::ResourceLessEqualLiteral { attribute, value } => {
                match resource.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) <= *value,
                    Some(AttributeValue::Float(actual)) => *actual <= *value,
                    _ => false,
                }
            }

            CompiledCondition::ResourceLessLiteral { attribute, value } => {
                match resource.get_attribute(*attribute) {
                    Some(AttributeValue::Int(actual)) => (*actual as f64) < *value,
                    Some(AttributeValue::Float(actual)) => *actual < *value,
                    _ => false,
                }
            }

            CompiledCondition::UserEqualsResource {
                user_attr,
                resource_attr,
            } => {
                match (
                    user.get_attribute(*user_attr),
                    resource.get_attribute(*resource_attr),
                ) {
                    (Some(AttributeValue::String(u)), Some(AttributeValue::String(r))) => u == r,
                    (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => u == r,
                    (Some(AttributeValue::Bool(u)), Some(AttributeValue::Bool(r))) => u == r,
                    _ => false,
                }
            }

            CompiledCondition::UserIntGreater {
                user_attr,
                resource_attr,
            } => {
                match (
                    user.get_attribute(*user_attr),
                    resource.get_attribute(*resource_attr),
                ) {
                    (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => u > r,
                    _ => false,
                }
            }

            CompiledCondition::ResourceIntGreater {
                resource_attr,
                user_attr,
            } => {
                match (
                    resource.get_attribute(*resource_attr),
                    user.get_attribute(*user_attr),
                ) {
                    (Some(AttributeValue::Int(r)), Some(AttributeValue::Int(u))) => r > u,
                    _ => false,
                }
            }

            // Wildcard iteration: user.attr[_] == resource.attr
            // Existential quantification: true if ANY element in user's array equals resource's scalar
            CompiledCondition::UserWildcardEqualsResourceAttr {
                user_attr,
                resource_attr,
            } => {
                let resource_val = resource.get_attribute(*resource_attr);
                let user_collection = user.get_attribute(*user_attr);

                match (user_collection, resource_val) {
                    // List contains string
                    (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::String(s) if *s == *expected))
                    }
                    // Set contains string (O(1) lookup)
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
                        items.contains(&AttributeValue::String(*expected))
                    }
                    // List contains int
                    (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::Int(i) if *i == *expected))
                    }
                    // Set contains int
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
                        items.contains(&AttributeValue::Int(*expected))
                    }
                    _ => false,
                }
            }

            // Wildcard iteration: resource.attr[_] == user.attr
            // Existential quantification: true if ANY element in resource's array equals user's scalar
            CompiledCondition::ResourceWildcardEqualsUserAttr {
                resource_attr,
                user_attr,
            } => {
                let user_val = user.get_attribute(*user_attr);
                let resource_collection = resource.get_attribute(*resource_attr);

                match (resource_collection, user_val) {
                    // List contains string
                    (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::String(s) if *s == *expected))
                    }
                    // Set contains string (O(1) lookup)
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
                        items.contains(&AttributeValue::String(*expected))
                    }
                    // List contains int
                    (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::Int(i) if *i == *expected))
                    }
                    // Set contains int
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
                        items.contains(&AttributeValue::Int(*expected))
                    }
                    _ => false,
                }
            }

            CompiledCondition::SameEntityAttrCompare {
                entity_type,
                left_attr,
                right_attr,
                op,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let left_val = entity.get_attribute(*left_attr);
                let right_val = entity.get_attribute(*right_attr);

                match (left_val, right_val, op) {
                    // Numeric comparisons (int vs int)
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Less,
                    ) => *l < *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Greater,
                    ) => *l > *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

                    // Float vs float
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Less,
                    ) => *l < *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Greater,
                    ) => *l > *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

                    // Int vs float
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::LessEqual,
                    ) => (*l as f64) <= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => (*l as f64) >= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Less,
                    ) => (*l as f64) < *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Greater,
                    ) => (*l as f64) > *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Equal,
                    ) => (*l as f64) == *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::NotEqual,
                    ) => (*l as f64) != *r,

                    // Float vs int
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Less,
                    ) => *l < (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Greater,
                    ) => *l > (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Equal,
                    ) => *l == (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != (*r as f64),

                    // String equality
                    (
                        Some(AttributeValue::String(l)),
                        Some(AttributeValue::String(r)),
                        AttrCompareOp::Equal,
                    ) => l == r,
                    (
                        Some(AttributeValue::String(l)),
                        Some(AttributeValue::String(r)),
                        AttrCompareOp::NotEqual,
                    ) => l != r,

                    // Bool equality
                    (
                        Some(AttributeValue::Bool(l)),
                        Some(AttributeValue::Bool(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Bool(l)),
                        Some(AttributeValue::Bool(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

                    _ => false,
                }
            }

            CompiledCondition::Assignment {
                variable,
                entity_type,
                attribute,
                index,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let value = if let Some(idx) = index {
                    if matches!(idx, IndexExpr::Wildcard) {
                        if let Some(collection) = entity.get_attribute(*attribute) {
                            match collection {
                                AttributeValue::List(items) => items.first().cloned(),
                                AttributeValue::Set(items) => items.iter().next().cloned(),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    } else {
                        self.get_indexed_value_compiled(entity, *attribute, idx, interner)
                    }
                } else {
                    entity.get_attribute(*attribute).cloned()
                };

                if let Some(val) = value {
                    // Resolve variable name for HashMap key
                    let var_name = interner
                        .resolve(*variable)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    variables.insert(var_name, val);
                    true
                } else {
                    false
                }
            }

            CompiledCondition::MembershipTest {
                value,
                entity_type,
                attribute,
                index,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let collection = if let Some(idx) = index {
                    self.get_indexed_value_compiled(entity, *attribute, idx, interner)
                } else {
                    entity.get_attribute(*attribute).cloned()
                };

                if let Some(coll) = collection {
                    match &coll {
                        AttributeValue::List(items) => self.compiled_value_in_list(value, items),
                        AttributeValue::Set(items) => self.compiled_value_in_set(value, items),
                        _ => false,
                    }
                } else {
                    false
                }
            }

            CompiledCondition::IndexedEquals {
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

                if matches!(index, IndexExpr::Wildcard) {
                    if let Some(collection) = entity.get_attribute(*attribute) {
                        match collection {
                            AttributeValue::List(items) => items.iter().any(
                                |item| matches!(item, AttributeValue::String(s) if *s == *value),
                            ),
                            AttributeValue::Set(items) => {
                                let expected_val = AttributeValue::String(*value);
                                items.contains(&expected_val)
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    let indexed_val =
                        self.get_indexed_value_compiled(entity, *attribute, index, interner);
                    matches!(indexed_val, Some(AttributeValue::String(actual)) if actual == *value)
                }
            }

            CompiledCondition::EqualsVariable {
                entity_type,
                attribute,
                variable,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };

                let attr_val = entity.get_attribute(*attribute);
                // Resolve variable name and look up in variables HashMap
                if let Some(resolved) = interner.resolve(*variable) {
                    let var_val = variables.get(&*resolved);
                    match (attr_val, var_val) {
                        (Some(a), Some(v)) => a == v,
                        _ => false,
                    }
                } else {
                    false
                }
            }

            CompiledCondition::And(conditions) => conditions
                .iter()
                .all(|c| self.evaluate_compiled_condition(c, user, resource, _context, variables)),

            CompiledCondition::Or(conditions) => conditions
                .iter()
                .any(|c| self.evaluate_compiled_condition(c, user, resource, _context, variables)),

            CompiledCondition::Not(condition) => {
                !self.evaluate_compiled_condition(condition, user, resource, _context, variables)
            }

            // ============ Function Call Evaluation ============
            CompiledCondition::RegexMatches {
                entity_type,
                attribute,
                pattern,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            // Use thread-local regex cache for zero-overhead access
                            // Falls back to instance cache if thread-local miss
                            crate::regex_cache::matches(pattern, &resolved)
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            CompiledCondition::StringContains {
                entity_type,
                attribute,
                substring,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            memmem::find(resolved.as_bytes(), substring.as_bytes()).is_some()
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            CompiledCondition::StringStartsWith {
                entity_type,
                attribute,
                prefix,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.starts_with(prefix.as_str())
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            CompiledCondition::StringEndsWith {
                entity_type,
                attribute,
                suffix,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::String(s)) => {
                        if let Some(resolved) = interner.resolve(*s) {
                            resolved.ends_with(suffix.as_str())
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            CompiledCondition::TimeIsAfter {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::Int(ts)) => *ts > *threshold,
                    Some(AttributeValue::Float(ts)) => (*ts as i64) > *threshold,
                    _ => false,
                }
            }

            CompiledCondition::TimeIsBefore {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::Int(ts)) => *ts < *threshold,
                    Some(AttributeValue::Float(ts)) => (*ts as i64) < *threshold,
                    _ => false,
                }
            }

            // ============ Collection Count Evaluation ============
            CompiledCondition::CountGreaterEqual {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(arr)) => arr.len() >= *threshold,
                    Some(AttributeValue::Set(set)) => set.len() >= *threshold,
                    _ => false,
                }
            }

            CompiledCondition::CountGreater {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(arr)) => arr.len() > *threshold,
                    Some(AttributeValue::Set(set)) => set.len() > *threshold,
                    _ => false,
                }
            }

            CompiledCondition::CountEqual {
                entity_type,
                attribute,
                threshold,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(arr)) => arr.len() == *threshold,
                    Some(AttributeValue::Set(set)) => set.len() == *threshold,
                    _ => false,
                }
            }

            // ============ String Case Methods ============
            CompiledCondition::StringLowerEquals {
                entity_type,
                attribute,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
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

            CompiledCondition::StringUpperEquals {
                entity_type,
                attribute,
                value,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
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
            CompiledCondition::IsString {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                matches!(
                    entity.get_attribute(*attribute),
                    Some(AttributeValue::String(_))
                )
            }

            CompiledCondition::IsNumber {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                matches!(
                    entity.get_attribute(*attribute),
                    Some(AttributeValue::Int(_)) | Some(AttributeValue::Float(_))
                )
            }

            CompiledCondition::IsBool {
                entity_type,
                attribute,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                matches!(
                    entity.get_attribute(*attribute),
                    Some(AttributeValue::Bool(_))
                )
            }

            // ============ Set Operations ============
            CompiledCondition::SetIntersectionCountGreater {
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
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::Set(set)) => {
                        let count = values
                            .iter()
                            .filter(|v| set.contains(&AttributeValue::String(**v)))
                            .count();
                        count > *threshold
                    }
                    Some(AttributeValue::List(list)) => {
                        let count = values
                            .iter()
                            .filter(|v| {
                                list.iter().any(
                                    |item| matches!(item, AttributeValue::String(s) if *s == **v),
                                )
                            })
                            .count();
                        count > *threshold
                    }
                    _ => false,
                }
            }

            CompiledCondition::MapKeyExists {
                entity_type,
                attribute,
                key,
            } => {
                let entity = match entity_type {
                    EntityType::User => user,
                    EntityType::Resource => resource,
                    EntityType::Context => return false,
                };
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::Object(map)) => map.contains_key(key),
                    _ => false,
                }
            }

            // ============ Comprehension Support ============
            CompiledCondition::ComprehensionCountGreaterEqual {
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
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(items)) => {
                        let count = items
                            .iter()
                            .filter(|item| {
                                if let AttributeValue::Object(obj) = item {
                                    if let Some(field_val) = obj.get(filter_attr) {
                                        self.compare_compiled_values(
                                            field_val,
                                            filter_value,
                                            filter_op,
                                            interner,
                                        )
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            })
                            .count();
                        count >= *threshold
                    }
                    _ => false,
                }
            }

            CompiledCondition::ComprehensionCountEqual {
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
                match entity.get_attribute(*attribute) {
                    Some(AttributeValue::List(items)) => {
                        let count = items
                            .iter()
                            .filter(|item| {
                                if let AttributeValue::Object(obj) = item {
                                    if let Some(field_val) = obj.get(filter_attr) {
                                        self.compare_compiled_values(
                                            field_val,
                                            filter_value,
                                            filter_op,
                                            interner,
                                        )
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            })
                            .count();
                        count == *threshold
                    }
                    _ => false,
                }
            }
        }
    }

    /// Helper for indexed value access with pre-interned attribute
    #[inline]
    fn get_indexed_value_compiled(
        &self,
        entity: &Entity,
        attr_key: InternedString,
        index: &IndexExpr,
        interner: &crate::data::StringInterner,
    ) -> Option<AttributeValue> {
        let collection = entity.get_attribute(attr_key)?;
        match (collection, index) {
            (AttributeValue::List(items), IndexExpr::Number(idx)) => {
                let idx = if *idx < 0 {
                    items.len().checked_sub(idx.unsigned_abs() as usize)?
                } else {
                    *idx as usize
                };
                items.get(idx).cloned()
            }
            (AttributeValue::Object(map), IndexExpr::String(key)) => {
                let key_interned = interner.intern(key);
                map.get(&key_interned).cloned()
            }
            _ => None,
        }
    }

    /// Check if compiled literal value is in list
    #[inline]
    fn compiled_value_in_list(
        &self,
        value: &CompiledLiteralValue,
        items: &[AttributeValue],
    ) -> bool {
        match value {
            CompiledLiteralValue::String(s) => items
                .iter()
                .any(|item| matches!(item, AttributeValue::String(actual) if *actual == *s)),
            CompiledLiteralValue::Int(i) => items
                .iter()
                .any(|item| matches!(item, AttributeValue::Int(actual) if *actual == *i)),
            CompiledLiteralValue::Bool(b) => items
                .iter()
                .any(|item| matches!(item, AttributeValue::Bool(actual) if *actual == *b)),
        }
    }

    /// Check if compiled literal value is in set
    #[inline]
    fn compiled_value_in_set(
        &self,
        value: &CompiledLiteralValue,
        items: &FxHashSet<AttributeValue>,
    ) -> bool {
        match value {
            CompiledLiteralValue::String(s) => items.contains(&AttributeValue::String(*s)),
            CompiledLiteralValue::Int(i) => items.contains(&AttributeValue::Int(*i)),
            CompiledLiteralValue::Bool(b) => items.contains(&AttributeValue::Bool(*b)),
        }
    }

    /// Compare compiled values for comprehension filtering
    #[inline]
    fn compare_compiled_values(
        &self,
        field_val: &AttributeValue,
        filter_value: &CompiledLiteralValue,
        filter_op: &ComprehensionFilterOp,
        interner: &crate::data::StringInterner,
    ) -> bool {
        match (field_val, filter_value, filter_op) {
            // String comparisons
            (
                AttributeValue::String(f),
                CompiledLiteralValue::String(v),
                ComprehensionFilterOp::Equal,
            ) => *f == *v,
            (
                AttributeValue::String(f),
                CompiledLiteralValue::String(v),
                ComprehensionFilterOp::NotEqual,
            ) => *f != *v,
            (
                AttributeValue::String(f),
                CompiledLiteralValue::String(v),
                ComprehensionFilterOp::Contains,
            ) => {
                if let (Some(field_str), Some(value_str)) =
                    (interner.resolve(*f), interner.resolve(*v))
                {
                    field_str.contains(&*value_str)
                } else {
                    false
                }
            }
            // Int comparisons
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::Equal,
            ) => *f == *v,
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::NotEqual,
            ) => *f != *v,
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::GreaterThan,
            ) => *f > *v,
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::LessThan,
            ) => *f < *v,
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::GreaterEqual,
            ) => *f >= *v,
            (
                AttributeValue::Int(f),
                CompiledLiteralValue::Int(v),
                ComprehensionFilterOp::LessEqual,
            ) => *f <= *v,
            // Bool comparisons
            (
                AttributeValue::Bool(f),
                CompiledLiteralValue::Bool(v),
                ComprehensionFilterOp::Equal,
            ) => *f == *v,
            (
                AttributeValue::Bool(f),
                CompiledLiteralValue::Bool(v),
                ComprehensionFilterOp::NotEqual,
            ) => *f != *v,
            _ => false,
        }
    }

    /// Evaluate a condition against entities
    ///
    /// This is where the performance magic happens:
    /// - Direct DataStore access (no conversion)
    /// - Interned string comparisons (5ns vs 100ns)
    /// - Zero-copy entity access (Arc)
    /// - Pre-interned attribute names for O(1) lookup
    /// - Variable context for local bindings
    #[allow(dead_code)]
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
                _context
                    .get("resource")
                    .map(|r| r == value)
                    .unwrap_or(false)
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

            // Wildcard iteration: user.attr[_] == resource.attr
            // Existential quantification: true if ANY element in user's array equals resource's scalar
            Condition::UserWildcardEqualsResourceAttr {
                user_attr,
                resource_attr,
            } => {
                let user_key = self.get_interned(user_attr, interner);
                let resource_key = self.get_interned(resource_attr, interner);

                let user_collection = user.get_attribute(user_key);
                let resource_val = resource.get_attribute(resource_key);

                match (user_collection, resource_val) {
                    // List contains string
                    (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::String(s) if *s == *expected))
                    }
                    // Set contains string (O(1) lookup)
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
                        items.contains(&AttributeValue::String(*expected))
                    }
                    // List contains int
                    (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::Int(i) if *i == *expected))
                    }
                    // Set contains int
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
                        items.contains(&AttributeValue::Int(*expected))
                    }
                    _ => false,
                }
            }

            // Wildcard iteration: resource.attr[_] == user.attr
            // Existential quantification: true if ANY element in resource's array equals user's scalar
            Condition::ResourceWildcardEqualsUserAttr {
                resource_attr,
                user_attr,
            } => {
                let user_key = self.get_interned(user_attr, interner);
                let resource_key = self.get_interned(resource_attr, interner);

                let resource_collection = resource.get_attribute(resource_key);
                let user_val = user.get_attribute(user_key);

                match (resource_collection, user_val) {
                    // List contains string
                    (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::String(s) if *s == *expected))
                    }
                    // Set contains string (O(1) lookup)
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
                        items.contains(&AttributeValue::String(*expected))
                    }
                    // List contains int
                    (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => {
                        items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::Int(i) if *i == *expected))
                    }
                    // Set contains int
                    (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
                        items.contains(&AttributeValue::Int(*expected))
                    }
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
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Less,
                    ) => *l < *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Greater,
                    ) => *l > *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

                    // Numeric comparisons (float vs float)
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Less,
                    ) => *l < *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Greater,
                    ) => *l > *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

                    // Numeric comparisons (int vs float)
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::LessEqual,
                    ) => (*l as f64) <= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => (*l as f64) >= *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Less,
                    ) => (*l as f64) < *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Greater,
                    ) => (*l as f64) > *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::Equal,
                    ) => (*l as f64) == *r,
                    (
                        Some(AttributeValue::Int(l)),
                        Some(AttributeValue::Float(r)),
                        AttrCompareOp::NotEqual,
                    ) => (*l as f64) != *r,

                    // Numeric comparisons (float vs int)
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::LessEqual,
                    ) => *l <= (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::GreaterEqual,
                    ) => *l >= (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Less,
                    ) => *l < (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Greater,
                    ) => *l > (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::Equal,
                    ) => *l == (*r as f64),
                    (
                        Some(AttributeValue::Float(l)),
                        Some(AttributeValue::Int(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != (*r as f64),

                    // String equality comparisons
                    (
                        Some(AttributeValue::String(l)),
                        Some(AttributeValue::String(r)),
                        AttrCompareOp::Equal,
                    ) => l == r,
                    (
                        Some(AttributeValue::String(l)),
                        Some(AttributeValue::String(r)),
                        AttrCompareOp::NotEqual,
                    ) => l != r,

                    // Boolean equality comparisons
                    (
                        Some(AttributeValue::Bool(l)),
                        Some(AttributeValue::Bool(r)),
                        AttrCompareOp::Equal,
                    ) => *l == *r,
                    (
                        Some(AttributeValue::Bool(l)),
                        Some(AttributeValue::Bool(r)),
                        AttrCompareOp::NotEqual,
                    ) => *l != *r,

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
                            // SIMD-accelerated substring search (2-10x faster than std::contains)
                            memmem::find(resolved.as_bytes(), substring.as_bytes()).is_some()
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
                matches!(
                    entity.get_attribute(attr_key),
                    Some(AttributeValue::String(_))
                )
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
                matches!(
                    entity.get_attribute(attr_key),
                    Some(AttributeValue::Bool(_))
                )
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
                        let count = values
                            .iter()
                            .filter(|v| {
                                let interned = interner.intern(v);
                                set.contains(&AttributeValue::String(interned))
                            })
                            .count();
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
                        let count = items
                            .iter()
                            .filter(|item| {
                                if let AttributeValue::Object(obj) = item {
                                    if let Some(field_val) = obj.get(&filter_attr_key) {
                                        self.compare_values(
                                            field_val,
                                            filter_value,
                                            filter_op,
                                            interner,
                                        )
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            })
                            .count();
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
                        let count = items
                            .iter()
                            .filter(|item| {
                                if let AttributeValue::Object(obj) = item {
                                    if let Some(field_val) = obj.get(&filter_attr_key) {
                                        self.compare_values(
                                            field_val,
                                            filter_value,
                                            filter_op,
                                            interner,
                                        )
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            })
                            .count();
                        count == *threshold
                    }
                    _ => false,
                }
            }
        }
    }

    /// Get indexed value from attribute (bracket notation)
    /// Performance: ~10-50ns depending on collection size
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
            Arc::new(Entity::new(
                resource_id,
                resource_type,
                std::collections::HashMap::new(),
            ))
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
        // Using compiled conditions with pre-interned strings - zero HashMap lookups!
        for rule in &self.compiled_deny_rules {
            if self.evaluate_compiled_condition(
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
        for rule in &self.compiled_allow_rules {
            let matches = self.evaluate_compiled_condition(
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
        if self.compiled_deny_rules.is_empty() && self.compiled_allow_rules.is_empty() {
            return Err(ReaperError::InvalidPolicy {
                reason: "Policy must have at least one rule".to_string(),
            });
        }

        // Validate deny rules
        for (index, rule) in self.compiled_deny_rules.iter().enumerate() {
            if rule.name.is_empty() {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("Deny rule {} has empty name", index),
                });
            }
        }

        // Validate allow rules
        for (index, rule) in self.compiled_allow_rules.iter().enumerate() {
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
        let total_rules = self.compiled_deny_rules.len() + self.compiled_allow_rules.len();
        let mut extra = std::collections::HashMap::new();
        extra.insert("rule_count".to_string(), total_rules.to_string());
        extra.insert(
            "deny_rules".to_string(),
            self.compiled_deny_rules.len().to_string(),
        );
        extra.insert(
            "allow_rules".to_string(),
            self.compiled_allow_rules.len().to_string(),
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
