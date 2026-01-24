//! Full Simulation Mode
//!
//! Runs a parameter matrix of benchmarks to find optimal tuning values.
//! Tests different combinations of volume, batch size, and concurrency
//! to determine the best configuration for the target environment.

use crate::benchmark::{run_latency_benchmark, run_throughput_benchmark, BenchmarkResult};
use crate::client::AgentClient;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use sysinfo::System;
use tracing::{info, warn};

/// Simulation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Policy name to benchmark
    #[serde(default = "default_policy_name")]
    pub policy_name: String,

    /// Volume levels to test
    #[serde(default = "default_volumes")]
    pub volumes: Vec<u32>,

    /// Batch sizes to test
    #[serde(default = "default_batch_sizes")]
    pub batch_sizes: Vec<u32>,

    /// Concurrency levels to test
    #[serde(default = "default_concurrency_levels")]
    pub concurrency_levels: Vec<u32>,

    /// Target p99 latency in microseconds (for recommendations)
    #[serde(default = "default_target_p99_us")]
    pub target_p99_us: u64,

    /// Minimum acceptable throughput (for recommendations)
    #[serde(default = "default_min_throughput")]
    pub min_throughput_rps: f64,

    /// Warmup requests before each test
    #[serde(default = "default_warmup")]
    pub warmup_requests: u32,
}

fn default_policy_name() -> String {
    "benchmark_rbac".to_string()
}

fn default_volumes() -> Vec<u32> {
    vec![100, 1000, 5000, 10000]
}

fn default_batch_sizes() -> Vec<u32> {
    vec![10, 50, 100, 200]
}

fn default_concurrency_levels() -> Vec<u32> {
    vec![1, 5, 10, 20]
}

fn default_target_p99_us() -> u64 {
    1000 // 1ms
}

fn default_min_throughput() -> f64 {
    10000.0 // 10K rps
}

fn default_warmup() -> u32 {
    50
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            policy_name: default_policy_name(),
            volumes: default_volumes(),
            batch_sizes: default_batch_sizes(),
            concurrency_levels: default_concurrency_levels(),
            target_p99_us: default_target_p99_us(),
            min_throughput_rps: default_min_throughput(),
            warmup_requests: default_warmup(),
        }
    }
}

/// Result of a single parameter combination test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterTestResult {
    pub batch_size: u32,
    pub concurrency: u32,
    pub volume: u32,
    pub throughput_rps: f64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub success_rate: f64,
    pub cpu_usage_percent: f32,
    pub memory_usage_mb: u64,
}

/// System resource snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub cpu_cores: usize,
    pub cpu_usage_percent: f32,
    pub total_memory_mb: u64,
    pub used_memory_mb: u64,
    pub available_memory_mb: u64,
}

/// Tuning recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningRecommendation {
    /// Recommended batch size
    pub batch_size: u32,
    /// Recommended concurrency level
    pub concurrency: u32,
    /// Expected throughput with these settings
    pub expected_throughput_rps: f64,
    /// Expected p99 latency with these settings
    pub expected_p99_us: u64,
    /// Confidence score (0-100)
    pub confidence: u8,
    /// Human-readable explanation
    pub explanation: String,
    /// Alternative configurations to consider
    pub alternatives: Vec<AlternativeConfig>,
}

/// Alternative configuration suggestion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeConfig {
    pub name: String,
    pub batch_size: u32,
    pub concurrency: u32,
    pub throughput_rps: f64,
    pub p99_us: u64,
    pub tradeoff: String,
}

/// Complete simulation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    /// Unique simulation ID
    pub id: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Configuration used
    pub config: SimulationConfig,
    /// Total duration of simulation
    pub duration_ms: u64,
    /// System resources at start
    pub resources_start: ResourceSnapshot,
    /// System resources at end
    pub resources_end: ResourceSnapshot,
    /// Results for each parameter combination
    pub parameter_results: Vec<ParameterTestResult>,
    /// Latency baseline (single-request mode)
    pub latency_baseline: Option<BenchmarkResult>,
    /// Tuning recommendation
    pub recommendation: TuningRecommendation,
    /// Summary statistics
    pub summary: SimulationSummary,
}

/// Summary of simulation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationSummary {
    pub total_combinations_tested: usize,
    pub total_requests_processed: u64,
    pub peak_throughput_rps: f64,
    pub best_p99_us: u64,
    pub optimal_found: bool,
    pub bottleneck_detected: Option<String>,
}

/// Get current system resource snapshot
pub fn get_resource_snapshot() -> ResourceSnapshot {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_usage = sys.global_cpu_usage();
    let total_memory = sys.total_memory() / (1024 * 1024); // Convert to MB
    let used_memory = sys.used_memory() / (1024 * 1024);
    let available_memory = sys.available_memory() / (1024 * 1024);

    ResourceSnapshot {
        cpu_cores: num_cpus::get(),
        cpu_usage_percent: cpu_usage,
        total_memory_mb: total_memory,
        used_memory_mb: used_memory,
        available_memory_mb: available_memory,
    }
}

/// Run full simulation with parameter matrix
pub async fn run_simulation(
    client: &AgentClient,
    agent_url: &str,
    config: SimulationConfig,
) -> anyhow::Result<SimulationResult> {
    info!("Starting full simulation");
    info!("  Policy: {}", config.policy_name);
    info!("  Volumes: {:?}", config.volumes);
    info!("  Batch sizes: {:?}", config.batch_sizes);
    info!("  Concurrency levels: {:?}", config.concurrency_levels);

    let start_time = Instant::now();
    let resources_start = get_resource_snapshot();

    // Get latency baseline first (sequential single-request mode)
    info!("Running latency baseline (1000 sequential requests)...");
    let latency_baseline = match run_latency_benchmark(
        client,
        agent_url,
        &config.policy_name,
        1000,
        config.warmup_requests,
    )
    .await
    {
        Ok(result) => {
            info!(
                "Baseline: p50={}µs, p99={}µs, throughput={:.0} rps",
                result.latency.median_us, result.latency.p99_us, result.throughput_rps
            );
            Some(result)
        }
        Err(e) => {
            warn!("Failed to get latency baseline: {}", e);
            None
        }
    };

    let mut parameter_results = Vec::new();

    // Test each parameter combination
    let total_combinations =
        config.batch_sizes.len() * config.concurrency_levels.len() * config.volumes.len();
    let mut combination_num = 0;

    for &volume in &config.volumes {
        for &batch_size in &config.batch_sizes {
            for &concurrency in &config.concurrency_levels {
                combination_num += 1;
                info!(
                    "[{}/{}] Testing: volume={}, batch_size={}, concurrency={}",
                    combination_num, total_combinations, volume, batch_size, concurrency
                );

                // Take resource snapshot before test (for future use tracking delta)
                let _pre_test = get_resource_snapshot();

                match run_throughput_benchmark(
                    client,
                    agent_url,
                    &config.policy_name,
                    volume,
                    batch_size,
                    concurrency,
                )
                .await
                {
                    Ok(result) => {
                        // Take resource snapshot after test
                        let post_test = get_resource_snapshot();

                        let success_rate = if result.total_requests > 0 {
                            result.successful as f64 / result.total_requests as f64 * 100.0
                        } else {
                            0.0
                        };

                        let test_result = ParameterTestResult {
                            batch_size,
                            concurrency,
                            volume,
                            throughput_rps: result.throughput_rps,
                            p50_us: result.latency.median_us,
                            p95_us: result.latency.p95_us,
                            p99_us: result.latency.p99_us,
                            success_rate,
                            cpu_usage_percent: post_test.cpu_usage_percent,
                            memory_usage_mb: post_test.used_memory_mb,
                        };

                        info!(
                            "  Result: {:.0} rps, p99={}µs, success={:.1}%, cpu={:.1}%",
                            test_result.throughput_rps,
                            test_result.p99_us,
                            test_result.success_rate,
                            test_result.cpu_usage_percent
                        );

                        parameter_results.push(test_result);
                    }
                    Err(e) => {
                        warn!(
                            "  Failed: volume={}, batch={}, concurrency={}: {}",
                            volume, batch_size, concurrency, e
                        );
                    }
                }

                // Small delay between tests to let system stabilize
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    let duration = start_time.elapsed();
    let resources_end = get_resource_snapshot();

    // Generate recommendation
    let recommendation = generate_recommendation(&config, &parameter_results);

    // Generate summary
    let summary = generate_summary(&parameter_results, &recommendation);

    Ok(SimulationResult {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        config,
        duration_ms: duration.as_millis() as u64,
        resources_start,
        resources_end,
        parameter_results,
        latency_baseline,
        recommendation,
        summary,
    })
}

/// Generate tuning recommendation from results
fn generate_recommendation(
    config: &SimulationConfig,
    results: &[ParameterTestResult],
) -> TuningRecommendation {
    if results.is_empty() {
        return TuningRecommendation {
            batch_size: 100,
            concurrency: 10,
            expected_throughput_rps: 0.0,
            expected_p99_us: 0,
            confidence: 0,
            explanation: "No valid test results to base recommendation on.".to_string(),
            alternatives: vec![],
        };
    }

    // Find configurations that meet latency requirements
    let mut valid_configs: Vec<_> = results
        .iter()
        .filter(|r| r.p99_us <= config.target_p99_us && r.success_rate >= 99.0)
        .collect();

    // Sort by throughput (highest first)
    valid_configs.sort_by(|a, b| b.throughput_rps.partial_cmp(&a.throughput_rps).unwrap());

    // Determine best config
    let (best_config, meets_target) = if valid_configs.is_empty() {
        // Find best latency config from all results
        let best_latency = results.iter().min_by_key(|r| r.p99_us);
        match best_latency {
            Some(config) => (config.clone(), false),
            None => {
                return TuningRecommendation {
                    batch_size: 100,
                    concurrency: 10,
                    expected_throughput_rps: 0.0,
                    expected_p99_us: 0,
                    confidence: 0,
                    explanation: "Unable to determine optimal configuration.".to_string(),
                    alternatives: vec![],
                };
            }
        }
    } else {
        (valid_configs[0].clone(), true)
    };

    // Calculate confidence based on consistency of results
    let confidence = calculate_confidence(results, &best_config);

    // Generate explanation
    let explanation = if meets_target {
        format!(
            "Recommended batch_size={} with concurrency={} achieves {:.0} req/s with p99={}µs, \
            meeting the target latency of {}µs. CPU usage is {:.1}%.",
            best_config.batch_size,
            best_config.concurrency,
            best_config.throughput_rps,
            best_config.p99_us,
            config.target_p99_us,
            best_config.cpu_usage_percent
        )
    } else {
        format!(
            "No configuration met the target p99 latency of {}µs. Best achieved: batch_size={}, \
            concurrency={} with {:.0} req/s and p99={}µs. Consider reducing load or optimizing policies.",
            config.target_p99_us,
            best_config.batch_size,
            best_config.concurrency,
            best_config.throughput_rps,
            best_config.p99_us
        )
    };

    // Find alternatives
    let alternatives = find_alternatives(results, &best_config, config);

    TuningRecommendation {
        batch_size: best_config.batch_size,
        concurrency: best_config.concurrency,
        expected_throughput_rps: best_config.throughput_rps,
        expected_p99_us: best_config.p99_us,
        confidence,
        explanation,
        alternatives,
    }
}

/// Calculate confidence score based on result consistency
fn calculate_confidence(results: &[ParameterTestResult], best: &ParameterTestResult) -> u8 {
    // Find similar configurations (same batch_size and concurrency, different volumes)
    let similar: Vec<_> = results
        .iter()
        .filter(|r| r.batch_size == best.batch_size && r.concurrency == best.concurrency)
        .collect();

    if similar.len() < 2 {
        return 60; // Low confidence with single data point
    }

    // Check throughput consistency
    let throughputs: Vec<f64> = similar.iter().map(|r| r.throughput_rps).collect();
    let avg_throughput: f64 = throughputs.iter().sum::<f64>() / throughputs.len() as f64;
    let variance: f64 = throughputs
        .iter()
        .map(|t| (t - avg_throughput).powi(2))
        .sum::<f64>()
        / throughputs.len() as f64;
    let stddev = variance.sqrt();
    let cv = if avg_throughput > 0.0 {
        stddev / avg_throughput
    } else {
        1.0
    };

    // Lower coefficient of variation = higher confidence
    let consistency_score = ((1.0 - cv.min(1.0)) * 40.0) as u8;

    // Check success rate
    let success_score = if best.success_rate >= 99.5 {
        30
    } else if best.success_rate >= 99.0 {
        20
    } else {
        10
    };

    // Base confidence
    let base = 30;

    (base + consistency_score + success_score).min(100)
}

/// Find alternative configurations
fn find_alternatives(
    results: &[ParameterTestResult],
    best: &ParameterTestResult,
    config: &SimulationConfig,
) -> Vec<AlternativeConfig> {
    let mut alternatives = Vec::new();

    // Find lowest latency config
    if let Some(lowest_latency) = results.iter().min_by_key(|r| r.p99_us) {
        if lowest_latency.batch_size != best.batch_size
            || lowest_latency.concurrency != best.concurrency
        {
            alternatives.push(AlternativeConfig {
                name: "Lowest Latency".to_string(),
                batch_size: lowest_latency.batch_size,
                concurrency: lowest_latency.concurrency,
                throughput_rps: lowest_latency.throughput_rps,
                p99_us: lowest_latency.p99_us,
                tradeoff: format!(
                    "Lower throughput ({:.0} vs {:.0} rps) but better latency",
                    lowest_latency.throughput_rps, best.throughput_rps
                ),
            });
        }
    }

    // Find highest throughput config
    if let Some(highest_throughput) = results
        .iter()
        .max_by(|a, b| a.throughput_rps.partial_cmp(&b.throughput_rps).unwrap())
    {
        if highest_throughput.batch_size != best.batch_size
            || highest_throughput.concurrency != best.concurrency
        {
            alternatives.push(AlternativeConfig {
                name: "Maximum Throughput".to_string(),
                batch_size: highest_throughput.batch_size,
                concurrency: highest_throughput.concurrency,
                throughput_rps: highest_throughput.throughput_rps,
                p99_us: highest_throughput.p99_us,
                tradeoff: format!(
                    "Higher latency ({}µs vs {}µs) but more throughput",
                    highest_throughput.p99_us, best.p99_us
                ),
            });
        }
    }

    // Find balanced config (good latency and throughput)
    let mut balanced: Vec<_> = results
        .iter()
        .filter(|r| r.p99_us <= config.target_p99_us * 2 && r.success_rate >= 99.0)
        .collect();
    balanced.sort_by(|a, b| {
        // Score = throughput / (latency / 1000)
        let score_a = a.throughput_rps / (a.p99_us as f64 / 1000.0);
        let score_b = b.throughput_rps / (b.p99_us as f64 / 1000.0);
        score_b.partial_cmp(&score_a).unwrap()
    });
    if let Some(balanced_config) = balanced.first() {
        if balanced_config.batch_size != best.batch_size
            || balanced_config.concurrency != best.concurrency
        {
            alternatives.push(AlternativeConfig {
                name: "Balanced".to_string(),
                batch_size: balanced_config.batch_size,
                concurrency: balanced_config.concurrency,
                throughput_rps: balanced_config.throughput_rps,
                p99_us: balanced_config.p99_us,
                tradeoff: "Good balance of throughput and latency".to_string(),
            });
        }
    }

    alternatives
}

/// Generate simulation summary
fn generate_summary(
    results: &[ParameterTestResult],
    recommendation: &TuningRecommendation,
) -> SimulationSummary {
    let peak_throughput = results
        .iter()
        .map(|r| r.throughput_rps)
        .fold(0.0f64, |a, b| a.max(b));

    let best_p99 = results.iter().map(|r| r.p99_us).min().unwrap_or(0);

    let total_requests: u64 = results.iter().map(|r| r.volume as u64).sum();

    // Detect bottlenecks
    let bottleneck = detect_bottleneck(results);

    SimulationSummary {
        total_combinations_tested: results.len(),
        total_requests_processed: total_requests,
        peak_throughput_rps: peak_throughput,
        best_p99_us: best_p99,
        optimal_found: recommendation.confidence >= 70,
        bottleneck_detected: bottleneck,
    }
}

/// Detect potential bottlenecks from results
fn detect_bottleneck(results: &[ParameterTestResult]) -> Option<String> {
    if results.is_empty() {
        return None;
    }

    // Check if high concurrency leads to worse results
    let high_concurrency: Vec<_> = results.iter().filter(|r| r.concurrency >= 10).collect();
    let low_concurrency: Vec<_> = results.iter().filter(|r| r.concurrency <= 5).collect();

    if !high_concurrency.is_empty() && !low_concurrency.is_empty() {
        let high_avg_latency: f64 =
            high_concurrency.iter().map(|r| r.p99_us as f64).sum::<f64>()
                / high_concurrency.len() as f64;
        let low_avg_latency: f64 = low_concurrency.iter().map(|r| r.p99_us as f64).sum::<f64>()
            / low_concurrency.len() as f64;

        if high_avg_latency > low_avg_latency * 2.0 {
            return Some(
                "High concurrency degrades latency. Consider connection pooling or reducing concurrent requests.".to_string()
            );
        }
    }

    // Check if large batches don't improve throughput
    let large_batch: Vec<_> = results.iter().filter(|r| r.batch_size >= 100).collect();
    let small_batch: Vec<_> = results.iter().filter(|r| r.batch_size <= 50).collect();

    if !large_batch.is_empty() && !small_batch.is_empty() {
        let large_avg_throughput: f64 =
            large_batch.iter().map(|r| r.throughput_rps).sum::<f64>() / large_batch.len() as f64;
        let small_avg_throughput: f64 =
            small_batch.iter().map(|r| r.throughput_rps).sum::<f64>() / small_batch.len() as f64;

        if large_avg_throughput < small_avg_throughput * 1.2 {
            return Some(
                "Large batch sizes show diminishing returns. Network or serialization may be the bottleneck.".to_string()
            );
        }
    }

    // Check CPU saturation
    let high_cpu: Vec<_> = results
        .iter()
        .filter(|r| r.cpu_usage_percent > 80.0)
        .collect();
    if !high_cpu.is_empty() {
        return Some(format!(
            "CPU utilization exceeded 80% in {} configurations. Consider scaling horizontally.",
            high_cpu.len()
        ));
    }

    None
}
