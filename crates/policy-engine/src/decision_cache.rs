//! LRU Decision Cache for Policy Evaluation
//!
//! Caches recent policy decisions to skip re-evaluation for identical requests.
//! Provides significant performance improvement for repeated authorization checks.
//!
//! # Performance Characteristics
//! - Cache hit: ~50-100ns (hash lookup + Arc clone)
//! - Cache miss: Full evaluation + ~100ns insert
//! - Memory: ~100 bytes per cached decision
//!
//! # Usage
//! ```rust,ignore
//! use policy_engine::decision_cache::DecisionCache;
//!
//! let cache = DecisionCache::new(10000); // 10K entries
//!
//! // Check cache first
//! if let Some(decision) = cache.get(&request) {
//!     return decision;
//! }
//!
//! // Evaluate and cache
//! let decision = evaluator.evaluate(&request)?;
//! cache.insert(&request, decision.clone());
//! ```

use crate::{PolicyAction, PolicyRequest};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cache key derived from PolicyRequest
///
/// Uses a compact hash-based key for fast lookups
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    /// Hash of the full request for fast comparison
    hash: u64,
    /// Principal ID (for collision detection)
    principal: Arc<str>,
    /// Action (for collision detection)
    action: Arc<str>,
    /// Resource (for collision detection)
    resource: Arc<str>,
}

impl CacheKey {
    fn from_request(request: &PolicyRequest) -> Self {
        use std::hash::BuildHasher;
        let hasher = rustc_hash::FxBuildHasher;

        let mut h = hasher.build_hasher();
        request.action.hash(&mut h);
        request.resource.hash(&mut h);
        if let Some(principal) = request.context.get("principal") {
            principal.hash(&mut h);
        }
        // Hash other context keys in sorted order for consistency
        let mut keys: Vec<_> = request.context.keys().collect();
        keys.sort();
        for key in keys {
            if key != "principal" {
                key.hash(&mut h);
                request.context.get(key).hash(&mut h);
            }
        }

        let principal = request
            .context
            .get("principal")
            .map(|s| Arc::from(s.as_str()))
            .unwrap_or_else(|| Arc::from(""));

        CacheKey {
            hash: h.finish(),
            principal,
            action: Arc::from(request.action.as_str()),
            resource: Arc::from(request.resource.as_str()),
        }
    }
}

/// Cached decision entry
struct CacheEntry {
    decision: PolicyAction,
    inserted_at: Instant,
    hits: AtomicU64,
}

/// LRU Decision Cache with TTL support
///
/// Thread-safe cache for policy decisions with:
/// - LRU eviction when capacity is reached
/// - Optional TTL for automatic expiration
/// - Hit statistics for monitoring
pub struct DecisionCache {
    /// Main cache storage
    cache: RwLock<FxHashMap<CacheKey, Arc<CacheEntry>>>,
    /// LRU order tracking
    lru_order: RwLock<VecDeque<CacheKey>>,
    /// Maximum cache capacity
    capacity: usize,
    /// Time-to-live for cached entries (None = no expiration)
    ttl: Option<Duration>,
    /// Statistics
    stats: CacheStats,
}

/// Cache statistics for monitoring
#[derive(Default)]
struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    expirations: AtomicU64,
}

/// Public cache statistics
#[derive(Debug, Clone)]
pub struct DecisionCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub size: usize,
    pub capacity: usize,
    pub hit_rate: f64,
}

impl DecisionCache {
    /// Create a new decision cache with the specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
            lru_order: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            ttl: None,
            stats: CacheStats::default(),
        }
    }

    /// Create a cache with TTL-based expiration
    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
            lru_order: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            ttl: Some(ttl),
            stats: CacheStats::default(),
        }
    }

    /// Get a cached decision for the request
    #[inline]
    pub fn get(&self, request: &PolicyRequest) -> Option<PolicyAction> {
        let key = CacheKey::from_request(request);

        // First, check if entry exists and get a clone of it
        let entry_opt = {
            let cache = self.cache.read();
            cache.get(&key).cloned()
        };

        if let Some(entry) = entry_opt {
            // Check TTL
            if let Some(ttl) = self.ttl {
                if entry.inserted_at.elapsed() > ttl {
                    self.remove(&key);
                    self.stats.expirations.fetch_add(1, Ordering::Relaxed);
                    self.stats.misses.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            }

            entry.hits.fetch_add(1, Ordering::Relaxed);
            self.stats.hits.fetch_add(1, Ordering::Relaxed);

            // Update LRU order (move to front)
            self.touch_lru(&key);

            return Some(entry.decision.clone());
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Insert a decision into the cache
    pub fn insert(&self, request: &PolicyRequest, decision: PolicyAction) {
        let key = CacheKey::from_request(request);

        let entry = Arc::new(CacheEntry {
            decision,
            inserted_at: Instant::now(),
            hits: AtomicU64::new(0),
        });

        let mut cache = self.cache.write();
        let mut lru = self.lru_order.write();

        // Check if we need to evict
        while cache.len() >= self.capacity && !lru.is_empty() {
            if let Some(old_key) = lru.pop_back() {
                cache.remove(&old_key);
                self.stats.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Insert new entry
        if cache.insert(key.clone(), entry).is_none() {
            lru.push_front(key);
        }
    }

    /// Remove an entry from the cache
    fn remove(&self, key: &CacheKey) {
        let mut cache = self.cache.write();
        cache.remove(key);

        let mut lru = self.lru_order.write();
        lru.retain(|k| k != key);
    }

    /// Update LRU order for a key (move to front)
    fn touch_lru(&self, key: &CacheKey) {
        let mut lru = self.lru_order.write();
        // Remove from current position
        lru.retain(|k| k != key);
        // Add to front
        lru.push_front(key.clone());
    }

    /// Clear all cached decisions
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        let mut lru = self.lru_order.write();
        cache.clear();
        lru.clear();
    }

    /// Invalidate cache entries for a specific principal
    pub fn invalidate_principal(&self, principal: &str) {
        let principal_arc: Arc<str> = Arc::from(principal);
        let mut cache = self.cache.write();
        let mut lru = self.lru_order.write();

        cache.retain(|k, _| k.principal != principal_arc);
        lru.retain(|k| k.principal != principal_arc);
    }

    /// Invalidate cache entries for a specific resource
    pub fn invalidate_resource(&self, resource: &str) {
        let resource_arc: Arc<str> = Arc::from(resource);
        let mut cache = self.cache.write();
        let mut lru = self.lru_order.write();

        cache.retain(|k, _| k.resource != resource_arc);
        lru.retain(|k| k.resource != resource_arc);
    }

    /// Get cache statistics
    pub fn stats(&self) -> DecisionCacheStats {
        let cache = self.cache.read();
        let hits = self.stats.hits.load(Ordering::Relaxed);
        let misses = self.stats.misses.load(Ordering::Relaxed);
        let total = hits + misses;

        DecisionCacheStats {
            hits,
            misses,
            evictions: self.stats.evictions.load(Ordering::Relaxed),
            expirations: self.stats.expirations.load(Ordering::Relaxed),
            size: cache.len(),
            capacity: self.capacity,
            hit_rate: if total > 0 {
                hits as f64 / total as f64
            } else {
                0.0
            },
        }
    }

    /// Get the current cache size
    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }
}

/// Cached evaluator wrapper that adds caching to any PolicyEvaluator
pub struct CachedEvaluator<E> {
    evaluator: E,
    cache: Arc<DecisionCache>,
}

impl<E: std::fmt::Debug> std::fmt::Debug for CachedEvaluator<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedEvaluator")
            .field("evaluator", &self.evaluator)
            .field("cache_size", &self.cache.len())
            .finish()
    }
}

impl<E> CachedEvaluator<E>
where
    E: crate::PolicyEvaluator,
{
    /// Create a cached evaluator with the specified cache
    pub fn new(evaluator: E, cache: Arc<DecisionCache>) -> Self {
        Self { evaluator, cache }
    }

    /// Create a cached evaluator with a new cache of the specified capacity
    pub fn with_capacity(evaluator: E, capacity: usize) -> Self {
        Self {
            evaluator,
            cache: Arc::new(DecisionCache::new(capacity)),
        }
    }

    /// Get the underlying cache
    pub fn cache(&self) -> &Arc<DecisionCache> {
        &self.cache
    }

    /// Get cache statistics
    pub fn stats(&self) -> DecisionCacheStats {
        self.cache.stats()
    }
}

impl<E> crate::PolicyEvaluator for CachedEvaluator<E>
where
    E: crate::PolicyEvaluator + std::fmt::Debug,
{
    fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyAction, reaper_core::ReaperError> {
        // Check cache first
        if let Some(decision) = self.cache.get(request) {
            return Ok(decision);
        }

        // Cache miss - evaluate
        let decision = self.evaluator.evaluate(request)?;

        // Cache the result
        self.cache.insert(request, decision.clone());

        Ok(decision)
    }

    fn validate(&self) -> Result<(), reaper_core::ReaperError> {
        self.evaluator.validate()
    }

    fn evaluator_type(&self) -> &str {
        "cached"
    }

    fn metadata(&self) -> Option<crate::EvaluatorMetadata> {
        self.evaluator.metadata()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn test_cache_hit() {
        let cache = DecisionCache::new(100);

        let request = make_request("alice", "read", "doc1");
        cache.insert(&request, PolicyAction::Allow);

        let result = cache.get(&request);
        assert!(matches!(result, Some(PolicyAction::Allow)));

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
    }

    #[test]
    fn test_cache_miss() {
        let cache = DecisionCache::new(100);

        let request = make_request("alice", "read", "doc1");
        let result = cache.get(&request);
        assert!(result.is_none());

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_eviction() {
        let cache = DecisionCache::new(3);

        // Fill cache
        for i in 0..3 {
            let request = make_request(&format!("user{}", i), "read", "doc");
            cache.insert(&request, PolicyAction::Allow);
        }
        assert_eq!(cache.len(), 3);

        // Trigger eviction
        let request = make_request("user3", "read", "doc");
        cache.insert(&request, PolicyAction::Allow);
        assert_eq!(cache.len(), 3);

        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
    }

    #[test]
    fn test_cache_ttl() {
        let cache = DecisionCache::with_ttl(100, Duration::from_millis(10));

        let request = make_request("alice", "read", "doc1");
        cache.insert(&request, PolicyAction::Allow);

        // Should hit immediately
        assert!(cache.get(&request).is_some());

        // Wait for TTL
        std::thread::sleep(Duration::from_millis(15));

        // Should miss after TTL
        assert!(cache.get(&request).is_none());

        let stats = cache.stats();
        assert_eq!(stats.expirations, 1);
    }

    #[test]
    fn test_invalidate_principal() {
        let cache = DecisionCache::new(100);

        // Add entries for multiple principals
        cache.insert(&make_request("alice", "read", "doc1"), PolicyAction::Allow);
        cache.insert(&make_request("alice", "write", "doc2"), PolicyAction::Allow);
        cache.insert(&make_request("bob", "read", "doc1"), PolicyAction::Deny);

        assert_eq!(cache.len(), 3);

        // Invalidate alice's entries
        cache.invalidate_principal("alice");

        assert_eq!(cache.len(), 1);
        assert!(cache.get(&make_request("bob", "read", "doc1")).is_some());
        assert!(cache.get(&make_request("alice", "read", "doc1")).is_none());
    }

    #[test]
    fn test_invalidate_resource() {
        let cache = DecisionCache::new(100);

        cache.insert(&make_request("alice", "read", "doc1"), PolicyAction::Allow);
        cache.insert(&make_request("bob", "read", "doc1"), PolicyAction::Deny);
        cache.insert(&make_request("alice", "read", "doc2"), PolicyAction::Allow);

        assert_eq!(cache.len(), 3);

        // Invalidate doc1 entries
        cache.invalidate_resource("doc1");

        assert_eq!(cache.len(), 1);
        assert!(cache.get(&make_request("alice", "read", "doc2")).is_some());
    }

    #[test]
    fn test_different_requests_different_keys() {
        let cache = DecisionCache::new(100);

        let req1 = make_request("alice", "read", "doc1");
        let req2 = make_request("alice", "write", "doc1");
        let req3 = make_request("alice", "read", "doc2");

        cache.insert(&req1, PolicyAction::Allow);
        cache.insert(&req2, PolicyAction::Deny);
        cache.insert(&req3, PolicyAction::Allow);

        assert!(matches!(cache.get(&req1), Some(PolicyAction::Allow)));
        assert!(matches!(cache.get(&req2), Some(PolicyAction::Deny)));
        assert!(matches!(cache.get(&req3), Some(PolicyAction::Allow)));
    }
}
