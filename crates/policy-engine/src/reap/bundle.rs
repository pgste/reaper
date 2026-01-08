//! Binary Bundle Format (.rbb)
//!
//! Compiles .reap policies into optimized binary bundles for instant loading.
//!
//! The bundle format preserves the full Reaper Policy AST, allowing the compiled
//! ReaperDSLEvaluator to be rebuilt at deployment time with full functionality.

use super::ast::{Decision as ReapDecision, Policy};
use super::compiler;
use crate::data::DataStore;
use crate::engine::{EnhancedPolicy, PolicyAction, PolicyLanguage, PolicyRule};
use crate::evaluators::PolicyEvaluator;
use reaper_core::ReaperError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;
use uuid::Uuid;

/// Bundle format metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFormat {
    /// Format version
    pub version: u32,
    /// Compilation timestamp
    pub compiled_at: u64,
    /// Original policy name
    pub policy_name: String,
    /// Policy version (if any)
    pub policy_version: Option<String>,
    /// Checksum of source
    pub source_checksum: u64,
}

/// Complete policy bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub metadata: BundleFormat,
    pub policy: Policy,
}

impl PolicyBundle {
    const MAGIC_BYTES: &'static [u8; 4] = b"REAP";
    const FORMAT_VERSION: u32 = 1;

    /// Create a new bundle from a policy
    pub fn new(policy: Policy) -> Self {
        let policy_version = policy.metadata.get("version").cloned();
        let source_checksum = calculate_checksum(&policy);

        let metadata = BundleFormat {
            version: Self::FORMAT_VERSION,
            compiled_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            policy_name: policy.name.clone(),
            policy_version,
            source_checksum,
        };

        Self { metadata, policy }
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();

        // Magic bytes
        bytes.extend_from_slice(Self::MAGIC_BYTES);

        // Serialize bundle
        let bundle_bytes = bincode::serialize(self).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;

        bytes.extend_from_slice(&bundle_bytes);

        Ok(bytes)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        // Check magic bytes
        if bytes.len() < 4 || &bytes[0..4] != Self::MAGIC_BYTES {
            return Err(ReaperError::InvalidPolicy {
                reason: "Invalid bundle format: magic bytes mismatch".to_string(),
            });
        }

        // Deserialize
        let bundle: Self =
            bincode::deserialize(&bytes[4..]).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize bundle: {}", e),
            })?;

        // Version check
        if bundle.metadata.version > Self::FORMAT_VERSION {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Bundle version {} is newer than supported version {}",
                    bundle.metadata.version,
                    Self::FORMAT_VERSION
                ),
            });
        }

        Ok(bundle)
    }

    /// Convert bundle to EnhancedPolicy for use with PolicyEngine
    ///
    /// This is a legacy method that creates a Simple policy.
    /// For full functionality, use `to_enhanced_policy_with_store()` instead.
    pub fn to_enhanced_policy(&self) -> Result<EnhancedPolicy, ReaperError> {
        // Convert Reaper DSL rules to Simple rules (limited functionality)
        let mut rules = Vec::new();

        for rule in &self.policy.rules {
            let action = match rule.decision {
                ReapDecision::Allow => PolicyAction::Allow,
                ReapDecision::Deny => PolicyAction::Deny,
            };

            let simple_rule = PolicyRule {
                action,
                resource: "*".to_string(),
                conditions: vec![],
            };

            rules.push(simple_rule);
        }

        if rules.is_empty() {
            let default_action = match self.policy.default_decision {
                ReapDecision::Allow => PolicyAction::Allow,
                ReapDecision::Deny => PolicyAction::Deny,
            };

            rules.push(PolicyRule {
                action: default_action,
                resource: "*".to_string(),
                conditions: vec![],
            });
        }

        let mut policy = EnhancedPolicy::new(
            self.policy.name.clone(),
            format!(
                "Deployed from bundle version {}",
                self.metadata.policy_version.as_deref().unwrap_or("unknown")
            ),
            rules,
        );

        policy.id = Uuid::new_v4();

        if let Some(version) = &self.metadata.policy_version {
            policy
                .metadata
                .insert("bundle_version".to_string(), version.clone());
        }
        policy.metadata.insert(
            "bundle_checksum".to_string(),
            self.metadata.source_checksum.to_string(),
        );

        Ok(policy)
    }

    /// Convert bundle to EnhancedPolicy with a compiled ReaperDSLEvaluator
    ///
    /// This method compiles the bundle's Policy AST into a full-featured
    /// ReaperDSLEvaluator, preserving all complex conditions, functions,
    /// and rule logic. This is the recommended method for bundle deployment.
    ///
    /// # Arguments
    /// * `store` - DataStore containing entity data for the evaluator
    ///
    /// # Returns
    /// EnhancedPolicy with the compiled evaluator attached
    pub fn to_enhanced_policy_with_store(
        &self,
        store: Arc<DataStore>,
    ) -> Result<EnhancedPolicy, ReaperError> {
        // Compile the policy AST using the ReaperDSL compiler
        let evaluator = compiler::compile_policy(self.policy.clone(), store)?;

        // Serialize the original policy content for the EnhancedPolicy
        let content = serde_json::to_string(&self.policy).unwrap_or_default();

        // Create EnhancedPolicy with the compiled evaluator
        let now = chrono::Utc::now();
        let policy = EnhancedPolicy {
            id: Uuid::new_v4(),
            version: 1,
            name: self.policy.name.clone(),
            description: format!(
                "Deployed from bundle version {}",
                self.metadata.policy_version.as_deref().unwrap_or("unknown")
            ),
            language: PolicyLanguage::Custom,
            content,
            rules: vec![], // Rules are in the compiled evaluator
            metadata: {
                let mut m = std::collections::HashMap::new();
                if let Some(version) = &self.metadata.policy_version {
                    m.insert("bundle_version".to_string(), version.clone());
                }
                m.insert(
                    "bundle_checksum".to_string(),
                    self.metadata.source_checksum.to_string(),
                );
                m.insert("compiled".to_string(), "true".to_string());
                m.insert(
                    "rules_count".to_string(),
                    self.policy.rules.len().to_string(),
                );
                m
            },
            priority: 100,
            created_at: now,
            updated_at: now,
            evaluator: Some(Arc::new(evaluator) as Arc<dyn PolicyEvaluator>),
            source_metadata: None,
        };

        Ok(policy)
    }
}

/// Compile a policy AST to a binary bundle
pub fn compile_to_bundle(policy: &Policy) -> Result<Vec<u8>, ReaperError> {
    let bundle = PolicyBundle::new(policy.clone());
    bundle.to_bytes()
}

/// Load a policy AST from a binary bundle
pub fn load_from_bundle(bytes: &[u8]) -> Result<Policy, ReaperError> {
    let bundle = PolicyBundle::from_bytes(bytes)?;
    Ok(bundle.policy)
}

/// Calculate a simple checksum for a policy
fn calculate_checksum(policy: &Policy) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    policy.name.hash(&mut hasher);
    policy.rules.len().hash(&mut hasher);

    for rule in &policy.rules {
        rule.name.hash(&mut hasher);
    }

    hasher.finish()
}

// ============================================================================
// Enhanced Bundle Format (.rpp) - Reaper Policy Package
// ============================================================================
//
// The .rpp format supports multiple policies with pre-extracted optimization hints.
// While conditions are still compiled at load time (reusing the existing compiler),
// the hints enable the agent to pre-allocate resources and avoid redundant work.

/// Pre-compilation hints extracted during bundle creation
/// These allow the agent to optimize loading by pre-allocating and pre-computing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrecompilationHints {
    /// Strings that should be interned (attribute names, literals, etc.)
    /// Stored as a deduplicated list for efficient bulk interning
    pub strings_to_intern: Vec<String>,
    /// Regex patterns that should be pre-compiled
    /// Stored as pattern strings; agent compiles them once on load
    pub regex_patterns: Vec<String>,
    /// Total number of rules across all policies (for pre-allocation)
    pub total_rules: usize,
    /// Entity IDs referenced in policies (for pre-loading from DataStore)
    pub referenced_entities: Vec<String>,
}

impl PrecompilationHints {
    /// Pre-warm the thread-local regex cache with patterns from these hints.
    ///
    /// Call this at bundle load time to avoid regex compilation latency
    /// during the first policy evaluation.
    ///
    /// # Returns
    /// Number of patterns successfully compiled
    ///
    /// # Example
    /// ```rust,ignore
    /// let package = PolicyPackage::from_bytes(&bytes)?;
    /// let count = package.hints.prewarm_regex_cache();
    /// println!("Pre-compiled {} regex patterns", count);
    /// ```
    pub fn prewarm_regex_cache(&self) -> usize {
        crate::regex_cache::prewarm_patterns_owned(&self.regex_patterns)
    }

    /// Pre-warm the global (cross-thread) regex cache with patterns from these hints.
    ///
    /// Use this when you want compiled regexes available to all threads
    /// without re-compilation per thread.
    ///
    /// # Returns
    /// Number of patterns successfully compiled
    pub fn prewarm_global_regex_cache(&self) -> usize {
        let patterns: Vec<&str> = self.regex_patterns.iter().map(|s| s.as_str()).collect();
        crate::regex_cache::global::prewarm_patterns(&patterns)
    }
}

/// Policy package entry - a single policy with its metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEntry {
    /// The policy AST (will be compiled at load time)
    pub policy: Policy,
    /// Priority for evaluation order (lower = higher priority)
    pub priority: u32,
}

/// Enhanced bundle format supporting multiple policies with optimization hints
///
/// The .rpp (Reaper Policy Package) format stores:
/// - Multiple policy ASTs for batch deployment
/// - Pre-extracted hints for agent-side optimization
/// - Metadata for version tracking and integrity
///
/// Benefits:
/// - Deploy multiple related policies atomically
/// - Hints enable pre-allocation, bulk string interning, and regex caching
/// - Smaller bundles than storing compiled forms (AST compresses better)
/// - Full compiler is used at load time - no functionality loss
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPackage {
    pub metadata: PackageMetadata,
    pub policies: Vec<PolicyEntry>,
    pub hints: PrecompilationHints,
}

/// Metadata for policy packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    /// Package format version
    pub format_version: u32,
    /// Package creation timestamp
    pub created_at: u64,
    /// Package name (e.g., "production-policies")
    pub name: String,
    /// Package version (semantic versioning recommended)
    pub version: String,
    /// SHA-256 hash of source for integrity verification
    pub source_hash: [u8; 32],
    /// Total policy count
    pub policy_count: usize,
}

impl PolicyPackage {
    const MAGIC_BYTES: &'static [u8; 4] = b"REPP"; // Reaper Policy Package
    const FORMAT_VERSION: u32 = 1;

    /// Create a new package from multiple policies
    ///
    /// This extracts optimization hints during packaging for faster loading.
    pub fn new(name: String, version: String, policies: Vec<Policy>) -> Self {
        use sha2::{Digest, Sha256};
        use std::collections::HashSet;

        // Calculate source hash
        let mut hasher = Sha256::new();
        for policy in &policies {
            hasher.update(policy.name.as_bytes());
            hasher.update(policy.rules.len().to_le_bytes());
        }
        let source_hash: [u8; 32] = hasher.finalize().into();

        // Extract hints
        let mut strings: HashSet<String> = HashSet::new();
        let mut regex_patterns: HashSet<String> = HashSet::new();
        let mut total_rules = 0;

        for policy in &policies {
            total_rules += policy.rules.len();
            extract_hints_from_policy(policy, &mut strings, &mut regex_patterns);
        }

        let metadata = PackageMetadata {
            format_version: Self::FORMAT_VERSION,
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            name,
            version,
            source_hash,
            policy_count: policies.len(),
        };

        let hints = PrecompilationHints {
            strings_to_intern: strings.into_iter().collect(),
            regex_patterns: regex_patterns.into_iter().collect(),
            total_rules,
            referenced_entities: Vec::new(),
        };

        let policy_entries: Vec<PolicyEntry> = policies
            .into_iter()
            .enumerate()
            .map(|(idx, policy)| PolicyEntry {
                policy,
                priority: (idx * 100) as u32,
            })
            .collect();

        Self {
            metadata,
            policies: policy_entries,
            hints,
        }
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC_BYTES);

        let bundle_bytes = bincode::serialize(self).map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize policy package: {}", e),
        })?;

        bytes.extend_from_slice(&bundle_bytes);
        Ok(bytes)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        if bytes.len() < 4 || &bytes[0..4] != Self::MAGIC_BYTES {
            return Err(ReaperError::InvalidPolicy {
                reason: "Invalid policy package: magic bytes mismatch".to_string(),
            });
        }

        let bundle: Self =
            bincode::deserialize(&bytes[4..]).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize policy package: {}", e),
            })?;

        if bundle.metadata.format_version > Self::FORMAT_VERSION {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Package version {} is newer than supported version {}",
                    bundle.metadata.format_version,
                    Self::FORMAT_VERSION
                ),
            });
        }

        Ok(bundle)
    }

    /// Deploy all policies in this package to an agent's PolicyEngine
    ///
    /// Uses the optimization hints to pre-intern strings and compile regexes
    /// before compiling and deploying each policy.
    pub fn deploy_to_engine(
        &self,
        engine: &crate::engine::PolicyEngine,
        store: Arc<DataStore>,
    ) -> Result<Vec<crate::engine::PolicyVersion>, ReaperError> {
        // Pre-intern all strings from hints
        let interner = store.interner();
        for s in &self.hints.strings_to_intern {
            interner.intern(s);
        }

        // Compile and deploy each policy
        let mut versions = Vec::with_capacity(self.policies.len());

        for entry in &self.policies {
            // Create a single-policy bundle for deployment
            let bundle = PolicyBundle::new(entry.policy.clone());
            let version = engine.deploy_bundle_with_store(bundle, store.clone(), true)?;
            versions.push(version);
        }

        Ok(versions)
    }
}

/// Extract optimization hints from a policy AST
fn extract_hints_from_policy(
    policy: &Policy,
    strings: &mut std::collections::HashSet<String>,
    regex_patterns: &mut std::collections::HashSet<String>,
) {
    // Add policy name
    strings.insert(policy.name.clone());

    // Walk through rules and conditions
    for rule in &policy.rules {
        strings.insert(rule.name.clone());
        extract_hints_from_condition(&rule.condition, strings, regex_patterns);
    }
}

/// Extract hints from a single condition
fn extract_hints_from_condition(
    condition: &super::ast::Condition,
    strings: &mut std::collections::HashSet<String>,
    regex_patterns: &mut std::collections::HashSet<String>,
) {
    use super::ast::{ComparisonLeft, ComparisonRight, Condition as AstCondition, Value};

    match condition {
        AstCondition::And(conditions) | AstCondition::Or(conditions) => {
            for cond in conditions {
                extract_hints_from_condition(cond, strings, regex_patterns);
            }
        }
        AstCondition::Not(inner) => {
            extract_hints_from_condition(inner, strings, regex_patterns);
        }
        AstCondition::Comparison { left, right, .. } => {
            // Extract attribute names from left side
            match left {
                ComparisonLeft::EntityAttr(ea) => {
                    strings.insert(ea.attribute.clone());
                }
                ComparisonLeft::VarAttr(va) => {
                    strings.insert(va.variable.clone());
                    strings.insert(va.attribute.clone());
                }
                ComparisonLeft::Expr(expr) => {
                    extract_hints_from_expr(expr, strings, regex_patterns);
                }
            }
            // Extract string literals from right side
            match right {
                ComparisonRight::Value(Value::String(s)) => {
                    strings.insert(s.clone());
                }
                ComparisonRight::EntityAttr(ea) => {
                    strings.insert(ea.attribute.clone());
                }
                ComparisonRight::VarAttr(va) => {
                    strings.insert(va.variable.clone());
                    strings.insert(va.attribute.clone());
                }
                ComparisonRight::Expr(expr) => {
                    extract_hints_from_expr(expr, strings, regex_patterns);
                }
                ComparisonRight::Variable(var) => {
                    strings.insert(var.clone());
                }
                _ => {}
            }
        }
        AstCondition::Expr(expr) => {
            extract_hints_from_expr(expr, strings, regex_patterns);
        }
        AstCondition::Assignment { variable, value } => {
            strings.insert(variable.clone());
            extract_hints_from_assignment_value(value, strings, regex_patterns);
        }
        _ => {}
    }
}

/// Extract hints from an expression
fn extract_hints_from_expr(
    expr: &super::ast::Expr,
    strings: &mut std::collections::HashSet<String>,
    regex_patterns: &mut std::collections::HashSet<String>,
) {
    use super::ast::{Expr, Value};

    match expr {
        Expr::Literal(Value::String(s)) => {
            strings.insert(s.clone());
        }
        Expr::FunctionCall {
            namespace,
            function,
            args,
        } => {
            if let Some(ns) = namespace {
                strings.insert(ns.clone());
            }
            strings.insert(function.clone());
            // Check for regex patterns in function args
            if function == "matches" || function == "regex_match" {
                if let Some(Expr::Literal(Value::String(pattern))) = args.get(1) {
                    regex_patterns.insert(pattern.clone());
                }
            }
            for arg in args {
                extract_hints_from_expr(arg, strings, regex_patterns);
            }
        }
        Expr::MethodCall {
            receiver,
            method: _,
            args,
        } => {
            extract_hints_from_expr(receiver, strings, regex_patterns);
            for arg in args {
                extract_hints_from_expr(arg, strings, regex_patterns);
            }
        }
        Expr::Variable(var) => {
            strings.insert(var.clone());
        }
        Expr::AttributeAccess {
            variable,
            attribute,
        } => {
            strings.insert(variable.clone());
            strings.insert(attribute.clone());
        }
        Expr::IndexedAccess {
            variable,
            attribute,
            ..
        } => {
            strings.insert(variable.clone());
            strings.insert(attribute.clone());
        }
        _ => {}
    }
}

/// Extract hints from an assignment value
fn extract_hints_from_assignment_value(
    value: &super::ast::AssignmentValue,
    strings: &mut std::collections::HashSet<String>,
    regex_patterns: &mut std::collections::HashSet<String>,
) {
    use super::ast::{AssignmentValue, Value};

    match value {
        AssignmentValue::EntityAttr(ea) => {
            strings.insert(ea.attribute.clone());
        }
        AssignmentValue::Value(Value::String(s)) => {
            strings.insert(s.clone());
        }
        AssignmentValue::Variable(var) => {
            strings.insert(var.clone());
        }
        AssignmentValue::Expr(expr) => {
            extract_hints_from_expr(expr, strings, regex_patterns);
        }
        AssignmentValue::Comparison { left, .. } => {
            use super::ast::ComparisonLeft;
            if let ComparisonLeft::EntityAttr(ea) = left {
                strings.insert(ea.attribute.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::ast::*;
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_bundle_roundtrip() {
        let policy = Policy {
            name: "test".to_string(),
            metadata: HashMap::new(),
            default_decision: Decision::Deny,
            rules: vec![Rule {
                name: "admin".to_string(),
                decision: Decision::Allow,
                condition: Condition::True,
            }],
        };

        // Compile to bundle
        let bytes = compile_to_bundle(&policy).unwrap();

        // Load from bundle
        let loaded = load_from_bundle(&bytes).unwrap();

        assert_eq!(loaded.name, policy.name);
        assert_eq!(loaded.rules.len(), policy.rules.len());
    }

    #[test]
    fn test_bundle_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("version".to_string(), "1.0.0".to_string());

        let policy = Policy {
            name: "versioned".to_string(),
            metadata,
            default_decision: Decision::Allow,
            rules: vec![],
        };

        let bundle = PolicyBundle::new(policy);

        assert_eq!(bundle.metadata.policy_version, Some("1.0.0".to_string()));
        assert_eq!(bundle.metadata.policy_name, "versioned");
    }
}
