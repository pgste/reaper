//! Decision Cache for Policy Evaluation
//!
//! Caches recent policy decisions to skip re-evaluation for identical requests.
//!
//! # When this cache helps (and when it does not)
//! A decision cache is only a win when evaluation is more expensive than a
//! lookup *and* requests repeat. For the compiled Reaper-DSL fast path
//! (sub-microsecond), re-evaluation is often cheaper than a cache probe, so the
//! cache mainly pays off for **expensive evaluators** (Cedar / complex ABAC,
//! 10-50µs) and hot-key workloads. Size/enable it accordingly.
//!
//! # Correctness model
//! Authorization decisions depend on the deployed policy *and* the entity data.
//! Both can change at runtime (hot-swap, data reload), so a stale cached
//! decision is a security defect. This cache is therefore:
//!
//! - **Epoch-invalidated**: every policy deploy or data change bumps a global
//!   generation counter via [`DecisionCache::invalidate`]. Entries carry the
//!   generation they were computed under; a mismatch is treated as a miss. This
//!   makes invalidation O(1) and race-free for evaluations in flight across a
//!   deploy (capture the generation *before* evaluating, pass it to `insert`).
//! - **Policy-scoped**: the cache key includes a caller-supplied `scope` hash
//!   (the resolved policy-id set) so decisions for different policies never
//!   collide on the same `(principal, action, resource)`.
//! - **Collision-safe**: keys are a 128-bit fingerprint (two independent
//!   hashes). The map is keyed by the first 64 bits; the second 64 bits are
//!   verified on read, so an unrelated request that hashes to the same slot is
//!   a miss rather than a wrong decision.
//!
//! # Performance characteristics
//! - Cache hit: one read-lock + two `u64` compares. No allocation, no write
//!   lock, no O(n) scan.
//! - Cache miss: hashing only.
//! - Insert: one write-lock; FIFO eviction when at capacity.

use crate::{PolicyAction, PolicyRequest};
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Salt mixed into the second fingerprint hash so it is independent of the
/// map-key hash. Any non-zero constant works.
const VERIFY_SALT: u64 = 0x9E37_79B9_7F4A_7C15;

/// Compute the 128-bit fingerprint `(key, verify)` for a request under a scope.
///
/// The `scope` distinguishes which policy (or policy set) the decision was
/// computed against, so identical requests evaluated against different policies
/// do not share a cache entry.
#[inline]
fn fingerprint(request: &PolicyRequest, scope: u64) -> (u64, u64) {
    #[inline]
    fn hash(request: &PolicyRequest, scope: u64, salt: u64) -> u64 {
        let mut h = rustc_hash::FxBuildHasher.build_hasher();
        salt.hash(&mut h);
        scope.hash(&mut h);
        request.action.hash(&mut h);
        request.resource.hash(&mut h);

        // Context must be hashed order-independently. Sort keys so the same
        // logical request always produces the same fingerprint.
        let mut keys: Vec<&String> = request.context.keys().collect();
        keys.sort_unstable();
        for key in keys {
            key.hash(&mut h);
            request.context.get(key).hash(&mut h);
        }
        h.finish()
    }

    (hash(request, scope, 0), hash(request, scope, VERIFY_SALT))
}

/// Cached decision entry.
struct CacheEntry {
    decision: PolicyAction,
    /// Second 64 bits of the fingerprint, verified on read to guard against
    /// map-key collisions between unrelated requests.
    verify: u64,
    /// Generation the decision was computed under. A mismatch with the current
    /// generation means a deploy/data-change has since invalidated it.
    generation: u64,
    inserted_at: Instant,
}

/// Decision cache with epoch invalidation, policy scoping, and TTL.
pub struct DecisionCache {
    cache: RwLock<FxHashMap<u64, CacheEntry>>,
    /// FIFO insertion order for eviction. Only touched on insert, never on read.
    order: Mutex<VecDeque<u64>>,
    capacity: usize,
    ttl: Option<Duration>,
    /// Global generation counter. Bumped on every invalidation.
    generation: AtomicU64,
    stats: CacheStats,
}

#[derive(Default)]
struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    expirations: AtomicU64,
    invalidations: AtomicU64,
}

/// Public cache statistics.
#[derive(Debug, Clone)]
pub struct DecisionCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub invalidations: u64,
    pub size: usize,
    pub capacity: usize,
    pub generation: u64,
    pub hit_rate: f64,
}

impl DecisionCache {
    /// Create a new decision cache with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
            order: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            ttl: None,
            generation: AtomicU64::new(0),
            stats: CacheStats::default(),
        }
    }

    /// Create a cache with TTL-based expiration.
    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
            order: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            ttl: Some(ttl),
            generation: AtomicU64::new(0),
            stats: CacheStats::default(),
        }
    }

    /// Current generation. Capture this *before* evaluating a request and pass
    /// it to [`DecisionCache::insert`] so a deploy that races with the
    /// evaluation cannot cache a stale decision under the new generation.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Invalidate every cached decision.
    ///
    /// Call on any policy deploy/update/delete or entity-data change. O(1) with
    /// respect to correctness (the generation bump makes all prior entries
    /// misses); also clears the map to reclaim memory since invalidations are
    /// rare relative to lookups.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        self.cache.write().clear();
        self.order.lock().clear();
        self.stats.invalidations.fetch_add(1, Ordering::Relaxed);
    }

    /// Look up a cached decision for `request` under `scope`.
    ///
    /// Read-only fast path: a shared lock plus two `u64` comparisons. Returns
    /// `None` (miss) if absent, stale (superseded generation), expired, or a
    /// fingerprint mismatch.
    #[inline]
    pub fn get(&self, request: &PolicyRequest, scope: u64) -> Option<PolicyAction> {
        let (key, verify) = fingerprint(request, scope);
        let current_gen = self.generation.load(Ordering::Acquire);

        let map = self.cache.read();
        if let Some(entry) = map.get(&key) {
            if entry.verify == verify && entry.generation == current_gen {
                if let Some(ttl) = self.ttl {
                    if entry.inserted_at.elapsed() > ttl {
                        self.stats.expirations.fetch_add(1, Ordering::Relaxed);
                        self.stats.misses.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                }
                let decision = entry.decision.clone();
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return Some(decision);
            }
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Insert a decision computed under generation `gen` (from
    /// [`DecisionCache::generation`], captured before evaluation).
    ///
    /// If `gen` is older than the current generation the decision is already
    /// stale (a deploy happened during evaluation) and is dropped.
    pub fn insert(&self, request: &PolicyRequest, scope: u64, decision: PolicyAction, gen: u64) {
        if self.capacity == 0 {
            return;
        }
        // Drop decisions that were invalidated while they were being computed.
        if gen != self.generation.load(Ordering::Acquire) {
            return;
        }

        let (key, verify) = fingerprint(request, scope);
        let entry = CacheEntry {
            decision,
            verify,
            generation: gen,
            inserted_at: Instant::now(),
        };

        let mut cache = self.cache.write();
        let mut order = self.order.lock();

        if !cache.contains_key(&key) {
            while cache.len() >= self.capacity {
                match order.pop_front() {
                    Some(old) => {
                        if cache.remove(&old).is_some() {
                            self.stats.evictions.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => break,
                }
            }
            order.push_back(key);
        }
        cache.insert(key, entry);
    }

    /// Clear all cached decisions without bumping the generation.
    pub fn clear(&self) {
        self.cache.write().clear();
        self.order.lock().clear();
    }

    /// Get cache statistics.
    pub fn stats(&self) -> DecisionCacheStats {
        let size = self.cache.read().len();
        let hits = self.stats.hits.load(Ordering::Relaxed);
        let misses = self.stats.misses.load(Ordering::Relaxed);
        let total = hits + misses;

        DecisionCacheStats {
            hits,
            misses,
            evictions: self.stats.evictions.load(Ordering::Relaxed),
            expirations: self.stats.expirations.load(Ordering::Relaxed),
            invalidations: self.stats.invalidations.load(Ordering::Relaxed),
            size,
            capacity: self.capacity,
            generation: self.generation.load(Ordering::Relaxed),
            hit_rate: if total > 0 {
                hits as f64 / total as f64
            } else {
                0.0
            },
        }
    }

    /// Current number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }
}

/// Compute a scope hash from a set of policy IDs.
///
/// Order-independent so the same policy set always maps to the same scope,
/// regardless of iteration order.
pub fn scope_hash<I, T>(policy_ids: I) -> u64
where
    I: IntoIterator<Item = T>,
    T: Hash,
{
    // XOR of per-id hashes: commutative, so ordering does not matter.
    let mut acc: u64 = 0;
    for id in policy_ids {
        let mut h = rustc_hash::FxBuildHasher.build_hasher();
        id.hash(&mut h);
        acc ^= h.finish().rotate_left(1);
    }
    acc
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
    fn test_cache_hit_and_miss() {
        let cache = DecisionCache::new(100);
        let request = make_request("alice", "read", "doc1");

        assert!(cache.get(&request, 0).is_none());

        let gen = cache.generation();
        cache.insert(&request, 0, PolicyAction::Allow, gen);

        assert!(matches!(cache.get(&request, 0), Some(PolicyAction::Allow)));
    }

    #[test]
    fn test_scope_isolates_policies() {
        // Same request, different policy scopes -> independent decisions.
        let cache = DecisionCache::new(100);
        let request = make_request("alice", "read", "doc1");
        let gen = cache.generation();

        let scope_a = scope_hash([1u32]);
        let scope_b = scope_hash([2u32]);

        cache.insert(&request, scope_a, PolicyAction::Allow, gen);
        cache.insert(&request, scope_b, PolicyAction::Deny, gen);

        assert!(matches!(
            cache.get(&request, scope_a),
            Some(PolicyAction::Allow)
        ));
        assert!(matches!(
            cache.get(&request, scope_b),
            Some(PolicyAction::Deny)
        ));
    }

    #[test]
    fn test_invalidate_makes_entries_miss() {
        let cache = DecisionCache::new(100);
        let request = make_request("alice", "read", "doc1");
        let gen = cache.generation();
        cache.insert(&request, 0, PolicyAction::Allow, gen);
        assert!(cache.get(&request, 0).is_some());

        cache.invalidate();

        // Old decision is gone after invalidation (e.g. policy hot-swap).
        assert!(cache.get(&request, 0).is_none());
    }

    #[test]
    fn test_stale_insert_after_invalidation_is_dropped() {
        // Simulate an evaluation in flight across a deploy: generation captured
        // before eval, deploy bumps generation, then the stale insert lands.
        let cache = DecisionCache::new(100);
        let request = make_request("alice", "read", "doc1");

        let gen = cache.generation(); // captured before evaluation
        cache.invalidate(); // deploy happens mid-evaluation
        cache.insert(&request, 0, PolicyAction::Allow, gen); // stale insert

        assert!(cache.get(&request, 0).is_none());
    }

    #[test]
    fn test_eviction() {
        let cache = DecisionCache::new(3);
        let gen = cache.generation();
        for i in 0..3 {
            cache.insert(&make_request(&format!("u{i}"), "read", "d"), 0, PolicyAction::Allow, gen);
        }
        assert_eq!(cache.len(), 3);

        cache.insert(&make_request("u3", "read", "d"), 0, PolicyAction::Allow, gen);
        assert_eq!(cache.len(), 3);
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn test_ttl_expiry() {
        let cache = DecisionCache::with_ttl(100, Duration::from_millis(10));
        let request = make_request("alice", "read", "doc1");
        let gen = cache.generation();
        cache.insert(&request, 0, PolicyAction::Allow, gen);
        assert!(cache.get(&request, 0).is_some());

        std::thread::sleep(Duration::from_millis(15));
        assert!(cache.get(&request, 0).is_none());
        assert_eq!(cache.stats().expirations, 1);
    }

    #[test]
    fn test_different_requests_distinct() {
        let cache = DecisionCache::new(100);
        let gen = cache.generation();
        let r1 = make_request("alice", "read", "doc1");
        let r2 = make_request("alice", "write", "doc1");
        let r3 = make_request("alice", "read", "doc2");

        cache.insert(&r1, 0, PolicyAction::Allow, gen);
        cache.insert(&r2, 0, PolicyAction::Deny, gen);
        cache.insert(&r3, 0, PolicyAction::Allow, gen);

        assert!(matches!(cache.get(&r1, 0), Some(PolicyAction::Allow)));
        assert!(matches!(cache.get(&r2, 0), Some(PolicyAction::Deny)));
        assert!(matches!(cache.get(&r3, 0), Some(PolicyAction::Allow)));
    }
}
