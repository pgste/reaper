//! Thread-Local Regex Cache
//!
//! High-performance regex caching using thread-local storage for zero-overhead
//! access on the evaluation hot path.
//!
//! # Performance Characteristics
//! - First compilation: ~10-50µs per pattern (one-time cost)
//! - Cached lookup: ~5-10ns (thread-local HashMap access)
//! - No Arc/Mutex overhead on the hot path
//!
//! # Usage
//! ```text
//! use policy_engine::regex_cache::{get_or_compile, prewarm_patterns};
//!
//! // Pre-warm cache with known patterns (e.g., from bundle hints)
//! prewarm_patterns(&["^admin_.*", ".*@company\\.com$"]);
//!
//! // Fast lookup during evaluation
//! if let Some(re) = get_or_compile("^admin_.*") {
//!     if re.is_match("admin_alice") {
//!         // Pattern matched
//!     }
//! }
//! ```

use regex::Regex;
use rustc_hash::FxHashMap;
use std::cell::RefCell;

// Thread-local regex cache
//
// Each thread maintains its own cache of compiled regexes.
// This eliminates synchronization overhead during evaluation.
thread_local! {
    static REGEX_CACHE: RefCell<FxHashMap<String, Regex>> = RefCell::new(FxHashMap::default());
}

/// Get a compiled regex from the thread-local cache, compiling if necessary.
///
/// This is the primary hot-path function for regex evaluation.
/// Performance: ~5-10ns for cached patterns, ~10-50µs for first compilation.
///
/// # Arguments
/// * `pattern` - The regex pattern string
///
/// # Returns
/// * `Some(Regex)` - Cloned compiled regex (Regex is cheap to clone)
/// * `None` - Pattern failed to compile
///
/// # Example
/// ```text
/// if let Some(re) = get_or_compile("^user_\\d+$") {
///     assert!(re.is_match("user_123"));
/// }
/// ```
#[inline]
pub fn get_or_compile(pattern: &str) -> Option<Regex> {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Fast path: pattern already cached
        if let Some(re) = cache.get(pattern) {
            return Some(re.clone());
        }

        // Slow path: compile and cache
        match Regex::new(pattern) {
            Ok(re) => {
                cache.insert(pattern.to_string(), re.clone());
                Some(re)
            }
            Err(_) => None,
        }
    })
}

/// Check if a pattern matches text using the thread-local cache.
///
/// Convenience function that combines cache lookup and matching.
/// Returns `false` for invalid patterns (fail-safe).
///
/// # Arguments
/// * `pattern` - The regex pattern string
/// * `text` - The text to match against
///
/// # Returns
/// * `true` - Pattern is valid and matches text
/// * `false` - Pattern is invalid or doesn't match
#[inline]
pub fn matches(pattern: &str, text: &str) -> bool {
    get_or_compile(pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

/// Pre-warm the thread-local cache with patterns.
///
/// Call this at thread startup or policy deployment to avoid
/// compilation latency during the first evaluation.
///
/// # Arguments
/// * `patterns` - Slice of regex patterns to pre-compile
///
/// # Returns
/// * Number of patterns successfully compiled
///
/// # Example
/// ```text
/// // Pre-warm with patterns from policy bundle hints
/// let hints = policy_bundle.precompilation_hints();
/// let count = prewarm_patterns(&hints.regex_patterns);
/// println!("Pre-compiled {} regex patterns", count);
/// ```
pub fn prewarm_patterns(patterns: &[&str]) -> usize {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let mut count = 0;

        for pattern in patterns {
            if !cache.contains_key(*pattern) {
                if let Ok(re) = Regex::new(pattern) {
                    cache.insert(pattern.to_string(), re);
                    count += 1;
                }
            }
        }

        count
    })
}

/// Pre-warm the thread-local cache with owned pattern strings.
///
/// Variant of `prewarm_patterns` that takes owned strings.
pub fn prewarm_patterns_owned(patterns: &[String]) -> usize {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let mut count = 0;

        for pattern in patterns {
            if !cache.contains_key(pattern) {
                if let Ok(re) = Regex::new(pattern) {
                    cache.insert(pattern.clone(), re);
                    count += 1;
                }
            }
        }

        count
    })
}

/// Clear the thread-local regex cache.
///
/// Useful for testing or when reloading policies with different patterns.
pub fn clear_cache() {
    REGEX_CACHE.with(|cache| {
        cache.borrow_mut().clear();
    });
}

/// Get the current size of the thread-local cache.
///
/// Useful for monitoring and debugging.
pub fn cache_size() -> usize {
    REGEX_CACHE.with(|cache| cache.borrow().len())
}

/// Get statistics about the thread-local cache.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub pattern_count: usize,
    pub estimated_memory_bytes: usize,
}

/// Get statistics about the current thread's regex cache.
pub fn cache_stats() -> CacheStats {
    REGEX_CACHE.with(|cache| {
        let cache = cache.borrow();
        let pattern_count = cache.len();

        // Rough estimate: each Regex is ~200-500 bytes depending on pattern
        // Plus the string key (~24 bytes + string content)
        let estimated_memory_bytes = cache
            .keys()
            .map(|k| k.len() + 24 + 300) // key + overhead + regex estimate
            .sum();

        CacheStats {
            pattern_count,
            estimated_memory_bytes,
        }
    })
}

/// Global regex cache for cross-thread access (with synchronization).
///
/// Use this when you need to share compiled patterns across threads
/// without re-compilation. Less performant than thread-local but
/// useful for pre-deployment compilation.
pub mod global {
    use super::*;
    use parking_lot::RwLock;
    use std::sync::Arc;

    lazy_static::lazy_static! {
        static ref GLOBAL_CACHE: RwLock<FxHashMap<String, Arc<Regex>>> =
            RwLock::new(FxHashMap::default());
    }

    /// Get or compile a regex in the global cache.
    ///
    /// Thread-safe but slower than thread-local version.
    /// Use for setup/initialization, not hot-path evaluation.
    pub fn get_or_compile(pattern: &str) -> Option<Arc<Regex>> {
        // Fast path: read lock
        {
            let cache = GLOBAL_CACHE.read();
            if let Some(re) = cache.get(pattern) {
                return Some(Arc::clone(re));
            }
        }

        // Slow path: write lock and compile
        let mut cache = GLOBAL_CACHE.write();

        // Double-check after acquiring write lock
        if let Some(re) = cache.get(pattern) {
            return Some(Arc::clone(re));
        }

        match Regex::new(pattern) {
            Ok(re) => {
                let re = Arc::new(re);
                cache.insert(pattern.to_string(), Arc::clone(&re));
                Some(re)
            }
            Err(_) => None,
        }
    }

    /// Pre-warm the global cache with patterns.
    pub fn prewarm_patterns(patterns: &[&str]) -> usize {
        let mut cache = GLOBAL_CACHE.write();
        let mut count = 0;

        for pattern in patterns {
            if !cache.contains_key(*pattern) {
                if let Ok(re) = Regex::new(pattern) {
                    cache.insert(pattern.to_string(), Arc::new(re));
                    count += 1;
                }
            }
        }

        count
    }

    /// Copy global cache patterns to current thread's local cache.
    ///
    /// Call this at thread startup to warm the thread-local cache
    /// with patterns already compiled globally.
    pub fn copy_to_thread_local() -> usize {
        let global = GLOBAL_CACHE.read();

        REGEX_CACHE.with(|cache| {
            let mut local = cache.borrow_mut();
            let mut count = 0;

            for (pattern, re) in global.iter() {
                if !local.contains_key(pattern) {
                    // Clone the inner Regex (not the Arc)
                    local.insert(pattern.clone(), (**re).clone());
                    count += 1;
                }
            }

            count
        })
    }

    /// Clear the global cache.
    pub fn clear_cache() {
        GLOBAL_CACHE.write().clear();
    }

    /// Get the size of the global cache.
    pub fn cache_size() -> usize {
        GLOBAL_CACHE.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_or_compile() {
        clear_cache();

        // First call compiles
        let re = get_or_compile("^test_\\d+$").unwrap();
        assert!(re.is_match("test_123"));
        assert!(!re.is_match("test_abc"));

        // Second call uses cache
        let re2 = get_or_compile("^test_\\d+$").unwrap();
        assert!(re2.is_match("test_456"));

        assert_eq!(cache_size(), 1);
    }

    #[test]
    fn test_matches() {
        clear_cache();

        assert!(matches("^hello", "hello world"));
        assert!(!matches("^hello", "world hello"));
        assert!(!matches("[invalid", "anything")); // Invalid pattern returns false
    }

    #[test]
    fn test_prewarm() {
        clear_cache();

        let patterns = ["^admin_.*", ".*@test\\.com$", "user_\\d+"];
        let count = prewarm_patterns(&patterns);

        assert_eq!(count, 3);
        assert_eq!(cache_size(), 3);

        // Verify patterns are cached
        assert!(matches("^admin_.*", "admin_alice"));
        assert!(matches(".*@test\\.com$", "alice@test.com"));
    }

    #[test]
    fn test_invalid_pattern() {
        clear_cache();

        assert!(get_or_compile("[invalid").is_none());
        assert!(!matches("[invalid", "anything"));

        // Invalid pattern shouldn't be cached
        assert_eq!(cache_size(), 0);
    }

    #[test]
    fn test_cache_stats() {
        clear_cache();

        prewarm_patterns(&["^a$", "^b$", "^c$"]);

        let stats = cache_stats();
        assert_eq!(stats.pattern_count, 3);
        assert!(stats.estimated_memory_bytes > 0);
    }

    #[test]
    fn test_global_cache() {
        global::clear_cache();
        clear_cache();

        // Pre-warm global cache
        let count = global::prewarm_patterns(&["^global_\\d+$"]);
        assert_eq!(count, 1);

        // Copy to thread-local
        let copied = global::copy_to_thread_local();
        assert_eq!(copied, 1);

        // Verify thread-local now has the pattern
        assert!(matches("^global_\\d+$", "global_42"));
    }
}
