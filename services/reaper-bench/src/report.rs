//! Benchmark report generation
//!
//! Generates JSON reports and HTML views for benchmark results.

use crate::benchmark::BenchmarkResult;
use serde::{Deserialize, Serialize};

/// Complete benchmark report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Unique report ID
    pub id: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Agent version (if available)
    pub agent_version: String,
    /// Policy name used for benchmark
    pub policy_name: String,
    /// Whether TLS was enabled
    pub tls_enabled: bool,
    /// Modes that were run
    pub modes_run: Vec<String>,
    /// Individual benchmark results
    pub results: Vec<BenchmarkResult>,
    /// Summary statistics
    pub summary: ReportSummary,
    /// System information
    pub system_info: SystemInfo,
}

/// Summary statistics across all benchmarks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    /// Total requests processed
    pub total_requests: u64,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// Latency mode summary (if run)
    pub latency_mode: Option<LatencySummary>,
    /// Throughput mode summary (if run)
    pub throughput_mode: Option<ThroughputSummary>,
    /// Whether a volume cap was detected
    pub volume_cap_detected: bool,
    /// Detected cap threshold (if any)
    pub cap_threshold_rps: Option<f64>,
    /// Performance recommendation
    pub recommendation: String,
}

/// Latency mode summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySummary {
    pub p50_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
}

/// Throughput mode summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputSummary {
    pub peak_rps: f64,
    pub sustained_rps: f64,
    pub batch_efficiency: f64,
}

/// System information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub cpu_cores: usize,
    pub cpu_usage_percent: Option<f32>,
    pub benchmark_service_memory_mb: u64,
    pub total_memory_mb: Option<u64>,
    pub agent_memory_mb: Option<u64>,
    pub tls_cipher: String,
    pub http_version: String,
}

/// Render benchmark results as HTML
pub fn render_results_html(report: &BenchmarkReport) -> String {
    let tls_status = if report.tls_enabled {
        "Enabled (mTLS)"
    } else {
        "Disabled"
    };

    let latency_html = if let Some(ref l) = report.summary.latency_mode {
        format!(
            r#"<div class="bg-blue-900 rounded-lg p-4 text-center">
                <div class="text-2xl font-bold">{}µs</div>
                <div class="text-sm text-gray-300">P99 Latency</div>
            </div>"#,
            l.p99_us
        )
    } else {
        String::new()
    };

    let throughput_html = if let Some(ref t) = report.summary.throughput_mode {
        format!(
            r#"<div class="bg-green-900 rounded-lg p-4 text-center">
                <div class="text-2xl font-bold">{:.0}</div>
                <div class="text-sm text-gray-300">Peak RPS</div>
            </div>"#,
            t.peak_rps
        )
    } else {
        String::new()
    };

    let results_rows: String = report
        .results
        .iter()
        .map(|r| {
            format!(
                r#"<tr class="border-b border-gray-700">
                    <td class="py-3"><span class="px-2 py-1 rounded text-xs {}">{}</span></td>
                    <td class="py-3">{}</td>
                    <td class="py-3">{}ms</td>
                    <td class="py-3">{:.0} req/s</td>
                    <td class="py-3">{}µs</td>
                    <td class="py-3">{}µs</td>
                    <td class="py-3">{}µs</td>
                    <td class="py-3"><span class="text-green-400">{}</span> / <span class="text-red-400">{}</span></td>
                </tr>"#,
                if r.mode == "latency" { "bg-blue-900" } else { "bg-green-900" },
                r.mode,
                r.volume,
                r.duration_ms,
                r.throughput_rps,
                r.latency.median_us,
                r.latency.p95_us,
                r.latency.p99_us,
                r.allowed,
                r.denied
            )
        })
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Benchmark Results - {}</title>
    <script src="https://cdn.tailwindcss.com"></script>
</head>
<body class="bg-gray-900 text-white p-8">
    <div class="max-w-4xl mx-auto">
        <h1 class="text-2xl font-bold mb-6">Reaper Benchmark Results</h1>

        <div class="bg-gray-800 rounded-lg p-6 mb-6">
            <div class="grid grid-cols-2 gap-4">
                <div><span class="text-gray-400">Timestamp:</span> {}</div>
                <div><span class="text-gray-400">Agent Version:</span> {}</div>
                <div><span class="text-gray-400">Policy:</span> {}</div>
                <div><span class="text-gray-400">TLS:</span> {}</div>
            </div>
        </div>

        <div class="grid grid-cols-4 gap-4 mb-6">
            {}
            {}
            <div class="bg-purple-900 rounded-lg p-4 text-center">
                <div class="text-2xl font-bold">{}</div>
                <div class="text-sm text-gray-300">Total Requests</div>
            </div>
            <div class="bg-orange-900 rounded-lg p-4 text-center">
                <div class="text-2xl font-bold">{}ms</div>
                <div class="text-sm text-gray-300">Total Duration</div>
            </div>
        </div>

        <div class="bg-gray-800 rounded-lg p-6 mb-6">
            <h2 class="text-xl font-semibold mb-4">Detailed Results</h2>
            <div class="overflow-x-auto">
                <table class="w-full text-left">
                    <thead>
                        <tr class="border-b border-gray-700">
                            <th class="pb-3">Mode</th>
                            <th class="pb-3">Volume</th>
                            <th class="pb-3">Duration</th>
                            <th class="pb-3">Throughput</th>
                            <th class="pb-3">P50</th>
                            <th class="pb-3">P95</th>
                            <th class="pb-3">P99</th>
                            <th class="pb-3">Allow/Deny</th>
                        </tr>
                    </thead>
                    <tbody>{}</tbody>
                </table>
            </div>
        </div>

        <div class="bg-gray-800 rounded-lg p-6">
            <h2 class="text-xl font-semibold mb-4">Recommendation</h2>
            <p class="text-gray-300">{}</p>
        </div>
    </div>
</body>
</html>"#,
        report.timestamp,
        report.timestamp,
        report.agent_version,
        report.policy_name,
        tls_status,
        latency_html,
        throughput_html,
        report.summary.total_requests,
        report.summary.total_duration_ms,
        results_rows,
        report.summary.recommendation
    )
}
