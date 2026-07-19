//! Capability verdict cache + verify rate limit (Plan 06 Phase D, R3-P2-2,
//! ADR-4).
//!
//! Before this, EVERY agentic request paid a full ed25519 `verify_raw`
//! (~30-50µs CPU) inline on the reactor — redundant for the common case
//! (the same derived capability presented on every call of a session), and
//! an authenticated asymmetric-cost DoS vector (a garbage signature costs
//! the agent a full verify but the sender nothing).
//!
//! ## What a cached verdict MEANS — and what it doesn't
//! An entry proves that a capability with **exactly these signed bytes**
//! (key = [`Capability::cache_digest`]: SHA-256 over the canonical message +
//! signature — see that method for why the plan's literal `(id, key_id,
//! signature, expiry)` key would be a signature bypass) passed the full
//! cryptographic verification **under revocation generation G** (the applied
//! revocation list's monotonic serial, folded into the key). It deliberately
//! proves nothing about:
//! - the CLOCK — the hit path re-runs [`Capability::check_validity_at`]
//!   (window + revocation, pure integer/set ops), so an entry can never
//!   outlive its capability's `expires_at`;
//! - the CURRENT revocation set — a new list bumps the generation, so every
//!   prior entry misses (ADR-4's scan-free invalidation), and belt-and-braces
//!   the hit path's `check_validity_at` also consults the live set.
//!
//! Only POSITIVE verdicts are cached: a failed verify stores nothing, so an
//! invalid capability can never be laundered into a pass, and garbage
//! signatures never hit — which is exactly why the [`VerifyRateLimiter`]
//! exists: it bounds full-verify *attempts* per principal per minute, the
//! cost the cache cannot amortize.
//!
//! [`Capability::cache_digest`]: reaper_core::capability::Capability::cache_digest
//! [`Capability::check_validity_at`]: reaper_core::capability::Capability::check_validity_at

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cache key: the capability's content digest + the revocation generation it
/// was verified under.
type Key = ([u8; 32], u64);

/// Bounded, sharded (DashMap) positive-verdict cache.
pub struct CapabilityVerdictCache {
    /// key -> unix second the verdict was cached at.
    entries: DashMap<Key, i64>,
    capacity: usize,
    ttl_secs: i64,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl CapabilityVerdictCache {
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            entries: DashMap::new(),
            capacity: capacity.max(1),
            ttl_secs: ttl_secs.max(1) as i64,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Is there a live positive verdict for `key`? Expired-TTL entries are
    /// removed on the way (lazy expiry).
    pub fn check(&self, key: &Key, now: i64) -> bool {
        if let Some(entry) = self.entries.get(key) {
            if now - *entry <= self.ttl_secs {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            drop(entry);
            self.entries
                .remove_if(key, |_, at| now - *at > self.ttl_secs);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        false
    }

    /// Record a positive verdict. At capacity: sweep expired entries first;
    /// if the cache is still full (all-live entries — an attacker cannot
    /// force this with garbage, only with VALID capabilities), drop
    /// arbitrary entries to make room. Eviction only ever costs a re-verify,
    /// never correctness.
    pub fn insert(&self, key: Key, now: i64) {
        if self.entries.len() >= self.capacity {
            self.entries.retain(|_, at| now - *at <= self.ttl_secs);
            // Still full: shed ~1/8 of the (all-live) entries arbitrarily.
            if self.entries.len() >= self.capacity {
                let shed = (self.capacity / 8).max(1);
                let victims: Vec<Key> = self.entries.iter().take(shed).map(|e| *e.key()).collect();
                for k in victims {
                    self.entries.remove(&k);
                }
            }
        }
        self.entries.insert(key, now);
    }

    pub fn hit_count(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }
    pub fn miss_count(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Companion to `len` (clippy len_without_is_empty); not otherwise called
    /// from the binary tree.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Per-principal fixed-window limiter on FULL capability verifications
/// (verdict-cache misses). Cache hits never consume budget — steady-state
/// agentic traffic is unlimited — so the limit only bites callers that force
/// real ed25519 work: garbage-signature floods, or pathological
/// capability-churn. Fail-closed direction: over the limit → the request is
/// DENIED (reason `capability_verify_rate_limited`), never verified anyway.
pub struct VerifyRateLimiter {
    /// principal -> (window index = unix_minute, count in window).
    windows: DashMap<String, (i64, u32)>,
    limit_per_min: u32,
}

impl VerifyRateLimiter {
    /// `limit_per_min` = 0 disables the limiter entirely.
    pub fn new(limit_per_min: u32) -> Self {
        Self {
            windows: DashMap::new(),
            limit_per_min,
        }
    }

    /// Try to consume one verify from `principal`'s current-minute budget.
    /// `true` = proceed with the verification; `false` = over limit, deny.
    pub fn admit(&self, principal: &str, now: i64) -> bool {
        if self.limit_per_min == 0 {
            return true;
        }
        let minute = now.div_euclid(60);
        let mut over = false;
        self.windows
            .entry(principal.to_string())
            .and_modify(|(win, count)| {
                if *win == minute {
                    if *count >= self.limit_per_min {
                        over = true;
                    } else {
                        *count += 1;
                    }
                } else {
                    *win = minute;
                    *count = 1;
                }
            })
            .or_insert((minute, 1));
        if over {
            return false;
        }
        // Bound the principal map itself (each entry is one authenticated
        // caller identity, but don't trust that to stay small): past-window
        // entries are dead weight — drop them once the map gets large.
        if self.windows.len() > 16_384 {
            self.windows.retain(|_, (win, _)| *win == minute);
        }
        true
    }
}

/// The gate's runtime state, built once from config and shared via
/// `AgentState`: the verdict cache, the verify rate limiter, and the
/// rollback flag. `capability_cache_enabled=false` restores the exact
/// pre-Phase-D behavior (inline verify, nothing cached).
pub struct CapabilityGateRuntime {
    pub cache: CapabilityVerdictCache,
    pub limiter: VerifyRateLimiter,
    pub cache_enabled: bool,
}

impl CapabilityGateRuntime {
    pub fn from_auth(auth: &reaper_core::config::AgentAuthSettings) -> Self {
        Self {
            cache: CapabilityVerdictCache::new(
                auth.capability_cache_capacity,
                auth.capability_cache_ttl_secs,
            ),
            limiter: VerifyRateLimiter::new(auth.capability_verify_limit_per_min),
            cache_enabled: auth.capability_cache_enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8, generation: u64) -> Key {
        ([b; 32], generation)
    }

    #[test]
    fn hit_only_within_ttl() {
        let cache = CapabilityVerdictCache::new(16, 10);
        cache.insert(key(1, 0), 1000);
        assert!(cache.check(&key(1, 0), 1005), "within ttl");
        assert!(!cache.check(&key(1, 0), 1011), "past ttl");
        assert!(!cache.check(&key(1, 0), 1005), "lazy-expired entry removed");
    }

    #[test]
    fn generation_partitions_verdicts() {
        let cache = CapabilityVerdictCache::new(16, 300);
        cache.insert(key(1, 0), 1000);
        assert!(cache.check(&key(1, 0), 1001));
        // Revocation-list bump: same content digest, new generation → miss.
        assert!(!cache.check(&key(1, 1), 1001));
    }

    #[test]
    fn capacity_is_bounded() {
        let cache = CapabilityVerdictCache::new(8, 300);
        for b in 0..64u8 {
            cache.insert(key(b, 0), 1000);
        }
        assert!(cache.len() <= 8 + 1, "len {} exceeds bound", cache.len());
    }

    #[test]
    fn rate_limiter_windows_and_disables() {
        let rl = VerifyRateLimiter::new(2);
        assert!(rl.admit("alice", 60));
        assert!(rl.admit("alice", 61));
        assert!(!rl.admit("alice", 62), "third verify in the minute denied");
        assert!(rl.admit("bob", 62), "per-principal budgets are independent");
        assert!(rl.admit("alice", 120), "next minute resets the window");

        let off = VerifyRateLimiter::new(0);
        for i in 0..1000 {
            assert!(off.admit("alice", i));
        }
    }
}
