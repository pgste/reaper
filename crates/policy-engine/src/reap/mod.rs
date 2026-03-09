// ! Reaper Policy Language Parser and Compiler
//!
//! Parses .reap files into AST and compiles to ReaperDSLEvaluator for sub-microsecond evaluation.

mod ast;
mod ast_evaluator;
mod bundle;
mod compiler;
mod parser;
mod yaml_parser;

pub use ast::{
    AssignmentValue, ComparisonLeft, ComparisonRight, Condition as ReapCondition, Decision, Entity,
    EntityAttr, Expr, Index, Operator, Policy, Rule as ReapRule, Value as ReapValue, VarAttr,
};
pub use ast_evaluator::ReapAstEvaluator;
pub use bundle::{
    BundleFormat, PackageMetadata, PolicyBundle, PolicyEntry, PolicyPackage, PrecompilationHints,
};
pub use compiler::compile_policy;
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

impl FromStr for ReaperPolicy {
    type Err = ReaperError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ast = ReapParser::parse(s)?;
        Ok(Self { ast })
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
}
