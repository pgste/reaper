//! Latency statistics using HDR Histogram
//!
//! Provides accurate percentile tracking for latency measurements.

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

/// Latency statistics from a benchmark run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    /// Minimum latency in microseconds
    pub min_us: u64,
    /// Maximum latency in microseconds
    pub max_us: u64,
    /// Mean latency in microseconds
    pub mean_us: f64,
    /// Median latency (p50) in microseconds
    pub median_us: u64,
    /// 90th percentile latency in microseconds
    pub p90_us: u64,
    /// 95th percentile latency in microseconds
    pub p95_us: u64,
    /// 99th percentile latency in microseconds
    pub p99_us: u64,
    /// 99.9th percentile latency in microseconds
    pub p999_us: u64,
    /// Standard deviation in microseconds
    pub stdev_us: f64,
}

impl LatencyStats {
    /// Create stats from an HDR histogram
    pub fn from_histogram(histogram: &Histogram<u64>) -> Self {
        if histogram.len() == 0 {
            return Self::empty();
        }

        Self {
            min_us: histogram.min(),
            max_us: histogram.max(),
            mean_us: histogram.mean(),
            median_us: histogram.value_at_percentile(50.0),
            p90_us: histogram.value_at_percentile(90.0),
            p95_us: histogram.value_at_percentile(95.0),
            p99_us: histogram.value_at_percentile(99.0),
            p999_us: histogram.value_at_percentile(99.9),
            stdev_us: histogram.stdev(),
        }
    }

    /// Create empty stats (no samples)
    pub fn empty() -> Self {
        Self {
            min_us: 0,
            max_us: 0,
            mean_us: 0.0,
            median_us: 0,
            p90_us: 0,
            p95_us: 0,
            p99_us: 0,
            p999_us: 0,
            stdev_us: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_from_histogram() {
        let mut histogram: Histogram<u64> = Histogram::new(3).unwrap();

        // Add some samples
        for i in 1..=100 {
            histogram.record(i * 10).unwrap(); // 10, 20, ..., 1000
        }

        let stats = LatencyStats::from_histogram(&histogram);

        assert_eq!(stats.min_us, 10);
        assert_eq!(stats.max_us, 1000);
        assert!(stats.mean_us > 400.0 && stats.mean_us < 600.0);
        assert!(stats.median_us >= 490 && stats.median_us <= 510);
    }

    #[test]
    fn test_empty_histogram() {
        let histogram: Histogram<u64> = Histogram::new(3).unwrap();
        let stats = LatencyStats::from_histogram(&histogram);

        assert_eq!(stats.min_us, 0);
        assert_eq!(stats.max_us, 0);
    }
}
