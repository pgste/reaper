// ! Reaper Policy Language Parser and Compiler
//!
//! Parses .reap files into AST and compiles to ReaperDSLEvaluator for sub-microsecond evaluation.

mod parser;
mod ast;
mod compiler;
mod bundle;
mod yaml_parser;

pub use parser::ReapParser;
pub use ast::{Policy, Rule as ReapRule, Condition as ReapCondition, Decision, Value as ReapValue};
pub use compiler::compile_policy;
pub use bundle::{PolicyBundle, BundleFormat};
pub use yaml_parser::YamlPolicy;

use reaper_core::ReaperError;
use crate::evaluators::reaper_dsl::ReaperDSLEvaluator;
use crate::data::DataStore;
use std::sync::Arc;
use std::path::Path;
use std::fs;

/// Main entry point for loading .reap policies
pub struct ReaperPolicy {
    ast: Policy,
}

impl ReaperPolicy {
    /// Parse a .reap file from a string
    pub fn from_str(input: &str) -> Result<Self, ReaperError> {
        let ast = ReapParser::parse(input)?;
        Ok(Self { ast })
    }

    /// Load a .reap file from disk
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Failed to read policy file: {}", e),
            }
        })?;
        Self::from_str(&content)
    }

    /// Parse a policy from YAML string
    pub fn from_yaml_str(input: &str) -> Result<Self, ReaperError> {
        let yaml_policy = YamlPolicy::from_yaml(input)?;
        let ast = yaml_policy.to_ast()?;
        Ok(Self { ast })
    }

    /// Load a YAML policy file from disk
    pub fn from_yaml_file<P: AsRef<Path>>(path: P) -> Result<Self, ReaperError> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Failed to read YAML policy file: {}", e),
            }
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
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            ReaperError::InvalidPolicy {
                reason: format!("Failed to read JSON policy file: {}", e),
            }
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

    /// Build a ReaperDSLEvaluator from this policy
    pub fn build(self, store: Arc<DataStore>) -> Result<ReaperDSLEvaluator, ReaperError> {
        compiler::compile_policy(self.ast, store)
    }

    /// Get the policy name
    pub fn name(&self) -> &str {
        &self.ast.name
    }

    /// Get the policy version (if any)
    pub fn version(&self) -> Option<&str> {
        self.ast.metadata.get("version").map(|s| s.as_str())
    }

    /// Compile to a binary bundle for fast loading
    pub fn compile_to_bundle(&self) -> Result<Vec<u8>, ReaperError> {
        bundle::compile_to_bundle(&self.ast)
    }

    /// Load from a binary bundle
    pub fn from_bundle(bytes: &[u8], store: Arc<DataStore>) -> Result<ReaperDSLEvaluator, ReaperError> {
        let policy = bundle::load_from_bundle(bytes)?;
        compiler::compile_policy(policy, store)
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
