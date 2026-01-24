//! Reaper Benchmark Service
//!
//! A high-performance benchmark service for testing Reaper Agent's policy evaluation.
//! Supports mTLS communication and provides detailed latency statistics.
//!
//! # Endpoints
//!
//! - `GET /` - Interactive HTML dashboard
//! - `GET /health` - Health check
//! - `POST /run-benchmark` - Run full benchmark suite (JSON)
//! - `POST /run-benchmark/:volume` - Run specific volume (JSON)
//! - `POST /run-latency` - Run latency mode only (JSON)
//! - `POST /run-throughput` - Run throughput mode only (JSON)

mod benchmark;
mod client;
mod packages;
mod report;
mod scenarios;
mod simulation;
mod stats;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use clap::Parser;
use dashmap::DashMap;
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use benchmark::{BenchmarkConfig, BenchmarkResult};
use client::AgentClient;
use report::BenchmarkReport;
use simulation::{SimulationConfig, SimulationResult};

/// CLI arguments
#[derive(Parser, Debug)]
#[command(name = "reaper-bench")]
#[command(about = "Reaper Benchmark Service - Policy Engine Performance Testing")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Reaper Agent URL
    #[arg(long, env = "REAPER_AGENT_URL", default_value = "http://localhost:8080")]
    agent_url: String,

    /// TLS CA certificate file
    #[arg(long, env = "REAPER_TLS_CA")]
    tls_ca: Option<String>,

    /// TLS client certificate file
    #[arg(long, env = "REAPER_TLS_CERT")]
    tls_cert: Option<String>,

    /// TLS client key file
    #[arg(long, env = "REAPER_TLS_KEY")]
    tls_key: Option<String>,
}

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    client: Arc<AgentClient>,
    agent_url: String,
    results_cache: Arc<DashMap<String, BenchmarkReport>>,
}

/// Request body for running benchmarks
#[derive(Debug, Deserialize)]
struct RunBenchmarkRequest {
    /// Policy name to benchmark
    #[serde(default = "default_policy_name")]
    policy_name: String,

    /// Request volumes to test
    #[serde(default = "default_volumes")]
    volumes: Vec<u32>,

    /// Benchmark modes to run
    #[serde(default = "default_modes")]
    modes: Vec<String>,

    /// Concurrent batch requests
    #[serde(default = "default_concurrency")]
    concurrency: u32,

    /// Requests per batch
    #[serde(default = "default_batch_size")]
    batch_size: u32,

    /// Warmup requests before timing
    #[serde(default = "default_warmup")]
    warmup_requests: u32,
}

fn default_policy_name() -> String {
    "benchmark_rbac".to_string()
}

fn default_volumes() -> Vec<u32> {
    vec![10, 100, 1000, 10000]
}

fn default_modes() -> Vec<String> {
    vec!["latency".to_string(), "throughput".to_string()]
}

fn default_concurrency() -> u32 {
    10
}

fn default_batch_size() -> u32 {
    100
}

fn default_warmup() -> u32 {
    100
}

/// Request for latency mode benchmark
#[derive(Debug, Deserialize)]
struct RunLatencyRequest {
    #[serde(default = "default_policy_name")]
    policy_name: String,
    #[serde(default = "default_latency_volume")]
    volume: u32,
    #[serde(default = "default_warmup")]
    warmup_requests: u32,
}

fn default_latency_volume() -> u32 {
    1000
}

/// Request for throughput mode benchmark
#[derive(Debug, Deserialize)]
struct RunThroughputRequest {
    #[serde(default = "default_policy_name")]
    policy_name: String,
    #[serde(default = "default_throughput_volume")]
    volume: u32,
    #[serde(default = "default_batch_size")]
    batch_size: u32,
    #[serde(default = "default_concurrency")]
    concurrency: u32,
}

fn default_throughput_volume() -> u32 {
    10000
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,reaper_bench=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse CLI arguments
    let args = Args::parse();

    info!("Starting Reaper Benchmark Service");
    info!("  Agent URL: {}", args.agent_url);
    info!("  Listen port: {}", args.port);

    // Create agent client
    let client = client::create_agent_client(
        args.tls_ca.as_deref(),
        args.tls_cert.as_deref(),
        args.tls_key.as_deref(),
    )?;

    info!(
        "Client configured with TLS: {}",
        args.tls_ca.is_some() && args.tls_cert.is_some()
    );

    let state = AppState {
        client: Arc::new(client),
        agent_url: args.agent_url.clone(),
        results_cache: Arc::new(DashMap::new()),
    };

    // Build router
    let app = Router::new()
        // HTML dashboard
        .route("/", get(dashboard_view))
        .route("/view", get(dashboard_view))
        .route("/results/{id}", get(results_view))
        // Health check
        .route("/health", get(health_check))
        // JSON API endpoints
        .route("/run-benchmark", post(run_benchmark))
        .route("/run-benchmark/{volume}", post(run_single_volume))
        .route("/run-latency", post(run_latency_mode))
        .route("/run-throughput", post(run_throughput_mode))
        .route("/run-simulation", post(run_simulation_mode))
        // Policy package endpoints
        .route("/packages", get(list_packages))
        .route("/packages/{name}", get(get_package))
        .route("/packages/{name}/run", post(run_package))
        .with_state(state);

    // Start server
    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Benchmark service listening on http://{}", addr);
    info!("");
    info!("Endpoints:");
    info!("  GET  /                    - Interactive dashboard");
    info!("  GET  /health              - Health check");
    info!("  POST /run-benchmark       - Run full benchmark suite");
    info!("  POST /run-latency         - Run latency mode only");
    info!("  POST /run-throughput      - Run throughput mode only");
    info!("  POST /run-simulation      - Run full simulation with auto-tuning");
    info!("  GET  /packages            - List available policy packages");
    info!("  GET  /packages/:name      - Get package details");
    info!("  POST /packages/:name/run  - Run package tests");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Health check endpoint
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "reaper-bench"
    }))
}

/// Interactive HTML dashboard
async fn dashboard_view() -> impl IntoResponse {
    Html(include_str!("templates/dashboard.html"))
}

/// View saved benchmark results
async fn results_view(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if let Some(report) = state.results_cache.get(&id) {
        let html = report::render_results_html(&report);
        Html(html).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Results not found").into_response()
    }
}

/// Run full benchmark suite
async fn run_benchmark(
    State(state): State<AppState>,
    Json(request): Json<RunBenchmarkRequest>,
) -> Result<Json<BenchmarkReport>, (StatusCode, String)> {
    info!("Starting benchmark: {:?}", request);

    let config = BenchmarkConfig {
        agent_url: state.agent_url.clone(),
        policy_name: request.policy_name,
        volumes: request.volumes,
        modes: request.modes,
        concurrency: request.concurrency,
        batch_size: request.batch_size,
        warmup_requests: request.warmup_requests,
    };

    let report = benchmark::run_full_benchmark(&state.client, config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Cache the report
    state.results_cache.insert(report.id.clone(), report.clone());

    Ok(Json(report))
}

/// Run benchmark for a specific volume
async fn run_single_volume(
    State(state): State<AppState>,
    Path(volume): Path<u32>,
) -> Result<Json<BenchmarkReport>, (StatusCode, String)> {
    let config = BenchmarkConfig {
        agent_url: state.agent_url.clone(),
        policy_name: "benchmark_rbac".to_string(),
        volumes: vec![volume],
        modes: vec!["latency".to_string(), "throughput".to_string()],
        concurrency: 10,
        batch_size: 100,
        warmup_requests: 100,
    };

    let report = benchmark::run_full_benchmark(&state.client, config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.results_cache.insert(report.id.clone(), report.clone());

    Ok(Json(report))
}

/// Run latency mode only
async fn run_latency_mode(
    State(state): State<AppState>,
    Json(request): Json<RunLatencyRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    info!("Running latency benchmark: {:?}", request);

    let result = benchmark::run_latency_benchmark(
        &state.client,
        &state.agent_url,
        &request.policy_name,
        request.volume,
        request.warmup_requests,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(result))
}

/// Run throughput mode only
async fn run_throughput_mode(
    State(state): State<AppState>,
    Json(request): Json<RunThroughputRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    info!("Running throughput benchmark: {:?}", request);

    let result = benchmark::run_throughput_benchmark(
        &state.client,
        &state.agent_url,
        &request.policy_name,
        request.volume,
        request.batch_size,
        request.concurrency,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(result))
}

/// Run full simulation with auto-tuning recommendations
async fn run_simulation_mode(
    State(state): State<AppState>,
    Json(config): Json<SimulationConfig>,
) -> Result<Json<SimulationResult>, (StatusCode, String)> {
    info!("Starting full simulation with config: {:?}", config);

    let result = simulation::run_simulation(&state.client, &state.agent_url, config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Simulation complete: {} combinations tested, peak={:.0} rps",
        result.summary.total_combinations_tested, result.summary.peak_throughput_rps
    );
    info!(
        "Recommendation: batch_size={}, concurrency={} ({:.0} rps expected)",
        result.recommendation.batch_size,
        result.recommendation.concurrency,
        result.recommendation.expected_throughput_rps
    );

    Ok(Json(result))
}

// =============================================================================
// Policy Package Endpoints
// =============================================================================

/// List all available policy packages
async fn list_packages() -> Json<Vec<PackageSummary>> {
    let packages = packages::get_packages();
    let summaries: Vec<PackageSummary> = packages
        .into_iter()
        .map(|p| PackageSummary {
            name: p.name,
            description: p.description,
            policy_count: p.policies.len(),
            scenario_count: p.scenarios.len(),
        })
        .collect();
    Json(summaries)
}

#[derive(Serialize)]
struct PackageSummary {
    name: String,
    description: String,
    policy_count: usize,
    scenario_count: usize,
}

/// Get details for a specific package
async fn get_package(Path(name): Path<String>) -> Result<Json<packages::PolicyPackage>, StatusCode> {
    packages::get_package(&name)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Request to run a package test
#[derive(Debug, Deserialize)]
struct RunPackageRequest {
    /// Number of iterations per scenario
    #[serde(default = "default_iterations")]
    iterations: u32,
    /// Run performance benchmark after correctness tests
    #[serde(default)]
    benchmark: bool,
    /// Benchmark volume (if benchmark is true)
    #[serde(default = "default_benchmark_volume")]
    benchmark_volume: u32,
}

fn default_iterations() -> u32 {
    10
}

fn default_benchmark_volume() -> u32 {
    1000
}

/// Result of running a package test
#[derive(Serialize)]
struct PackageTestResult {
    package_name: String,
    total_scenarios: usize,
    passed: usize,
    failed: usize,
    scenario_results: Vec<ScenarioResult>,
    benchmark_result: Option<BenchmarkResult>,
}

#[derive(Serialize)]
struct ScenarioResult {
    name: String,
    expected: String,
    actual: String,
    passed: bool,
    latency_us: u64,
    error: Option<String>,
}

/// Run all tests for a package
async fn run_package(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<RunPackageRequest>,
) -> Result<Json<PackageTestResult>, (StatusCode, String)> {
    let package = packages::get_package(&name)
        .ok_or((StatusCode::NOT_FOUND, format!("Package '{}' not found", name)))?;

    info!(
        "Running package '{}': {} scenarios, {} iterations each",
        name,
        package.scenarios.len(),
        request.iterations
    );

    // Load data file if specified
    if let Some(data_file) = &package.data_file {
        let data_path = format!("/app/policies/{}", data_file);
        info!("Loading data from: {}", data_path);

        match std::fs::read_to_string(&data_path) {
            Ok(data_json) => {
                match state.client.load_data(&state.agent_url, &data_json).await {
                    Ok(result) => {
                        info!("Data loaded successfully: {:?}", result);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load data: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read data file {}: {}", data_path, e);
            }
        }
    }

    let mut scenario_results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;

    // Get the first policy name for this package
    let policy_name = package.policies.first()
        .ok_or((StatusCode::BAD_REQUEST, "Package has no policies".to_string()))?;

    for scenario in &package.scenarios {
        let mut scenario_passed = true;
        let mut last_latency = 0u64;
        let mut error_msg = None;

        // Run multiple iterations
        for _ in 0..request.iterations {
            let policy_req = client::PolicyRequest {
                policy_name: policy_name.clone(),
                principal: scenario.user.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                action: scenario.action.clone(),
                resource: scenario.resource.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                context: Some(build_context(&scenario.user, &scenario.resource)),
            };

            let start = std::time::Instant::now();
            match state.client.evaluate(&state.agent_url, &policy_req).await {
                Ok(response) => {
                    last_latency = start.elapsed().as_micros() as u64;
                    if response.decision != scenario.expected {
                        scenario_passed = false;
                        error_msg = Some(format!(
                            "Expected '{}' but got '{}'",
                            scenario.expected, response.decision
                        ));
                    }
                }
                Err(e) => {
                    scenario_passed = false;
                    error_msg = Some(e.to_string());
                    break;
                }
            }
        }

        if scenario_passed {
            passed += 1;
        } else {
            failed += 1;
        }

        scenario_results.push(ScenarioResult {
            name: scenario.name.clone(),
            expected: scenario.expected.clone(),
            actual: if scenario_passed { scenario.expected.clone() } else { "different".to_string() },
            passed: scenario_passed,
            latency_us: last_latency,
            error: error_msg,
        });
    }

    // Run benchmark if requested
    let benchmark_result = if request.benchmark {
        info!("Running benchmark for package '{}'", name);
        match benchmark::run_latency_benchmark(
            &state.client,
            &state.agent_url,
            policy_name,
            request.benchmark_volume,
            100, // warmup
        )
        .await
        {
            Ok(result) => Some(result),
            Err(e) => {
                info!("Benchmark failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    info!(
        "Package '{}' complete: {}/{} passed",
        name,
        passed,
        package.scenarios.len()
    );

    Ok(Json(PackageTestResult {
        package_name: name,
        total_scenarios: package.scenarios.len(),
        passed,
        failed,
        scenario_results,
        benchmark_result,
    }))
}

/// Build context from user and resource attributes
fn build_context(
    user: &std::collections::HashMap<String, serde_json::Value>,
    resource: &std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, String> {
    let mut context = std::collections::HashMap::new();

    // Add user attributes with "user." prefix
    for (key, value) in user {
        let str_value = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => value.to_string(),
        };
        context.insert(format!("user.{}", key), str_value);
    }

    // Add resource attributes with "resource." prefix
    for (key, value) in resource {
        let str_value = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => value.to_string(),
        };
        context.insert(format!("resource.{}", key), str_value);
    }

    context
}

use serde::Serialize;
