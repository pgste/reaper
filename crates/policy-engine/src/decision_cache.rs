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
//! # Concurrency model (sharded)
//! The cache is split into power-of-two shards, each with its own lock, so
//! inserts on different shards never contend. This matters on **low-hit-rate**
//! workloads, where every request takes a write lock: a single global lock
//! serializes all cores (Plan 08 Phase C); with shards, writers proceed in
//! parallel. Shard count scales with capacity (up to [`MAX_SHARDS`]) so small
//! caches keep exact capacity semantics with one shard.
//!
//! Capacity is enforced per shard (`capacity / shards`, rounded up), so the
//! effective total can exceed the configured capacity by at most `shards - 1`
//! entries.
//!
//! # Performance characteristics
//! - Cache hit: one shard read-lock + two `u64` compares. No allocation, no
//!   write lock, no O(n) scan.
//! - Cache miss: hashing only. The fingerprint folds context entries with a
//!   commutative combiner — no sorted key `Vec` allocation per probe.
//! - Insert: one shard write-lock; FIFO eviction within the shard at capacity.

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

/// Upper bound on shard count. 64 shards is enough that 32+ writer threads
/// rarely collide, while keeping the per-shard maps large enough to stay
/// cache-friendly.
const MAX_SHARDS: usize = 64;

/// Minimum entries per shard before the cache splits into more shards. Keeps
/// small caches single-sharded (exact capacity/FIFO semantics) and prevents
/// degenerate 1-entry shards.
const MIN_SHARD_CAPACITY: usize = 64;

/// Order-independent fold of the request context under `salt`.
///
/// Each `(key, value)` pair is hashed to one `u64` and the pair hashes are
/// combined with commutative operations (wrapping add + rotated xor), so the
/// result does not depend on `HashMap` iteration order and needs no sorted
/// key `Vec` per probe. Distinct salts produce independent folds, preserving
/// the 128-bit two-independent-hashes collision model.
#[inline]
fn context_fold(request: &PolicyRequest, salt: u64) -> u64 {
    let mut sum: u64 = 0;
    let mut xor: u64 = 0;
    for (key, value) in &request.context {
        let mut eh = rustc_hash::FxBuildHasher.build_hasher();
        salt.hash(&mut eh);
        key.hash(&mut eh);
        value.hash(&mut eh);
        let pair = eh.finish();
        sum = sum.wrapping_add(pair);
        xor ^= pair.rotate_left(32);
    }
    sum ^ xor
}

/// Order-independent fold of the context-provenance map (F1 taint), same
/// commutative construction as [`context_fold`]. `None` (taint mode off) and
/// `Some(empty)` fold differently on purpose: they have different eval
/// semantics (off = platform for all keys; on+unlabeled = llm floor).
#[inline]
fn provenance_fold(request: &PolicyRequest, salt: u64) -> u64 {
    let Some(map) = &request.context_provenance else {
        return 0;
    };
    let mut sum: u64 = 0x9e37_79b9_7f4a_7c15; // non-zero base ≠ the None fold
    let mut xor: u64 = 0;
    for (key, level) in map {
        let mut eh = rustc_hash::FxBuildHasher.build_hasher();
        salt.hash(&mut eh);
        key.hash(&mut eh);
        (*level as u8).hash(&mut eh);
        let pair = eh.finish();
        sum = sum.wrapping_add(pair);
        xor ^= pair.rotate_left(32);
    }
    sum ^ xor
}

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
        context_fold(request, salt).hash(&mut h);
        // F1 agentic authz: the actor and per-key provenance influence the
        // decision (actor.* conditions, taint predicates), so they MUST be in
        // the fingerprint — otherwise two requests differing only in actor or
        // taint would share a cached decision (cross-actor cache poisoning).
        request.actor.hash(&mut h);
        provenance_fold(request, salt).hash(&mut h);
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

/// One independently locked slice of the cache.
struct Shard {
    map: RwLock<FxHashMap<u64, CacheEntry>>,
    /// FIFO insertion order for eviction. Only touched on insert, never on read.
    order: Mutex<VecDeque<u64>>,
}

/// Decision cache with epoch invalidation, policy scoping, TTL, and N-way
/// sharding (see the module docs for the concurrency model).
pub struct DecisionCache {
    shards: Box<[Shard]>,
    /// `shards.len() - 1`; shard count is a power of two so selection is a mask.
    shard_mask: usize,
    /// Per-shard entry cap (`capacity / shards`, rounded up).
    shard_capacity: usize,
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

/// Shard count for `capacity`: one shard per `MIN_SHARD_CAPACITY` entries,
/// rounded up to a power of two, clamped to `[1, MAX_SHARDS]`.
fn shard_count(capacity: usize) -> usize {
    (capacity / MIN_SHARD_CAPACITY)
        .next_power_of_two()
        .clamp(1, MAX_SHARDS)
}

impl DecisionCache {
    /// Create a new decision cache with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self::build(capacity, None)
    }

    /// Create a cache with TTL-based expiration.
    pub fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        Self::build(capacity, Some(ttl))
    }

    fn build(capacity: usize, ttl: Option<Duration>) -> Self {
        let shards = shard_count(capacity);
        let shard_capacity = capacity.div_ceil(shards);
        let shards: Box<[Shard]> = (0..shards)
            .map(|_| Shard {
                map: RwLock::new(FxHashMap::default()),
                order: Mutex::new(VecDeque::with_capacity(shard_capacity)),
            })
            .collect();
        Self {
            shard_mask: shards.len() - 1,
            shards,
            shard_capacity,
            capacity,
            ttl,
            generation: AtomicU64::new(0),
            stats: CacheStats::default(),
        }
    }

    /// The shard owning fingerprint `key`. Folds the high bits in so shard
    /// selection isn't just the map hash's low bits.
    #[inline]
    fn shard(&self, key: u64) -> &Shard {
        &self.shards[((key >> 32) ^ key) as usize & self.shard_mask]
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
    /// misses); also clears the shards to reclaim memory since invalidations
    /// are rare relative to lookups.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        for shard in &self.shards {
            shard.map.write().clear();
            shard.order.lock().clear();
        }
        self.stats.invalidations.fetch_add(1, Ordering::Relaxed);
    }

    /// Look up a cached decision for `request` under `scope`.
    ///
    /// Read-only fast path: one shard's shared lock plus two `u64` comparisons.
    /// Returns `None` (miss) if absent, stale (superseded generation), expired,
    /// or a fingerprint mismatch.
    #[inline]
    pub fn get(&self, request: &PolicyRequest, scope: u64) -> Option<PolicyAction> {
        let (key, verify) = fingerprint(request, scope);
        let current_gen = self.generation.load(Ordering::Acquire);

        let map = self.shard(key).map.read();
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

        let shard = self.shard(key);
        let mut cache = shard.map.write();
        let mut order = shard.order.lock();

        if !cache.contains_key(&key) {
            while cache.len() >= self.shard_capacity {
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
        for shard in &self.shards {
            shard.map.write().clear();
            shard.order.lock().clear();
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> DecisionCacheStats {
        let size = self.len();
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

    /// Current number of cached entries (summed across shards).
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.map.read().len()).sum()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|s| s.map.read().is_empty())
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
        acc ^= rustc_hash::FxBuildHasher.hash_one(&id).rotate_left(1);
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

            ..Default::default()
        }
    }

    #[test]
    fn actor_isolates_cache_entries() {
        // Two requests identical except for the actor must NOT share a
        // decision — actor.* conditions make the actor decision-relevant.
        let cache = DecisionCache::new(100);
        let gen = cache.generation();

        let mut with_actor = make_request("alice", "deploy", "svc");
        with_actor.actor = Some("agent-ci".to_string());
        cache.insert(&with_actor, 0, PolicyAction::Allow, gen);

        let mut other_actor = with_actor.clone();
        other_actor.actor = Some("agent-rogue".to_string());
        assert!(cache.get(&other_actor, 0).is_none(), "different actor");

        let actorless = make_request("alice", "deploy", "svc");
        assert!(cache.get(&actorless, 0).is_none(), "no actor");
        assert!(matches!(
            cache.get(&with_actor, 0),
            Some(PolicyAction::Allow)
        ));
    }

    #[test]
    fn provenance_isolates_cache_entries() {
        // Taint labels are decision-relevant (taint::trusted gates); a request
        // with LLM-tainted context must not hit a platform-trusted entry, and
        // taint-mode-off (None) must not collide with an empty map (their eval
        // semantics differ).
        let cache = DecisionCache::new(100);
        let gen = cache.generation();

        let mut platform = make_request("alice", "act", "r");
        platform.context_provenance = Some(
            [("approved".to_string(), crate::TrustLevel::Platform)]
                .into_iter()
                .collect(),
        );
        cache.insert(&platform, 0, PolicyAction::Allow, gen);

        let mut llm = platform.clone();
        llm.context_provenance = Some(
            [("approved".to_string(), crate::TrustLevel::Llm)]
                .into_iter()
                .collect(),
        );
        assert!(cache.get(&llm, 0).is_none(), "different trust level");

        let taint_off = make_request("alice", "act", "r");
        assert!(cache.get(&taint_off, 0).is_none(), "taint off ≠ labeled");

        let mut empty_map = make_request("alice", "act", "r");
        empty_map.context_provenance = Some(HashMap::new());
        assert!(cache.get(&empty_map, 0).is_none(), "taint on+empty ≠ off");
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
            cache.insert(
                &make_request(&format!("u{i}"), "read", "d"),
                0,
                PolicyAction::Allow,
                gen,
            );
        }
        assert_eq!(cache.len(), 3);

        cache.insert(
            &make_request("u3", "read", "d"),
            0,
            PolicyAction::Allow,
            gen,
        );
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

    #[test]
    fn test_fingerprint_is_context_order_independent() {
        // Two maps with the same entries inserted in different orders (and
        // therefore different iteration orders) must produce the same
        // fingerprint — the fold is commutative, no sorting involved.
        let mut ctx_a = HashMap::new();
        ctx_a.insert("principal".to_string(), "alice".to_string());
        ctx_a.insert("dept".to_string(), "eng".to_string());
        ctx_a.insert("region".to_string(), "eu".to_string());

        let mut ctx_b = HashMap::new();
        ctx_b.insert("region".to_string(), "eu".to_string());
        ctx_b.insert("dept".to_string(), "eng".to_string());
        ctx_b.insert("principal".to_string(), "alice".to_string());

        let r_a = PolicyRequest {
            action: "read".to_string(),
            resource: "doc1".to_string(),
            context: ctx_a,

            ..Default::default()
        };
        let r_b = PolicyRequest {
            action: "read".to_string(),
            resource: "doc1".to_string(),
            context: ctx_b,

            ..Default::default()
        };

        assert_eq!(fingerprint(&r_a, 7), fingerprint(&r_b, 7));

        // And a differing value must change the fingerprint.
        let mut ctx_c = r_b.context.clone();
        ctx_c.insert("dept".to_string(), "sales".to_string());
        let r_c = PolicyRequest {
            action: "read".to_string(),
            resource: "doc1".to_string(),
            context: ctx_c,

            ..Default::default()
        };
        assert_ne!(fingerprint(&r_a, 7), fingerprint(&r_c, 7));
    }

    #[test]
    fn test_sharded_capacity_bound() {
        // Large enough to shard (1024 /64 = 16 shards): total size stays at
        // the configured capacity (per-shard cap divides evenly here) and
        // eviction kicks in across shards.
        let capacity = 1024;
        let cache = DecisionCache::new(capacity);
        assert!(cache.shards.len() > 1);
        let gen = cache.generation();
        for i in 0..5000 {
            cache.insert(
                &make_request(&format!("user-{i}"), "read", &format!("doc-{i}")),
                0,
                PolicyAction::Allow,
                gen,
            );
        }
        assert!(cache.len() <= capacity);
        assert!(cache.stats().evictions > 0);

        cache.invalidate();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_concurrent_insert_and_get_across_shards() {
        use std::sync::Arc;

        // Keep the key count well under capacity: eviction is per-shard, so a
        // near-full cache can evict from hot shards before the global total
        // reaches capacity. 2000 keys across 64 shards (cap 64 each) leaves
        // every shard far from its limit, so all inserts must survive.
        let cache = Arc::new(DecisionCache::new(4096));
        let gen = cache.generation();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let cache = Arc::clone(&cache);
                std::thread::spawn(move || {
                    for i in 0..250 {
                        let req = make_request(&format!("u{t}-{i}"), "read", "doc");
                        cache.insert(&req, 0, PolicyAction::Allow, gen);
                        assert!(matches!(cache.get(&req, 0), Some(PolicyAction::Allow)));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(cache.len(), 8 * 250);
    }
}
