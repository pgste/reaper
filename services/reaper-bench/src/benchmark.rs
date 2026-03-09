//! Benchmark execution logic
//!
//! Three benchmark execution modes:
//! - **Individual**: Evaluate policies one at a time (traditional mode)
//! - **Package**: Evaluate all policies in a package together (bundle mode)
//! - **All**: Evaluate all policies across all packages
//!
//! Two benchmark measurement modes:
//! - **Latency mode**: Sequential requests to measure individual latency (fast-messages)
//! - **Throughput mode**: Batch requests to measure max throughput (batch-messages)

use crate::client::{AgentClient, BatchRequest, BatchRequestItem, EvaluateRequest, PolicyRequest};
use crate::comparison_scenarios;
use crate::eopa_client::{EopaClient, OpaInput, OpaInputPrincipal};
use crate::policy_mapping;
use crate::report::{
    BenchmarkReport, LatencySummary, ReportSummary, SystemInfo, ThroughputSummary,
};
use crate::scenarios;
use crate::scenarios::TestRequest;
use crate::stats::LatencyStats;
use futures::stream::{self, StreamExt};
use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use sysinfo::System;
use tracing::{debug, info, warn};

/// Benchmark execution mode
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum BenchmarkExecutionMode {
    /// Evaluate individual policies one at a time
    #[default]
    Individual,
    /// Evaluate all policies in a package together (bundle mode)
    Package {
        /// Name of the package to evaluate
        package_name: String,
    },
    /// Evaluate all policies across all packages
    All,
}

/// Benchmark configuration
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub agent_url: String,
    pub policy_name: String,
    pub volumes: Vec<u32>,
    pub modes: Vec<String>,
    pub concurrency: u32,
    pub batch_size: u32,
    pub warmup_requests: u32,
    /// Execution mode: Individual, Package, or All
    #[allow(dead_code)]
    pub execution_mode: BenchmarkExecutionMode,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            agent_url: "http://localhost:8080".to_string(),
            policy_name: "default".to_string(),
            volumes: vec![1000],
            modes: vec!["latency".to_string()],
            concurrency: 10,
            batch_size: 100,
            warmup_requests: 100,
            execution_mode: BenchmarkExecutionMode::default(),
        }
    }
}

/// Result from a mode comparison benchmark
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeComparisonResult {
    pub package_name: String,
    pub individual_mode: BenchmarkResult,
    pub package_mode: BenchmarkResult,
    pub improvement: ModeImprovement,
}

/// Improvement metrics when comparing individual vs package mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeImprovement {
    /// Percentage reduction in p99 latency
    pub latency_reduction_percent: f64,
    /// Percentage increase in throughput
    pub throughput_increase_percent: f64,
}

/// Result from a single benchmark run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub mode: String,
    pub volume: u32,
    pub total_requests: u32,
    pub successful: u32,
    pub allowed: u32,
    pub denied: u32,
    pub errors: u32,
    pub duration_ms: u64,
    pub throughput_rps: f64,
    pub latency: LatencyStats,
}

/// Run the full benchmark suite
pub async fn run_full_benchmark(
    client: &AgentClient,
    config: BenchmarkConfig,
) -> anyhow::Result<BenchmarkReport> {
    info!("Starting full benchmark suite");
    info!("  Agent URL: {}", config.agent_url);
    info!("  Policy: {}", config.policy_name);
    info!("  Volumes: {:?}", config.volumes);
    info!("  Modes: {:?}", config.modes);

    // Check agent health first
    match client.health(&config.agent_url).await {
        Ok(health) => info!("Agent health check passed: {:?}", health),
        Err(e) => {
            warn!("Agent health check failed: {}", e);
            anyhow::bail!("Agent not healthy: {}", e);
        }
    }

    let mut results = Vec::new();
    let start_time = Instant::now();

    // Run warmup
    if config.warmup_requests > 0 {
        info!("Running {} warmup requests...", config.warmup_requests);
        let warmup_requests = scenarios::generate_requests(config.warmup_requests as usize);
        for req in warmup_requests.iter().take(100) {
            let policy_req = PolicyRequest {
                policy_name: config.policy_name.clone(),
                principal: req.principal.clone(),
                action: req.action.clone(),
                resource: req.resource.clone(),
                context: req.context.clone(),
            };
            let _ = client.evaluate(&config.agent_url, &policy_req).await;
        }
        info!("Warmup complete");
    }

    // Run benchmarks for each volume
    for volume in &config.volumes {
        // Latency mode
        if config.modes.contains(&"latency".to_string()) {
            info!("Running latency benchmark: {} requests", volume);
            match run_latency_benchmark(
                client,
                &config.agent_url,
                &config.policy_name,
                *volume,
                0, // No additional warmup
            )
            .await
            {
                Ok(result) => {
                    info!(
                        "Latency {} requests: p50={}µs, p99={}µs, throughput={:.0} rps",
                        volume,
                        result.latency.median_us,
                        result.latency.p99_us,
                        result.throughput_rps
                    );
                    results.push(result);
                }
                Err(e) => {
                    warn!("Latency benchmark failed for volume {}: {}", volume, e);
                }
            }
        }

        // Throughput mode
        if config.modes.contains(&"throughput".to_string()) {
            info!("Running throughput benchmark: {} requests", volume);
            match run_throughput_benchmark(
                client,
                &config.agent_url,
                &config.policy_name,
                *volume,
                config.batch_size,
                config.concurrency,
            )
            .await
            {
                Ok(result) => {
                    info!(
                        "Throughput {} requests: throughput={:.0} rps, p99={}µs",
                        volume, result.throughput_rps, result.latency.p99_us
                    );
                    results.push(result);
                }
                Err(e) => {
                    warn!("Throughput benchmark failed for volume {}: {}", volume, e);
                }
            }
        }
    }

    let total_duration = start_time.elapsed();

    // Generate summary
    let summary = generate_summary(&results, total_duration);

    // Get system info
    let mut sys = System::new_all();
    sys.refresh_all();

    // Get current process memory usage (not total system memory)
    let current_pid = sysinfo::get_current_pid().ok();
    let bench_memory_mb = current_pid
        .and_then(|pid| sys.process(pid))
        .map(|p| p.memory() / (1024 * 1024))
        .unwrap_or(0);

    let system_info = SystemInfo {
        cpu_cores: num_cpus::get(),
        cpu_usage_percent: Some(sys.global_cpu_usage()),
        benchmark_service_memory_mb: bench_memory_mb,
        total_memory_mb: Some(sys.total_memory() / (1024 * 1024)),
        agent_memory_mb: None,
        tls_cipher: "TLS_AES_256_GCM_SHA384".to_string(), // Assumed for rustls
        http_version: "HTTP/2".to_string(),
    };

    let report = BenchmarkReport {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_version: "unknown".to_string(), // Would need to query agent
        policy_name: config.policy_name,
        tls_enabled: config.agent_url.starts_with("https"),
        modes_run: config.modes,
        results,
        summary,
        system_info,
    };

    Ok(report)
}

/// Run latency mode benchmark (sequential requests)
///
/// If `custom_requests` is provided, those are used instead of generic scenarios.
/// This is used by comparison benchmarks to use policy-specific scenarios.
pub async fn run_latency_benchmark(
    client: &AgentClient,
    agent_url: &str,
    policy_name: &str,
    volume: u32,
    warmup: u32,
) -> anyhow::Result<BenchmarkResult> {
    run_latency_benchmark_with(client, agent_url, policy_name, volume, warmup, None).await
}

/// Run latency mode benchmark with optional custom request set.
pub async fn run_latency_benchmark_with(
    client: &AgentClient,
    agent_url: &str,
    policy_name: &str,
    volume: u32,
    warmup: u32,
    custom_requests: Option<&[TestRequest]>,
) -> anyhow::Result<BenchmarkResult> {
    // Generate test requests or use custom ones
    let owned_requests;
    let requests: &[TestRequest] = if let Some(reqs) = custom_requests {
        reqs
    } else {
        owned_requests = scenarios::generate_requests((volume + warmup) as usize);
        &owned_requests
    };

    // Create histogram for latency tracking (1µs to 1s range, 3 significant figures)
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    // Warmup phase
    for req in requests.iter().take(warmup as usize) {
        let policy_req = PolicyRequest {
            policy_name: policy_name.to_string(),
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };
        let _ = client.evaluate(agent_url, &policy_req).await;
    }

    // Timed phase
    let start = Instant::now();

    for req in requests.iter().skip(warmup as usize).take(volume as usize) {
        let policy_req = PolicyRequest {
            policy_name: policy_name.to_string(),
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };

        let req_start = Instant::now();
        match client.evaluate(agent_url, &policy_req).await {
            Ok(response) => {
                let elapsed = req_start.elapsed();
                histogram.record(elapsed.as_micros() as u64)?;
                successful += 1;

                if response.decision == "allow" {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("Request error: {}", e);
                errors += 1;
            }
        }
    }

    let duration = start.elapsed();
    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "latency".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run throughput mode benchmark (batch requests)
pub async fn run_throughput_benchmark(
    client: &AgentClient,
    agent_url: &str,
    policy_name: &str,
    volume: u32,
    batch_size: u32,
    concurrency: u32,
) -> anyhow::Result<BenchmarkResult> {
    // Generate test requests
    let requests = scenarios::generate_requests(volume as usize);

    // Create histogram for per-request latency tracking
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    // Split into batches
    let mut request_idx = 0u32;
    let batches: Vec<BatchRequest> = requests
        .chunks(batch_size as usize)
        .map(|chunk| BatchRequest {
            policy_name: policy_name.to_string(),
            requests: chunk
                .iter()
                .map(|r| {
                    let id = format!("req-{}", request_idx);
                    request_idx += 1;
                    BatchRequestItem {
                        id,
                        principal: r.principal.clone(),
                        action: r.action.clone(),
                        resource: r.resource.clone(),
                        context: r.context.clone(),
                    }
                })
                .collect(),
        })
        .collect();

    let num_batches = batches.len();
    info!(
        "Created {} batches of ~{} requests",
        num_batches, batch_size
    );

    let start = Instant::now();

    // Execute batches with concurrency control using the SDK transport
    let results: Vec<_> = stream::iter(batches)
        .map(|batch| {
            let client_ref = &client;
            let url = agent_url.to_string();
            async move { client_ref.evaluate_batch(&url, &batch).await }
        })
        .buffer_unordered(concurrency as usize)
        .collect()
        .await;

    let duration = start.elapsed();

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    // Process results
    for result in results {
        match result {
            Ok(batch_resp) => {
                // Use summary for allow/deny counts
                allowed += batch_resp.summary.allowed;
                denied += batch_resp.summary.denied;
                successful += batch_resp.request_count;

                // Record per-request evaluation times from results array
                for item in &batch_resp.results {
                    if let Some(eval_time) = item.evaluation_time_microseconds {
                        let _ = histogram.record(eval_time as u64);
                    }
                }
            }
            Err(e) => {
                warn!("Batch request error: {}", e);
                errors += batch_size;
            }
        }
    }

    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "throughput".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Generate benchmark summary from results
fn generate_summary(results: &[BenchmarkResult], total_duration: Duration) -> ReportSummary {
    let total_requests: u64 = results.iter().map(|r| r.total_requests as u64).sum();

    // Extract latency mode results
    let latency_results: Vec<_> = results.iter().filter(|r| r.mode == "latency").collect();
    let latency_summary = if !latency_results.is_empty() {
        Some(LatencySummary {
            p50_us: latency_results
                .iter()
                .map(|r| r.latency.median_us)
                .max()
                .unwrap_or(0),
            p99_us: latency_results
                .iter()
                .map(|r| r.latency.p99_us)
                .max()
                .unwrap_or(0),
            p999_us: latency_results
                .iter()
                .map(|r| r.latency.p999_us)
                .max()
                .unwrap_or(0),
            max_us: latency_results
                .iter()
                .map(|r| r.latency.max_us)
                .max()
                .unwrap_or(0),
        })
    } else {
        None
    };

    // Extract throughput mode results
    let throughput_results: Vec<_> = results.iter().filter(|r| r.mode == "throughput").collect();
    let throughput_summary = if !throughput_results.is_empty() {
        let peak_rps = throughput_results
            .iter()
            .map(|r| r.throughput_rps)
            .fold(0.0f64, |a, b| a.max(b));

        let sustained_rps = if throughput_results.len() > 1 {
            let sum: f64 = throughput_results.iter().map(|r| r.throughput_rps).sum();
            sum / throughput_results.len() as f64
        } else {
            peak_rps
        };

        // Calculate batch efficiency (throughput vs latency mode ratio)
        let batch_efficiency = if !latency_results.is_empty() {
            let latency_peak = latency_results
                .iter()
                .map(|r| r.throughput_rps)
                .fold(0.0f64, |a, b| a.max(b));
            if latency_peak > 0.0 {
                peak_rps / latency_peak
            } else {
                1.0
            }
        } else {
            1.0
        };

        Some(ThroughputSummary {
            peak_rps,
            sustained_rps,
            batch_efficiency,
        })
    } else {
        None
    };

    // Generate recommendation
    let recommendation = generate_recommendation(&latency_summary, &throughput_summary);

    ReportSummary {
        total_requests,
        total_duration_ms: total_duration.as_millis() as u64,
        latency_mode: latency_summary,
        throughput_mode: throughput_summary,
        volume_cap_detected: false,
        cap_threshold_rps: None,
        recommendation,
    }
}

/// Generate performance recommendation based on results
fn generate_recommendation(
    latency: &Option<LatencySummary>,
    throughput: &Option<ThroughputSummary>,
) -> String {
    let mut parts = Vec::new();

    if let Some(t) = throughput {
        parts.push(format!("Peak throughput: {:.0} req/s.", t.peak_rps));
        if t.batch_efficiency > 10.0 {
            parts.push(format!(
                "Batch mode provides {:.1}x throughput improvement.",
                t.batch_efficiency
            ));
        }
    }

    if let Some(l) = latency {
        if l.p99_us < 1000 {
            parts.push(format!("Sub-millisecond p99 latency: {}µs.", l.p99_us));
        } else {
            parts.push(format!("P99 latency: {}µs.", l.p99_us));
        }
    }

    if parts.is_empty() {
        "No benchmark data available.".to_string()
    } else {
        parts.join(" ")
    }
}

// ============================================================================
// Package Benchmark Functions
// ============================================================================

/// Run package benchmark (evaluate all policies in a package together)
pub async fn run_package_benchmark(
    client: &AgentClient,
    agent_url: &str,
    package_name: &str,
    volume: u32,
    warmup: u32,
) -> anyhow::Result<BenchmarkResult> {
    // Generate test requests
    let requests = scenarios::generate_requests((volume + warmup) as usize);

    // Create histogram for latency tracking
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    // Warmup phase
    for req in requests.iter().take(warmup as usize) {
        let eval_req = EvaluateRequest {
            policy_id: None,
            policy_name: None,
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };
        let _ = client
            .evaluate_package(agent_url, package_name, &eval_req)
            .await;
    }

    // Timed phase
    let start = Instant::now();

    for req in requests.iter().skip(warmup as usize).take(volume as usize) {
        let eval_req = EvaluateRequest {
            policy_id: None,
            policy_name: None,
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };

        let req_start = Instant::now();
        match client
            .evaluate_package(agent_url, package_name, &eval_req)
            .await
        {
            Ok(response) => {
                let elapsed = req_start.elapsed();
                histogram.record(elapsed.as_micros() as u64)?;
                successful += 1;

                if response.decision == "allow" {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("Package evaluation error: {}", e);
                errors += 1;
            }
        }
    }

    let duration = start.elapsed();
    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "package".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run benchmark evaluating all policies across all packages
pub async fn run_all_policies_benchmark(
    client: &AgentClient,
    agent_url: &str,
    volume: u32,
    warmup: u32,
) -> anyhow::Result<BenchmarkResult> {
    // Generate test requests
    let requests = scenarios::generate_requests((volume + warmup) as usize);

    // Create histogram for latency tracking
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    // Warmup phase
    for req in requests.iter().take(warmup as usize) {
        let eval_req = EvaluateRequest {
            policy_id: None,
            policy_name: None,
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };
        let _ = client.evaluate_all(agent_url, &eval_req).await;
    }

    // Timed phase
    let start = Instant::now();

    for req in requests.iter().skip(warmup as usize).take(volume as usize) {
        let eval_req = EvaluateRequest {
            policy_id: None,
            policy_name: None,
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };

        let req_start = Instant::now();
        match client.evaluate_all(agent_url, &eval_req).await {
            Ok(response) => {
                let elapsed = req_start.elapsed();
                histogram.record(elapsed.as_micros() as u64)?;
                successful += 1;

                if response.decision == "allow" {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("All policies evaluation error: {}", e);
                errors += 1;
            }
        }
    }

    let duration = start.elapsed();
    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "all".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

// ============================================================================
// Comparison Benchmark Types & Functions (Reaper vs eOPA)
// ============================================================================

/// Result from comparing Reaper vs eOPA for a single policy/volume combination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub policy_name: String,
    pub mode: String,
    pub volume: u32,
    pub reaper_uds: BenchmarkResult,
    pub reaper_tcp: BenchmarkResult,
    pub eopa: BenchmarkResult,
    pub speedup: SpeedupMetrics,
}

/// Speedup metrics comparing Reaper (UDS & TCP) to eOPA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedupMetrics {
    /// reaper_uds_rps / eopa_rps
    pub throughput_ratio: f64,
    /// reaper_tcp_rps / eopa_rps
    pub tcp_throughput_ratio: f64,
    /// reaper_uds_rps / reaper_tcp_rps
    pub uds_vs_tcp_ratio: f64,
    /// eopa_p50 / reaper_uds_p50
    pub p50_ratio: f64,
    /// eopa_p99 / reaper_uds_p99
    pub p99_ratio: f64,
    /// eopa_p99 / reaper_tcp_p99
    pub tcp_p99_ratio: f64,
    /// (eopa_p99 - reaper_uds_p99) / eopa_p99 * 100
    pub latency_reduction_percent: f64,
}

/// Full comparison report across multiple volumes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub id: String,
    pub timestamp: String,
    pub policy_name: String,
    pub comparisons: Vec<ComparisonResult>,
    pub summary: ComparisonSummary,
    pub system_info: SystemInfo,
}

/// Summary of a full comparison run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonSummary {
    pub avg_speedup: f64,
    pub avg_p99_ratio: f64,
    pub avg_latency_reduction_percent: f64,
    pub reaper_uds_peak_rps: f64,
    pub reaper_tcp_peak_rps: f64,
    pub eopa_peak_rps: f64,
}

impl SpeedupMetrics {
    fn compute(
        reaper_uds: &BenchmarkResult,
        reaper_tcp: &BenchmarkResult,
        eopa: &BenchmarkResult,
    ) -> Self {
        let throughput_ratio = if eopa.throughput_rps > 0.0 {
            reaper_uds.throughput_rps / eopa.throughput_rps
        } else {
            0.0
        };
        let tcp_throughput_ratio = if eopa.throughput_rps > 0.0 {
            reaper_tcp.throughput_rps / eopa.throughput_rps
        } else {
            0.0
        };
        let uds_vs_tcp_ratio = if reaper_tcp.throughput_rps > 0.0 {
            reaper_uds.throughput_rps / reaper_tcp.throughput_rps
        } else {
            0.0
        };
        let p50_ratio = if reaper_uds.latency.median_us > 0 {
            eopa.latency.median_us as f64 / reaper_uds.latency.median_us as f64
        } else {
            0.0
        };
        let p99_ratio = if reaper_uds.latency.p99_us > 0 {
            eopa.latency.p99_us as f64 / reaper_uds.latency.p99_us as f64
        } else {
            0.0
        };
        let tcp_p99_ratio = if reaper_tcp.latency.p99_us > 0 {
            eopa.latency.p99_us as f64 / reaper_tcp.latency.p99_us as f64
        } else {
            0.0
        };
        let latency_reduction_percent = if eopa.latency.p99_us > 0 {
            (eopa.latency.p99_us as f64 - reaper_uds.latency.p99_us as f64)
                / eopa.latency.p99_us as f64
                * 100.0
        } else {
            0.0
        };
        SpeedupMetrics {
            throughput_ratio,
            tcp_throughput_ratio,
            uds_vs_tcp_ratio,
            p50_ratio,
            p99_ratio,
            tcp_p99_ratio,
            latency_reduction_percent,
        }
    }
}

/// Run latency benchmark against eOPA (sequential requests).
pub async fn run_eopa_latency_benchmark(
    eopa: &EopaClient,
    policy_name: &str,
    volume: u32,
    warmup: u32,
) -> anyhow::Result<BenchmarkResult> {
    run_eopa_latency_benchmark_with(eopa, policy_name, volume, warmup, None).await
}

/// Run eOPA latency benchmark with optional custom request set.
pub async fn run_eopa_latency_benchmark_with(
    eopa: &EopaClient,
    policy_name: &str,
    volume: u32,
    warmup: u32,
    custom_requests: Option<&[TestRequest]>,
) -> anyhow::Result<BenchmarkResult> {
    let mapping = policy_mapping::get_mapping(policy_name)
        .ok_or_else(|| anyhow::anyhow!("No OPA mapping for policy '{}'", policy_name))?;

    let owned_requests;
    let requests: &[TestRequest] = if let Some(reqs) = custom_requests {
        reqs
    } else {
        owned_requests = scenarios::generate_requests((volume + warmup) as usize);
        &owned_requests
    };
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    // Warmup
    for req in requests.iter().take(warmup as usize) {
        let input = test_request_to_opa_input(req);
        let _ = eopa
            .evaluate(mapping.opa_package_path, mapping.opa_rule, &input)
            .await;
    }

    // Timed phase
    let start = Instant::now();

    for req in requests.iter().skip(warmup as usize).take(volume as usize) {
        let input = test_request_to_opa_input(req);

        match eopa
            .evaluate(mapping.opa_package_path, mapping.opa_rule, &input)
            .await
        {
            Ok(result) => {
                histogram.record(result.duration.as_micros() as u64)?;
                successful += 1;
                if result.allowed {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("eOPA request error: {}", e);
                errors += 1;
            }
        }
    }

    let duration = start.elapsed();
    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "latency".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run throughput benchmark against eOPA (concurrent individual requests via buffer_unordered).
///
/// eOPA has no batch endpoint, so we simulate throughput with concurrent individual requests.
pub async fn run_eopa_throughput_benchmark(
    eopa: &EopaClient,
    policy_name: &str,
    volume: u32,
    concurrency: u32,
) -> anyhow::Result<BenchmarkResult> {
    let mapping = policy_mapping::get_mapping(policy_name)
        .ok_or_else(|| anyhow::anyhow!("No OPA mapping for policy '{}'", policy_name))?;

    let requests = scenarios::generate_requests(volume as usize);
    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    let start = Instant::now();

    let results: Vec<_> = stream::iter(requests.iter().map(|req| {
        let input = test_request_to_opa_input(req);
        let pkg = mapping.opa_package_path;
        let rule = mapping.opa_rule;
        async move { eopa.evaluate(pkg, rule, &input).await }
    }))
    .buffer_unordered(concurrency as usize)
    .collect()
    .await;

    let duration = start.elapsed();

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    for result in results {
        match result {
            Ok(eval) => {
                let _ = histogram.record(eval.duration.as_micros() as u64);
                successful += 1;
                if eval.allowed {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("eOPA throughput error: {}", e);
                errors += 1;
            }
        }
    }

    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "throughput".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run concurrent benchmark for Reaper (individual requests via buffer_unordered).
///
/// Uses the same pattern as eOPA throughput benchmark for fair comparison:
/// each request is sent individually, with `concurrency` in-flight at once.
pub async fn run_concurrent_benchmark_with(
    client: &AgentClient,
    agent_url: &str,
    policy_name: &str,
    volume: u32,
    warmup: u32,
    concurrency: u32,
    custom_requests: Option<&[TestRequest]>,
) -> anyhow::Result<BenchmarkResult> {
    let owned_requests;
    let requests: &[TestRequest] = if let Some(reqs) = custom_requests {
        reqs
    } else {
        owned_requests = scenarios::generate_requests((volume + warmup) as usize);
        &owned_requests
    };

    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    // Warmup phase (sequential)
    for req in requests.iter().take(warmup as usize) {
        let policy_req = PolicyRequest {
            policy_name: policy_name.to_string(),
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        };
        let _ = client.evaluate(agent_url, &policy_req).await;
    }

    // Pre-build owned PolicyRequests for the timed phase
    let policy_reqs: Vec<PolicyRequest> = requests
        .iter()
        .skip(warmup as usize)
        .take(volume as usize)
        .map(|req| PolicyRequest {
            policy_name: policy_name.to_string(),
            principal: req.principal.clone(),
            action: req.action.clone(),
            resource: req.resource.clone(),
            context: req.context.clone(),
        })
        .collect();

    // Timed phase (concurrent individual requests)
    let start = Instant::now();
    let url = agent_url.to_string();

    let results: Vec<_> = stream::iter(policy_reqs.into_iter().map(|policy_req| {
        let url = url.clone();
        async move {
            let req_start = Instant::now();
            let result = client.evaluate(&url, &policy_req).await;
            (result, req_start.elapsed())
        }
    }))
    .buffer_unordered(concurrency as usize)
    .collect()
    .await;

    let duration = start.elapsed();

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    for (result, elapsed) in results {
        match result {
            Ok(response) => {
                let _ = histogram.record(elapsed.as_micros() as u64);
                successful += 1;
                if response.decision == "allow" {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("Concurrent request error: {}", e);
                errors += 1;
            }
        }
    }

    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "concurrent".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run concurrent benchmark for eOPA (individual requests via buffer_unordered).
///
/// Same `buffer_unordered` pattern as Reaper concurrent benchmark for fair comparison.
pub async fn run_eopa_concurrent_benchmark_with(
    eopa: &EopaClient,
    policy_name: &str,
    volume: u32,
    warmup: u32,
    concurrency: u32,
    custom_requests: Option<&[TestRequest]>,
) -> anyhow::Result<BenchmarkResult> {
    let mapping = policy_mapping::get_mapping(policy_name)
        .ok_or_else(|| anyhow::anyhow!("No OPA mapping for policy '{}'", policy_name))?;

    let owned_requests;
    let requests: &[TestRequest] = if let Some(reqs) = custom_requests {
        reqs
    } else {
        owned_requests = scenarios::generate_requests((volume + warmup) as usize);
        &owned_requests
    };

    let mut histogram: Histogram<u64> = Histogram::new_with_bounds(1, 1_000_000_000, 3)?;

    // Warmup phase (sequential)
    for req in requests.iter().take(warmup as usize) {
        let input = test_request_to_opa_input(req);
        let _ = eopa
            .evaluate(mapping.opa_package_path, mapping.opa_rule, &input)
            .await;
    }

    // Pre-build owned OPA inputs for the timed phase
    let opa_inputs: Vec<OpaInput> = requests
        .iter()
        .skip(warmup as usize)
        .take(volume as usize)
        .map(test_request_to_opa_input)
        .collect();

    // Timed phase (concurrent individual requests)
    let start = Instant::now();

    let results: Vec<_> = stream::iter(opa_inputs.into_iter().map(|input| {
        let pkg = mapping.opa_package_path;
        let rule = mapping.opa_rule;
        async move { eopa.evaluate(pkg, rule, &input).await }
    }))
    .buffer_unordered(concurrency as usize)
    .collect()
    .await;

    let duration = start.elapsed();

    let mut successful = 0u32;
    let mut errors = 0u32;
    let mut allowed = 0u32;
    let mut denied = 0u32;

    for result in results {
        match result {
            Ok(eval) => {
                let _ = histogram.record(eval.duration.as_micros() as u64);
                successful += 1;
                if eval.allowed {
                    allowed += 1;
                } else {
                    denied += 1;
                }
            }
            Err(e) => {
                debug!("eOPA concurrent error: {}", e);
                errors += 1;
            }
        }
    }

    let throughput = if duration.as_secs_f64() > 0.0 {
        volume as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        mode: "concurrent".to_string(),
        volume,
        total_requests: volume,
        successful,
        allowed,
        denied,
        errors,
        duration_ms: duration.as_millis() as u64,
        throughput_rps: throughput,
        latency: LatencyStats::from_histogram(&histogram),
    })
}

/// Run a full comparison benchmark: same scenario on Reaper (UDS + TCP) then eOPA, compute speedup.
#[allow(clippy::too_many_arguments)]
pub async fn run_comparison_benchmark(
    uds_client: &AgentClient,
    tcp_client: &AgentClient,
    agent_url: &str,
    eopa: &EopaClient,
    policy_name: &str,
    volumes: &[u32],
    warmup: u32,
    concurrency: u32,
) -> anyhow::Result<ComparisonReport> {
    let mode_label = if concurrency > 1 {
        "concurrent"
    } else {
        "latency"
    };
    info!(
        "Running comparison benchmark for '{}' at volumes {:?} (mode={}, concurrency={})",
        policy_name, volumes, mode_label, concurrency
    );

    let mut comparisons = Vec::new();

    for &volume in volumes {
        // Generate policy-specific scenarios for fair comparison
        let requests = comparison_scenarios::generate_comparison_requests(
            policy_name,
            (volume + warmup) as usize,
        );

        if concurrency > 1 {
            info!(
                "  Volume {} — Reaper UDS concurrent(c={})...",
                volume, concurrency
            );
            let uds_result = run_concurrent_benchmark_with(
                uds_client,
                agent_url,
                policy_name,
                volume,
                warmup,
                concurrency,
                Some(&requests),
            )
            .await?;

            info!(
                "  Volume {} — Reaper TCP concurrent(c={})...",
                volume, concurrency
            );
            let tcp_result = run_concurrent_benchmark_with(
                tcp_client,
                agent_url,
                policy_name,
                volume,
                warmup,
                concurrency,
                Some(&requests),
            )
            .await?;

            info!(
                "  Volume {} — eOPA concurrent(c={})...",
                volume, concurrency
            );
            let eopa_result = run_eopa_concurrent_benchmark_with(
                eopa,
                policy_name,
                volume,
                warmup,
                concurrency,
                Some(&requests),
            )
            .await?;

            let speedup = SpeedupMetrics::compute(&uds_result, &tcp_result, &eopa_result);

            info!(
                "  Volume {} — UDS p99={}us, TCP p99={}us, eOPA p99={}us, speedup={:.1}x",
                volume,
                uds_result.latency.p99_us,
                tcp_result.latency.p99_us,
                eopa_result.latency.p99_us,
                speedup.p99_ratio
            );

            comparisons.push(ComparisonResult {
                policy_name: policy_name.to_string(),
                mode: format!("concurrent(c={})", concurrency),
                volume,
                reaper_uds: uds_result,
                reaper_tcp: tcp_result,
                eopa: eopa_result,
                speedup,
            });
        } else {
            info!("  Volume {} — Reaper UDS latency...", volume);
            let uds_result = run_latency_benchmark_with(
                uds_client,
                agent_url,
                policy_name,
                volume,
                warmup,
                Some(&requests),
            )
            .await?;

            info!("  Volume {} — Reaper TCP latency...", volume);
            let tcp_result = run_latency_benchmark_with(
                tcp_client,
                agent_url,
                policy_name,
                volume,
                warmup,
                Some(&requests),
            )
            .await?;

            info!("  Volume {} — eOPA latency...", volume);
            let eopa_result =
                run_eopa_latency_benchmark_with(eopa, policy_name, volume, warmup, Some(&requests))
                    .await?;

            let speedup = SpeedupMetrics::compute(&uds_result, &tcp_result, &eopa_result);

            info!(
                "  Volume {} — UDS p99={}us, TCP p99={}us, eOPA p99={}us, speedup={:.1}x",
                volume,
                uds_result.latency.p99_us,
                tcp_result.latency.p99_us,
                eopa_result.latency.p99_us,
                speedup.p99_ratio
            );

            comparisons.push(ComparisonResult {
                policy_name: policy_name.to_string(),
                mode: "latency".to_string(),
                volume,
                reaper_uds: uds_result,
                reaper_tcp: tcp_result,
                eopa: eopa_result,
                speedup,
            });
        }
    }

    // Build summary
    let n = comparisons.len() as f64;
    let avg_speedup = if !comparisons.is_empty() {
        comparisons
            .iter()
            .map(|c| c.speedup.throughput_ratio)
            .sum::<f64>()
            / n
    } else {
        0.0
    };
    let avg_p99_ratio = if !comparisons.is_empty() {
        comparisons.iter().map(|c| c.speedup.p99_ratio).sum::<f64>() / n
    } else {
        0.0
    };
    let avg_latency_reduction = if !comparisons.is_empty() {
        comparisons
            .iter()
            .map(|c| c.speedup.latency_reduction_percent)
            .sum::<f64>()
            / n
    } else {
        0.0
    };
    let reaper_uds_peak_rps = comparisons
        .iter()
        .map(|c| c.reaper_uds.throughput_rps)
        .fold(0.0f64, f64::max);
    let reaper_tcp_peak_rps = comparisons
        .iter()
        .map(|c| c.reaper_tcp.throughput_rps)
        .fold(0.0f64, f64::max);
    let eopa_peak_rps = comparisons
        .iter()
        .map(|c| c.eopa.throughput_rps)
        .fold(0.0f64, f64::max);

    // System info
    let system_info = collect_system_info();

    Ok(ComparisonReport {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        policy_name: policy_name.to_string(),
        comparisons,
        summary: ComparisonSummary {
            avg_speedup,
            avg_p99_ratio,
            avg_latency_reduction_percent: avg_latency_reduction,
            reaper_uds_peak_rps,
            reaper_tcp_peak_rps,
            eopa_peak_rps,
        },
        system_info,
    })
}

/// Collect current system information for reports.
fn collect_system_info() -> SystemInfo {
    let mut sys = System::new_all();
    sys.refresh_all();
    let current_pid = sysinfo::get_current_pid().ok();
    let bench_memory_mb = current_pid
        .and_then(|pid| sys.process(pid))
        .map(|p| p.memory() / (1024 * 1024))
        .unwrap_or(0);

    SystemInfo {
        cpu_cores: num_cpus::get(),
        cpu_usage_percent: Some(sys.global_cpu_usage()),
        benchmark_service_memory_mb: bench_memory_mb,
        total_memory_mb: Some(sys.total_memory() / (1024 * 1024)),
        agent_memory_mb: None,
        tls_cipher: "N/A".to_string(),
        http_version: "HTTP/1.1".to_string(),
    }
}

/// Convert a test request into an OPA input structure.
fn test_request_to_opa_input(req: &scenarios::TestRequest) -> OpaInput {
    let role = req
        .context
        .as_ref()
        .and_then(|c| c.get("role"))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let department = req
        .context
        .as_ref()
        .and_then(|c| c.get("department"))
        .cloned();

    OpaInput {
        principal: OpaInputPrincipal {
            id: req.principal.clone(),
            role,
            department,
        },
        action: req.action.clone(),
        resource: req.resource.clone(),
    }
}

/// Compare individual vs package mode for a package
pub async fn compare_execution_modes(
    client: &AgentClient,
    agent_url: &str,
    package_name: &str,
    volume: u32,
) -> anyhow::Result<ModeComparisonResult> {
    info!("Comparing execution modes for package '{}'", package_name);

    // Get policies in the package
    let package_info = client.list_packages(agent_url).await?;
    let pkg = package_info
        .iter()
        .find(|p| p.name == package_name)
        .ok_or_else(|| anyhow::anyhow!("Package '{}' not found", package_name))?;

    // Run individual mode benchmark (one policy at a time)
    info!("Running individual mode benchmark...");
    let individual_result = if let Some(first_policy) = pkg.policy_names.first() {
        run_latency_benchmark(client, agent_url, first_policy, volume, 100).await?
    } else {
        return Err(anyhow::anyhow!("Package has no policies"));
    };

    // Run package mode benchmark (all policies together)
    info!("Running package mode benchmark...");
    let package_result =
        run_package_benchmark(client, agent_url, package_name, volume, 100).await?;

    // Calculate improvement
    let latency_reduction = if individual_result.latency.p99_us > 0 {
        ((individual_result.latency.p99_us as f64 - package_result.latency.p99_us as f64)
            / individual_result.latency.p99_us as f64)
            * 100.0
    } else {
        0.0
    };

    let throughput_increase = if individual_result.throughput_rps > 0.0 {
        ((package_result.throughput_rps - individual_result.throughput_rps)
            / individual_result.throughput_rps)
            * 100.0
    } else {
        0.0
    };

    Ok(ModeComparisonResult {
        package_name: package_name.to_string(),
        individual_mode: individual_result,
        package_mode: package_result,
        improvement: ModeImprovement {
            latency_reduction_percent: latency_reduction,
            throughput_increase_percent: throughput_increase,
        },
    })
}

// ============================================================================
// Multi-Policy Comparison Types & Functions (Run All)
// ============================================================================

/// Full multi-policy comparison report across all 12 policy types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPolicyComparisonReport {
    pub id: String,
    pub timestamp: String,
    pub policy_reports: Vec<ComparisonReport>,
    pub overall_summary: MultiPolicySummary,
    pub system_info: SystemInfo,
}

/// Aggregated summary across all policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPolicySummary {
    pub total_policies: usize,
    pub avg_speedup: f64,
    pub avg_p99_ratio: f64,
    pub avg_latency_reduction_percent: f64,
    pub reaper_uds_peak_rps: f64,
    pub reaper_tcp_peak_rps: f64,
    pub eopa_peak_rps: f64,
    pub per_policy: Vec<PolicySummaryEntry>,
}

/// Per-policy summary entry with fairness fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySummaryEntry {
    pub policy_name: String,
    pub reaper_uds_avg_p99_us: f64,
    pub reaper_tcp_avg_p99_us: f64,
    pub eopa_avg_p99_us: f64,
    pub reaper_uds_avg_rps: f64,
    pub reaper_tcp_avg_rps: f64,
    pub eopa_avg_rps: f64,
    pub avg_speedup: f64,
    // Fairness fields
    pub reaper_uds_allowed: u32,
    pub reaper_uds_denied: u32,
    pub reaper_tcp_allowed: u32,
    pub reaper_tcp_denied: u32,
    pub eopa_allowed: u32,
    pub eopa_denied: u32,
    pub decisions_match: bool,
}

/// Run comparison benchmarks for all mapped policies, aggregating into a multi-policy report.
pub async fn run_all_comparison_benchmarks(
    uds_client: &AgentClient,
    tcp_client: &AgentClient,
    agent_url: &str,
    eopa: &EopaClient,
    volumes: &[u32],
    warmup: u32,
    concurrency: u32,
) -> anyhow::Result<MultiPolicyComparisonReport> {
    let policy_names = policy_mapping::available_policy_names();
    info!(
        "Running comparison benchmarks for all {} policies at volumes {:?} (concurrency={})",
        policy_names.len(),
        volumes,
        concurrency
    );

    let mut policy_reports = Vec::new();

    for policy_name in &policy_names {
        info!("=== Policy: {} ===", policy_name);
        match run_comparison_benchmark(
            uds_client,
            tcp_client,
            agent_url,
            eopa,
            policy_name,
            volumes,
            warmup,
            concurrency,
        )
        .await
        {
            Ok(report) => {
                info!(
                    "  {} complete: avg speedup {:.1}x",
                    policy_name, report.summary.avg_speedup
                );
                policy_reports.push(report);
            }
            Err(e) => {
                warn!("  {} FAILED, skipping: {}", policy_name, e);
            }
        }
    }

    // Build per-policy summary entries
    let per_policy: Vec<PolicySummaryEntry> = policy_reports
        .iter()
        .map(|report| {
            let comps = &report.comparisons;
            let n = comps.len() as f64;

            let uds_allowed: u32 = comps.iter().map(|c| c.reaper_uds.allowed).sum();
            let uds_denied: u32 = comps.iter().map(|c| c.reaper_uds.denied).sum();
            let tcp_allowed: u32 = comps.iter().map(|c| c.reaper_tcp.allowed).sum();
            let tcp_denied: u32 = comps.iter().map(|c| c.reaper_tcp.denied).sum();
            let eopa_allowed: u32 = comps.iter().map(|c| c.eopa.allowed).sum();
            let eopa_denied: u32 = comps.iter().map(|c| c.eopa.denied).sum();

            // Decisions match if all three engines agree on allow/deny counts
            let decisions_match = uds_allowed == tcp_allowed
                && tcp_allowed == eopa_allowed
                && uds_denied == tcp_denied
                && tcp_denied == eopa_denied;

            PolicySummaryEntry {
                policy_name: report.policy_name.clone(),
                reaper_uds_avg_p99_us: if n > 0.0 {
                    comps
                        .iter()
                        .map(|c| c.reaper_uds.latency.p99_us as f64)
                        .sum::<f64>()
                        / n
                } else {
                    0.0
                },
                reaper_tcp_avg_p99_us: if n > 0.0 {
                    comps
                        .iter()
                        .map(|c| c.reaper_tcp.latency.p99_us as f64)
                        .sum::<f64>()
                        / n
                } else {
                    0.0
                },
                eopa_avg_p99_us: if n > 0.0 {
                    comps
                        .iter()
                        .map(|c| c.eopa.latency.p99_us as f64)
                        .sum::<f64>()
                        / n
                } else {
                    0.0
                },
                reaper_uds_avg_rps: if n > 0.0 {
                    comps
                        .iter()
                        .map(|c| c.reaper_uds.throughput_rps)
                        .sum::<f64>()
                        / n
                } else {
                    0.0
                },
                reaper_tcp_avg_rps: if n > 0.0 {
                    comps
                        .iter()
                        .map(|c| c.reaper_tcp.throughput_rps)
                        .sum::<f64>()
                        / n
                } else {
                    0.0
                },
                eopa_avg_rps: if n > 0.0 {
                    comps.iter().map(|c| c.eopa.throughput_rps).sum::<f64>() / n
                } else {
                    0.0
                },
                avg_speedup: report.summary.avg_speedup,
                reaper_uds_allowed: uds_allowed,
                reaper_uds_denied: uds_denied,
                reaper_tcp_allowed: tcp_allowed,
                reaper_tcp_denied: tcp_denied,
                eopa_allowed,
                eopa_denied,
                decisions_match,
            }
        })
        .collect();

    // Aggregate overall summary
    let total_policies = policy_reports.len();
    let overall_n = total_policies as f64;
    let avg_speedup = if overall_n > 0.0 {
        policy_reports
            .iter()
            .map(|r| r.summary.avg_speedup)
            .sum::<f64>()
            / overall_n
    } else {
        0.0
    };
    let avg_p99_ratio = if overall_n > 0.0 {
        policy_reports
            .iter()
            .map(|r| r.summary.avg_p99_ratio)
            .sum::<f64>()
            / overall_n
    } else {
        0.0
    };
    let avg_latency_reduction_percent = if overall_n > 0.0 {
        policy_reports
            .iter()
            .map(|r| r.summary.avg_latency_reduction_percent)
            .sum::<f64>()
            / overall_n
    } else {
        0.0
    };
    let reaper_uds_peak_rps = policy_reports
        .iter()
        .map(|r| r.summary.reaper_uds_peak_rps)
        .fold(0.0f64, f64::max);
    let reaper_tcp_peak_rps = policy_reports
        .iter()
        .map(|r| r.summary.reaper_tcp_peak_rps)
        .fold(0.0f64, f64::max);
    let eopa_peak_rps = policy_reports
        .iter()
        .map(|r| r.summary.eopa_peak_rps)
        .fold(0.0f64, f64::max);

    let system_info = collect_system_info();

    Ok(MultiPolicyComparisonReport {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        policy_reports,
        overall_summary: MultiPolicySummary {
            total_policies,
            avg_speedup,
            avg_p99_ratio,
            avg_latency_reduction_percent,
            reaper_uds_peak_rps,
            reaper_tcp_peak_rps,
            eopa_peak_rps,
            per_policy,
        },
        system_info,
    })
}
