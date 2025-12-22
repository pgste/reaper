//! Advanced tier strategies for entity data storage
//!
//! Implements three-tier storage system for efficient entity lookups:
//! - Tier 1: Direct maps (<10K entities) - single BPF map, 50ns latency
//! - Tier 2: Sharded maps (10K-100K entities) - 16 shards, 100ns latency
//! - Tier 3: Bloom filter + partitioned (100K-1M entities) - bloom pre-filter, 150ns latency

use crate::entity::DataTier;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Number of shards for Tier 2
pub const TIER2_SHARD_COUNT: usize = 16;

/// Number of partitions for Tier 3
pub const TIER3_PARTITION_COUNT: usize = 64;

/// Bloom filter size for Tier 3 (bits)
pub const TIER3_BLOOM_SIZE_BITS: usize = 1024 * 8; // 1KB

/// Number of hash functions for bloom filter
pub const TIER3_BLOOM_HASH_COUNT: usize = 3;

/// Tier strategy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierStrategy {
    /// Selected tier
    pub tier: DataTier,

    /// Number of shards (for Tier 2)
    pub shard_count: Option<usize>,

    /// Number of partitions (for Tier 3)
    pub partition_count: Option<usize>,

    /// Whether bloom filter is enabled (for Tier 3)
    pub bloom_enabled: bool,

    /// Estimated memory overhead for the tier
    pub memory_overhead_bytes: usize,
}

impl TierStrategy {
    /// Create strategy for the given tier and entity count
    pub fn for_tier(tier: DataTier, _entity_count: usize) -> Self {
        match tier {
            DataTier::Tier1Direct => Self {
                tier,
                shard_count: None,
                partition_count: None,
                bloom_enabled: false,
                memory_overhead_bytes: 0, // Direct map has no overhead
            },
            DataTier::Tier2Sharded => Self {
                tier,
                shard_count: Some(TIER2_SHARD_COUNT),
                partition_count: None,
                bloom_enabled: false,
                // Overhead: 16 map headers + shard metadata
                memory_overhead_bytes: TIER2_SHARD_COUNT * 64,
            },
            DataTier::Tier3Partitioned => {
                // Bloom filter size + partition metadata
                let bloom_size = TIER3_BLOOM_SIZE_BITS / 8; // Convert to bytes
                let partition_metadata = TIER3_PARTITION_COUNT * 64;
                Self {
                    tier,
                    shard_count: None,
                    partition_count: Some(TIER3_PARTITION_COUNT),
                    bloom_enabled: true,
                    memory_overhead_bytes: bloom_size + partition_metadata,
                }
            }
        }
    }

    /// Calculate shard ID for an entity (Tier 2)
    pub fn calculate_shard(&self, entity_id: &str) -> Option<usize> {
        self.shard_count.map(|count| {
            let mut hasher = DefaultHasher::new();
            entity_id.hash(&mut hasher);
            (hasher.finish() as usize) % count
        })
    }

    /// Calculate partition ID for an entity (Tier 3)
    pub fn calculate_partition(&self, entity_id: &str) -> Option<usize> {
        self.partition_count.map(|count| {
            let mut hasher = DefaultHasher::new();
            entity_id.hash(&mut hasher);
            (hasher.finish() as usize) % count
        })
    }

    /// Get total expected latency including tier overhead
    pub fn total_latency_ns(&self) -> u32 {
        self.tier.latency_ns()
    }
}

/// Bloom filter for fast negative lookups (Tier 3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BloomFilter {
    /// Bit array
    bits: Vec<bool>,

    /// Number of hash functions to use
    hash_count: usize,
}

impl BloomFilter {
    /// Create a new bloom filter
    pub fn new(size_bits: usize, hash_count: usize) -> Self {
        Self {
            bits: vec![false; size_bits],
            hash_count,
        }
    }

    /// Create with default settings for Tier 3
    pub fn new_tier3() -> Self {
        Self::new(TIER3_BLOOM_SIZE_BITS, TIER3_BLOOM_HASH_COUNT)
    }

    /// Insert an entity ID into the bloom filter
    pub fn insert(&mut self, entity_id: &str) {
        for i in 0..self.hash_count {
            let index = self.hash_with_seed(entity_id, i);
            self.bits[index] = true;
        }
    }

    /// Check if an entity ID might be in the set
    ///
    /// Returns:
    /// - `true`: Entity might be present (check actual storage)
    /// - `false`: Entity is definitely NOT present (skip lookup)
    pub fn might_contain(&self, entity_id: &str) -> bool {
        for i in 0..self.hash_count {
            let index = self.hash_with_seed(entity_id, i);
            if !self.bits[index] {
                return false; // Definitely not present
            }
        }
        true // Might be present
    }

    /// Calculate false positive rate
    pub fn false_positive_rate(&self, n_inserted: usize) -> f64 {
        // Formula: (1 - e^(-k*n/m))^k
        // k = hash_count, n = items inserted, m = bits
        let k = self.hash_count as f64;
        let n = n_inserted as f64;
        let m = self.bits.len() as f64;

        let exp_term = (-k * n / m).exp();
        (1.0 - exp_term).powf(k)
    }

    /// Get memory usage in bytes
    pub fn memory_bytes(&self) -> usize {
        self.bits.len() / 8 // Bits to bytes
    }

    /// Hash with seed for bloom filter
    fn hash_with_seed(&self, entity_id: &str, seed: usize) -> usize {
        let mut hasher = DefaultHasher::new();
        entity_id.hash(&mut hasher);
        seed.hash(&mut hasher);
        (hasher.finish() as usize) % self.bits.len()
    }

    /// Get statistics about the bloom filter
    pub fn stats(&self) -> BloomFilterStats {
        let bits_set = self.bits.iter().filter(|&&b| b).count();
        let fill_ratio = bits_set as f64 / self.bits.len() as f64;

        BloomFilterStats {
            size_bits: self.bits.len(),
            bits_set,
            fill_ratio,
            hash_count: self.hash_count,
            memory_bytes: self.memory_bytes(),
        }
    }
}

/// Bloom filter statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BloomFilterStats {
    /// Total size in bits
    pub size_bits: usize,

    /// Number of bits set to true
    pub bits_set: usize,

    /// Ratio of bits set (0.0 to 1.0)
    pub fill_ratio: f64,

    /// Number of hash functions
    pub hash_count: usize,

    /// Memory usage in bytes
    pub memory_bytes: usize,
}

/// Tier 2 sharded storage strategy
#[derive(Debug, Clone)]
pub struct Tier2Strategy {
    strategy: TierStrategy,
}

impl Tier2Strategy {
    /// Create a new Tier 2 strategy
    pub fn new(entity_count: usize) -> Self {
        Self {
            strategy: TierStrategy::for_tier(DataTier::Tier2Sharded, entity_count),
        }
    }

    /// Get shard for an entity
    pub fn get_shard(&self, entity_id: &str) -> usize {
        self.strategy
            .calculate_shard(entity_id)
            .expect("Tier 2 should have shards")
    }

    /// Get total shard count
    pub fn shard_count(&self) -> usize {
        self.strategy.shard_count.unwrap_or(TIER2_SHARD_COUNT)
    }

    /// Get strategy details
    pub fn strategy(&self) -> &TierStrategy {
        &self.strategy
    }
}

/// Tier 3 partitioned storage with bloom filter
#[derive(Debug, Clone)]
pub struct Tier3Strategy {
    strategy: TierStrategy,
    bloom_filter: BloomFilter,
}

impl Tier3Strategy {
    /// Create a new Tier 3 strategy
    pub fn new(entity_count: usize) -> Self {
        Self {
            strategy: TierStrategy::for_tier(DataTier::Tier3Partitioned, entity_count),
            bloom_filter: BloomFilter::new_tier3(),
        }
    }

    /// Build bloom filter from entity IDs
    pub fn build_bloom_filter(&mut self, entity_ids: &[String]) {
        self.bloom_filter = BloomFilter::new_tier3();
        for id in entity_ids {
            self.bloom_filter.insert(id);
        }
    }

    /// Check if entity might exist (using bloom filter)
    pub fn might_contain(&self, entity_id: &str) -> bool {
        self.bloom_filter.might_contain(entity_id)
    }

    /// Get partition for an entity
    pub fn get_partition(&self, entity_id: &str) -> usize {
        self.strategy
            .calculate_partition(entity_id)
            .expect("Tier 3 should have partitions")
    }

    /// Get total partition count
    pub fn partition_count(&self) -> usize {
        self.strategy
            .partition_count
            .unwrap_or(TIER3_PARTITION_COUNT)
    }

    /// Get bloom filter statistics
    pub fn bloom_stats(&self) -> BloomFilterStats {
        self.bloom_filter.stats()
    }

    /// Get strategy details
    pub fn strategy(&self) -> &TierStrategy {
        &self.strategy
    }

    /// Calculate false positive rate for given entity count
    pub fn false_positive_rate(&self, entity_count: usize) -> f64 {
        self.bloom_filter.false_positive_rate(entity_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_strategy_creation() {
        let tier1 = TierStrategy::for_tier(DataTier::Tier1Direct, 5000);
        assert_eq!(tier1.tier, DataTier::Tier1Direct);
        assert_eq!(tier1.shard_count, None);
        assert_eq!(tier1.memory_overhead_bytes, 0);

        let tier2 = TierStrategy::for_tier(DataTier::Tier2Sharded, 50_000);
        assert_eq!(tier2.tier, DataTier::Tier2Sharded);
        assert_eq!(tier2.shard_count, Some(16));
        assert!(tier2.memory_overhead_bytes > 0);

        let tier3 = TierStrategy::for_tier(DataTier::Tier3Partitioned, 500_000);
        assert_eq!(tier3.tier, DataTier::Tier3Partitioned);
        assert_eq!(tier3.partition_count, Some(64));
        assert!(tier3.bloom_enabled);
        assert!(tier3.memory_overhead_bytes > 0);
    }

    #[test]
    fn test_shard_calculation() {
        let strategy = TierStrategy::for_tier(DataTier::Tier2Sharded, 50_000);

        let shard1 = strategy.calculate_shard("user:alice").unwrap();
        let shard2 = strategy.calculate_shard("user:alice").unwrap();
        let shard3 = strategy.calculate_shard("user:bob").unwrap();

        // Same entity should always get same shard
        assert_eq!(shard1, shard2);

        // Shards should be in valid range
        assert!(shard1 < TIER2_SHARD_COUNT);
        assert!(shard3 < TIER2_SHARD_COUNT);
    }

    #[test]
    fn test_partition_calculation() {
        let strategy = TierStrategy::for_tier(DataTier::Tier3Partitioned, 500_000);

        let part1 = strategy.calculate_partition("user:alice").unwrap();
        let part2 = strategy.calculate_partition("user:alice").unwrap();
        let part3 = strategy.calculate_partition("user:bob").unwrap();

        // Same entity should always get same partition
        assert_eq!(part1, part2);

        // Partitions should be in valid range
        assert!(part1 < TIER3_PARTITION_COUNT);
        assert!(part3 < TIER3_PARTITION_COUNT);
    }

    #[test]
    fn test_bloom_filter_basic() {
        let mut bloom = BloomFilter::new_tier3();

        // Insert some entities
        bloom.insert("user:alice");
        bloom.insert("user:bob");
        bloom.insert("user:charlie");

        // Should definitely contain inserted items
        assert!(bloom.might_contain("user:alice"));
        assert!(bloom.might_contain("user:bob"));
        assert!(bloom.might_contain("user:charlie"));

        // Might have false positives, but should mostly reject non-inserted items
        let non_inserted = [
            "user:dave",
            "user:eve",
            "user:frank",
            "user:grace",
            "user:heidi",
        ];

        let false_positives = non_inserted
            .iter()
            .filter(|id| bloom.might_contain(id))
            .count();

        // With 3 items and 1KB bloom filter, false positive rate should be very low
        assert!(false_positives <= 2, "Too many false positives");
    }

    #[test]
    fn test_bloom_filter_false_positive_rate() {
        let mut bloom = BloomFilter::new_tier3();

        // Insert 1000 entities
        for i in 0..1000 {
            bloom.insert(&format!("user:{}", i));
        }

        let fp_rate = bloom.false_positive_rate(1000);

        // With 3 hashes, 1KB filter (8192 bits), and 1000 items, FP rate ~2-3%
        // Formula: (1 - e^(-k*n/m))^k where k=3, n=1000, m=8192
        assert!(fp_rate < 0.05, "False positive rate too high: {}", fp_rate);
        assert!(
            fp_rate > 0.01,
            "False positive rate unexpectedly low: {}",
            fp_rate
        );
    }

    #[test]
    fn test_tier2_strategy() {
        let tier2 = Tier2Strategy::new(50_000);

        assert_eq!(tier2.shard_count(), 16);

        let shard = tier2.get_shard("user:alice");
        assert!(shard < 16);

        // Same entity should always hash to same shard
        assert_eq!(shard, tier2.get_shard("user:alice"));
    }

    #[test]
    fn test_tier3_strategy() {
        let mut tier3 = Tier3Strategy::new(500_000);

        // Build bloom filter
        let entities: Vec<String> = (0..100).map(|i| format!("user:{}", i)).collect();
        tier3.build_bloom_filter(&entities);

        // Should find inserted entities
        assert!(tier3.might_contain("user:50"));

        // Get partition
        let partition = tier3.get_partition("user:alice");
        assert!(partition < 64);

        // Check bloom stats
        let stats = tier3.bloom_stats();
        assert!(stats.bits_set > 0);
        assert!(stats.fill_ratio > 0.0 && stats.fill_ratio < 1.0);
    }

    #[test]
    fn test_tier3_false_positive_rate() {
        let mut tier3 = Tier3Strategy::new(100_000);

        // Build with 1K entities (reasonable for 1KB bloom filter)
        let entities: Vec<String> = (0..1_000).map(|i| format!("user:{}", i)).collect();
        tier3.build_bloom_filter(&entities);

        let fp_rate = tier3.false_positive_rate(1_000);
        // With 1K entities in 1KB filter, FP rate should be ~2-3%
        assert!(fp_rate < 0.05, "False positive rate too high: {}", fp_rate);
    }

    #[test]
    fn test_bloom_filter_stats() {
        let mut bloom = BloomFilter::new_tier3();

        for i in 0..100 {
            bloom.insert(&format!("entity:{}", i));
        }

        let stats = bloom.stats();
        assert_eq!(stats.size_bits, TIER3_BLOOM_SIZE_BITS);
        assert!(stats.bits_set > 0);
        assert!(stats.fill_ratio > 0.0);
        assert_eq!(stats.hash_count, 3);
        assert_eq!(stats.memory_bytes, 1024); // 1KB
    }
}
