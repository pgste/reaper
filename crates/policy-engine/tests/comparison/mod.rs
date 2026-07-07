//! OPA/Reaper Semantic Equivalence Comparison Runner
//!
//! Tests that Reaper policies produce the same decisions as equivalent OPA policies.
//! Can run in "Reaper-only" mode for CI, or with live OPA for full comparison.

use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest, ReaperPolicy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Test suite for semantic equivalence testing
// Test fixture: some fields are deserialized from YAML but not read in Reaper-only CI mode.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ComparisonSuite {
    pub name: String,
    pub description: String,
    pub reaper_policy: String,
    pub opa_policy: String,
    pub data_file: String,
    pub test_cases: Vec<ComparisonTestCase>,
}

/// Individual test case for comparison
#[derive(Debug, Deserialize)]
pub struct ComparisonTestCase {
    pub id: String,
    pub description: String,
    pub input: TestInput,
    pub expected: ExpectedDecision,
}

/// Test input (principal, action, resource)
#[derive(Debug, Deserialize)]
pub struct TestInput {
    pub principal: String,
    pub action: String,
    pub resource: String,
}

/// Expected decision
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpectedDecision {
    Allow,
    Deny,
}

/// Result of a single test case
// Test fixture: OPA-comparison fields are unused in Reaper-only CI mode.
#[allow(dead_code)]
#[derive(Debug)]
pub struct TestCaseResult {
    pub id: String,
    pub description: String,
    pub expected: ExpectedDecision,
    pub reaper_decision: Option<ExpectedDecision>,
    pub opa_decision: Option<ExpectedDecision>,
    pub reaper_time_us: u64,
    pub opa_time_us: Option<u64>,
    pub passed: bool,
    pub error: Option<String>,
}

/// Result of running a full comparison suite
#[derive(Debug)]
pub struct ComparisonResult {
    pub suite_name: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub reaper_only: bool,
    pub test_results: Vec<TestCaseResult>,
    pub reaper_avg_time_us: f64,
    pub opa_avg_time_us: Option<f64>,
}

/// Load a comparison suite from YAML
pub fn load_comparison_suite(path: &Path) -> Result<ComparisonSuite, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let suite: ComparisonSuite = serde_yaml::from_str(&content)?;
    Ok(suite)
}

/// Run a comparison suite (Reaper-only mode)
pub fn run_comparison_reaper_only(
    suite: &ComparisonSuite,
    base_path: &Path,
) -> Result<ComparisonResult, Box<dyn std::error::Error>> {
    // Load data
    let store = DataStore::new();
    let data_path = base_path.join(&suite.data_file);

    if data_path.exists() {
        let loader = DataLoader::new(store.clone());
        let content = std::fs::read_to_string(&data_path)
            .map_err(|e| format!("Failed to read data file {}: {}", data_path.display(), e))?;
        loader
            .load_json(&content)
            .map_err(|e| format!("Failed to load data: {:?}", e))?;
    } else if !suite.data_file.is_empty() {
        // A declared-but-missing data file must FAIL the suite, not silently
        // evaluate every case against an empty store (which turns "entity
        // not found" errors into bogus test outcomes).
        return Err(format!(
            "suite declares data_file {:?} but it does not exist",
            data_path.display()
        )
        .into());
    }

    let store_arc = Arc::new(store);

    // Load Reaper policy
    let policy_path = base_path.join(&suite.reaper_policy);
    let policy = ReaperPolicy::from_file_auto(&policy_path)
        .map_err(|e| format!("Failed to load policy {}: {:?}", policy_path.display(), e))?;

    // Try compiled evaluator first, fall back to AST evaluator
    let evaluator: Box<dyn PolicyEvaluator> = match policy.clone().build(store_arc.clone()) {
        Ok(compiled) => Box::new(compiled),
        Err(_) => Box::new(policy.build_ast_evaluator(store_arc.clone())),
    };

    let mut test_results = Vec::with_capacity(suite.test_cases.len());
    let mut total_reaper_time_us: u64 = 0;
    let mut passed = 0;
    let mut failed = 0;

    for test_case in &suite.test_cases {
        // Build request with principal in context
        let mut context = HashMap::new();
        context.insert("principal".to_string(), test_case.input.principal.clone());

        let request = PolicyRequest {
            resource: test_case.input.resource.clone(),
            action: test_case.input.action.clone(),
            context,
        };

        // Evaluate with Reaper
        let start = Instant::now();
        let reaper_result = evaluator.evaluate(&request);
        let reaper_time_us = start.elapsed().as_micros() as u64;
        total_reaper_time_us += reaper_time_us;

        let (reaper_decision, error) = match reaper_result {
            Ok(action) => {
                let decision_str = format!("{:?}", action);
                let decision = if decision_str.to_lowercase().contains("allow") {
                    ExpectedDecision::Allow
                } else {
                    ExpectedDecision::Deny
                };
                (Some(decision), None)
            }
            Err(e) => (None, Some(format!("{:?}", e))),
        };

        // Determine pass/fail (Reaper-only mode)
        let test_passed = match reaper_decision {
            Some(decision) => decision == test_case.expected,
            None => false,
        };

        if test_passed {
            passed += 1;
        } else {
            failed += 1;
        }

        test_results.push(TestCaseResult {
            id: test_case.id.clone(),
            description: test_case.description.clone(),
            expected: test_case.expected,
            reaper_decision,
            opa_decision: None,
            reaper_time_us,
            opa_time_us: None,
            passed: test_passed,
            error,
        });
    }

    let total = suite.test_cases.len();
    let reaper_avg = if total > 0 {
        total_reaper_time_us as f64 / total as f64
    } else {
        0.0
    };

    Ok(ComparisonResult {
        suite_name: suite.name.clone(),
        total,
        passed,
        failed,
        reaper_only: true,
        test_results,
        reaper_avg_time_us: reaper_avg,
        opa_avg_time_us: None,
    })
}

/// Print comparison results
pub fn print_comparison_results(result: &ComparisonResult) {
    println!("\n=== {} ===", result.suite_name);
    println!(
        "Mode: {}",
        if result.reaper_only {
            "Reaper-only"
        } else {
            "Reaper vs OPA"
        }
    );
    println!(
        "Results: {}/{} passed ({} failed)",
        result.passed, result.total, result.failed
    );
    println!("Reaper avg latency: {:.2}µs", result.reaper_avg_time_us);
    if let Some(opa_avg) = result.opa_avg_time_us {
        println!("OPA avg latency: {:.2}µs", opa_avg);
        println!(
            "Speedup: {:.1}x",
            opa_avg / result.reaper_avg_time_us.max(0.001)
        );
    }

    if result.failed > 0 {
        println!("\nFailed tests:");
        for test in &result.test_results {
            if !test.passed {
                println!(
                    "  - {} ({}): expected {:?}, got Reaper={:?}",
                    test.id, test.description, test.expected, test.reaper_decision
                );
                if let Some(ref error) = test.error {
                    println!("    Error: {}", error);
                }
            }
        }
    }
}
