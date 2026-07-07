//! String Interning for Memory Efficiency
//!
//! String interning stores each unique string once and references it by a small ID.
//! This dramatically reduces memory usage when the same strings appear repeatedly
//! (e.g., roles, departments, resource types).
//!
//! # Memory Savings
//! - "admin" repeated 10,000 times: 50KB → 5 bytes (string) + 40KB (u32 IDs) = 45KB savings
//! - Typical policy dataset: 60-80% memory reduction

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Interned string ID (4 bytes instead of 8-24 bytes for String)
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct InternedString(u32);

impl InternedString {
    /// Get the raw ID
    pub fn id(&self) -> u32 {
        self.0
    }

    /// Create from raw ID (unsafe - caller must ensure ID is valid)
    pub fn from_id(id: u32) -> Self {
        Self(id)
    }
}

/// A string a compiled policy (or any caller of [`StringInterner::intern`])
/// references and that must therefore never be evicted. Stored in a slot's
/// `refs` field; [`StringInterner::release`] is a no-op on a pinned slot.
const PIN: u32 = u32::MAX;

/// Interner slot: the assigned id plus a reference count. Storing the count
/// alongside the id in the `string_to_id` map means a string's count and its
/// very existence are mutated under the *same* DashMap shard lock — so
/// counting, releasing, and eviction of a given string are serialized without a
/// global lock, and a concurrent `intern_counted` can never revive a string
/// that `release` is simultaneously evicting.
#[derive(Debug, Clone, Copy)]
struct Slot {
    id: InternedString,
    /// `PIN` = pinned (never evicted). Otherwise the number of live
    /// `intern_counted` references; the string is evicted when it reaches 0.
    refs: u32,
}

/// High-performance string interner
///
/// Uses DashMap for lock-free concurrent access.
///
/// # Reference counting & eviction
/// Strings interned via [`Self::intern`] are **pinned** — never evicted — which
/// is what makes eviction safe for everything else: a value a compiled policy
/// holds can never be reclaimed out from under it. Strings interned via
/// [`Self::intern_counted`] carry a reference count and are **evicted when the
/// last reference is [`released`](Self::release)**. The DataStore counts the
/// strings each entity owns and releases them when the entity is removed, so a
/// long-lived read-replica applying a high-cardinality delta stream no longer
/// grows the interner without bound (the append-only behaviour it had before).
///
/// # Performance
/// - Interning: ~100 ns (first time), ~50 ns (cached)
/// - Lookup / resolve: ~20 ns (unaffected by refcounting — read-only)
/// - Memory: 4 bytes per reference vs 24 bytes for String
#[derive(Debug, Clone)]
pub struct StringInterner {
    /// String -> slot (id + refcount)
    string_to_id: Arc<DashMap<Arc<str>, Slot>>,
    /// ID -> String mapping (for resolve); entries removed on eviction
    id_to_string: Arc<DashMap<InternedString, Arc<str>>>,
    /// Next available ID. Monotonic — ids are never recycled, so a stale id can
    /// only ever resolve to `None`, never alias a different string.
    next_id: Arc<AtomicU32>,
}

impl StringInterner {
    /// Create a new string interner
    pub fn new() -> Self {
        Self {
            string_to_id: Arc::new(DashMap::new()),
            id_to_string: Arc::new(DashMap::new()),
            next_id: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Intern a string and **pin** it (never evicted), returning its ID.
    ///
    /// This is the default intern used by policy compilation, entity types, and
    /// any caller that does not manage a reference lifecycle. Pinning is the
    /// safety foundation for eviction: a string a compiled policy references can
    /// never be evicted by [`Self::release`], so ids held by compiled policies
    /// stay valid for the life of the store.
    ///
    /// # Performance
    /// - Already pinned: ~20 ns (read-only fast path, no allocation)
    /// - First call / upgrade from counted: ~100 ns (allocation + insert)
    pub fn intern(&self, s: &str) -> InternedString {
        // Fast path: already interned AND already pinned — nothing to change.
        if let Some(slot) = self.string_to_id.get(s) {
            let slot = *slot.value();
            if slot.refs == PIN {
                return slot.id;
            }
        }

        // Slow path: create the slot pinned, or upgrade an existing counted slot
        // to pinned. entry() holds the shard lock so this is race-free against a
        // concurrent release/eviction of the same string.
        let arc_str: Arc<str> = Arc::from(s);
        let mut entry = self.string_to_id.entry(arc_str.clone()).or_insert_with(|| {
            let new_id = InternedString(self.next_id.fetch_add(1, Ordering::Relaxed));
            self.id_to_string.insert(new_id, arc_str.clone());
            Slot {
                id: new_id,
                refs: PIN,
            }
        });
        entry.refs = PIN;
        entry.id
    }

    /// Intern a string and **increment its reference count**, returning its ID.
    ///
    /// Used by the DataStore for the strings an entity owns (its id, type,
    /// attribute keys, string attribute values, parent). Each such reference is
    /// balanced by a [`Self::release`] when the entity is removed; when the last
    /// counted reference is released the string is evicted. Interning a string
    /// that is already pinned leaves it pinned (a no-op on the count).
    pub fn intern_counted(&self, s: &str) -> InternedString {
        // Fast path: already pinned — counting is a no-op, avoid the write.
        if let Some(slot) = self.string_to_id.get(s) {
            let slot = *slot.value();
            if slot.refs == PIN {
                return slot.id;
            }
        }

        let arc_str: Arc<str> = Arc::from(s);
        let mut entry = self.string_to_id.entry(arc_str.clone()).or_insert_with(|| {
            let new_id = InternedString(self.next_id.fetch_add(1, Ordering::Relaxed));
            self.id_to_string.insert(new_id, arc_str.clone());
            Slot {
                id: new_id,
                refs: 0,
            }
        });
        if entry.refs != PIN {
            entry.refs = entry.refs.saturating_add(1);
        }
        entry.id
    }

    /// Release one counted reference to `id`, evicting the string when the last
    /// reference is dropped. No-op on a pinned string or an already-evicted id.
    ///
    /// Safety: the caller must hold a reference that was added via
    /// [`Self::intern_counted`] (the DataStore guarantees this — it releases
    /// exactly the strings an entity owns, once, when that entity is removed).
    /// Because a live reference keeps the count ≥ 1, the string cannot have been
    /// evicted while the caller held it, so `id_to_string[id]` is still valid
    /// here; the `string_to_id` shard lock then serializes this decrement with
    /// any concurrent intern/release of the same string.
    pub fn release(&self, id: InternedString) {
        use dashmap::mapref::entry::Entry;

        let arc = match self.id_to_string.get(&id) {
            Some(s) => s.value().clone(),
            None => return, // already evicted or never interned
        };

        if let Entry::Occupied(mut occ) = self.string_to_id.entry(arc) {
            let slot = occ.get_mut();
            if slot.refs == PIN {
                return; // pinned — never evict
            }
            slot.refs = slot.refs.saturating_sub(1);
            if slot.refs == 0 {
                let evicted = slot.id;
                occ.remove();
                self.id_to_string.remove(&evicted);
            }
        }
    }

    /// Drop every non-pinned string, resetting counted memory. Used by
    /// `DataStore::clear` — after a clear no entity references any counted
    /// string, so they are all safe to evict; pinned strings (policy literals,
    /// types) survive.
    pub fn reset_counted(&self) {
        let mut evicted = Vec::new();
        self.string_to_id.retain(|_s, slot| {
            if slot.refs == PIN {
                true
            } else {
                evicted.push(slot.id);
                false
            }
        });
        for id in evicted {
            self.id_to_string.remove(&id);
        }
    }

    /// Look up an already-interned string's ID **without inserting** it.
    ///
    /// Unlike [`Self::intern`], this never allocates or mutates the interner, so
    /// it's safe for read-only paths (e.g. the decision-log "explain" snapshot)
    /// that must not pollute the interner with transient request strings.
    pub fn lookup(&self, s: &str) -> Option<InternedString> {
        self.string_to_id.get(s).map(|entry| entry.value().id)
    }

    /// Get the string for an interned ID
    ///
    /// # Performance
    /// - ~20 ns (hash lookup)
    pub fn resolve(&self, id: InternedString) -> Option<Arc<str>> {
        self.id_to_string
            .get(&id)
            .map(|entry| entry.value().clone())
    }

    /// Resolve an interned ID and run `f` on the borrowed `&str`, without
    /// cloning the `Arc<str>`.
    ///
    /// Use this on the hot path when the resolved string is only needed
    /// transiently (e.g. a substring/prefix comparison): it avoids the atomic
    /// refcount inc/dec of `resolve()`. The DashMap shard read-lock is held for
    /// the (short) duration of `f`, so keep `f` cheap and non-blocking.
    #[inline]
    pub fn with_resolved<R>(&self, id: InternedString, f: impl FnOnce(&str) -> R) -> Option<R> {
        self.id_to_string.get(&id).map(|entry| f(entry.value()))
    }

    /// Get the string for an interned ID as &str
    ///
    /// This is less efficient than resolve() as it requires cloning the Arc,
    /// but provides a convenient &str interface.
    pub fn resolve_str(&self, id: InternedString) -> Option<String> {
        self.resolve(id).map(|arc| arc.to_string())
    }

    /// Get statistics about the interner
    pub fn stats(&self) -> InternerStats {
        InternerStats {
            unique_strings: self.string_to_id.len(),
            next_id: self.next_id.load(Ordering::Relaxed),
            estimated_memory_bytes: self.estimate_memory(),
        }
    }

    /// Estimate memory usage (rough approximation)
    fn estimate_memory(&self) -> usize {
        let num_strings = self.string_to_id.len();
        let avg_string_size = 16; // Rough average

        // String storage + two hashmaps + Arc overhead
        (num_strings * avg_string_size) + (num_strings * 32 * 2)
    }

    /// Pre-intern common strings for better performance
    ///
    /// Call this with common values like roles, departments, etc.
    /// at startup to avoid interning overhead during policy evaluation.
    pub fn prewarm(&self, strings: &[&str]) {
        for s in strings {
            self.intern(s);
        }
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about string interning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternerStats {
    /// Number of unique strings stored
    pub unique_strings: usize,
    /// Next ID to be assigned
    pub next_id: u32,
    /// Estimated memory usage in bytes
    pub estimated_memory_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_and_resolve() {
        let interner = StringInterner::new();

        let id1 = interner.intern("hello");
        let id2 = interner.intern("world");
        let id3 = interner.intern("hello"); // Duplicate

        assert_eq!(id1, id3); // Same ID for same string
        assert_ne!(id1, id2); // Different IDs for different strings

        assert_eq!(interner.resolve_str(id1).unwrap(), "hello");
        assert_eq!(interner.resolve_str(id2).unwrap(), "world");
    }

    #[test]
    fn test_concurrent_interning() {
        use std::thread;

        let interner = StringInterner::new();
        let mut handles = vec![];

        // Spawn 10 threads all interning the same strings
        for _ in 0..10 {
            let interner_clone = interner.clone();
            let handle = thread::spawn(move || {
                let id1 = interner_clone.intern("admin");
                let id2 = interner_clone.intern("user");
                (id1, id2)
            });
            handles.push(handle);
        }

        // All threads should get the same IDs
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let (first_admin, first_user) = results[0];

        for (admin_id, user_id) in results {
            assert_eq!(admin_id, first_admin);
            assert_eq!(user_id, first_user);
        }
    }

    #[test]
    fn test_prewarm() {
        let interner = StringInterner::new();

        interner.prewarm(&["admin", "user", "manager", "viewer"]);

        let stats = interner.stats();
        assert_eq!(stats.unique_strings, 4);
    }

    #[test]
    fn test_memory_efficiency() {
        let interner = StringInterner::new();

        // Intern "admin" 1000 times
        for _ in 0..1000 {
            interner.intern("admin");
        }

        let stats = interner.stats();
        assert_eq!(stats.unique_strings, 1); // Only stored once
    }

    #[test]
    fn test_counted_string_evicted_at_zero() {
        let interner = StringInterner::new();
        let id = interner.intern_counted("ephemeral");
        assert_eq!(interner.resolve_str(id).as_deref(), Some("ephemeral"));
        assert_eq!(interner.stats().unique_strings, 1);

        interner.release(id);
        // Last reference gone -> evicted.
        assert_eq!(interner.resolve(id), None);
        assert_eq!(interner.stats().unique_strings, 0);
        assert_eq!(interner.lookup("ephemeral"), None);
    }

    #[test]
    fn test_counted_refcount_balances() {
        let interner = StringInterner::new();
        // Three live references to the same string.
        let a = interner.intern_counted("shared");
        let b = interner.intern_counted("shared");
        let c = interner.intern_counted("shared");
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(interner.stats().unique_strings, 1);

        interner.release(a);
        interner.release(b);
        // Still one live reference -> not evicted.
        assert_eq!(interner.resolve_str(c).as_deref(), Some("shared"));
        assert_eq!(interner.stats().unique_strings, 1);

        interner.release(c);
        assert_eq!(interner.resolve(c), None);
        assert_eq!(interner.stats().unique_strings, 0);
    }

    #[test]
    fn test_pinned_string_never_evicted() {
        let interner = StringInterner::new();
        // Pinned by default intern (as a policy literal would be)...
        let id = interner.intern("admin");
        // ...and also referenced as counted data.
        let id2 = interner.intern_counted("admin");
        assert_eq!(id, id2);

        // Releasing the counted reference must NOT evict a pinned string — this
        // is the invariant that keeps compiled-policy literal ids valid.
        interner.release(id2);
        interner.release(id2); // extra releases are safe no-ops
        assert_eq!(interner.resolve_str(id).as_deref(), Some("admin"));
        assert_eq!(interner.stats().unique_strings, 1);
    }

    #[test]
    fn test_intern_after_count_pins() {
        let interner = StringInterner::new();
        let id = interner.intern_counted("val"); // counted, refs = 1
        let pinned = interner.intern("val"); // upgrades to pinned
        assert_eq!(id, pinned);
        // Now releasing the original counted ref must not evict it.
        interner.release(id);
        assert_eq!(interner.resolve_str(id).as_deref(), Some("val"));
    }

    #[test]
    fn test_reset_counted_keeps_pinned() {
        let interner = StringInterner::new();
        let pinned = interner.intern("policy_literal");
        let counted = interner.intern_counted("entity_value");
        assert_eq!(interner.stats().unique_strings, 2);

        interner.reset_counted();
        // Pinned survives, counted is dropped.
        assert_eq!(
            interner.resolve_str(pinned).as_deref(),
            Some("policy_literal")
        );
        assert_eq!(interner.resolve(counted), None);
        assert_eq!(interner.stats().unique_strings, 1);
    }

    #[test]
    fn test_release_unknown_id_is_noop() {
        let interner = StringInterner::new();
        // Releasing an id that was never interned must not panic or corrupt.
        interner.release(InternedString::from_id(9999));
        assert_eq!(interner.stats().unique_strings, 0);
    }

    #[test]
    fn test_evicted_id_not_reused() {
        let interner = StringInterner::new();
        let a = interner.intern_counted("first");
        interner.release(a); // evicts "first"
        let b = interner.intern_counted("second");
        // Monotonic ids: the new string must not reuse the evicted id.
        assert_ne!(a, b);
        assert_eq!(interner.resolve(a), None);
        assert_eq!(interner.resolve_str(b).as_deref(), Some("second"));
    }
}
