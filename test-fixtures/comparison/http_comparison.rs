//! HTTP-based Comparison Runner
//!
//! This is a standalone binary for running comparison tests against
//! live Reaper Agent and OPA services.
//!
//! Usage:
//!   cargo run --release -p http-comparison
//!
//! Environment:
//!   REAPER_URL=http://localhost:8080
//!   OPA_URL=http://localhost:8181
//!
//! This file is a reference implementation. For actual use, run the shell script:
//!   ./run-comparison.sh

use std::collections::HashMap;
use std::env;
use std::time::Instant;

fn main() {
    let reaper_url = env::var("REAPER_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let opa_url = env::var("OPA_URL").unwrap_or_else(|_| "http://localhost:8181".to_string());
    let reaper_only = env::var("REAPER_ONLY").is_ok();

    println!("Reaper vs OPA HTTP Comparison");
    println!("==============================");
    println!("Reaper URL: {}", reaper_url);
    println!("OPA URL: {}", opa_url);
    println!("Mode: {}", if reaper_only { "Reaper-only" } else { "Full comparison" });
    println!();

    // Define test cases
    let test_cases = vec![
        // RBAC tests
        ("admin_read", "user_0", "read", "resource_100", "allow", "rbac"),
        ("admin_write", "user_0", "write", "resource_200", "allow", "rbac"),
        ("owner_read", "user_50", "read", "resource_50", "allow", "rbac"),
        ("viewer_read", "user_700", "read", "resource_900", "allow", "rbac"),
        // String tests
        ("case_insensitive", "user_mixed_case", "read", "case_insensitive", "allow", "string_ops"),
        ("email_contains", "user_email_contains_company", "view", "internal_docs", "allow", "string_ops"),
        ("gov_email", "user_gov_email", "access", "classified_docs", "allow", "string_ops"),
    ];

    let client = reqwest::blocking::Client::new();
    let mut passed = 0;
    let mut failed = 0;

    for (id, principal, action, resource, expected, opa_package) in test_cases {
        // Query Reaper
        let reaper_start = Instant::now();
        let reaper_body = serde_json::json!({
            "principal": principal,
            "action": action,
            "resource": resource
        });

        let reaper_decision = match client
            .post(&format!("{}/api/v1/evaluate", reaper_url))
            .json(&reaper_body)
            .send()
        {
            Ok(resp) => {
                let json: serde_json::Value = resp.json().unwrap_or_default();
                let decision = json.get("decision")
                    .or_else(|| json.get("allowed"))
                    .map(|v| {
                        if v.as_bool().unwrap_or(false) || v.as_str().map(|s| s.to_lowercase().contains("allow")).unwrap_or(false) {
                            "allow"
                        } else {
                            "deny"
                        }
                    })
                    .unwrap_or("deny");
                decision.to_string()
            }
            Err(e) => {
                eprintln!("Reaper error: {}", e);
                "error".to_string()
            }
        };
        let reaper_time = reaper_start.elapsed().as_micros();

        // Query OPA (if not Reaper-only)
        let (opa_decision, opa_time) = if !reaper_only {
            let opa_start = Instant::now();
            let opa_body = serde_json::json!({
                "input": {
                    "principal": principal,
                    "action": action,
                    "resource": resource
                }
            });

            let decision = match client
                .post(&format!("{}/v1/data/{}/allow", opa_url, opa_package))
                .json(&opa_body)
                .send()
            {
                Ok(resp) => {
                    let json: serde_json::Value = resp.json().unwrap_or_default();
                    if json.get("result").and_then(|v| v.as_bool()).unwrap_or(false) {
                        "allow".to_string()
                    } else {
                        "deny".to_string()
                    }
                }
                Err(e) => {
                    eprintln!("OPA error: {}", e);
                    "error".to_string()
                }
            };
            let time = opa_start.elapsed().as_micros();
            (decision, Some(time))
        } else {
            ("skipped".to_string(), None)
        };

        // Check results
        let reaper_pass = reaper_decision == expected;
        let opa_pass = reaper_only || opa_decision == expected;
        let test_passed = reaper_pass && opa_pass;

        if test_passed {
            passed += 1;
            print!("\x1b[32m✓\x1b[0m");
        } else {
            failed += 1;
            print!("\x1b[31m✗\x1b[0m");
        }

        print!(" {}: Reaper={} ({}µs)", id, reaper_decision, reaper_time);
        if let Some(opa_t) = opa_time {
            print!(", OPA={} ({}µs)", opa_decision, opa_t);
        }
        println!(", expected={}", expected);
    }

    println!();
    println!("==============================");
    println!("Results: {} passed, {} failed", passed, failed);

    std::process::exit(if failed > 0 { 1 } else { 0 });
}
