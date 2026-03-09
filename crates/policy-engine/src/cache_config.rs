//! Decision Cache Configuration
//!
//! Provides configurable caching with environment variable overrides.
//!
//! # Environment Variables
//!
//! - `REAPER_CACHE_ENABLED` - Enable/disable cache ("true"/"false", default: "true")
//! - `REAPER_CACHE_CAPACITY` - Maximum cache entries (default: 10000)
//! - `REAPER_CACHE_TTL_SECS` - Time-to-live in seconds (default: 300, 0 = no TTL)
//!
//! # Usage
//! ```text
//! use policy_engine::cache_config::CacheConfig;
//!
//! // Load from environment (with defaults)
//! let config = CacheConfig::from_env();
//!
//! // Create cache from config
//! if let Some(cache) = config.build_cache() {
//!     // Use cache
//! }
//!
//! // Or use builder pattern
//! let config = CacheConfig::builder()
//!     .enabled(true)
//!     .capacity(50000)
//!     .ttl_secs(600)
//!     .build();
//! ```

use crate::decision_cache::DecisionCache;
use std::env;
use std::sync::Arc;
use std::time::Duration;

/// Default cache capacity (10,000 entries)
pub const DEFAULT_CACHE_CAPACITY: usize = 10_000;

/// Default TTL in seconds (10 seconds)
pub const DEFAULT_CACHE_TTL_SECS: u64 = 10;

/// Environment variable names
pub const ENV_CACHE_ENABLED: &str = "REAPER_CACHE_ENABLED";
pub const ENV_CACHE_CAPACITY: &str = "REAPER_CACHE_CAPACITY";
pub const ENV_CACHE_TTL_SECS: &str = "REAPER_CACHE_TTL_SECS";

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Whether caching is enabled
    pub enabled: bool,
    /// Maximum number of cached decisions
    pub capacity: usize,
    /// Time-to-live for cached entries (None = no expiration)
    pub ttl: Option<Duration>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capacity: DEFAULT_CACHE_CAPACITY,
            ttl: Some(Duration::from_secs(DEFAULT_CACHE_TTL_SECS)),
        }
    }
}

impl CacheConfig {
    /// Create a new cache config with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a disabled cache config
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            capacity: 0,
            ttl: None,
        }
    }

    /// Load configuration from environment variables
    ///
    /// Environment variables:
    /// - `REAPER_CACHE_ENABLED`: "true" or "false" (default: "true")
    /// - `REAPER_CACHE_CAPACITY`: positive integer (default: 10000)
    /// - `REAPER_CACHE_TTL_SECS`: seconds, 0 for no TTL (default: 300)
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Check if cache is enabled
        if let Ok(val) = env::var(ENV_CACHE_ENABLED) {
            config.enabled = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes" | "on");
        }

        // Parse capacity
        if let Ok(val) = env::var(ENV_CACHE_CAPACITY) {
            if let Ok(capacity) = val.parse::<usize>() {
                config.capacity = capacity;
            }
        }

        // Parse TTL
        if let Ok(val) = env::var(ENV_CACHE_TTL_SECS) {
            if let Ok(secs) = val.parse::<u64>() {
                config.ttl = if secs == 0 {
                    None // 0 means no TTL
                } else {
                    Some(Duration::from_secs(secs))
                };
            }
        }

        config
    }

    /// Create a builder for custom configuration
    pub fn builder() -> CacheConfigBuilder {
        CacheConfigBuilder::new()
    }

    /// Build a DecisionCache from this config
    ///
    /// Returns None if caching is disabled
    pub fn build_cache(&self) -> Option<DecisionCache> {
        if !self.enabled || self.capacity == 0 {
            return None;
        }

        Some(match self.ttl {
            Some(ttl) => DecisionCache::with_ttl(self.capacity, ttl),
            None => DecisionCache::new(self.capacity),
        })
    }

    /// Build an Arc-wrapped DecisionCache from this config
    ///
    /// Returns None if caching is disabled
    pub fn build_cache_arc(&self) -> Option<Arc<DecisionCache>> {
        self.build_cache().map(Arc::new)
    }

    /// Check if caching is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.capacity > 0
    }

    /// Get a summary string for logging
    pub fn summary(&self) -> String {
        if !self.enabled {
            "disabled".to_string()
        } else {
            let ttl_str = match self.ttl {
                Some(d) => format!("{}s TTL", d.as_secs()),
                None => "no TTL".to_string(),
            };
            format!("enabled ({} entries, {})", self.capacity, ttl_str)
        }
    }
}

/// Builder for CacheConfig
#[derive(Debug, Clone)]
pub struct CacheConfigBuilder {
    enabled: bool,
    capacity: usize,
    ttl_secs: Option<u64>,
}

impl CacheConfigBuilder {
    /// Create a new builder with defaults
    pub fn new() -> Self {
        Self {
            enabled: true,
            capacity: DEFAULT_CACHE_CAPACITY,
            ttl_secs: Some(DEFAULT_CACHE_TTL_SECS),
        }
    }

    /// Set whether caching is enabled
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set cache capacity
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Set TTL in seconds (0 or None = no TTL)
    pub fn ttl_secs(mut self, secs: u64) -> Self {
        self.ttl_secs = if secs == 0 { None } else { Some(secs) };
        self
    }

    /// Set TTL as Duration (None = no TTL)
    pub fn ttl(mut self, ttl: Option<Duration>) -> Self {
        self.ttl_secs = ttl.map(|d| d.as_secs());
        self
    }

    /// Disable TTL (entries never expire)
    pub fn no_ttl(mut self) -> Self {
        self.ttl_secs = None;
        self
    }

    /// Build the config
    pub fn build(self) -> CacheConfig {
        CacheConfig {
            enabled: self.enabled,
            capacity: self.capacity,
            ttl: self.ttl_secs.map(Duration::from_secs),
        }
    }
}

impl Default for CacheConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Global cache configuration singleton
///
/// Loads configuration once from environment variables and caches it.
pub mod global {
    use super::*;
    use std::sync::OnceLock;

    static CONFIG: OnceLock<CacheConfig> = OnceLock::new();
    static CACHE: OnceLock<Option<Arc<DecisionCache>>> = OnceLock::new();

    /// Get the global cache configuration (loaded from env on first call)
    pub fn config() -> &'static CacheConfig {
        CONFIG.get_or_init(CacheConfig::from_env)
    }

    /// Get or create the global shared cache
    pub fn cache() -> Option<&'static Arc<DecisionCache>> {
        CACHE.get_or_init(|| config().build_cache_arc()).as_ref()
    }

    /// Check if the global cache is enabled
    pub fn is_enabled() -> bool {
        config().is_enabled()
    }

    /// Reset the global configuration (for testing)
    #[cfg(test)]
    pub fn reset() {
        // Note: OnceLock doesn't have a reset, so this only works in fresh test processes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to serialize env var tests (they share process environment)
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_default_config() {
        let config = CacheConfig::default();
        assert!(config.enabled);
        assert_eq!(config.capacity, DEFAULT_CACHE_CAPACITY);
        assert_eq!(
            config.ttl,
            Some(Duration::from_secs(DEFAULT_CACHE_TTL_SECS))
        );
    }

    #[test]
    fn test_disabled_config() {
        let config = CacheConfig::disabled();
        assert!(!config.enabled);
        assert!(config.build_cache().is_none());
    }

    #[test]
    fn test_builder() {
        let config = CacheConfig::builder()
            .enabled(true)
            .capacity(5000)
            .ttl_secs(120)
            .build();

        assert!(config.enabled);
        assert_eq!(config.capacity, 5000);
        assert_eq!(config.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    fn test_builder_no_ttl() {
        let config = CacheConfig::builder().capacity(1000).no_ttl().build();

        assert!(config.ttl.is_none());
    }

    #[test]
    fn test_build_cache() {
        let config = CacheConfig::builder().capacity(100).ttl_secs(60).build();

        let cache = config.build_cache();
        assert!(cache.is_some());
    }

    #[test]
    fn test_build_cache_disabled() {
        let config = CacheConfig::builder().enabled(false).build();

        assert!(config.build_cache().is_none());
    }

    #[test]
    fn test_build_cache_zero_capacity() {
        let config = CacheConfig::builder().capacity(0).build();

        assert!(config.build_cache().is_none());
    }

    #[test]
    fn test_summary() {
        let enabled = CacheConfig::builder().capacity(10000).ttl_secs(10).build();
        assert!(enabled.summary().contains("10000 entries"));
        assert!(enabled.summary().contains("10s TTL"));

        let no_ttl = CacheConfig::builder().capacity(5000).no_ttl().build();
        assert!(no_ttl.summary().contains("no TTL"));

        let disabled = CacheConfig::disabled();
        assert_eq!(disabled.summary(), "disabled");
    }

    #[test]
    fn test_from_env_defaults() {
        let _lock = ENV_MUTEX.lock().unwrap();

        // Clear any existing env vars for this test
        env::remove_var(ENV_CACHE_ENABLED);
        env::remove_var(ENV_CACHE_CAPACITY);
        env::remove_var(ENV_CACHE_TTL_SECS);

        let config = CacheConfig::from_env();
        assert!(config.enabled);
        assert_eq!(config.capacity, DEFAULT_CACHE_CAPACITY);
        assert_eq!(
            config.ttl,
            Some(Duration::from_secs(DEFAULT_CACHE_TTL_SECS))
        );
    }

    #[test]
    fn test_from_env_custom() {
        let _lock = ENV_MUTEX.lock().unwrap();

        env::set_var(ENV_CACHE_ENABLED, "true");
        env::set_var(ENV_CACHE_CAPACITY, "50000");
        env::set_var(ENV_CACHE_TTL_SECS, "600");

        let config = CacheConfig::from_env();
        assert!(config.enabled);
        assert_eq!(config.capacity, 50000);
        assert_eq!(config.ttl, Some(Duration::from_secs(600)));

        // Cleanup
        env::remove_var(ENV_CACHE_ENABLED);
        env::remove_var(ENV_CACHE_CAPACITY);
        env::remove_var(ENV_CACHE_TTL_SECS);
    }

    #[test]
    fn test_from_env_disabled() {
        let _lock = ENV_MUTEX.lock().unwrap();

        env::set_var(ENV_CACHE_ENABLED, "false");

        let config = CacheConfig::from_env();
        assert!(!config.enabled);

        env::remove_var(ENV_CACHE_ENABLED);
    }

    #[test]
    fn test_from_env_no_ttl() {
        let _lock = ENV_MUTEX.lock().unwrap();

        env::set_var(ENV_CACHE_TTL_SECS, "0");

        let config = CacheConfig::from_env();
        assert!(config.ttl.is_none());

        env::remove_var(ENV_CACHE_TTL_SECS);
    }
}
