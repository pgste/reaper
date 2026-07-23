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

/// Fixed namespace for deriving stable policy ids from names (UUIDv5).
const POLICY_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x72, 0x65, 0x61, 0x70, 0x65, 0x72, 0x2d, 0x70, 0x6f, 0x6c, 0x69, 0x63, 0x79, 0x2d, 0x69, 0x64,
]);

/// Derive a deterministic policy id from a policy name.
///
/// Same name -> same id, always. Makes bundle deploys idempotent: re-deploying a
/// policy overwrites the existing entry instead of inserting a new random-id copy.
pub fn stable_policy_id(name: &str) -> Uuid {
    Uuid::new_v5(&POLICY_ID_NAMESPACE, name.as_bytes())
}

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

/// The v2 wire shape of a policy — exactly the four fields v2 encoders wrote.
/// Postcard is positional (not self-describing), so `serde(default)` cannot
/// express "field absent on old bytes"; decoding old bundles requires the old
/// shape verbatim, then converting.
#[derive(Serialize, Deserialize)]
struct PolicyWireV2 {
    name: String,
    metadata: std::collections::HashMap<String, String>,
    default_decision: ReapDecision,
    rules: Vec<super::ast::Rule>,
}

impl From<PolicyWireV2> for Policy {
    fn from(p: PolicyWireV2) -> Self {
        Policy {
            name: p.name,
            metadata: p.metadata,
            default_decision: p.default_decision,
            rules: p.rules,
            functions: Vec::new(),
            imports: Vec::new(),
        }
    }
}

impl From<Policy> for PolicyWireV2 {
    fn from(p: Policy) -> Self {
        PolicyWireV2 {
            name: p.name,
            metadata: p.metadata,
            default_decision: p.default_decision,
            rules: p.rules,
        }
    }
}

impl PolicyBundle {
    const MAGIC_BYTES: &'static [u8; 4] = b"REAP";
    /// Format version 3: the policy carries `functions`/`imports` (language
    /// v3, R4-01 Phase C). Version 2 (postcard, replacing bincode v1.3 —
    /// RUSTSEC-2025-0141) is still WRITTEN for policies that use neither, so
    /// v2 engines keep loading function-free bundles; a v3-encoded bundle is
    /// rejected by v2 engines on its wire version AND its `language_version`
    /// metadata — fail closed twice over, never silently dropping functions.
    const FORMAT_VERSION: u32 = 3;
    /// The function-free wire encoding (see [`Self::FORMAT_VERSION`]).
    const LEGACY_FORMAT_VERSION: u32 = 2;

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

    /// Serialize to bytes. Function-free policies encode as wire version 2 —
    /// byte-compatible with v2 engines — so the format only ratchets forward
    /// for policies that actually use v3 constructs.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();

        // Magic bytes
        bytes.extend_from_slice(Self::MAGIC_BYTES);

        let legacy = self.policy.functions.is_empty() && self.policy.imports.is_empty();
        // Postcard encodes a tuple as its fields concatenated — identical
        // bytes to the `PolicyBundle { metadata, policy }` struct encoding.
        let bundle_bytes = if legacy {
            let metadata = BundleFormat {
                version: Self::LEGACY_FORMAT_VERSION,
                ..self.metadata.clone()
            };
            postcard::to_allocvec(&(metadata, PolicyWireV2::from(self.policy.clone())))
        } else {
            let metadata = BundleFormat {
                version: Self::FORMAT_VERSION,
                ..self.metadata.clone()
            };
            postcard::to_allocvec(&(metadata, self.policy.clone()))
        }
        .map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize bundle: {}", e),
        })?;

        bytes.extend_from_slice(&bundle_bytes);

        Ok(bytes)
    }

    /// Deserialize from bytes. Staged decode: the metadata prefix first, then
    /// the policy in the wire shape that metadata's version declares.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        // Check magic bytes
        if bytes.len() < 4 || &bytes[0..4] != Self::MAGIC_BYTES {
            return Err(ReaperError::InvalidPolicy {
                reason: "Invalid bundle format: magic bytes mismatch".to_string(),
            });
        }

        let (metadata, rest): (BundleFormat, &[u8]) = postcard::take_from_bytes(&bytes[4..])
            .map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize bundle metadata: {}", e),
            })?;

        // Wire-format version check — BEFORE decoding the policy, since the
        // version dictates the policy's wire shape.
        if metadata.version > Self::FORMAT_VERSION {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Bundle version {} is newer than supported version {}",
                    metadata.version,
                    Self::FORMAT_VERSION
                ),
            });
        }

        let policy: Policy = if metadata.version >= Self::FORMAT_VERSION {
            postcard::from_bytes(rest).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize bundle: {}", e),
            })?
        } else {
            postcard::from_bytes::<PolicyWireV2>(rest)
                .map_err(|e| ReaperError::InvalidPolicy {
                    reason: format!("Failed to deserialize bundle: {}", e),
                })?
                .into()
        };

        let bundle = Self { metadata, policy };

        // DSL language-version check (round-3 Plan 04): the language version
        // rides in the compiled policy's metadata. A bundle whose policy targets
        // a newer language than this engine implements is rejected — fail closed,
        // same posture as the wire-format check above — never misinterpreted.
        if let Some(raw) = bundle.policy.metadata.get("language_version") {
            let got = raw.parse::<u32>().map_err(|_| ReaperError::InvalidPolicy {
                reason: format!("bundle policy declares a malformed language_version {raw:?}"),
            })?;
            if got > crate::reap::CURRENT_LANGUAGE_VERSION {
                return Err(ReaperError::LanguageVersionUnsupported {
                    got,
                    supported: crate::reap::CURRENT_LANGUAGE_VERSION,
                });
            }
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
    /// Derive a stable policy id from a policy name.
    ///
    /// Uses UUIDv5 over a fixed namespace so the same name always maps to the
    /// same id across processes and redeploys — making bundle deploys idempotent
    /// (same-named policy overwrites in place).
    pub fn stable_policy_id_for(name: &str) -> Uuid {
        stable_policy_id(name)
    }

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

        // Create EnhancedPolicy with the compiled evaluator.
        //
        // The id is a STABLE UUIDv5 derived from the policy name (not a random
        // v4). Re-deploying a bundle for the same policy therefore overwrites the
        // existing entry rather than inserting a new random-id copy — the old bug
        // where redeploys silently accumulated duplicate policies and the version
        // guard never matched.
        let now = chrono::Utc::now();
        let policy = EnhancedPolicy {
            id: stable_policy_id(&self.policy.name),
            version: 1,
            name: self.policy.name.clone(),
            description: format!(
                "Deployed from bundle version {}",
                self.metadata.policy_version.as_deref().unwrap_or("unknown")
            ),
            language: PolicyLanguage::ReaperDsl,
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
    /// Map of package name -> policy indices in the policies array
    /// Enables package-based deployment and evaluation
    #[serde(default)]
    pub package_groups: std::collections::HashMap<String, Vec<usize>>,
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
    /// ```text
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
    /// Package name for grouping related policies
    /// Defaults to "default" if not specified in policy metadata
    #[serde(default = "default_package")]
    pub package: String,
}

fn default_package() -> String {
    "default".to_string()
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

/// The v2 wire shape of a package policy entry (see [`PolicyWireV2`]).
#[derive(Serialize, Deserialize)]
struct PolicyEntryWireV2 {
    policy: PolicyWireV2,
    priority: u32,
    package: String,
}

impl PolicyPackage {
    const MAGIC_BYTES: &'static [u8; 4] = b"REPP"; // Reaper Policy Package
    /// Format version 3: policies carry `functions`/`imports` (language v3,
    /// R4-01 Phase C). Version 2 (postcard) is still written when no policy
    /// in the package uses them, so v2 engines keep loading such packages.
    const FORMAT_VERSION: u32 = 3;
    /// The function-free wire encoding (see [`Self::FORMAT_VERSION`]).
    const LEGACY_FORMAT_VERSION: u32 = 2;

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

        // Build policy entries with package names extracted from metadata
        let policy_entries: Vec<PolicyEntry> = policies
            .into_iter()
            .enumerate()
            .map(|(idx, policy)| {
                let package = policy
                    .metadata
                    .get("package")
                    .cloned()
                    .unwrap_or_else(|| "default".to_string());
                PolicyEntry {
                    policy,
                    priority: (idx * 100) as u32,
                    package,
                }
            })
            .collect();

        // Build package groups index
        let mut package_groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, entry) in policy_entries.iter().enumerate() {
            package_groups
                .entry(entry.package.clone())
                .or_default()
                .push(idx);
        }

        let hints = PrecompilationHints {
            strings_to_intern: strings.into_iter().collect(),
            regex_patterns: regex_patterns.into_iter().collect(),
            total_rules,
            referenced_entities: Vec::new(),
            package_groups,
        };

        Self {
            metadata,
            policies: policy_entries,
            hints,
        }
    }

    /// Serialize to bytes. Packages whose policies are all function-free
    /// encode as wire version 2 — loadable by v2 engines.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReaperError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC_BYTES);

        let legacy = self
            .policies
            .iter()
            .all(|e| e.policy.functions.is_empty() && e.policy.imports.is_empty());
        // Tuple encoding == struct encoding under postcard (fields
        // concatenated), mirroring `PolicyBundle::to_bytes`.
        let bundle_bytes = if legacy {
            let metadata = PackageMetadata {
                format_version: Self::LEGACY_FORMAT_VERSION,
                ..self.metadata.clone()
            };
            let entries: Vec<PolicyEntryWireV2> = self
                .policies
                .iter()
                .map(|e| PolicyEntryWireV2 {
                    policy: e.policy.clone().into(),
                    priority: e.priority,
                    package: e.package.clone(),
                })
                .collect();
            postcard::to_allocvec(&(metadata, entries, self.hints.clone()))
        } else {
            let metadata = PackageMetadata {
                format_version: Self::FORMAT_VERSION,
                ..self.metadata.clone()
            };
            postcard::to_allocvec(&(metadata, self.policies.clone(), self.hints.clone()))
        }
        .map_err(|e| ReaperError::InvalidPolicy {
            reason: format!("Failed to serialize policy package: {}", e),
        })?;

        bytes.extend_from_slice(&bundle_bytes);
        Ok(bytes)
    }

    /// Deserialize from bytes. Staged decode: metadata prefix first, then the
    /// entries in the wire shape the declared version dictates.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReaperError> {
        if bytes.len() < 4 || &bytes[0..4] != Self::MAGIC_BYTES {
            return Err(ReaperError::InvalidPolicy {
                reason: "Invalid policy package: magic bytes mismatch".to_string(),
            });
        }

        let (metadata, rest): (PackageMetadata, &[u8]) = postcard::take_from_bytes(&bytes[4..])
            .map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to deserialize package metadata: {}", e),
            })?;

        if metadata.format_version > Self::FORMAT_VERSION {
            return Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Package version {} is newer than supported version {}",
                    metadata.format_version,
                    Self::FORMAT_VERSION
                ),
            });
        }

        let (policies, hints): (Vec<PolicyEntry>, PrecompilationHints) =
            if metadata.format_version >= Self::FORMAT_VERSION {
                postcard::from_bytes(rest).map_err(|e| ReaperError::InvalidPolicy {
                    reason: format!("Failed to deserialize policy package: {}", e),
                })?
            } else {
                let (entries, hints): (Vec<PolicyEntryWireV2>, PrecompilationHints) =
                    postcard::from_bytes(rest).map_err(|e| ReaperError::InvalidPolicy {
                        reason: format!("Failed to deserialize policy package: {}", e),
                    })?;
                (
                    entries
                        .into_iter()
                        .map(|e| PolicyEntry {
                            policy: e.policy.into(),
                            priority: e.priority,
                            package: e.package,
                        })
                        .collect(),
                    hints,
                )
            };

        let bundle = Self {
            metadata,
            policies,
            hints,
        };

        // Same fail-closed language gate as the single-policy bundle: any
        // packaged policy targeting a newer DSL than this engine rejects the
        // whole package rather than being misread.
        for entry in &bundle.policies {
            if let Some(raw) = entry.policy.metadata.get("language_version") {
                let got = raw.parse::<u32>().map_err(|_| ReaperError::InvalidPolicy {
                    reason: format!(
                        "packaged policy '{}' declares a malformed language_version {raw:?}",
                        entry.policy.name
                    ),
                })?;
                if got > crate::reap::CURRENT_LANGUAGE_VERSION {
                    return Err(ReaperError::LanguageVersionUnsupported {
                        got,
                        supported: crate::reap::CURRENT_LANGUAGE_VERSION,
                    });
                }
            }
        }

        Ok(bundle)
    }

    /// Deploy all policies in this package to an agent's PolicyEngine
    ///
    /// Uses the optimization hints to pre-intern strings and compile regexes
    /// before compiling and deploying each policy.
    ///
    /// **Deprecated**: Use `deploy_to_engine_atomic()` instead for atomic
    /// all-or-nothing deployment with rollback on failure.
    #[deprecated(
        since = "0.2.0",
        note = "Use deploy_to_engine_atomic() for atomic package deployment"
    )]
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

    /// Deploy all policies in this package atomically using two-phase commit
    ///
    /// This method provides atomic, all-or-nothing deployment:
    /// - Phase 1 (Stage): All policies are validated and compiled
    /// - Phase 2 (Commit): All policies are activated atomically
    ///
    /// If any policy fails validation during staging, no policies are deployed.
    /// Concurrent policy evaluations will either see all old policies or all
    /// new policies - never a mix.
    ///
    /// # Arguments
    /// * `engine` - The PolicyEngine to deploy to
    /// * `store` - DataStore containing entity data for evaluators
    ///
    /// # Returns
    /// - Ok(Vec<PolicyVersion>) on success with version info for each policy
    /// - Err on failure (no policies deployed)
    ///
    /// # Example
    /// ```text
    /// let package = PolicyPackage::from_bytes(&bundle_bytes)?;
    /// let versions = package.deploy_to_engine_atomic(&engine, store)?;
    /// println!("Deployed {} policies atomically", versions.len());
    /// ```
    pub fn deploy_to_engine_atomic(
        &self,
        engine: &crate::engine::PolicyEngine,
        store: Arc<DataStore>,
    ) -> Result<Vec<crate::engine::PolicyVersion>, ReaperError> {
        engine.deploy_package_atomic(self, store)
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
                message: None,
                name: "admin".to_string(),
                decision: Decision::Allow,
                condition: Condition::True,
            }],
            functions: vec![],
            imports: vec![],
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
            functions: vec![],
            imports: vec![],
        };

        let bundle = PolicyBundle::new(policy);

        assert_eq!(bundle.metadata.policy_version, Some("1.0.0".to_string()));
        assert_eq!(bundle.metadata.policy_name, "versioned");
    }
}
