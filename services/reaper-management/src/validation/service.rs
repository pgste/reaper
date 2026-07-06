//! Policy validation service
//!
//! Validates policy syntax for different policy languages.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repositories::PolicyRepository;
use crate::db::Database;
use crate::domain::policy::PolicyLanguage;

/// Validation error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Error type (syntax, semantic, warning)
    pub error_type: String,
    /// Error message
    pub message: String,
    /// Line number if applicable
    pub line: Option<u32>,
    /// Column number if applicable
    pub column: Option<u32>,
    /// Code snippet around the error
    pub snippet: Option<String>,
}

/// Test case for policy evaluation (for future use)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Test case name
    pub name: String,
    /// Principal (subject) making the request
    pub principal: String,
    /// Action being performed
    pub action: String,
    /// Resource being accessed
    pub resource: String,
    /// Additional context as JSON
    #[serde(default)]
    pub context: serde_json::Value,
    /// Expected decision (allow/deny)
    pub expected: String,
}

/// Result of running a test case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Test case name
    pub name: String,
    /// Whether the test passed
    pub passed: bool,
    /// Expected decision
    pub expected: String,
    /// Actual decision
    pub actual: String,
    /// Evaluation time in microseconds
    pub evaluation_time_us: u64,
    /// Error message if failed
    pub error: Option<String>,
}

/// Complete validation result for a policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyValidationResult {
    /// Policy ID
    pub policy_id: Uuid,
    /// Policy name
    pub policy_name: String,
    /// Policy language
    pub language: String,
    /// Whether the policy is valid (no syntax errors)
    pub is_valid: bool,
    /// Syntax errors (blocking)
    pub syntax_errors: Vec<ValidationError>,
    /// Semantic warnings (non-blocking)
    pub warnings: Vec<ValidationError>,
    /// Test results if tests were run
    pub test_results: Option<Vec<TestResult>>,
    /// Summary statistics
    pub summary: ValidationSummary,
}

/// Summary of validation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub syntax_error_count: u32,
    pub warning_count: u32,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub tests_total: u32,
}

/// Validation service
pub struct ValidationService {
    db: Arc<Database>,
}

impl ValidationService {
    /// Create a new validation service
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Validate a policy by ID
    pub async fn validate_policy(
        &self,
        policy_id: Uuid,
        _test_cases: Option<Vec<TestCase>>,
    ) -> Result<PolicyValidationResult, crate::db::DatabaseError> {
        let policy_repo = PolicyRepository::new(&self.db);

        // Get the policy
        let policy = policy_repo
            .get_by_id(policy_id)
            .await?
            .ok_or_else(|| crate::db::DatabaseError::NotFound(policy_id.to_string()))?;

        // Get the latest version content
        let version = policy_repo
            .get_latest_version(policy_id)
            .await?
            .ok_or_else(|| {
                crate::db::DatabaseError::NotFound(format!(
                    "No version found for policy {}",
                    policy_id
                ))
            })?;

        self.validate_content(policy_id, &policy.name, policy.language, &version.content)
    }

    /// Validate policy content directly (for preview before saving)
    pub fn validate_content(
        &self,
        policy_id: Uuid,
        policy_name: &str,
        language: PolicyLanguage,
        content: &str,
    ) -> Result<PolicyValidationResult, crate::db::DatabaseError> {
        let mut syntax_errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate syntax based on language
        match language {
            PolicyLanguage::Reaper => {
                self.validate_reaper_syntax(content, &mut syntax_errors, &mut warnings);
            }
            PolicyLanguage::Cedar => {
                self.validate_cedar_syntax(content, &mut syntax_errors, &mut warnings);
            }
            PolicyLanguage::Simple => {
                self.validate_simple_syntax(content, &mut syntax_errors, &mut warnings);
            }
        }

        let is_valid = syntax_errors.is_empty();

        Ok(PolicyValidationResult {
            policy_id,
            policy_name: policy_name.to_string(),
            language: language.to_string(),
            is_valid,
            syntax_errors: syntax_errors.clone(),
            warnings: warnings.clone(),
            test_results: None, // Test evaluation requires agent runtime
            summary: ValidationSummary {
                syntax_error_count: syntax_errors.len() as u32,
                warning_count: warnings.len() as u32,
                tests_passed: 0,
                tests_failed: 0,
                tests_total: 0,
            },
        })
    }

    /// Validate Reaper DSL syntax
    fn validate_reaper_syntax(
        &self,
        content: &str,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationError>,
    ) {
        use policy_engine::reap::ReapParser;

        match ReapParser::parse(content) {
            Ok(policy) => {
                // Check for semantic issues
                if policy.rules.is_empty() {
                    warnings.push(ValidationError {
                        error_type: "warning".to_string(),
                        message: "Policy has no rules defined".to_string(),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                }

                // Check for duplicate rule names
                let mut rule_names = std::collections::HashSet::new();
                for rule in &policy.rules {
                    if !rule_names.insert(&rule.name) {
                        warnings.push(ValidationError {
                            error_type: "warning".to_string(),
                            message: format!("Duplicate rule name: {}", rule.name),
                            line: None,
                            column: None,
                            snippet: None,
                        });
                    }
                }
            }
            Err(e) => {
                let (line, column, snippet) = self.extract_error_location(content, &e.to_string());
                errors.push(ValidationError {
                    error_type: "syntax".to_string(),
                    message: e.to_string(),
                    line,
                    column,
                    snippet,
                });
            }
        }
    }

    /// Validate Cedar syntax using the policy-engine's Cedar evaluator
    fn validate_cedar_syntax(
        &self,
        content: &str,
        errors: &mut Vec<ValidationError>,
        _warnings: &mut Vec<ValidationError>,
    ) {
        // Try to create a Cedar evaluator to validate syntax
        use policy_engine::CedarPolicyEvaluator;

        match CedarPolicyEvaluator::new(content.to_string()) {
            Ok(_) => {
                // Cedar parsed successfully
            }
            Err(e) => {
                errors.push(ValidationError {
                    error_type: "syntax".to_string(),
                    message: e.to_string(),
                    line: None,
                    column: None,
                    snippet: None,
                });
            }
        }
    }

    /// Validate Simple policy syntax (JSON-based)
    fn validate_simple_syntax(
        &self,
        content: &str,
        errors: &mut Vec<ValidationError>,
        warnings: &mut Vec<ValidationError>,
    ) {
        // Simple policies are JSON-based
        match serde_json::from_str::<serde_json::Value>(content) {
            Ok(value) => {
                // Check for required fields
                if !value.is_object() {
                    errors.push(ValidationError {
                        error_type: "syntax".to_string(),
                        message: "Simple policy must be a JSON object".to_string(),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                    return;
                }

                let obj = value.as_object().unwrap();

                // Check for rules array
                if !obj.contains_key("rules") {
                    warnings.push(ValidationError {
                        error_type: "warning".to_string(),
                        message: "Simple policy missing 'rules' array".to_string(),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                } else if let Some(rules) = obj.get("rules") {
                    if !rules.is_array() {
                        errors.push(ValidationError {
                            error_type: "syntax".to_string(),
                            message: "'rules' must be an array".to_string(),
                            line: None,
                            column: None,
                            snippet: None,
                        });
                    } else {
                        // Validate each rule
                        for (idx, rule) in rules.as_array().unwrap().iter().enumerate() {
                            self.validate_simple_rule(rule, idx, errors, warnings);
                        }
                    }
                }

                // Check for default decision
                if !obj.contains_key("default") {
                    warnings.push(ValidationError {
                        error_type: "warning".to_string(),
                        message: "Simple policy missing 'default' decision".to_string(),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                } else if let Some(default) = obj.get("default") {
                    if let Some(d) = default.as_str() {
                        if d != "allow" && d != "deny" {
                            errors.push(ValidationError {
                                error_type: "syntax".to_string(),
                                message: format!(
                                    "'default' must be 'allow' or 'deny', got '{}'",
                                    d
                                ),
                                line: None,
                                column: None,
                                snippet: None,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                let (line, column) = (Some(e.line() as u32), Some(e.column() as u32));
                errors.push(ValidationError {
                    error_type: "syntax".to_string(),
                    message: format!("JSON parse error: {}", e),
                    line,
                    column,
                    snippet: self.get_snippet(content, line),
                });
            }
        }
    }

    /// Validate a simple policy rule
    fn validate_simple_rule(
        &self,
        rule: &serde_json::Value,
        idx: usize,
        errors: &mut Vec<ValidationError>,
        _warnings: &mut Vec<ValidationError>,
    ) {
        if !rule.is_object() {
            errors.push(ValidationError {
                error_type: "syntax".to_string(),
                message: format!("Rule {} must be an object", idx),
                line: None,
                column: None,
                snippet: None,
            });
            return;
        }

        // Check for decision field
        if let Some(decision) = rule.get("decision") {
            if let Some(d) = decision.as_str() {
                if d != "allow" && d != "deny" {
                    errors.push(ValidationError {
                        error_type: "syntax".to_string(),
                        message: format!(
                            "Rule {} 'decision' must be 'allow' or 'deny', got '{}'",
                            idx, d
                        ),
                        line: None,
                        column: None,
                        snippet: None,
                    });
                }
            }
        }
    }

    /// Extract error location from error message
    fn extract_error_location(
        &self,
        content: &str,
        error_msg: &str,
    ) -> (Option<u32>, Option<u32>, Option<String>) {
        // Try to extract line:column from error message
        // Common patterns: "at line 5, column 10" or "line 5:10"

        // Simple pattern matching without regex
        let line = self.extract_line_from_error(error_msg);
        let column = self.extract_column_from_error(error_msg);
        let snippet = self.get_snippet(content, line);

        (line, column, snippet)
    }

    /// Extract line number from error message
    fn extract_line_from_error(&self, error_msg: &str) -> Option<u32> {
        // Look for "line X" pattern
        let lower = error_msg.to_lowercase();
        if let Some(pos) = lower.find("line ") {
            let after = &error_msg[pos + 5..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            num_str.parse().ok()
        } else {
            None
        }
    }

    /// Extract column number from error message
    fn extract_column_from_error(&self, error_msg: &str) -> Option<u32> {
        // Look for "column X" pattern
        let lower = error_msg.to_lowercase();
        if let Some(pos) = lower.find("column ") {
            let after = &error_msg[pos + 7..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            num_str.parse().ok()
        } else {
            None
        }
    }

    /// Get a snippet of code around a line
    fn get_snippet(&self, content: &str, line: Option<u32>) -> Option<String> {
        let line = line? as usize;
        let lines: Vec<&str> = content.lines().collect();

        if line == 0 || line > lines.len() {
            return None;
        }

        let start = line.saturating_sub(2);
        let end = (line + 1).min(lines.len());

        Some(
            lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:4} | {}", start + i + 1, l))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// Validate multiple policies (e.g., all policies in a bundle)
#[allow(dead_code)] // exercised by the bundle CLI path; kept as public surface
pub async fn validate_bundle_policies(
    db: &Database,
    policy_ids: &[Uuid],
) -> Result<Vec<PolicyValidationResult>, crate::db::DatabaseError> {
    let service = ValidationService::new(Arc::new(db.clone()));
    let mut results = Vec::new();

    for policy_id in policy_ids {
        let result = service.validate_policy(*policy_id, None).await?;
        results.push(result);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_service() -> ValidationService {
        ValidationService::new(Arc::new(Database::new_mock()))
    }

    #[test]
    fn test_validate_simple_syntax_valid() {
        let content = r#"
{
    "default": "deny",
    "rules": [
        {
            "principal": "admin",
            "action": "*",
            "resource": "*",
            "decision": "allow"
        }
    ]
}
"#;
        let service = create_test_service();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        service.validate_simple_syntax(content, &mut errors, &mut warnings);

        assert!(errors.is_empty(), "Expected no errors: {:?}", errors);
    }

    #[test]
    fn test_validate_simple_syntax_invalid_json() {
        let content = r#"{ invalid json }"#;
        let service = create_test_service();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        service.validate_simple_syntax(content, &mut errors, &mut warnings);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].error_type, "syntax");
    }

    #[test]
    fn test_validate_simple_syntax_invalid_decision() {
        let content = r#"
{
    "default": "maybe",
    "rules": []
}
"#;
        let service = create_test_service();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        service.validate_simple_syntax(content, &mut errors, &mut warnings);

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("must be 'allow' or 'deny'"));
    }

    #[test]
    fn test_extract_line_from_error() {
        let service = create_test_service();

        assert_eq!(
            service.extract_line_from_error("Error at line 5, column 10"),
            Some(5)
        );
        assert_eq!(
            service.extract_line_from_error("Parse error on line 123"),
            Some(123)
        );
        assert_eq!(service.extract_line_from_error("No line info"), None);
    }

    #[test]
    fn test_get_snippet() {
        let service = create_test_service();
        let content = "line1\nline2\nline3\nline4\nline5";

        let snippet = service.get_snippet(content, Some(3));
        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert!(snippet.contains("line2"));
        assert!(snippet.contains("line3"));
    }
}
