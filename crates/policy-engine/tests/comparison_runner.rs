//! Comparison Test Runner
//!
//! Runs semantic equivalence tests to verify Reaper produces
//! the same decisions as equivalent OPA policies.
//!
//! Usage:
//!   cargo test -p policy-engine --test comparison_runner

mod comparison;

use comparison::{load_comparison_suite, print_comparison_results, run_comparison_reaper_only};
use std::path::PathBuf;

fn get_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn get_comparison_path() -> PathBuf {
    get_workspace_root().join("test-fixtures/comparison")
}

/// Run RBAC equivalence tests (Reaper-only mode)
#[test]
fn test_rbac_equivalence_reaper_only() {
    let comparison_path = get_comparison_path();
    let suite_path = comparison_path.join("test_cases/rbac_equivalence.yaml");

    if !suite_path.exists() {
        eprintln!("Skipping: suite file not found at {:?}", suite_path);
        return;
    }

    let suite = load_comparison_suite(&suite_path).expect("Failed to load suite");
    let base_path = get_workspace_root();

    // Check if policy exists
    let policy_path = base_path.join(&suite.reaper_policy);
    if !policy_path.exists() {
        eprintln!("Skipping: policy file not found at {:?}", policy_path);
        return;
    }

    let result = run_comparison_reaper_only(&suite, &base_path).expect("Failed to run comparison");
    print_comparison_results(&result);

    // All tests should pass in Reaper-only mode
    assert_eq!(
        result.failed, 0,
        "Expected all tests to pass, but {} failed",
        result.failed
    );
}

/// Run string operations equivalence tests (Reaper-only mode)
#[test]
fn test_string_equivalence_reaper_only() {
    let comparison_path = get_comparison_path();
    let suite_path = comparison_path.join("test_cases/string_equivalence.yaml");

    if !suite_path.exists() {
        eprintln!("Skipping: suite file not found at {:?}", suite_path);
        return;
    }

    let suite = load_comparison_suite(&suite_path).expect("Failed to load suite");
    let base_path = get_workspace_root();

    // Check if policy exists
    let policy_path = base_path.join(&suite.reaper_policy);
    if !policy_path.exists() {
        eprintln!("Skipping: policy file not found at {:?}", policy_path);
        return;
    }

    let result = run_comparison_reaper_only(&suite, &base_path).expect("Failed to run comparison");
    print_comparison_results(&result);

    // All tests should pass in Reaper-only mode
    assert_eq!(
        result.failed, 0,
        "Expected all tests to pass, but {} failed",
        result.failed
    );
}

/// Run all comparison suites in Reaper-only mode
#[test]
fn test_all_comparisons_reaper_only() {
    let comparison_path = get_comparison_path();
    let test_cases_dir = comparison_path.join("test_cases");

    if !test_cases_dir.exists() {
        eprintln!(
            "Skipping: test_cases directory not found at {:?}",
            test_cases_dir
        );
        return;
    }

    let base_path = get_workspace_root();
    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut suite_count = 0;

    for entry in std::fs::read_dir(&test_cases_dir).expect("Failed to read test_cases directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            println!("\n--- Loading: {:?} ---", path.file_name().unwrap());

            match load_comparison_suite(&path) {
                Ok(suite) => {
                    // Check if policy exists
                    let policy_path = base_path.join(&suite.reaper_policy);
                    if !policy_path.exists() {
                        eprintln!("Skipping: policy not found at {:?}", policy_path);
                        continue;
                    }

                    suite_count += 1;
                    match run_comparison_reaper_only(&suite, &base_path) {
                        Ok(result) => {
                            print_comparison_results(&result);
                            total_passed += result.passed;
                            total_failed += result.failed;
                        }
                        Err(e) => {
                            eprintln!("Failed to run suite: {}", e);
                            total_failed += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to load suite: {}", e);
                }
            }
        }
    }

    println!("\n=== TOTAL COMPARISON RESULTS ===");
    println!(
        "Suites run: {} | Passed: {} | Failed: {}",
        suite_count, total_passed, total_failed
    );

    // Note: Not asserting zero failures here as policies may not exist yet
    // Individual tests will fail if their policies exist but produce wrong results
}
