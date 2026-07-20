// ! Reaper Policy Language Parser and Compiler
//!
//! Parses .reap files into AST and compiles to ReaperDSLEvaluator for sub-microsecond evaluation.

mod ast;
mod ast_evaluator;
mod bundle;
mod compiler;
mod limits;
mod mixed_evaluator;
mod parser;
mod yaml_parser;

pub use ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Condition as ReapCondition, Decision, Entity,
    EntityAttr, Expr, Index, Operator, Policy, Rule as ReapRule, Value as ReapValue, VarAttr,
};
pub use ast_evaluator::{CheckResult, ReapAstEvaluator, Violation};
pub use bundle::{
    stable_policy_id, BundleFormat, PackageMetadata, PolicyBundle, PolicyEntry, PolicyPackage,
    PrecompilationHints,
};
pub use compiler::compile_policy;
pub use limits::{
    configured_max_nesting_depth, enforce_policy_depth, enforce_source_nesting,
    DEFAULT_MAX_NESTING_DEPTH,
};
pub use mixed_evaluator::{MixedReapEvaluator, PerRuleBuild};
pub use parser::ReapParser;
pub use yaml_parser::YamlPolicy;

use crate::data::DataStore;
use crate::evaluators::reaper_dsl::ReaperDSLEvaluator;
use reaper_core::ReaperError;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

/// Main entry point for loading .reap policies
#[derive(Clone)]
pub struct ReaperPolicy {
    ast: Policy,
}

impl ReaperPolicy {
    /// Load a .reap file from disk
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to read policy file: {}", e),
            })?;
        content.parse()
    }

    /// Parse a policy from YAML string
    pub fn from_yaml_str(input: &str) -> Result<Self, ReaperError> {
        let yaml_policy = YamlPolicy::from_yaml(input)?;
        let ast = yaml_policy.to_ast()?;
        Ok(Self { ast })
    }

    /// Load a YAML policy file from disk
    pub fn from_yaml_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to read YAML policy file: {}", e),
            })?;
        Self::from_yaml_str(&content)
    }

    /// Parse a policy from JSON string
    pub fn from_json_str(input: &str) -> Result<Self, ReaperError> {
        let yaml_policy = YamlPolicy::from_json(input)?;
        let ast = yaml_policy.to_ast()?;
        Ok(Self { ast })
    }

    /// Load a JSON policy file from disk
    pub fn from_json_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| ReaperError::InvalidPolicy {
                reason: format!("Failed to read JSON policy file: {}", e),
            })?;
        Self::from_json_str(&content)
    }

    /// Auto-detect format and load from file based on extension
    /// Supports .reap, .yaml, .yml, .json
    pub fn from_file_auto<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let path_ref = path.as_ref();
        let extension = path_ref
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| ReaperError::InvalidPolicy {
                reason: "File has no extension. Use .reap, .yaml, .yml, or .json".to_string(),
            })?;

        match extension.to_lowercase().as_str() {
            "reap" => Self::from_file(path_ref),
            "yaml" | "yml" => Self::from_yaml_file(path_ref),
            "json" => Self::from_json_file(path_ref),
            _ => Err(ReaperError::InvalidPolicy {
                reason: format!(
                    "Unsupported file extension '.{}'. Use .reap, .yaml, .yml, or .json",
                    extension
                ),
            }),
        }
    }

    /// Build a ReaperDSLEvaluator from this policy (compiled, optimized)
    pub fn build(self, store: Arc<DataStore>) -> Result<ReaperDSLEvaluator, ReaperError> {
        compiler::compile_policy(self.ast, store)
    }

    /// Build a ReapAstEvaluator from this policy (direct AST evaluation, supports all features)
    ///
    /// Use this for policies that use advanced features like comprehensions,
    /// variable assignments, or other features not yet supported by the compiler.
    pub fn build_ast_evaluator(self, store: Arc<DataStore>) -> ReapAstEvaluator {
        ReapAstEvaluator::new(store, self.ast)
    }

    /// Build the PREFERRED evaluator: the compiled `ReaperDSLEvaluator` when
    /// this policy compiles whole; otherwise a per-RULE mixed evaluator that
    /// keeps every compilable rule on the compiled path and interprets only
    /// the rules that need it (R4-01 Phase A.2); a whole-policy
    /// `ReapAstEvaluator` only when no rule compiles at all.
    ///
    /// The compiled path is faster; the AST path supports every feature. All
    /// three shapes are required to produce identical decisions for any
    /// policy (pinned by the compiled-vs-AST and mixed-mode differentials),
    /// so falling back never changes an authorization outcome — it only
    /// trades speed for coverage on constructs the compiler doesn't yet
    /// handle. This is the entry point production code should use unless it
    /// specifically needs one implementation.
    pub fn build_preferred(
        self,
        store: Arc<DataStore>,
    ) -> Result<Box<dyn crate::evaluators::PolicyEvaluator>, ReaperError> {
        match compiler::compile_policy(self.clone().ast, store.clone()) {
            Ok(compiled) => Ok(Box::new(compiled)),
            Err(compile_err) => {
                match mixed_evaluator::MixedReapEvaluator::build(self.ast, store)? {
                    PerRuleBuild::Mixed(mixed) => {
                        let (compiled_rules, ast_rules) = mixed.rule_modes();
                        tracing::info!(
                            %compile_err,
                            compiled_rules,
                            ast_rules,
                            ast_rule_names = %mixed.ast_rule_names().join(","),
                            "policy did not compile whole; serving mixed-mode \
                             (per-rule compiled/AST fallback)"
                        );
                        Ok(Box::new(mixed))
                    }
                    PerRuleBuild::AllAst(ast) => {
                        tracing::debug!(
                            %compile_err,
                            "no rule compiled; falling back to whole-policy AST evaluator"
                        );
                        Ok(Box::new(ast))
                    }
                }
            }
        }
    }

    /// Get the policy name
    pub fn name(&self) -> &str {
        &self.ast.name
    }

    /// Get the policy version (if any)
    pub fn version(&self) -> Option<&str> {
        self.ast.metadata.get("version").map(|s| s.as_str())
    }

    /// Get the policy package name (defaults to "default" if not specified)
    pub fn package(&self) -> &str {
        self.ast
            .metadata
            .get("package")
            .map(|s| s.as_str())
            .unwrap_or("default")
    }

    /// Get the policy description (if any)
    pub fn description(&self) -> Option<&str> {
        self.ast.metadata.get("description").map(|s| s.as_str())
    }

    /// Get all policy metadata as a HashMap
    pub fn metadata(&self) -> &std::collections::HashMap<String, String> {
        &self.ast.metadata
    }

    /// Compile to a binary bundle for fast loading
    pub fn compile_to_bundle(&self) -> Result<Vec<u8>, ReaperError> {
        bundle::compile_to_bundle(&self.ast)
    }

    /// Load from a binary bundle
    pub fn from_bundle(
        bytes: &[u8],
        store: Arc<DataStore>,
    ) -> Result<ReaperDSLEvaluator, ReaperError> {
        let policy = bundle::load_from_bundle(bytes)?;
        compiler::compile_policy(policy, store)
    }
}

/// The `.reap` DSL language version this engine implements.
///
/// A policy may declare its target version with a `language_version: "N"`
/// metadata field. A policy declaring a NEWER version than this is rejected
/// (fail-closed), the same posture the bundle wire format takes one layer down
/// (`reap/bundle.rs`) — an old engine must never silently misinterpret a policy
/// written against a newer language. A policy that declares no version is
/// treated as this current (implicit) version. Bump this only alongside a
/// documented, frozen-corpus-gated language change (see
/// `docs/reference/DSL_COMPATIBILITY.md`).
pub const CURRENT_LANGUAGE_VERSION: u32 = 2;

impl FromStr for ReaperPolicy {
    type Err = ReaperError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ast = ReapParser::parse(s)?;
        let policy = Self { ast };
        policy.check_language_version()?;
        Ok(policy)
    }
}

impl ReaperPolicy {
    /// The DSL language version this policy targets — its declared
    /// `language_version`, or [`CURRENT_LANGUAGE_VERSION`] if it declares none.
    pub fn language_version(&self) -> u32 {
        self.ast
            .metadata
            .get("language_version")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(CURRENT_LANGUAGE_VERSION)
    }

    /// Fail closed if the policy declares a language version this engine does
    /// not implement (newer than [`CURRENT_LANGUAGE_VERSION`]), or a malformed
    /// one. Never down-levels or best-effort parses a newer policy.
    pub(crate) fn check_language_version(&self) -> Result<(), ReaperError> {
        if let Some(raw) = self.ast.metadata.get("language_version") {
            let got = raw.parse::<u32>().map_err(|_| ReaperError::InvalidPolicy {
                reason: format!(
                    "malformed language_version {raw:?}: expected an integer like \"2\""
                ),
            })?;
            if got > CURRENT_LANGUAGE_VERSION {
                return Err(ReaperError::LanguageVersionUnsupported {
                    got,
                    supported: CURRENT_LANGUAGE_VERSION,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_policy() {
        let policy_text = r#"
            policy test_policy {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let policy = ReaperPolicy::from_str(policy_text).unwrap();
        assert_eq!(policy.name(), "test_policy");
    }

    #[test]
    fn test_parse_with_metadata() {
        let policy_text = r#"
            policy test {
                version: "1.0.0",
                description: "Test policy",
                default: allow,
                rule test { deny if user.suspended == true }
            }
        "#;

        let policy = ReaperPolicy::from_str(policy_text).unwrap();
        assert_eq!(policy.version(), Some("1.0.0"));
    }

    // --- DSL language versioning (round-3 Plan 04, Step 2) ---

    #[test]
    fn headerless_policy_is_current_language_version() {
        let policy = ReaperPolicy::from_str(
            r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#,
        )
        .expect("headerless policy must still parse (back-compat)");
        assert_eq!(policy.language_version(), CURRENT_LANGUAGE_VERSION);
    }

    #[test]
    fn declared_current_language_version_parses() {
        let src = format!(
            r#"policy p {{ language_version: "{CURRENT_LANGUAGE_VERSION}", default: deny,
               rule r {{ allow if user.role == "admin" }} }}"#
        );
        let policy = ReaperPolicy::from_str(&src).expect("current version must parse");
        assert_eq!(policy.language_version(), CURRENT_LANGUAGE_VERSION);
    }

    #[test]
    fn newer_language_version_fails_closed() {
        let src = r#"policy p { language_version: "999", default: deny,
                     rule r { allow if user.role == "admin" } }"#;
        match ReaperPolicy::from_str(src) {
            Err(ReaperError::LanguageVersionUnsupported { got, supported }) => {
                assert_eq!(got, 999);
                assert_eq!(supported, CURRENT_LANGUAGE_VERSION);
            }
            Ok(_) => {
                panic!("a policy targeting a newer language must be rejected, not down-levelled")
            }
            Err(other) => panic!("expected LanguageVersionUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn malformed_language_version_rejected() {
        let src = r#"policy p { language_version: "two", default: deny,
                     rule r { allow if user.role == "admin" } }"#;
        assert!(matches!(
            ReaperPolicy::from_str(src),
            Err(ReaperError::InvalidPolicy { .. })
        ));
    }
}
