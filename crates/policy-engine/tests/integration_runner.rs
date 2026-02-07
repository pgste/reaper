//! Integration Test Runner
//!
//! Runs YAML-based test suites against the policy engine.
//! Usage: cargo test -p policy-engine --test integration_runner

mod integration;

use integration::{load_suite, print_results, run_suite};
use std::path::PathBuf;

fn get_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn get_fixtures_path() -> PathBuf {
    get_workspace_root().join("test-fixtures")
}

#[test]
fn test_rbac_core_suite() {
    let fixtures = get_fixtures_path();
    let suite_path = fixtures.join("suites/rbac_core.yaml");

    if !suite_path.exists() {
        eprintln!("Skipping test: suite file not found at {:?}", suite_path);
        return;
    }

    let suite = load_suite(&suite_path).expect("Failed to load suite");

    // Run suite with workspace root as base path
    let base_path = get_workspace_root();
    let result = run_suite(&suite, &base_path).expect("Failed to run suite");

    print_results(&result);

    // All tests should pass
    assert_eq!(
        result.failed, 0,
        "Expected all tests to pass, but {} failed",
        result.failed
    );

    // Performance check
    if let Some(perf) = &suite.performance {
        if let Some(p99_threshold) = perf.p99_threshold_us {
            assert!(
                result.p99_evaluation_time_us <= p99_threshold,
                "P99 latency {}µs exceeds threshold {}µs",
                result.p99_evaluation_time_us,
                p99_threshold
            );
        }
    }
}

#[test]
fn test_string_operations_suite() {
    let fixtures = get_fixtures_path();
    let suite_path = fixtures.join("suites/string_operations.yaml");

    if !suite_path.exists() {
        eprintln!("Skipping test: suite file not found at {:?}", suite_path);
        return;
    }

    let suite = load_suite(&suite_path).expect("Failed to load suite");
    let base_path = get_workspace_root();
    let result = run_suite(&suite, &base_path).expect("Failed to run suite");

    print_results(&result);

    // Note: String operations suite may have context-dependent results
    // For now, just verify it runs without errors
    println!(
        "String operations suite completed: {}/{} passed",
        result.passed, result.total
    );
}

/// Run all suites in the test-fixtures/suites directory
#[test]
fn test_all_suites() {
    let fixtures = get_fixtures_path();
    let suites_dir = fixtures.join("suites");

    if !suites_dir.exists() {
        eprintln!(
            "Skipping test: suites directory not found at {:?}",
            suites_dir
        );
        return;
    }

    let base_path = get_workspace_root();
    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut suite_count = 0;

    for entry in std::fs::read_dir(&suites_dir).expect("Failed to read suites directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            suite_count += 1;
            println!("\n--- Running suite: {:?} ---", path.file_name().unwrap());

            match load_suite(&path) {
                Ok(suite) => match run_suite(&suite, &base_path) {
                    Ok(result) => {
                        print_results(&result);
                        total_passed += result.passed;
                        total_failed += result.failed;
                    }
                    Err(e) => {
                        eprintln!("Failed to run suite: {}", e);
                        total_failed += 1;
                    }
                },
                Err(e) => {
                    eprintln!("Failed to load suite: {}", e);
                    total_failed += 1;
                }
            }
        }
    }

    println!("\n=== TOTAL RESULTS ===");
    println!(
        "Suites: {} | Passed: {} | Failed: {}",
        suite_count, total_passed, total_failed
    );

    // Fail the test if any test cases failed
    assert_eq!(total_failed, 0, "Some test cases failed");
}
