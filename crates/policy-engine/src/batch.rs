//! Parallel Batch Evaluation for Policy Requests
//!
//! Provides high-throughput evaluation of multiple policy requests in parallel
//! using rayon for work-stealing parallelism.
//!
//! # Performance Characteristics
//! - Linear scaling with CPU cores for independent requests
//! - Automatic work stealing for load balancing
//! - Optional decision cache integration for repeated requests
//! - Zero-copy request handling where possible
//!
//! # Usage
//! ```text
//! use policy_engine::batch::{BatchEvaluator, BatchResult};
//!
//! let evaluator = policy.build_ast_evaluator(store);
//! let batch = BatchEvaluator::new(evaluator);
//!
//! let requests = vec![request1, request2, request3, ...];
//! let results: Vec<BatchResult> = batch.evaluate_all(&requests);
//!
//! // With caching
//! let cached_batch = batch.with_cache(10000);
//! let results = cached_batch.evaluate_all(&requests);
//! ```

use crate::decision_cache::DecisionCache;
use crate::{PolicyAction, PolicyEvaluator, PolicyRequest};
use rayon::prelude::*;
use reaper_core::ReaperError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Result of a single evaluation in a batch
#[derive(Debug)]
pub struct BatchResult {
    /// Index of the request in the original batch
    pub index: usize,
    /// The evaluation result
    pub result: Result<PolicyAction, ReaperError>,
    /// Time taken for this evaluation
    pub duration: Duration,
    /// Whether this was a cache hit
    pub cache_hit: bool,
}

impl BatchResult {
    /// Check if evaluation succeeded
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// Check if evaluation failed
    pub fn is_err(&self) -> bool {
        self.result.is_err()
    }

    /// Get the decision if successful
    pub fn decision(&self) -> Option<&PolicyAction> {
        self.result.as_ref().ok()
    }
}

/// Statistics from a batch evaluation
#[derive(Debug, Clone)]
pub struct BatchStats {
    /// Total requests processed
    pub total_requests: usize,
    /// Successful evaluations
    pub successful: usize,
    /// Failed evaluations
    pub failed: usize,
    /// Allow decisions
    pub allowed: usize,
    /// Deny decisions
    pub denied: usize,
    /// Cache hits (if caching enabled)
    pub cache_hits: usize,
    /// Total batch duration
    pub total_duration: Duration,
    /// Mean latency per request
    pub mean_latency: Duration,
    /// P50 latency
    pub p50_latency: Duration,
    /// P95 latency
    pub p95_latency: Duration,
    /// P99 latency
    pub p99_latency: Duration,
    /// Throughput (requests/second)
    pub throughput: f64,
}

/// Parallel batch evaluator
///
/// Wraps a PolicyEvaluator to provide parallel batch evaluation
/// with optional caching support.
pub struct BatchEvaluator<E> {
    evaluator: Arc<E>,
    cache: Option<Arc<DecisionCache>>,
    /// Statistics counters
    stats: BatchStatsCounters,
}

#[derive(Default)]
struct BatchStatsCounters {
    total_requests: AtomicU64,
    successful: AtomicU64,
    failed: AtomicU64,
    allowed: AtomicU64,
    denied: AtomicU64,
    cache_hits: AtomicU64,
}

impl<E> BatchEvaluator<E>
where
    E: PolicyEvaluator + Sync + Send,
{
    /// Create a new batch evaluator
    pub fn new(evaluator: E) -> Self {
        Self {
            evaluator: Arc::new(evaluator),
            cache: None,
            stats: BatchStatsCounters::default(),
        }
    }

    /// Create a batch evaluator from an Arc'd evaluator
    pub fn from_arc(evaluator: Arc<E>) -> Self {
        Self {
            evaluator,
            cache: None,
            stats: BatchStatsCounters::default(),
        }
    }

    /// Add a decision cache with the specified capacity
    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(Arc::new(DecisionCache::new(capacity)));
        self
    }

    /// Add an existing decision cache
    pub fn with_shared_cache(mut self, cache: Arc<DecisionCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Evaluate a batch of requests in parallel
    ///
    /// Returns results in the same order as the input requests.
    pub fn evaluate_all(&self, requests: &[PolicyRequest]) -> Vec<BatchResult> {
        let start = Instant::now();

        let results: Vec<BatchResult> = requests
            .par_iter()
            .enumerate()
            .map(|(index, request)| self.evaluate_one(index, request))
            .collect();

        // Update aggregate stats
        for result in &results {
            self.stats.total_requests.fetch_add(1, Ordering::Relaxed);
            if result.is_ok() {
                self.stats.successful.fetch_add(1, Ordering::Relaxed);
                match result.decision() {
                    Some(PolicyAction::Allow) => {
                        self.stats.allowed.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(PolicyAction::Deny) => {
                        self.stats.denied.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            } else {
                self.stats.failed.fetch_add(1, Ordering::Relaxed);
            }
            if result.cache_hit {
                self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            }
        }

        let _total_duration = start.elapsed();
        results
    }

    /// Evaluate a batch and return statistics
    pub fn evaluate_with_stats(
        &self,
        requests: &[PolicyRequest],
    ) -> (Vec<BatchResult>, BatchStats) {
        let start = Instant::now();
        let results = self.evaluate_all(requests);
        let total_duration = start.elapsed();

        let stats = self.compute_stats(&results, total_duration);
        (results, stats)
    }

    /// Evaluate a single request (internal)
    fn evaluate_one(&self, index: usize, request: &PolicyRequest) -> BatchResult {
        let start = Instant::now();

        // A BatchEvaluator wraps a single evaluator, so all entries share one
        // cache scope. Capture the generation before evaluating so a concurrent
        // invalidation cannot cache a stale decision.
        let cache_gen = self.cache.as_ref().map(|c| c.generation()).unwrap_or(0);

        // Check cache first if available
        if let Some(ref cache) = self.cache {
            if let Some(decision) = cache.get(request, 0) {
                return BatchResult {
                    index,
                    result: Ok(decision),
                    duration: start.elapsed(),
                    cache_hit: true,
                };
            }
        }

        // Evaluate
        let result = self.evaluator.evaluate(request);

        // Cache result if successful
        if let (Some(ref cache), Ok(ref decision)) = (&self.cache, &result) {
            cache.insert(request, 0, decision.clone(), cache_gen);
        }

        BatchResult {
            index,
            result,
            duration: start.elapsed(),
            cache_hit: false,
        }
    }

    /// Compute statistics from batch results
    fn compute_stats(&self, results: &[BatchResult], total_duration: Duration) -> BatchStats {
        let mut latencies: Vec<Duration> = results.iter().map(|r| r.duration).collect();
        latencies.sort();

        let total = results.len();
        let successful = results.iter().filter(|r| r.is_ok()).count();
        let failed = total - successful;
        let allowed = results
            .iter()
            .filter(|r| matches!(r.decision(), Some(PolicyAction::Allow)))
            .count();
        let denied = results
            .iter()
            .filter(|r| matches!(r.decision(), Some(PolicyAction::Deny)))
            .count();
        let cache_hits = results.iter().filter(|r| r.cache_hit).count();

        let mean_latency = if total > 0 {
            latencies.iter().sum::<Duration>() / total as u32
        } else {
            Duration::ZERO
        };

        let p50_latency = if total > 0 {
            latencies[total / 2]
        } else {
            Duration::ZERO
        };

        let p95_latency = if total > 0 {
            latencies[(total as f64 * 0.95) as usize]
        } else {
            Duration::ZERO
        };

        let p99_latency = if total > 0 {
            latencies[((total as f64 * 0.99) as usize).min(total - 1)]
        } else {
            Duration::ZERO
        };

        let throughput = if total_duration.as_secs_f64() > 0.0 {
            total as f64 / total_duration.as_secs_f64()
        } else {
            0.0
        };

        BatchStats {
            total_requests: total,
            successful,
            failed,
            allowed,
            denied,
            cache_hits,
            total_duration,
            mean_latency,
            p50_latency,
            p95_latency,
            p99_latency,
            throughput,
        }
    }

    /// Get the underlying evaluator
    pub fn evaluator(&self) -> &E {
        &self.evaluator
    }

    /// Get the cache if present
    pub fn cache(&self) -> Option<&Arc<DecisionCache>> {
        self.cache.as_ref()
    }

    /// Get cache statistics if caching is enabled
    pub fn cache_stats(&self) -> Option<crate::decision_cache::DecisionCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    /// Reset internal statistics counters
    pub fn reset_stats(&self) {
        self.stats.total_requests.store(0, Ordering::Relaxed);
        self.stats.successful.store(0, Ordering::Relaxed);
        self.stats.failed.store(0, Ordering::Relaxed);
        self.stats.allowed.store(0, Ordering::Relaxed);
        self.stats.denied.store(0, Ordering::Relaxed);
        self.stats.cache_hits.store(0, Ordering::Relaxed);
    }
}

/// Builder for configuring batch evaluation
pub struct BatchEvaluatorBuilder<E> {
    evaluator: E,
    cache_capacity: Option<usize>,
    cache: Option<Arc<DecisionCache>>,
}

impl<E> BatchEvaluatorBuilder<E>
where
    E: PolicyEvaluator + Sync + Send,
{
    /// Create a new builder
    pub fn new(evaluator: E) -> Self {
        Self {
            evaluator,
            cache_capacity: None,
            cache: None,
        }
    }

    /// Enable caching with the specified capacity
    pub fn cache_capacity(mut self, capacity: usize) -> Self {
        self.cache_capacity = Some(capacity);
        self
    }

    /// Use an existing shared cache
    pub fn shared_cache(mut self, cache: Arc<DecisionCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Build the batch evaluator
    pub fn build(self) -> BatchEvaluator<E> {
        let mut batch = BatchEvaluator::new(self.evaluator);

        if let Some(cache) = self.cache {
            batch = batch.with_shared_cache(cache);
        } else if let Some(capacity) = self.cache_capacity {
            batch = batch.with_cache(capacity);
        }

        batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolicyAction;
    use std::collections::HashMap;

    // Mock evaluator for testing
    #[derive(Debug)]
    struct MockEvaluator {
        default_decision: PolicyAction,
    }

    impl PolicyEvaluator for MockEvaluator {
        fn evaluate(&self, _request: &PolicyRequest) -> Result<PolicyAction, ReaperError> {
            // Simulate some work
            std::thread::sleep(Duration::from_micros(10));
            Ok(self.default_decision.clone())
        }

        fn validate(&self) -> Result<(), ReaperError> {
            Ok(())
        }

        fn evaluator_type(&self) -> &str {
            "mock"
        }

        fn metadata(&self) -> Option<crate::EvaluatorMetadata> {
            Some(crate::EvaluatorMetadata {
                rule_count: 0,
                complexity: 0,
                extra: std::collections::HashMap::new(),
            })
        }
    }

    fn make_request(principal: &str, action: &str, resource: &str) -> PolicyRequest {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal.to_string());
        PolicyRequest {
            action: action.to_string(),
            resource: resource.to_string(),
            context,
        }
    }

    #[test]
    fn test_batch_evaluation() {
        let evaluator = MockEvaluator {
            default_decision: PolicyAction::Allow,
        };
        let batch = BatchEvaluator::new(evaluator);

        let requests: Vec<_> = (0..100)
            .map(|i| make_request(&format!("user{}", i), "read", "resource"))
            .collect();

        let results = batch.evaluate_all(&requests);

        assert_eq!(results.len(), 100);
        assert!(results.iter().all(|r| r.is_ok()));
        assert!(results
            .iter()
            .all(|r| matches!(r.decision(), Some(PolicyAction::Allow))));
    }

    #[test]
    fn test_batch_with_cache() {
        let evaluator = MockEvaluator {
            default_decision: PolicyAction::Allow,
        };
        let batch = BatchEvaluator::new(evaluator).with_cache(1000);

        // First batch - all cache misses
        let requests: Vec<_> = (0..10)
            .map(|i| make_request(&format!("user{}", i), "read", "resource"))
            .collect();

        let results1 = batch.evaluate_all(&requests);
        assert!(results1.iter().all(|r| !r.cache_hit));

        // Second batch with same requests - all cache hits
        let results2 = batch.evaluate_all(&requests);
        assert!(results2.iter().all(|r| r.cache_hit));
    }

    #[test]
    fn test_batch_stats() {
        let evaluator = MockEvaluator {
            default_decision: PolicyAction::Allow,
        };
        let batch = BatchEvaluator::new(evaluator);

        let requests: Vec<_> = (0..100)
            .map(|i| make_request(&format!("user{}", i), "read", "resource"))
            .collect();

        let (results, stats) = batch.evaluate_with_stats(&requests);

        assert_eq!(results.len(), 100);
        assert_eq!(stats.total_requests, 100);
        assert_eq!(stats.successful, 100);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.allowed, 100);
        assert!(stats.throughput > 0.0);
    }

    #[test]
    fn test_result_order_preserved() {
        let evaluator = MockEvaluator {
            default_decision: PolicyAction::Allow,
        };
        let batch = BatchEvaluator::new(evaluator);

        let requests: Vec<_> = (0..50)
            .map(|i| make_request(&format!("user{}", i), "read", "resource"))
            .collect();

        let results = batch.evaluate_all(&requests);

        // Verify order is preserved
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.index, i);
        }
    }
}
