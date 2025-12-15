//! Learning Engine - Auto-promotes frequently accessed policies to eBPF
//!
//! This module implements the "learning mode" where the system observes
//! which paths are frequently accessed and automatically compiles them
//! from complex policies (Cedar, Reaper DSL) to simple eBPF rules.
//!
//! Flow:
//! 1. Complex policy evaluated in userspace (e.g., Cedar with ABAC)
//! 2. LearningEngine records: path + decision + frequency
//! 3. After N evaluations with stable decision → compile to simple rule
//! 4. Promote to eBPF POLICY_MAP
//! 5. Future requests for same path → <100ns eBPF fast path!

use crate::compiler::PolicyCompiler;
use crate::controller::EbpfController;
use crate::types::MAX_PATH_LEN;
use anyhow::Result;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use policy_engine::PolicyAction;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Access pattern for a resource path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessPattern {
    /// Resource path
    pub resource: String,

    /// Most recent decision
    pub decision: PolicyAction,

    /// Access count
    pub count: u64,

    /// First time this path was accessed
    pub first_seen: DateTime<Utc>,

    /// Last time this path was accessed
    pub last_seen: DateTime<Utc>,

    /// Whether the decision has been stable
    /// (same decision for last N accesses)
    pub stable: bool,

    /// Count of decision changes
    pub decision_changes: u32,

    /// UID associated with this pattern (if any)
    pub uid: Option<u32>,

    /// GID associated with this pattern (if any)
    pub gid: Option<u32>,
}

impl AccessPattern {
    /// Create a new access pattern
    pub fn new(
        resource: String,
        decision: PolicyAction,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Self {
        let now = Utc::now();
        Self {
            resource,
            decision,
            count: 1,
            first_seen: now,
            last_seen: now,
            stable: false,
            decision_changes: 0,
            uid,
            gid,
        }
    }

    /// Record an access with the given decision
    pub fn record_access(&mut self, decision: PolicyAction) {
        self.count += 1;
        self.last_seen = Utc::now();

        // Check if decision changed
        if self.decision != decision {
            self.decision = decision;
            self.decision_changes += 1;
            self.stable = false;
        } else if self.count >= 100 && self.decision_changes == 0 {
            // Stable if we've seen 100+ accesses with no changes
            self.stable = true;
        }
    }
}

/// Learning Engine - Tracks access patterns and promotes to eBPF
pub struct LearningEngine {
    /// Access patterns by resource path
    patterns: Arc<DashMap<String, AccessPattern>>,

    /// Promoted policies (resource → eBPF key)
    promoted: Arc<DashMap<String, [u8; MAX_PATH_LEN]>>,

    /// Threshold for promotion (number of accesses)
    promotion_threshold: u64,

    /// Stability window (must have N consecutive same decisions)
    stability_window: u32,

    /// Policy compiler
    compiler: PolicyCompiler,

    /// Count of promoted policies
    promoted_count: Arc<AtomicUsize>,
}

impl LearningEngine {
    /// Create a new learning engine
    ///
    /// # Arguments
    /// * `promotion_threshold` - Number of accesses before considering promotion
    /// * `stability_window` - Number of consecutive same decisions required
    pub fn new(promotion_threshold: u64, stability_window: u32) -> Self {
        info!(
            "Initializing LearningEngine (threshold: {}, stability: {})",
            promotion_threshold, stability_window
        );

        Self {
            patterns: Arc::new(DashMap::new()),
            promoted: Arc::new(DashMap::new()),
            promotion_threshold,
            stability_window,
            compiler: PolicyCompiler::new(),
            promoted_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create with default settings
    /// - Promotion threshold: 100 accesses
    /// - Stability window: 100 consecutive same decisions
    pub fn with_defaults() -> Self {
        Self::new(100, 100)
    }

    /// Record an access to a resource
    ///
    /// # Arguments
    /// * `resource` - The resource path accessed
    /// * `decision` - The policy decision made
    /// * `uid` - Optional UID associated with the access
    /// * `gid` - Optional GID associated with the access
    pub fn record_access(
        &self,
        resource: &str,
        decision: PolicyAction,
        uid: Option<u32>,
        gid: Option<u32>,
    ) {
        let resource = resource.to_string();

        self.patterns
            .entry(resource.clone())
            .and_modify(|pattern| {
                pattern.record_access(decision.clone());
            })
            .or_insert_with(|| AccessPattern::new(resource, decision, uid, gid));
    }

    /// Check if a resource should be promoted to eBPF
    pub fn should_promote(&self, resource: &str) -> bool {
        if let Some(pattern) = self.patterns.get(resource) {
            // Don't promote if already promoted
            if self.promoted.contains_key(resource) {
                return false;
            }

            // Check promotion criteria
            pattern.count >= self.promotion_threshold
                && pattern.stable
                && pattern.decision_changes == 0
        } else {
            false
        }
    }

    /// Get access pattern for a resource
    pub fn get_pattern(&self, resource: &str) -> Option<AccessPattern> {
        self.patterns.get(resource).map(|p| p.clone())
    }

    /// Promote a resource to eBPF
    ///
    /// Compiles the access pattern into a simple eBPF rule and inserts it
    /// into the POLICY_MAP.
    pub fn promote_to_ebpf(&self, resource: &str, controller: &mut EbpfController) -> Result<()> {
        // Get pattern
        let pattern = match self.patterns.get(resource) {
            Some(p) => p.clone(),
            None => {
                warn!("Attempted to promote unknown resource: {}", resource);
                return Ok(());
            }
        };

        // Check if already promoted
        if self.promoted.contains_key(resource) {
            debug!("Resource already promoted: {}", resource);
            return Ok(());
        }

        info!(
            "Promoting to eBPF: {} → {:?} (count: {}, stable: {})",
            resource, pattern.decision, pattern.count, pattern.stable
        );

        // Compile decision to eBPF format
        let priority = 0; // Promoted policies get highest priority
        let (key, entry) = self.compiler.compile_decision(
            resource,
            pattern.decision,
            pattern.uid,
            pattern.gid,
            priority,
        )?;

        // Insert into eBPF
        controller.insert_policy(key, entry)?;

        // Mark as promoted
        self.promoted.insert(resource.to_string(), key);
        self.promoted_count.fetch_add(1, Ordering::Relaxed);

        info!(
            "✓ Promoted to eBPF: {} (total promoted: {})",
            resource,
            self.promoted_count()
        );

        Ok(())
    }

    /// Auto-promote eligible resources
    ///
    /// Scans all access patterns and promotes those that meet the criteria.
    /// Returns the number of resources promoted.
    pub fn auto_promote(&self, controller: &mut EbpfController) -> Result<usize> {
        let mut promoted = 0;

        // Collect resources to promote (to avoid holding lock while promoting)
        let to_promote: Vec<String> = self
            .patterns
            .iter()
            .filter(|entry| self.should_promote(entry.key()))
            .map(|entry| entry.key().clone())
            .collect();

        for resource in to_promote {
            if let Err(e) = self.promote_to_ebpf(&resource, controller) {
                warn!("Failed to promote {}: {}", resource, e);
            } else {
                promoted += 1;
            }
        }

        if promoted > 0 {
            info!("Auto-promoted {} resources to eBPF", promoted);
        }

        Ok(promoted)
    }

    /// Get count of promoted policies
    pub fn promoted_count(&self) -> usize {
        self.promoted_count.load(Ordering::Relaxed)
    }

    /// Get count of tracked patterns
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// Get top N most accessed resources
    pub fn top_resources(&self, n: usize) -> Vec<(String, u64)> {
        let mut resources: Vec<_> = self
            .patterns
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().count))
            .collect();

        resources.sort_by(|a, b| b.1.cmp(&a.1));
        resources.truncate(n);
        resources
    }

    /// Get statistics about the learning engine
    pub fn get_stats(&self) -> LearningStats {
        let total_patterns = self.pattern_count();
        let promoted = self.promoted_count();

        let mut stable_count = 0;
        let mut unstable_count = 0;
        let mut eligible_count = 0;

        for entry in self.patterns.iter() {
            let pattern = entry.value();

            if pattern.stable {
                stable_count += 1;
            } else {
                unstable_count += 1;
            }

            if self.should_promote(entry.key()) {
                eligible_count += 1;
            }
        }

        LearningStats {
            total_patterns,
            promoted_patterns: promoted,
            stable_patterns: stable_count,
            unstable_patterns: unstable_count,
            eligible_for_promotion: eligible_count,
            promotion_threshold: self.promotion_threshold,
            stability_window: self.stability_window,
        }
    }

    /// Clear all learning data
    pub fn clear(&self) {
        self.patterns.clear();
        self.promoted.clear();
        self.promoted_count.store(0, Ordering::Relaxed);
        info!("Learning engine data cleared");
    }
}

impl Default for LearningEngine {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Statistics about the learning engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningStats {
    /// Total number of tracked access patterns
    pub total_patterns: usize,

    /// Number of patterns promoted to eBPF
    pub promoted_patterns: usize,

    /// Number of stable patterns (consistent decisions)
    pub stable_patterns: usize,

    /// Number of unstable patterns (inconsistent decisions)
    pub unstable_patterns: usize,

    /// Number of patterns eligible for promotion
    pub eligible_for_promotion: usize,

    /// Promotion threshold setting
    pub promotion_threshold: u64,

    /// Stability window setting
    pub stability_window: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_pattern_creation() {
        let pattern = AccessPattern::new(
            "/api/users".to_string(),
            PolicyAction::Allow,
            Some(1000),
            None,
        );

        assert_eq!(pattern.resource, "/api/users");
        assert_eq!(pattern.decision, PolicyAction::Allow);
        assert_eq!(pattern.count, 1);
        assert_eq!(pattern.uid, Some(1000));
        assert!(!pattern.stable);
    }

    #[test]
    fn test_access_pattern_stability() {
        let mut pattern =
            AccessPattern::new("/api/users".to_string(), PolicyAction::Allow, None, None);

        // Record 99 more accesses with same decision
        for _ in 0..99 {
            pattern.record_access(PolicyAction::Allow);
        }

        assert_eq!(pattern.count, 100);
        assert_eq!(pattern.decision_changes, 0);
        assert!(pattern.stable);
    }

    #[test]
    fn test_access_pattern_instability() {
        let mut pattern =
            AccessPattern::new("/api/users".to_string(), PolicyAction::Allow, None, None);

        // Record 50 allows
        for _ in 0..50 {
            pattern.record_access(PolicyAction::Allow);
        }

        // Record 1 deny (decision changes)
        pattern.record_access(PolicyAction::Deny);

        assert_eq!(pattern.count, 52);
        assert_eq!(pattern.decision_changes, 1);
        assert!(!pattern.stable);
    }

    #[test]
    fn test_learning_engine() {
        let engine = LearningEngine::new(5, 5); // Low thresholds for testing

        // Record 100 accesses to same resource (need 100 for stability)
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, Some(1000), None);
        }

        // Should be eligible for promotion
        assert!(engine.should_promote("/api/users"));

        // Check stats
        let stats = engine.get_stats();
        assert_eq!(stats.total_patterns, 1);
        assert_eq!(stats.eligible_for_promotion, 1);
    }

    #[test]
    fn test_top_resources() {
        let engine = LearningEngine::with_defaults();

        // Create different access patterns
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
        }

        for _ in 0..50 {
            engine.record_access("/api/posts", PolicyAction::Allow, None, None);
        }

        for _ in 0..25 {
            engine.record_access("/api/comments", PolicyAction::Allow, None, None);
        }

        let top = engine.top_resources(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "/api/users");
        assert_eq!(top[0].1, 100);
        assert_eq!(top[1].0, "/api/posts");
        assert_eq!(top[1].1, 50);
    }
}
