//! Reaper vs OPA Benchmark Tool
//!
//! Comprehensive performance comparison between Reaper and Open Policy Agent (OPA).
//! Measures latency, throughput, and provides detailed statistical analysis.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use hdrhistogram::Histogram;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tabled::{Table, Tabled};
use tokio::sync::Semaphore;

#[derive(Parser)]
#[command(name = "reaper-vs-opa-benchmark")]
#[command(about = "Benchmark Reaper vs OPA policy engines", long_about = None)]
struct Args {
    /// Number of requests to send
    #[arg(short, long, default_value = "10000")]
    requests: usize,

    /// Concurrent requests
    #[arg(short, long, default_value = "50")]
    concurrency: usize,

    /// Reaper endpoint
    #[arg(long, default_value = "http://localhost:8080")]
    reaper_url: String,

    /// OPA endpoint
    #[arg(long, default_value = "http://localhost:8181")]
    opa_url: String,

    /// Policy scenario to test (rbac, abac, all)
    #[arg(short, long, default_value = "rbac")]
    scenario: String,

    /// Output format (table, json, csv)
    #[arg(short, long, default_value = "table")]
    output: String,

    /// Save results to file
    #[arg(long)]
    save: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchmarkResult {
    engine: String,
    scenario: String,
    total_requests: usize,
    successful: usize,
    failed: usize,
    allowed: usize,
    denied: usize,
    validation_errors: usize,
    duration_secs: f64,
    throughput_rps: f64,
    latency_p50_us: f64,
    latency_p95_us: f64,
    latency_p99_us: f64,
    latency_max_us: f64,
    latency_mean_us: f64,
    #[serde(default)]
    memory_mb: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
struct DecisionResult {
    decision: Decision,
    expected: Option<Decision>,
}

impl DecisionResult {
    fn is_valid(&self) -> bool {
        match &self.expected {
            Some(expected) => matches!(
                (&self.decision, expected),
                (Decision::Allow, Decision::Allow) | (Decision::Deny, Decision::Deny)
            ),
            None => true, // No expectation means validation passes
        }
    }
}

#[derive(Tabled)]
struct BenchmarkRow {
    #[tabled(rename = "Engine")]
    engine: String,
    #[tabled(rename = "Scenario")]
    scenario: String,
    #[tabled(rename = "Requests")]
    requests: String,
    #[tabled(rename = "Success")]
    success_rate: String,
    #[tabled(rename = "Allow")]
    allowed: String,
    #[tabled(rename = "Deny")]
    denied: String,
    #[tabled(rename = "RPS")]
    throughput: String,
    #[tabled(rename = "P50 (μs)")]
    p50: String,
    #[tabled(rename = "P95 (μs)")]
    p95: String,
    #[tabled(rename = "P99 (μs)")]
    p99: String,
    #[tabled(rename = "Max (μs)")]
    max: String,
}

impl From<&BenchmarkResult> for BenchmarkRow {
    fn from(result: &BenchmarkResult) -> Self {
        let allow_pct = (result.allowed as f64 / result.total_requests as f64) * 100.0;
        let deny_pct = (result.denied as f64 / result.total_requests as f64) * 100.0;

        Self {
            engine: result.engine.clone(),
            scenario: result.scenario.clone(),
            requests: format!("{}/{}", result.successful, result.total_requests),
            success_rate: format!(
                "{:.2}%",
                (result.successful as f64 / result.total_requests as f64) * 100.0
            ),
            allowed: format!("{} ({:.0}%)", result.allowed, allow_pct),
            denied: format!("{} ({:.0}%)", result.denied, deny_pct),
            throughput: format!("{:.0}", result.throughput_rps),
            p50: format!("{:.0}", result.latency_p50_us),
            p95: format!("{:.0}", result.latency_p95_us),
            p99: format!("{:.0}", result.latency_p99_us),
            max: format!("{:.0}", result.latency_max_us),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct PolicyRequest {
    principal: Principal,
    action: String,
    resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<RequestContext>,
    #[serde(skip)]
    expected_decision: Option<Decision>,
}

#[derive(Debug, Clone, Serialize)]
struct Principal {
    id: String,
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clearance: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RequestContext {
    hour: i32,
    day_of_week: String,
    source_ip: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
struct Resource {
    #[serde(skip_serializing_if = "Option::is_none")]
    department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let json_output = args.output == "json";

    // Use eprintln for progress messages so JSON output stays clean on stdout
    eprintln!("{}", "\n🚀 Reaper vs OPA Benchmark".bold().cyan());
    eprintln!("{}", "=".repeat(80).dimmed());
    eprintln!("  Requests:     {}", args.requests.to_string().yellow());
    eprintln!("  Concurrency:  {}", args.concurrency.to_string().yellow());
    eprintln!("  Scenario:     {}", args.scenario.yellow());
    eprintln!("  Reaper URL:   {}", args.reaper_url.dimmed());
    eprintln!("  OPA URL:      {}", args.opa_url.dimmed());
    eprintln!("{}\n", "=".repeat(80).dimmed());

    // Test connectivity
    eprintln!("{}", "🔍 Testing connectivity...".bold());
    test_connectivity(&args.reaper_url, "Reaper").await?;
    test_connectivity(&args.opa_url, "OPA").await?;
    eprintln!();

    // Validate policy logic with known test cases
    eprintln!("{}", "🧪 Validating policy logic...".bold());
    let scenarios = if args.scenario == "all" {
        vec!["rbac", "abac"]
    } else {
        vec![args.scenario.as_str()]
    };

    for scenario in &scenarios {
        validate_policy_logic(&args.reaper_url, "Reaper", scenario).await?;
        validate_policy_logic(&args.opa_url, "OPA", scenario).await?;
    }
    eprintln!("{}", "  ✓ All validation tests passed!".green());
    eprintln!();

    // Decision-parity gate: both engines must agree on the actual decisions
    // across a representative sample before any speedup is measured. A benchmark
    // that compares latency over non-equivalent decisions is meaningless, so a
    // divergence aborts the run rather than producing a misleading number.
    eprintln!("{}", "⚖️  Enforcing cross-engine decision parity...".bold());
    let parity_sample = args.requests.min(2000).max(200);
    for scenario in &scenarios {
        enforce_decision_parity(&args.reaper_url, &args.opa_url, scenario, parity_sample).await?;
    }
    eprintln!();

    // Run benchmarks

    let mut all_results = Vec::new();

    for scenario in scenarios {
        eprintln!("{} {}", "📊 Running scenario:".bold(), scenario.yellow());
        eprintln!();

        // Benchmark Reaper
        let reaper_result = run_benchmark(
            "Reaper",
            &args.reaper_url,
            scenario,
            args.requests,
            args.concurrency,
        )
        .await?;
        all_results.push(reaper_result);

        // Benchmark OPA
        let opa_result = run_benchmark(
            "OPA",
            &args.opa_url,
            scenario,
            args.requests,
            args.concurrency,
        )
        .await?;
        all_results.push(opa_result);

        eprintln!();
    }

    // Display results
    display_results(&all_results, &args.output)?;

    // Save results if requested
    if let Some(path) = &args.save {
        save_results(&all_results, path)?;
        eprintln!("\n{} {}", "💾 Results saved to:".green(), path);
    }

    // Display winner (always to stderr so JSON stays clean)
    if !json_output {
        display_winner(&all_results);
    }

    Ok(())
}

async fn validate_policy_logic(url: &str, engine: &str, scenario: &str) -> Result<()> {
    let client = reqwest::Client::new();

    // Define test cases with expected outcomes
    let test_cases = match scenario {
        "rbac" => vec![
            // Should ALLOW - admin with full access
            (
                PolicyRequest {
                    principal: Principal {
                        id: "user_admin".to_string(),
                        role: "admin".to_string(),
                        department: None,
                        clearance: None,
                        region: None,
                    },
                    action: "delete".to_string(),
                    resource: "/api/hr/records".to_string(),
                    context: None,
                    expected_decision: Some(Decision::Allow),
                },
                "admin can delete any resource",
            ),
            // Should ALLOW - viewer can read
            (
                PolicyRequest {
                    principal: Principal {
                        id: "user_viewer".to_string(),
                        role: "viewer".to_string(),
                        department: None,
                        clearance: None,
                        region: None,
                    },
                    action: "read".to_string(),
                    resource: "/api/engineering/data".to_string(),
                    context: None,
                    expected_decision: Some(Decision::Allow),
                },
                "viewer can read",
            ),
            // Should DENY - viewer cannot write
            (
                PolicyRequest {
                    principal: Principal {
                        id: "user_viewer".to_string(),
                        role: "viewer".to_string(),
                        department: None,
                        clearance: None,
                        region: None,
                    },
                    action: "write".to_string(),
                    resource: "/api/engineering/data".to_string(),
                    context: None,
                    expected_decision: Some(Decision::Deny),
                },
                "viewer cannot write",
            ),
        ],
        "abac" => vec![
            // Should ALLOW - engineer with high clearance
            (
                PolicyRequest {
                    principal: Principal {
                        id: "user_engineer_high".to_string(),
                        role: "engineer".to_string(),
                        department: Some("engineering".to_string()),
                        clearance: Some(4),
                        region: Some("us-west".to_string()),
                    },
                    action: "read".to_string(),
                    resource: "resource_eng_confidential".to_string(),
                    context: Some(RequestContext {
                        hour: 10,
                        day_of_week: "mon".to_string(),
                        source_ip: "10.0.1.1".to_string(),
                    }),
                    expected_decision: Some(Decision::Allow),
                },
                "engineer with clearance can read confidential engineering resource",
            ),
        ],
        _ => vec![],
    };

    for (test_case, description) in test_cases {
        let expected = test_case.expected_decision.clone().unwrap();
        let result = if engine == "Reaper" {
            send_reaper_request(&client, url, scenario, test_case).await?
        } else {
            send_opa_request(&client, url, scenario, test_case).await?
        };

        let decision_str = match result.decision {
            Decision::Allow => "Allow".green(),
            Decision::Deny => "Deny".red(),
        };

        let expected_str = match expected {
            Decision::Allow => "Allow",
            Decision::Deny => "Deny",
        };

        if !result.is_valid() {
            return Err(anyhow::anyhow!(
                "  {} {} {}: Expected {}, got {}",
                "✗".red(),
                engine,
                description,
                expected_str,
                decision_str
            ));
        }

        eprintln!(
            "  {} {} {}: {} (expected: {})",
            "✓".green(),
            engine.dimmed(),
            description.dimmed(),
            decision_str,
            expected_str
        );
    }

    Ok(())
}

async fn test_connectivity(url: &str, name: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let health_url = format!("{}/health", url);

    match client.get(&health_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            eprintln!("  {} {} is reachable", "✓".green(), name);
            Ok(())
        }
        Ok(resp) => {
            anyhow::bail!("{} returned HTTP {}", name, resp.status())
        }
        Err(e) => {
            anyhow::bail!("Cannot reach {}: {}", name, e)
        }
    }
}

async fn run_benchmark(
    engine: &str,
    url: &str,
    scenario: &str,
    total_requests: usize,
    concurrency: usize,
) -> Result<BenchmarkResult> {
    eprintln!("  {} {}...", "Testing".dimmed(), engine.bold());

    let client = Arc::new(reqwest::Client::new());
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut histogram = Histogram::<u64>::new(3).expect("Failed to create histogram");

    let pb = ProgressBar::new(total_requests as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let start = Instant::now();
    let mut tasks = Vec::new();
    let mut successful = 0;
    let mut failed = 0;
    let mut allowed = 0;
    let mut denied = 0;
    let mut validation_errors = 0;

    for i in 0..total_requests {
        let client = Arc::clone(&client);
        let semaphore = Arc::clone(&semaphore);
        let url = url.to_string();
        let scenario = scenario.to_string();
        let engine = engine.to_string();
        let pb = pb.clone();

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();

            let request = generate_request(&scenario, i);
            let req_start = Instant::now();

            let result = if engine == "Reaper" {
                send_reaper_request(&client, &url, &scenario, request).await
            } else {
                send_opa_request(&client, &url, &scenario, request).await
            };

            let latency = req_start.elapsed();
            pb.inc(1);

            (result, latency)
        });

        tasks.push(task);
    }

    // Collect results
    for task in tasks {
        let (result, latency) = task.await?;
        match result {
            Ok(decision_result) => {
                successful += 1;
                histogram.record(latency.as_micros() as u64).ok();

                // Track allow vs deny
                match decision_result.decision {
                    Decision::Allow => allowed += 1,
                    Decision::Deny => denied += 1,
                }

                // Track validation errors (if expected decision doesn't match)
                if !decision_result.is_valid() {
                    validation_errors += 1;
                }
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    let duration = start.elapsed();
    pb.finish_and_clear();

    let result = BenchmarkResult {
        engine: engine.to_string(),
        scenario: scenario.to_string(),
        total_requests,
        successful,
        failed,
        allowed,
        denied,
        validation_errors,
        duration_secs: duration.as_secs_f64(),
        throughput_rps: successful as f64 / duration.as_secs_f64(),
        latency_p50_us: histogram.value_at_quantile(0.50) as f64,
        latency_p95_us: histogram.value_at_quantile(0.95) as f64,
        latency_p99_us: histogram.value_at_quantile(0.99) as f64,
        latency_max_us: histogram.max() as f64,
        latency_mean_us: histogram.mean(),
        memory_mb: 0.0, // Will be filled in by benchmark.sh script
    };

    eprintln!(
        "  {} {} - {} req/s, p99: {:.0}μs",
        "✓".green(),
        engine.bold(),
        format!("{:.0}", result.throughput_rps).yellow(),
        result.latency_p99_us
    );

    Ok(result)
}

fn generate_request(scenario: &str, index: usize) -> PolicyRequest {
    let actions = ["read", "write", "delete"];

    match scenario {
        "rbac" => {
            // RBAC: Supports both small (user_admin) and large (user_admin_123) datasets
            let roles = ["admin", "manager", "engineer", "viewer"];
            let resources = [
                "/api/engineering/data",
                "/api/hr/records",
                "/api/finance/reports",
            ];

            let role = roles[index % roles.len()];
            // Generate ID: user_{role}_{num} for large datasets, or user_{role} for small
            let user_id = format!("user_{}_{}", role, index % 1000);

            PolicyRequest {
                principal: Principal {
                    id: user_id,
                    role: "admin".to_string(), // Not used for entity lookup
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "abac" => {
            // ABAC: Format is user_{dept}_{role}_{num}
            let departments = ["engineering", "hr", "finance", "sales", "operations"];
            let roles = ["admin", "manager", "engineer", "analyst", "intern"];
            let classifications = ["public", "internal", "confidential", "secret"];

            let dept = departments[index % departments.len()];
            let role = roles[index % roles.len()];
            let classification = classifications[index % classifications.len()];

            let user_id = format!("user_{}_{}_{}", dept, role, index % 1000);
            let resource_id = format!("resource_{}_{}_{}", dept, classification, index % 100);

            PolicyRequest {
                principal: Principal {
                    id: user_id,
                    role: "engineer".to_string(),
                    department: Some(dept.to_string()),
                    clearance: Some(((index % 5) + 1) as i32),
                    region: Some("us-west".to_string()),
                },
                action: actions[index % actions.len()].to_string(),
                resource: resource_id,
                context: Some(RequestContext {
                    hour: 10,
                    day_of_week: "mon".to_string(),
                    source_ip: "10.0.1.1".to_string(),
                }),
                expected_decision: None,
            }
        }
        "rebac" => {
            // ReBAC: Format is user_{team}_{num}
            let teams = ["alpha", "beta", "gamma", "delta", "omega"];

            let team = teams[index % teams.len()];
            let user_id = format!("user_{}_{}", team, index % 1000);
            let resource_id = format!("resource_{}_{}", team, index % 200);

            PolicyRequest {
                principal: Principal {
                    id: user_id,
                    role: "engineer".to_string(),
                    department: Some("engineering".to_string()),
                    clearance: Some(3),
                    region: Some("us-west".to_string()),
                },
                action: actions[index % actions.len()].to_string(),
                resource: resource_id,
                context: Some(RequestContext {
                    hour: 10,
                    day_of_week: "mon".to_string(),
                    source_ip: "10.0.1.1".to_string(),
                }),
                expected_decision: None,
            }
        }
        "multilayer" => {
            // Multilayer: Format is user_{role}_{dept}_{num}
            let departments = ["engineering", "hr", "finance", "executive", "operations"];
            let roles = [
                "admin",
                "executive",
                "senior",
                "engineer",
                "manager",
                "intern",
            ];
            let classifications = ["public", "internal", "confidential", "secret"];

            let dept = departments[index % departments.len()];
            let role = roles[index % roles.len()];
            let classification = classifications[index % classifications.len()];

            let user_id = format!("user_{}_{}_{}", role, dept, index % 1000);
            let resource_id = format!("resource_{}_{}_{}", dept, classification, index % 200);

            PolicyRequest {
                principal: Principal {
                    id: user_id,
                    role: "engineer".to_string(),
                    department: Some(dept.to_string()),
                    clearance: Some(((index % 5) + 1) as i32),
                    region: Some("us-west".to_string()),
                },
                action: actions[index % actions.len()].to_string(),
                resource: resource_id,
                context: Some(RequestContext {
                    hour: 10,
                    day_of_week: "mon".to_string(),
                    source_ip: "10.0.1.1".to_string(),
                }),
                expected_decision: None,
            }
        }
        "math" => {
            // Math policy: numeric validation rules
            let resources = [
                "premium_loan",        // credit_score >= 700
                "shopping_cart",       // order_total <= budget_limit
                "featured_listing",    // average_rating >= 4.0
                "marketplace",         // list_price 1-10000
                "premium_tier",        // score >= 90
                "temperature_monitor", // temp -50 to 50
                "loyalty_reward",      // total_points >= 1000
                "sale_item",           // discount 0-50%
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("math_user_{}", index % 100000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "regex" => {
            // Regex policy: pattern validation rules using actual regex::matches()
            let resources = [
                "email_service",   // regex::matches(user.email, email_pattern)
                "phone_service",   // regex::matches(user.phone, phone_pattern)
                "uuid_service",    // regex::matches(user.uuid, uuid_pattern)
                "payment_service", // regex::matches(user.credit_card, cc_pattern)
                "web_service",     // regex::matches(user.url, url_pattern)
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("regex_user_{}", index % 100000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "time" => {
            // Time policy: time-based access control
            let roles = [
                "employee",
                "operator",
                "event_planner",
                "contractor",
                "system",
                "audit_logger",
                "api_client",
                "archiver",
            ];
            let resources = [
                "api_endpoint",
                "office_system",
                "apartment",
                "production_system",
                "web_session",
                "conference_room",
                "project_files",
                "timestamp_data",
                "audit_trail",
                "rate_limited_endpoint",
                "data",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("time_user_{}", index % 100000),
                    role: roles[index % roles.len()].to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "string" => {
            // String policy: string method calls (.contains, .startswith, .endswith)
            let resources = [
                "internal_docs",    // user.email.endswith("@company.com")
                "partner_portal",   // user.email.contains("partner")
                "admin_panel",      // user.username.startswith("admin_")
                "gov_service",      // user.email.endswith(".gov")
                "test_environment", // user.username.contains("test")
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("string_user_{}", index % 100000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "collection" => {
            // Collection policy: array/set/map operations
            let resources = [
                "document",
                "senior_position",
                "shared_resource",
                "content",
                "system",
                "invoice",
                "profile",
                "email_campaign",
                "workflow",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("collection_user_{}", index % 100000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: ["view", "edit", "read"][index % 3].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "comprehension" => {
            // Comprehension policy: list comprehension rules
            let resources = [
                "set_result",
                "array_result",
                "object_result",
                "complex_filter",
                "nested_result",
                "transformed_data",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("comp_user_{}", index % 10000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "json" => {
            // JSON policy: JSON structure validation
            let resources = [
                "api_endpoint",
                "user_profile",
                "payment",
                "order",
                "form_data",
                "text_field",
                "number_field",
                "boolean_field",
                "structured_data",
                "data_merge",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("json_user_{}", index % 10000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        "mega" => {
            // Mega policy: 105 rules covering all patterns
            let resources = [
                // Math
                "premium_loan",
                "shopping_cart",
                "featured_listing",
                "marketplace_low",
                "marketplace_medium",
                "marketplace_high",
                "bronze_tier",
                "silver_tier",
                "gold_tier",
                "cold_storage",
                "normal_storage",
                "warm_storage",
                "loyalty_basic",
                "loyalty_premium",
                "loyalty_elite",
                // String
                "admin_access",
                "manager_access",
                "user_access",
                "code_a",
                "code_b",
                "code_c",
                "company_docs",
                "partner_docs",
                "external_docs",
                "admin_panel",
                "manager_panel",
                "user_panel",
                "gov_classified",
                "mil_classified",
                "edu_resources",
                // Regex
                "email_validation",
                "phone_validation",
                "url_validation",
                "ip_validation",
                "uuid_validation",
                // Time
                "api_endpoint",
                "office_morning",
                "office_afternoon",
                "adult_content",
                "apartment_access",
                // Collection
                "doc_read",
                "doc_write",
                "doc_delete",
                "junior_position",
                "mid_position",
                "senior_position",
                "eng_resource",
                "admin_resource",
                "manager_resource",
                // Comprehension
                "numbers_gt5",
                "numbers_gt10",
                "high_priority",
                "medium_priority",
                "active_records",
                // JSON
                "api_submit",
                "profile_basic",
                "profile_full",
                "payment_card",
                "payment_billing",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("mega_user_{}", index % 10000),
                    role: "user".to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
        _ => {
            // Default fallback for unknown scenarios
            let roles = ["admin", "manager", "engineer", "viewer"];
            let resources = [
                "/api/engineering/data",
                "/api/public/info",
                "/api/hr/records",
                "/api/finance/reports",
            ];

            PolicyRequest {
                principal: Principal {
                    id: format!("user{}", index % 100),
                    role: roles[index % roles.len()].to_string(),
                    department: None,
                    clearance: None,
                    region: None,
                },
                action: actions[index % actions.len()].to_string(),
                resource: resources[index % resources.len()].to_string(),
                context: None,
                expected_decision: None,
            }
        }
    }
}

async fn send_reaper_request(
    client: &reqwest::Client,
    url: &str,
    scenario: &str,
    request: PolicyRequest,
) -> Result<DecisionResult> {
    // Map scenario to policy ID
    let policy_id = format!("{}-policy", scenario);

    let payload = json!({
        "policy_id": policy_id,
        "principal": request.principal.id,  // Send entity ID for DataStore lookup
        "action": request.action,
        "resource": request.resource,
        "context": {}
    });

    let resp = client
        .post(format!("{}/api/v1/messages", url))
        .json(&payload)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await?;

    // Parse decision from response
    let decision = if status.is_success() {
        // Check if response contains "allow" decision
        if let Some(decision_val) = body.get("decision") {
            if decision_val.as_str() == Some("allow") {
                Decision::Allow
            } else {
                Decision::Deny
            }
        } else {
            // Fallback: HTTP 200 = Allow, anything else = Deny
            eprintln!("DEBUG: No decision field found in response, defaulting to Allow");
            Decision::Allow
        }
    } else {
        Decision::Deny
    };

    Ok(DecisionResult {
        decision,
        expected: request.expected_decision,
    })
}

/// Cross-engine decision-parity gate.
///
/// Sends an identical sample of generated requests to both Reaper and OPA and
/// verifies they produce the same allow/deny decision on every one. Returns an
/// error (aborting the benchmark) on any divergence: a speedup measured over
/// requests where the engines disagree is not a valid comparison.
async fn enforce_decision_parity(
    reaper_url: &str,
    opa_url: &str,
    scenario: &str,
    sample_size: usize,
) -> Result<()> {
    let client = reqwest::Client::new();
    let mut mismatches: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for i in 0..sample_size {
        let request = generate_request(scenario, i);
        let principal = request.principal.id.clone();
        let action = request.action.clone();
        let resource = request.resource.clone();

        let reaper = send_reaper_request(&client, reaper_url, scenario, request.clone()).await?;
        let opa = send_opa_request(&client, opa_url, scenario, request).await?;
        checked += 1;

        if reaper.decision != opa.decision && mismatches.len() < 10 {
            mismatches.push(format!(
                "      principal={principal} action={action} resource={resource} → Reaper={:?}, OPA={:?}",
                reaper.decision, opa.decision
            ));
        }
    }

    if mismatches.is_empty() {
        eprintln!(
            "  {} {} ({checked} sampled requests agree)",
            "✓ decision parity:".green(),
            scenario
        );
        Ok(())
    } else {
        eprintln!(
            "  {} {} — {} of {checked} sampled requests disagree:",
            "✗ decision parity FAILED:".red().bold(),
            scenario,
            mismatches.len()
        );
        for m in &mismatches {
            eprintln!("{}", m.red());
        }
        anyhow::bail!(
            "Reaper and OPA disagree on scenario '{scenario}'. A speedup over non-equivalent \
             decisions is not a valid comparison — align the rego policy and OPA data mapping \
             with the Reaper policy before benchmarking."
        )
    }
}

async fn send_opa_request(
    client: &reqwest::Client,
    url: &str,
    scenario: &str,
    request: PolicyRequest,
) -> Result<DecisionResult> {
    // Uniform input shape for EVERY scenario: the principal is sent as its id
    // string (all rego resolve the entity via `data.entities[input.principal]`),
    // alongside action, resource, and context. This matches how Reaper receives
    // the request and keeps the two engines evaluating the same thing.
    let payload = json!({
        "input": {
            "principal": request.principal.id,
            "action": request.action,
            "resource": request.resource,
            "context": request.context,
        }
    });

    let policy_path = format!("reaper/{}/allow", scenario);

    let resp = client
        .post(format!("{}/v1/data/{}", url, policy_path))
        .json(&payload)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await?;

    // Parse decision from OPA response
    let decision = if status.is_success() {
        // OPA returns {"result": true/false} for allow rules
        if let Some(result) = body.get("result") {
            if result.as_bool().unwrap_or(false) {
                Decision::Allow
            } else {
                Decision::Deny
            }
        } else {
            Decision::Deny
        }
    } else {
        Decision::Deny
    };

    Ok(DecisionResult {
        decision,
        expected: request.expected_decision,
    })
}

fn display_results(results: &[BenchmarkResult], format: &str) -> Result<()> {
    match format {
        "json" => {
            // JSON goes to stdout (clean, no extra text)
            println!("{}", serde_json::to_string_pretty(results)?);
        }
        "csv" => {
            eprintln!("\n{}", "📈 Benchmark Results".bold().cyan());
            eprintln!("{}", "=".repeat(80).dimmed());
            println!("Engine,Scenario,Requests,Success,Failed,Duration(s),RPS,P50(μs),P95(μs),P99(μs),Max(μs)");
            for r in results {
                println!(
                    "{},{},{},{},{},{:.2},{:.0},{:.0},{:.0},{:.0},{:.0}",
                    r.engine,
                    r.scenario,
                    r.total_requests,
                    r.successful,
                    r.failed,
                    r.duration_secs,
                    r.throughput_rps,
                    r.latency_p50_us,
                    r.latency_p95_us,
                    r.latency_p99_us,
                    r.latency_max_us
                );
            }
        }
        _ => {
            // Table format
            eprintln!("\n{}", "📈 Benchmark Results".bold().cyan());
            eprintln!("{}", "=".repeat(80).dimmed());
            let rows: Vec<BenchmarkRow> = results.iter().map(BenchmarkRow::from).collect();
            let table = Table::new(rows).to_string();
            eprintln!("{}", table);
        }
    }

    Ok(())
}

fn save_results(results: &[BenchmarkResult], path: &str) -> Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn display_winner(results: &[BenchmarkResult]) {
    eprintln!("\n{}", "🏆 Performance Comparison".bold().cyan());
    eprintln!("{}", "=".repeat(80).dimmed());

    // Group by scenario
    let scenarios: Vec<&str> = results.iter().map(|r| r.scenario.as_str()).collect();
    let unique_scenarios: Vec<&str> = scenarios
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for scenario in unique_scenarios {
        let scenario_results: Vec<&BenchmarkResult> =
            results.iter().filter(|r| r.scenario == scenario).collect();

        if scenario_results.len() < 2 {
            continue;
        }

        let reaper = scenario_results
            .iter()
            .find(|r| r.engine == "Reaper")
            .unwrap();
        let opa = scenario_results.iter().find(|r| r.engine == "OPA").unwrap();

        eprintln!("\n{} Scenario:", scenario.bold());

        // Throughput comparison
        let throughput_diff = ((reaper.throughput_rps / opa.throughput_rps) - 1.0) * 100.0;
        if throughput_diff > 0.0 {
            eprintln!(
                "  Throughput: {} is {:.1}% faster ({:.0} vs {:.0} req/s)",
                "Reaper".green().bold(),
                throughput_diff,
                reaper.throughput_rps,
                opa.throughput_rps
            );
        } else {
            eprintln!(
                "  Throughput: {} is {:.1}% faster ({:.0} vs {:.0} req/s)",
                "OPA".green().bold(),
                -throughput_diff,
                opa.throughput_rps,
                reaper.throughput_rps
            );
        }

        // P99 latency comparison
        let latency_diff = ((opa.latency_p99_us / reaper.latency_p99_us) - 1.0) * 100.0;
        if latency_diff > 0.0 {
            eprintln!(
                "  P99 Latency: {} is {:.1}% lower ({:.0}μs vs {:.0}μs)",
                "Reaper".green().bold(),
                latency_diff,
                reaper.latency_p99_us,
                opa.latency_p99_us
            );
        } else {
            eprintln!(
                "  P99 Latency: {} is {:.1}% lower ({:.0}μs vs {:.0}μs)",
                "OPA".green().bold(),
                -latency_diff,
                opa.latency_p99_us,
                reaper.latency_p99_us
            );
        }
    }

    eprintln!();
}
