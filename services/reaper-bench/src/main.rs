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
mod comparison_scenarios;
pub mod eopa_client;
mod packages;
pub mod policy_mapping;
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

use benchmark::{BenchmarkConfig, BenchmarkResult, ComparisonReport, MultiPolicyComparisonReport};
use client::AgentClient;
use eopa_client::EopaClient;
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

    /// Reaper Agent URL (used for HTTP/HTTPS transport)
    #[arg(
        long,
        env = "REAPER_AGENT_URL",
        default_value = "http://localhost:8080"
    )]
    agent_url: String,

    /// Unix Domain Socket path for agent connection (overrides agent_url when set)
    #[arg(long, env = "REAPER_AGENT_UDS_PATH")]
    agent_uds_path: Option<String>,

    /// Enterprise OPA URL for comparison benchmarks
    #[arg(long, env = "EOPA_URL")]
    eopa_url: Option<String>,

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
    /// Primary agent client (UDS when available, HTTP otherwise)
    client: Arc<AgentClient>,
    /// TCP-only agent client for transport comparison benchmarks
    tcp_client: Arc<AgentClient>,
    agent_url: String,
    results_cache: Arc<DashMap<String, BenchmarkReport>>,
    eopa_client: Option<Arc<EopaClient>>,
    comparison_cache: Arc<DashMap<String, ComparisonReport>>,
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
    if let Some(ref uds) = args.agent_uds_path {
        info!("  Agent UDS: {}", uds);
    }
    info!("  Listen port: {}", args.port);

    // Create primary agent client (UDS takes priority over HTTP when set)
    let client = client::create_agent_client(
        &args.agent_url,
        args.agent_uds_path.as_deref(),
        args.tls_ca.as_deref(),
        args.tls_cert.as_deref(),
        args.tls_key.as_deref(),
    )?;

    // Create TCP-only client for transport comparison (always HTTP, no UDS)
    let tcp_client = client::create_agent_client(
        &args.agent_url,
        None, // force HTTP by passing None for UDS
        args.tls_ca.as_deref(),
        args.tls_cert.as_deref(),
        args.tls_key.as_deref(),
    )?;

    let transport_mode = if args.agent_uds_path.is_some() {
        "UDS"
    } else if args.tls_ca.is_some() && args.tls_cert.is_some() {
        "HTTPS/mTLS"
    } else {
        "HTTP"
    };
    info!("Primary client transport: {}", transport_mode);
    info!("TCP comparison client transport: HTTP");

    // Create eOPA client if URL is configured
    let eopa_client = args.eopa_url.as_ref().map(|url| {
        info!("  eOPA URL: {}", url);
        Arc::new(EopaClient::new(url))
    });
    if eopa_client.is_none() {
        info!("  eOPA: not configured (set EOPA_URL to enable comparison benchmarks)");
    }

    let state = AppState {
        client: Arc::new(client),
        tcp_client: Arc::new(tcp_client),
        agent_url: args.agent_url.clone(),
        results_cache: Arc::new(DashMap::new()),
        eopa_client,
        comparison_cache: Arc::new(DashMap::new()),
    };

    // Build router
    let app = Router::new()
        // HTML dashboard
        .route("/", get(dashboard_view))
        .route("/view", get(dashboard_view))
        .route("/results/{id}", get(results_view))
        // Health check
        .route("/health", get(health_check))
        // Agent stats endpoint
        .route("/agent-stats", get(get_agent_stats))
        // Initialize agent with benchmark policies
        .route("/init", post(initialize_agent))
        // JSON API endpoints
        .route("/run-benchmark", post(run_benchmark))
        .route("/run-benchmark/{volume}", post(run_single_volume))
        .route("/run-latency", post(run_latency_mode))
        .route("/run-throughput", post(run_throughput_mode))
        .route("/run-simulation", post(run_simulation_mode))
        // Policy package endpoints (local packages)
        .route("/packages", get(list_packages))
        .route("/packages/{name}", get(get_package))
        .route("/packages/{name}/run", post(run_package))
        // Agent package evaluation endpoints (live agent)
        .route("/agent-packages", get(list_agent_packages))
        .route(
            "/agent-packages/{name}/evaluate",
            post(evaluate_agent_package),
        )
        .route(
            "/agent-packages/{name}/benchmark",
            post(benchmark_agent_package),
        )
        .route("/agent-evaluate-all", post(evaluate_all_agent_policies))
        .route("/agent-benchmark-all", post(benchmark_all_policies))
        .route("/compare-modes", post(compare_execution_modes))
        // eOPA comparison endpoints
        .route("/eopa-health", get(eopa_health_check))
        .route("/init-eopa", post(initialize_eopa))
        .route("/init-all", post(initialize_all))
        .route("/run-comparison", post(run_comparison))
        .route("/run-comparison-all", post(run_comparison_all))
        .route("/compare/{policy}", post(compare_single_policy))
        .with_state(state);

    // Start server
    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Benchmark service listening on http://{}", addr);
    info!("");
    info!("Endpoints:");
    info!("  GET  /                              - Interactive dashboard");
    info!("  GET  /health                        - Health check");
    info!("  POST /run-benchmark                 - Run full benchmark suite");
    info!("  POST /run-latency                   - Run latency mode only");
    info!("  POST /run-throughput                - Run throughput mode only");
    info!("  POST /run-simulation                - Run full simulation with auto-tuning");
    info!("");
    info!("Local Package Endpoints:");
    info!("  GET  /packages                      - List local policy packages");
    info!("  GET  /packages/:name                - Get package details");
    info!("  POST /packages/:name/run            - Run package tests");
    info!("");
    info!("Agent Package Endpoints:");
    info!("  GET  /agent-packages                - List packages from agent");
    info!("  POST /agent-packages/:name/evaluate - Evaluate request against package");
    info!("  POST /agent-packages/:name/benchmark- Benchmark package evaluation");
    info!("  POST /agent-evaluate-all            - Evaluate against ALL policies");
    info!("  POST /agent-benchmark-all           - Benchmark all policies evaluation");
    info!("  POST /compare-modes                 - Compare individual vs package modes");
    info!("");
    info!("eOPA Comparison Endpoints:");
    info!("  GET  /eopa-health                   - eOPA health check");
    info!("  POST /init-eopa                     - Load .rego policies + data into eOPA");
    info!("  POST /init-all                      - Initialize both Reaper agent and eOPA");
    info!("  POST /run-comparison                - Run Reaper vs eOPA comparison");
    info!("  POST /run-comparison-all            - Run all 12 policies comparison");
    info!("  POST /compare/:policy               - Quick single-policy comparison");

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

/// Get agent stats from health endpoint
async fn get_agent_stats(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Query agent health endpoint which includes stats
    match state.client.health(&state.agent_url).await {
        Ok(health) => {
            // Extract stats from health response
            let policies_loaded = health.get("policies_loaded").and_then(|v| v.as_u64());
            let total_evaluations = health.get("total_evaluations").and_then(|v| v.as_u64());
            let decisions_allow = health.get("decisions_allow").and_then(|v| v.as_u64());
            let decisions_deny = health.get("decisions_deny").and_then(|v| v.as_u64());
            let cache_hits = health.get("cache_hits").and_then(|v| v.as_u64());
            let cache_misses = health.get("cache_misses").and_then(|v| v.as_u64());

            Ok(Json(serde_json::json!({
                "status": "connected",
                "agent_url": state.agent_url,
                "health": health.get("status").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "policies_loaded": policies_loaded,
                "total_evaluations": total_evaluations,
                "decisions_allow": decisions_allow,
                "decisions_deny": decisions_deny,
                "cache_hits": cache_hits,
                "cache_misses": cache_misses
            })))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("Failed to reach agent: {}", e),
        )),
    }
}

/// Initialize agent with all benchmark policies and data
///
/// POST /init
/// Deploys all benchmark policies from /app/policies/*.reap to the agent
/// and loads all data files from /app/policies/data/*.json
async fn initialize_agent(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    info!("Initializing agent with benchmark policies...");

    let policies_dir = std::path::Path::new("/app/policies");
    let mut deployed_policies = Vec::new();
    let mut failed_policies = Vec::new();
    let mut loaded_data_files = Vec::new();

    // First, load all data files
    let data_dir = policies_dir.join("data");
    if data_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    let filename = path.file_name().unwrap().to_string_lossy().to_string();
                    info!("Loading data file: {}", filename);
                    match std::fs::read_to_string(&path) {
                        Ok(data_json) => {
                            match state.client.load_data(&state.agent_url, &data_json).await {
                                Ok(_) => {
                                    info!("  ✓ Loaded {}", filename);
                                    loaded_data_files.push(filename);
                                }
                                Err(e) => {
                                    tracing::warn!("  ✗ Failed to load {}: {}", filename, e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("  ✗ Failed to read {}: {}", filename, e);
                        }
                    }
                }
            }
        }
    }

    // Then deploy all .reap policies
    if let Ok(entries) = std::fs::read_dir(policies_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "reap").unwrap_or(false) {
                let filename = path.file_name().unwrap().to_string_lossy().to_string();

                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        // Extract actual policy name from content (e.g., "policy rbac_simple {")
                        let policy_name = extract_policy_name(&content)
                            .unwrap_or_else(|| filename.trim_end_matches(".reap").to_string());
                        info!("Deploying policy: {} (from {})", policy_name, filename);

                        match state
                            .client
                            .deploy_policy(&state.agent_url, &policy_name, &content)
                            .await
                        {
                            Ok(_) => {
                                info!("  ✓ Deployed {}", policy_name);
                                deployed_policies.push(policy_name);
                            }
                            Err(e) => {
                                tracing::warn!("  ✗ Failed to deploy {}: {}", policy_name, e);
                                failed_policies.push(serde_json::json!({
                                    "policy": policy_name,
                                    "error": e.to_string()
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        let policy_name = filename.trim_end_matches(".reap");
                        tracing::warn!("  ✗ Failed to read {}: {}", filename, e);
                        failed_policies.push(serde_json::json!({
                            "policy": policy_name,
                            "error": format!("Failed to read file: {}", e)
                        }));
                    }
                }
            }
        }
    }

    info!(
        "Initialization complete: {} policies deployed, {} failed, {} data files loaded",
        deployed_policies.len(),
        failed_policies.len(),
        loaded_data_files.len()
    );

    Ok(Json(serde_json::json!({
        "status": "initialized",
        "deployed_policies": deployed_policies,
        "failed_policies": failed_policies,
        "loaded_data_files": loaded_data_files,
        "summary": {
            "total_deployed": deployed_policies.len(),
            "total_failed": failed_policies.len(),
            "total_data_files": loaded_data_files.len()
        }
    })))
}

/// Interactive HTML dashboard
async fn dashboard_view() -> impl IntoResponse {
    Html(include_str!("templates/dashboard.html"))
}

/// View saved benchmark results
async fn results_view(Path(id): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
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

    // Load appropriate data file for the policy
    if let Some(data_file) = get_data_file_for_policy(&request.policy_name) {
        let data_path = format!("/app/policies/{}", data_file);
        info!("Loading data for benchmark from: {}", data_path);
        if let Ok(data_json) = std::fs::read_to_string(&data_path) {
            match state.client.load_data(&state.agent_url, &data_json).await {
                Ok(_) => info!("Data loaded successfully"),
                Err(e) => tracing::warn!("Failed to load data: {}", e),
            }
        }
    }

    let config = BenchmarkConfig {
        agent_url: state.agent_url.clone(),
        policy_name: request.policy_name,
        volumes: request.volumes,
        modes: request.modes,
        concurrency: request.concurrency,
        batch_size: request.batch_size,
        warmup_requests: request.warmup_requests,
        execution_mode: benchmark::BenchmarkExecutionMode::Individual,
    };

    let report = benchmark::run_full_benchmark(&state.client, config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Cache the report
    state
        .results_cache
        .insert(report.id.clone(), report.clone());

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
        execution_mode: benchmark::BenchmarkExecutionMode::Individual,
    };

    let report = benchmark::run_full_benchmark(&state.client, config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .results_cache
        .insert(report.id.clone(), report.clone());

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

    // Load appropriate data file for the policy
    if let Some(data_file) = get_data_file_for_policy(&config.policy_name) {
        let data_path = format!("/app/policies/{}", data_file);
        info!("Loading data for simulation from: {}", data_path);
        if let Ok(data_json) = std::fs::read_to_string(&data_path) {
            match state.client.load_data(&state.agent_url, &data_json).await {
                Ok(_) => info!("Data loaded successfully"),
                Err(e) => tracing::warn!("Failed to load data: {}", e),
            }
        }
    }

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
async fn get_package(
    Path(name): Path<String>,
) -> Result<Json<packages::PolicyPackage>, StatusCode> {
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
    let package = packages::get_package(&name).ok_or((
        StatusCode::NOT_FOUND,
        format!("Package '{}' not found", name),
    ))?;

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
            Ok(data_json) => match state.client.load_data(&state.agent_url, &data_json).await {
                Ok(result) => {
                    info!("Data loaded successfully: {:?}", result);
                }
                Err(e) => {
                    tracing::warn!("Failed to load data: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read data file {}: {}", data_path, e);
            }
        }
    }

    let mut scenario_results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;

    // Get the first policy name for this package
    let policy_name = package.policies.first().ok_or((
        StatusCode::BAD_REQUEST,
        "Package has no policies".to_string(),
    ))?;

    for scenario in &package.scenarios {
        let mut scenario_passed = true;
        let mut last_latency = 0u64;
        let mut error_msg = None;

        // Run multiple iterations
        for _ in 0..request.iterations {
            let policy_req = client::PolicyRequest {
                policy_name: policy_name.clone(),
                principal: scenario
                    .user
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                action: scenario.action.clone(),
                resource: scenario
                    .resource
                    .get("id")
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
            actual: if scenario_passed {
                scenario.expected.clone()
            } else {
                "different".to_string()
            },
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

/// Extract policy name from .reap content (e.g., "policy rbac_simple {" -> "rbac_simple")
fn extract_policy_name(content: &str) -> Option<String> {
    // Look for "policy <name> {" pattern
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("policy ") {
            // Extract name between "policy " and " {"
            let after_policy = trimmed.trim_start_matches("policy ");
            if let Some(name_end) = after_policy.find(|c: char| c == ' ' || c == '{') {
                let name = after_policy[..name_end].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Get the data file path for a given policy name
fn get_data_file_for_policy(policy_name: &str) -> Option<&'static str> {
    match policy_name {
        "rbac_simple" => Some("data/rbac_data.json"),
        "abac_clearance" => Some("data/abac_data.json"),
        "rebac_relationships" => Some("data/rebac_data.json"),
        "multilayer_enterprise" => Some("data/multilayer_data.json"),
        "benchmark_rbac" => Some("data/benchmark_data.json"),
        "string_operations" => Some("data/string_data.json"),
        "math_validation" => Some("data/math_data.json"),
        "regex_validation" => Some("data/regex_data.json"),
        "collection_operations" => Some("data/collection_data.json"),
        "conditionals" => Some("data/conditional_data.json"),
        "time_based_access" => Some("data/time_data.json"),
        "comprehensions" | "comprehension_test" => Some("data/comprehension_data.json"),
        "json_operations" | "json_processing" => Some("data/json_data.json"),
        "mega_policy" => Some("data/mega_data.json"),
        _ => None,
    }
}

use serde::Serialize;

// =============================================================================
// Agent Package Evaluation Endpoints
// =============================================================================

/// List packages from the agent (live)
async fn list_agent_packages(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.client.list_packages(&state.agent_url).await {
        Ok(packages) => Ok(Json(serde_json::json!({
            "source": "agent",
            "agent_url": state.agent_url,
            "packages": packages,
            "total": packages.len()
        }))),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("Failed to list packages from agent: {}", e),
        )),
    }
}

/// Request for evaluating a package
#[derive(Debug, Deserialize)]
struct AgentPackageEvaluateRequest {
    principal: String,
    action: String,
    resource: String,
    #[serde(default)]
    context: Option<std::collections::HashMap<String, String>>,
}

/// Evaluate a request against a package on the agent
async fn evaluate_agent_package(
    State(state): State<AppState>,
    Path(package): Path<String>,
    Json(request): Json<AgentPackageEvaluateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let eval_req = client::EvaluateRequest {
        policy_id: None,
        policy_name: None,
        principal: request.principal,
        action: request.action,
        resource: request.resource,
        context: request.context,
    };

    match state
        .client
        .evaluate_package(&state.agent_url, &package, &eval_req)
        .await
    {
        Ok(response) => Ok(Json(serde_json::json!({
            "package": response.package,
            "decision": response.decision,
            "denied_by": response.denied_by,
            "policies_evaluated": response.policies_evaluated,
            "evaluation_time_microseconds": response.total_evaluation_time_microseconds
        }))),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("Package evaluation failed: {}", e),
        )),
    }
}

/// Evaluate a request against ALL policies on the agent
async fn evaluate_all_agent_policies(
    State(state): State<AppState>,
    Json(request): Json<AgentPackageEvaluateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let eval_req = client::EvaluateRequest {
        policy_id: None,
        policy_name: None,
        principal: request.principal,
        action: request.action,
        resource: request.resource,
        context: request.context,
    };

    match state.client.evaluate_all(&state.agent_url, &eval_req).await {
        Ok(response) => Ok(Json(serde_json::json!({
            "decision": response.decision,
            "denied_by": response.denied_by,
            "policies_evaluated": response.policies_evaluated,
            "packages_evaluated": response.packages_evaluated,
            "evaluation_time_microseconds": response.total_evaluation_time_microseconds
        }))),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("All policies evaluation failed: {}", e),
        )),
    }
}

/// Request for benchmarking package evaluation
#[derive(Debug, Deserialize)]
struct PackageBenchmarkRequest {
    #[serde(default = "default_benchmark_volume")]
    volume: u32,
    #[serde(default = "default_warmup")]
    warmup: u32,
}

/// Benchmark package evaluation on the agent
async fn benchmark_agent_package(
    State(state): State<AppState>,
    Path(package): Path<String>,
    Json(request): Json<PackageBenchmarkRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    info!(
        "Running package benchmark for '{}': {} requests",
        package, request.volume
    );

    match benchmark::run_package_benchmark(
        &state.client,
        &state.agent_url,
        &package,
        request.volume,
        request.warmup,
    )
    .await
    {
        Ok(result) => {
            info!(
                "Package '{}' benchmark complete: p99={}µs, throughput={:.0} rps",
                package, result.latency.p99_us, result.throughput_rps
            );
            Ok(Json(result))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Package benchmark failed: {}", e),
        )),
    }
}

/// Benchmark all policies evaluation on the agent
async fn benchmark_all_policies(
    State(state): State<AppState>,
    Json(request): Json<PackageBenchmarkRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    info!(
        "Running all-policies benchmark: {} requests",
        request.volume
    );

    match benchmark::run_all_policies_benchmark(
        &state.client,
        &state.agent_url,
        request.volume,
        request.warmup,
    )
    .await
    {
        Ok(result) => {
            info!(
                "All-policies benchmark complete: p99={}µs, throughput={:.0} rps",
                result.latency.p99_us, result.throughput_rps
            );
            Ok(Json(result))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("All-policies benchmark failed: {}", e),
        )),
    }
}

/// Request for comparing execution modes
#[derive(Debug, Deserialize)]
struct CompareModeRequest {
    package: String,
    #[serde(default = "default_benchmark_volume")]
    volume: u32,
}

/// Compare individual vs package evaluation modes
async fn compare_execution_modes(
    State(state): State<AppState>,
    Json(request): Json<CompareModeRequest>,
) -> Result<Json<benchmark::ModeComparisonResult>, (StatusCode, String)> {
    info!(
        "Comparing execution modes for package '{}': {} requests",
        request.package, request.volume
    );

    match benchmark::compare_execution_modes(
        &state.client,
        &state.agent_url,
        &request.package,
        request.volume,
    )
    .await
    {
        Ok(result) => {
            info!(
                "Mode comparison complete: latency reduction={:.1}%, throughput increase={:.1}%",
                result.improvement.latency_reduction_percent,
                result.improvement.throughput_increase_percent
            );
            Ok(Json(result))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Mode comparison failed: {}", e),
        )),
    }
}

// =============================================================================
// eOPA Comparison Endpoints
// =============================================================================

/// Helper: return the eOPA client or 503 if not configured.
fn require_eopa(state: &AppState) -> Result<Arc<EopaClient>, (StatusCode, String)> {
    state.eopa_client.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "eOPA not configured. Set EOPA_URL to enable comparison benchmarks.".to_string(),
    ))
}

/// eOPA health check
async fn eopa_health_check(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let eopa = require_eopa(&state)?;
    match eopa.health().await {
        Ok(true) => Ok(Json(serde_json::json!({
            "status": "healthy",
            "eopa_url": eopa.base_url()
        }))),
        Ok(false) => Err((
            StatusCode::BAD_GATEWAY,
            "eOPA health check returned non-success status".to_string(),
        )),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("eOPA health check failed: {}", e),
        )),
    }
}

/// Initialize eOPA with .rego policies and data.
///
/// POST /init-eopa
/// Loads all .rego policies from /app/opa-policies/ and data from /app/policies/data/*.json
async fn initialize_eopa(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let eopa = require_eopa(&state)?;
    info!("Initializing eOPA with policies and data...");

    // Load .rego policies
    let loaded_policies = eopa_client::load_rego_policies_from_dir(&eopa, "/app/opa-policies")
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load rego policies: {}", e),
            )
        })?;

    // Load data files — merge ALL entity data into a single document before loading.
    // OPA's PUT /v1/data replaces all data, so we must merge first to avoid overwrites.
    let mut loaded_data_files = Vec::new();
    let mut merged_entities = serde_json::Map::new();
    let data_dir = std::path::Path::new("/app/policies/data");
    if data_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(data_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    let filename = path.file_name().unwrap().to_string_lossy().to_string();
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                            let opa_data = eopa_client::transform_entities_for_opa(&data);
                            // Merge entities from this file into the accumulated map
                            if let Some(entities) = opa_data.get("entities") {
                                if let Some(obj) = entities.as_object() {
                                    for (k, v) in obj {
                                        merged_entities.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                            info!(
                                "  eOPA data merged: {} ({} entities)",
                                filename,
                                opa_data
                                    .get("entities")
                                    .and_then(|e| e.as_object())
                                    .map(|o| o.len())
                                    .unwrap_or(0)
                            );
                            loaded_data_files.push(filename);
                        }
                    }
                }
            }
        }
    }

    // Load the merged entity data into eOPA in a single PUT
    if !merged_entities.is_empty() {
        let merged_data = serde_json::json!({"entities": merged_entities});
        let entity_count = merged_data
            .get("entities")
            .and_then(|e| e.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        match eopa.load_data(&merged_data).await {
            Ok(_) => info!(
                "  eOPA merged data loaded: {} total entities from {} files",
                entity_count,
                loaded_data_files.len()
            ),
            Err(e) => tracing::warn!("  eOPA merged data load failed: {}", e),
        }
    }

    info!(
        "eOPA initialization complete: {} policies, {} data files",
        loaded_policies.len(),
        loaded_data_files.len()
    );

    Ok(Json(serde_json::json!({
        "status": "initialized",
        "loaded_policies": loaded_policies,
        "loaded_data_files": loaded_data_files,
        "summary": {
            "total_policies": loaded_policies.len(),
            "total_data_files": loaded_data_files.len()
        }
    })))
}

/// Initialize both Reaper agent and eOPA.
///
/// POST /init-all
async fn initialize_all(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    info!("Initializing both Reaper agent and eOPA...");

    // Initialize Reaper agent (reuse existing logic)
    let agent_result = initialize_agent(State(state.clone())).await;
    let agent_json = match agent_result {
        Ok(Json(v)) => v,
        Err((code, msg)) => serde_json::json!({"error": msg, "status_code": code.as_u16()}),
    };

    // Initialize eOPA if configured
    let eopa_json = if state.eopa_client.is_some() {
        match initialize_eopa(State(state.clone())).await {
            Ok(Json(v)) => v,
            Err((code, msg)) => serde_json::json!({"error": msg, "status_code": code.as_u16()}),
        }
    } else {
        serde_json::json!({"status": "skipped", "reason": "eOPA not configured"})
    };

    Ok(Json(serde_json::json!({
        "agent": agent_json,
        "eopa": eopa_json
    })))
}

fn default_comparison_concurrency() -> u32 {
    1
}

/// Request for running a comparison benchmark
#[derive(Debug, Deserialize)]
struct RunComparisonRequest {
    /// Reaper policy name to compare
    #[serde(default = "default_comparison_policy")]
    policy_name: String,
    /// Request volumes to test
    #[serde(default = "default_comparison_volumes")]
    volumes: Vec<u32>,
    /// Warmup requests
    #[serde(default = "default_warmup")]
    warmup_requests: u32,
    /// Concurrency level (1 = sequential, >1 = concurrent individual requests)
    #[serde(default = "default_comparison_concurrency")]
    concurrency: u32,
}

fn default_comparison_policy() -> String {
    "rbac_simple".to_string()
}

fn default_comparison_volumes() -> Vec<u32> {
    vec![100, 1000, 10000]
}

/// Run a full comparison benchmark (Reaper UDS vs Reaper TCP vs eOPA).
///
/// POST /run-comparison
async fn run_comparison(
    State(state): State<AppState>,
    Json(request): Json<RunComparisonRequest>,
) -> Result<Json<ComparisonReport>, (StatusCode, String)> {
    let eopa = require_eopa(&state)?;

    info!(
        "Running comparison: policy='{}', volumes={:?}",
        request.policy_name, request.volumes
    );

    // Load data for the policy into both engines
    load_data_for_comparison(&state, &eopa, &request.policy_name).await;

    let report = benchmark::run_comparison_benchmark(
        &state.client,
        &state.tcp_client,
        &state.agent_url,
        &eopa,
        &request.policy_name,
        &request.volumes,
        request.warmup_requests,
        request.concurrency,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Comparison benchmark failed: {}", e),
        )
    })?;

    // Cache the report
    state
        .comparison_cache
        .insert(report.id.clone(), report.clone());

    Ok(Json(report))
}

/// Request for running all comparison benchmarks
#[derive(Debug, Deserialize)]
struct RunComparisonAllRequest {
    /// Request volumes to test
    #[serde(default = "default_comparison_volumes")]
    volumes: Vec<u32>,
    /// Warmup requests
    #[serde(default = "default_warmup")]
    warmup_requests: u32,
    /// Concurrency level (1 = sequential, >1 = concurrent individual requests)
    #[serde(default = "default_comparison_concurrency")]
    concurrency: u32,
}

/// Run comparison benchmarks for all 12 policy types (Reaper UDS vs Reaper TCP vs eOPA).
///
/// POST /run-comparison-all
async fn run_comparison_all(
    State(state): State<AppState>,
    Json(request): Json<RunComparisonAllRequest>,
) -> Result<Json<MultiPolicyComparisonReport>, (StatusCode, String)> {
    let eopa = require_eopa(&state)?;

    info!(
        "Running ALL policy comparisons at volumes {:?}",
        request.volumes
    );

    // Pre-load data for all policies into both engines
    for policy_name in policy_mapping::available_policy_names() {
        load_data_for_comparison(&state, &eopa, policy_name).await;
    }

    let report = benchmark::run_all_comparison_benchmarks(
        &state.client,
        &state.tcp_client,
        &state.agent_url,
        &eopa,
        &request.volumes,
        request.warmup_requests,
        request.concurrency,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("All-policy comparison failed: {}", e),
        )
    })?;

    info!(
        "All-policy comparison complete: {} policies, avg speedup {:.1}x",
        report.overall_summary.total_policies, report.overall_summary.avg_speedup
    );

    Ok(Json(report))
}

/// Load data for a policy into both Reaper agent and eOPA.
async fn load_data_for_comparison(state: &AppState, _eopa: &EopaClient, policy_name: &str) {
    if let Some(data_file) = get_data_file_for_policy(policy_name) {
        let data_path = format!("/app/policies/{}", data_file);
        if let Ok(data_json) = std::fs::read_to_string(&data_path) {
            // Load into Reaper agent (additive — DataStore accumulates entities)
            let _ = state.client.load_data(&state.agent_url, &data_json).await;

            // NOTE: Do NOT reload into eOPA here. OPA's PUT /v1/data replaces ALL data,
            // which would discard entities from other data files. All data is pre-loaded
            // during init-eopa as a single merged document.
        }
    }
}

/// Quick single-policy comparison
///
/// POST /compare/{policy}
async fn compare_single_policy(
    State(state): State<AppState>,
    Path(policy): Path<String>,
    Json(request): Json<RunComparisonRequest>,
) -> Result<Json<ComparisonReport>, (StatusCode, String)> {
    let mut req = request;
    req.policy_name = policy;
    run_comparison(State(state), Json(req)).await
}
