//! Integration Test Framework
//!
//! YAML-based test suite runner for policy engine integration tests.
//! Loads real policies and data, runs test cases, and validates results.

use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Test suite configuration loaded from YAML
#[derive(Debug, Deserialize)]
pub struct TestSuite {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub policies: Vec<PolicyPath>,
    pub data: Vec<DataPath>,
    pub test_cases: Vec<TestCase>,
    #[serde(default)]
    pub performance: Option<PerformanceThresholds>,
}

#[derive(Debug, Deserialize)]
pub struct PolicyPath {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct DataPath {
    pub path: String,
}

/// Single test case
#[derive(Debug, Clone, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    #[serde(default)]
    pub context: HashMap<String, String>,
    pub expected: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Performance thresholds for the suite
#[derive(Debug, Deserialize)]
pub struct PerformanceThresholds {
    #[serde(default)]
    pub p50_threshold_us: Option<u64>,
    #[serde(default)]
    pub p99_threshold_us: Option<u64>,
    #[serde(default)]
    pub max_threshold_us: Option<u64>,
}

/// Result of running a single test case
#[derive(Debug, Serialize)]
pub struct TestCaseResult {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub evaluation_time_us: u64,
    pub error: Option<String>,
}

/// Result of running a full test suite
#[derive(Debug, Serialize)]
pub struct SuiteResult {
    pub suite_name: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<TestCaseResult>,
    pub avg_evaluation_time_us: u64,
    pub p99_evaluation_time_us: u64,
}

/// Load a test suite from YAML file
pub fn load_suite(path: &Path) -> Result<TestSuite, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let suite: TestSuite = serde_yaml::from_str(&content)?;
    Ok(suite)
}

/// Run a test suite against the policy engine
pub fn run_suite(
    suite: &TestSuite,
    base_path: &Path,
) -> Result<SuiteResult, Box<dyn std::error::Error>> {
    // Load data into store
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    for data_path in &suite.data {
        let full_path = base_path.join(&data_path.path);
        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read data file {}: {}", full_path.display(), e))?;
        loader
            .load_json(&content)
            .map_err(|e| format!("Failed to load data from {}: {:?}", full_path.display(), e))?;
    }

    let store_arc = Arc::new(store);

    // Load policies and build evaluators
    let mut evaluators: Vec<Box<dyn PolicyEvaluator>> = Vec::new();

    for policy_path in &suite.policies {
        let full_path = base_path.join(&policy_path.path);
        let policy = ReaperPolicy::from_file_auto(&full_path)
            .map_err(|e| format!("Failed to load policy {}: {:?}", full_path.display(), e))?;

        // Try compiled evaluator first, fall back to AST evaluator
        let evaluator: Box<dyn PolicyEvaluator> = match policy.clone().build(store_arc.clone()) {
            Ok(compiled) => Box::new(compiled),
            Err(_) => Box::new(policy.build_ast_evaluator(store_arc.clone())),
        };

        evaluators.push(evaluator);
    }

    // Run test cases
    let mut results = Vec::with_capacity(suite.test_cases.len());
    let mut evaluation_times = Vec::with_capacity(suite.test_cases.len());

    for case in &suite.test_cases {
        let result = run_test_case(&evaluators, case);
        evaluation_times.push(result.evaluation_time_us);
        results.push(result);
    }

    // Calculate statistics
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;

    let avg_time = if !evaluation_times.is_empty() {
        evaluation_times.iter().sum::<u64>() / evaluation_times.len() as u64
    } else {
        0
    };

    evaluation_times.sort();
    let p99_time = if !evaluation_times.is_empty() {
        let p99_idx = (evaluation_times.len() as f64 * 0.99) as usize;
        evaluation_times[p99_idx.min(evaluation_times.len() - 1)]
    } else {
        0
    };

    Ok(SuiteResult {
        suite_name: suite.name.clone(),
        total: results.len(),
        passed,
        failed,
        results,
        avg_evaluation_time_us: avg_time,
        p99_evaluation_time_us: p99_time,
    })
}

/// Run a single test case against all evaluators
fn run_test_case(evaluators: &[Box<dyn PolicyEvaluator>], case: &TestCase) -> TestCaseResult {
    let start = Instant::now();

    // Build the request with principal in context
    let mut context = case.context.clone();
    context.insert("principal".to_string(), case.principal.clone());

    let request = PolicyRequest {
        resource: case.resource.clone(),
        action: case.action.clone(),
        context,
    };

    // Evaluate against all policies - default deny, any allow wins
    let mut final_decision = "deny";
    let mut error_msg: Option<String> = None;

    for evaluator in evaluators {
        match evaluator.evaluate(&request) {
            Ok(action) => {
                let decision_str = format!("{:?}", action);
                if decision_str.to_lowercase().contains("allow") {
                    final_decision = "allow";
                    break;
                }
            }
            Err(e) => {
                error_msg = Some(format!("{:?}", e));
            }
        }
    }

    let elapsed = start.elapsed();
    let actual = final_decision.to_string();
    let passed = actual == case.expected;

    TestCaseResult {
        name: case.name.clone(),
        passed,
        expected: case.expected.clone(),
        actual,
        evaluation_time_us: elapsed.as_micros() as u64,
        error: error_msg,
    }
}

/// Print suite results in a readable format
pub fn print_results(result: &SuiteResult) {
    println!("\n=== {} ===", result.suite_name);
    println!(
        "Total: {} | Passed: {} | Failed: {}",
        result.total, result.passed, result.failed
    );
    println!(
        "Avg time: {}µs | P99: {}µs",
        result.avg_evaluation_time_us, result.p99_evaluation_time_us
    );
    println!();

    for case_result in &result.results {
        let status = if case_result.passed { "✓" } else { "✗" };
        println!(
            "  {} {} ({}µs) - expected: {}, got: {}",
            status,
            case_result.name,
            case_result.evaluation_time_us,
            case_result.expected,
            case_result.actual
        );
        if let Some(ref err) = case_result.error {
            println!("      Error: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn get_fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("test-fixtures")
    }

    #[test]
    fn test_load_suite() {
        let fixtures = get_fixtures_path();
        let suite_path = fixtures.join("suites/rbac_core.yaml");

        if suite_path.exists() {
            let suite = load_suite(&suite_path).expect("Failed to load suite");
            assert_eq!(suite.name, "RBAC Core");
            assert!(!suite.test_cases.is_empty());
        }
    }
}
