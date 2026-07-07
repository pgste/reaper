//! Learning Engine - Auto-promotes frequently accessed policies to eBPF
//!
//! This module implements the "learning mode" where the system observes
//! which paths are frequently accessed and automatically compiles them
//! from complex policies (Cedar, Reaper DSL) to simple eBPF rules.
//!
//! Flow:
//! 1. Complex policy evaluated in userspace (e.g., Cedar with ABAC)
//! 2. LearningEngine records: path + decision + frequency + policy context
//! 3. ConditionAnalyzer checks if pattern is eBPF-promotable
//! 4. After N evaluations with stable decision → compile to simple rule
//! 5. Promote to eBPF POLICY_MAP if eBPF-compatible
//! 6. Future requests for same path → <100ns eBPF fast path!

use crate::analyzer::ConditionAnalyzer;
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

    /// Policy rule name that produced this pattern
    pub policy_rule: Option<String>,

    /// Whether this pattern is eBPF-promotable
    pub ebpf_promotable: Option<bool>,

    /// Estimated complexity (1-10) from ConditionAnalyzer
    pub complexity: Option<u8>,

    /// Estimated latency in nanoseconds
    pub estimated_latency_ns: Option<u32>,

    /// Entity attributes accessed (for tracking)
    pub entity_lookups: Vec<String>,

    /// Reasons why pattern can't be promoted (if any)
    pub blocking_reasons: Vec<String>,
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
            policy_rule: None,
            ebpf_promotable: None,
            complexity: None,
            estimated_latency_ns: None,
            entity_lookups: Vec::new(),
            blocking_reasons: Vec::new(),
        }
    }

    /// Create with policy context
    pub fn with_policy_context(
        resource: String,
        decision: PolicyAction,
        uid: Option<u32>,
        gid: Option<u32>,
        policy_rule: Option<String>,
    ) -> Self {
        let mut pattern = Self::new(resource, decision, uid, gid);
        pattern.policy_rule = policy_rule;
        pattern
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

/// Result of auto-promotion operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPromotionResult {
    /// Number of patterns successfully promoted to eBPF
    pub promoted: usize,

    /// Number of patterns skipped due to eBPF incompatibility
    pub skipped_incompatible: usize,

    /// Number of patterns that failed promotion
    pub failed: usize,
}

impl AutoPromotionResult {
    /// Get total number of patterns processed
    pub fn total(&self) -> usize {
        self.promoted + self.skipped_incompatible + self.failed
    }

    /// Get success rate as percentage
    pub fn success_rate(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            (self.promoted as f64 / self.total() as f64) * 100.0
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

    /// Condition analyzer for eBPF compatibility checking
    analyzer: ConditionAnalyzer,

    /// Count of promoted policies
    promoted_count: Arc<AtomicUsize>,

    /// Count of patterns rejected due to eBPF incompatibility
    rejected_count: Arc<AtomicUsize>,
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
            analyzer: ConditionAnalyzer::new(),
            promoted_count: Arc::new(AtomicUsize::new(0)),
            rejected_count: Arc::new(AtomicUsize::new(0)),
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

    /// Record an access with policy context
    ///
    /// # Arguments
    /// * `resource` - The resource path accessed
    /// * `decision` - The policy decision made
    /// * `uid` - Optional UID associated with the access
    /// * `gid` - Optional GID associated with the access
    /// * `policy_rule` - Name of the policy rule that made the decision
    pub fn record_access_with_context(
        &self,
        resource: &str,
        decision: PolicyAction,
        uid: Option<u32>,
        gid: Option<u32>,
        policy_rule: Option<String>,
    ) {
        let resource = resource.to_string();

        self.patterns
            .entry(resource.clone())
            .and_modify(|pattern| {
                pattern.record_access(decision.clone());
                if policy_rule.is_some() {
                    pattern.policy_rule = policy_rule.clone();
                }
            })
            .or_insert_with(|| {
                AccessPattern::with_policy_context(resource, decision, uid, gid, policy_rule)
            });
    }

    /// Analyze a pattern for eBPF compatibility
    ///
    /// This method uses the ConditionAnalyzer to check if a pattern with a given
    /// policy condition can be promoted to eBPF. Updates the pattern with analysis results.
    ///
    /// # Arguments
    /// * `resource` - The resource to analyze
    /// * `condition` - The policy condition that produced this pattern
    ///
    /// # Returns
    /// Returns true if the pattern is eBPF-promotable
    pub fn analyze_pattern(
        &self,
        resource: &str,
        condition: &policy_engine::reap::ReapCondition,
    ) -> bool {
        let mut pattern_ref = match self.patterns.get_mut(resource) {
            Some(p) => p,
            None => return false,
        };

        // Analyze the condition
        let analysis = self.analyzer.analyze(condition);

        // Update pattern with analysis results
        pattern_ref.ebpf_promotable = Some(analysis.promotable);
        pattern_ref.complexity = Some(analysis.complexity);
        pattern_ref.estimated_latency_ns = Some(analysis.estimated_latency_ns);
        pattern_ref.entity_lookups = analysis.entity_lookups.clone();
        pattern_ref.blocking_reasons = analysis.blocking_reasons.clone();

        if !analysis.promotable {
            self.rejected_count.fetch_add(1, Ordering::Relaxed);
            debug!(
                "Pattern '{}' not eBPF-promotable: {:?}",
                resource, analysis.blocking_reasons
            );
        }

        analysis.promotable
    }

    /// Check if a resource should be promoted to eBPF
    pub fn should_promote(&self, resource: &str) -> bool {
        if let Some(pattern) = self.patterns.get(resource) {
            // Don't promote if already promoted
            if self.promoted.contains_key(resource) {
                return false;
            }

            // Check promotion criteria
            let meets_threshold = pattern.count >= self.promotion_threshold
                && pattern.stable
                && pattern.decision_changes == 0;

            // If we have eBPF compatibility info, check it
            if let Some(promotable) = pattern.ebpf_promotable {
                return meets_threshold && promotable;
            }

            // Without analysis info, use basic criteria
            meets_threshold
        } else {
            false
        }
    }

    /// Get access pattern for a resource
    pub fn get_pattern(&self, resource: &str) -> Option<AccessPattern> {
        self.patterns.get(resource).map(|p| p.clone())
    }

    /// Promote a resource to eBPF with compatibility check
    ///
    /// Compiles the access pattern into a simple eBPF rule and inserts it
    /// into the POLICY_MAP. Includes eBPF compatibility validation.
    ///
    /// # Arguments
    /// * `resource` - The resource path to promote
    /// * `controller` - The eBPF controller to insert the policy into
    /// * `force` - If true, skip eBPF compatibility check (use with caution)
    pub fn promote_to_ebpf_checked(
        &self,
        resource: &str,
        controller: &mut EbpfController,
        force: bool,
    ) -> Result<()> {
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

        // Check eBPF compatibility (unless forced)
        if !force {
            if let Some(false) = pattern.ebpf_promotable {
                warn!(
                    "Skipping promotion of '{}': Not eBPF-compatible. Reasons: {:?}",
                    resource, pattern.blocking_reasons
                );
                return Ok(());
            }

            // If not analyzed yet, warn but allow promotion for backward compatibility
            if pattern.ebpf_promotable.is_none() {
                warn!(
                    "Promoting '{}' without eBPF analysis - recommend calling analyze_pattern() first",
                    resource
                );
            }
        }

        info!(
            "Promoting to eBPF: {} → {:?} (count: {}, stable: {}, complexity: {:?}, latency: {:?}ns)",
            resource,
            pattern.decision,
            pattern.count,
            pattern.stable,
            pattern.complexity,
            pattern.estimated_latency_ns
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

    /// Promote a resource to eBPF (backward compatible - no forced checking)
    ///
    /// Compiles the access pattern into a simple eBPF rule and inserts it
    /// into the POLICY_MAP.
    pub fn promote_to_ebpf(&self, resource: &str, controller: &mut EbpfController) -> Result<()> {
        self.promote_to_ebpf_checked(resource, controller, false)
    }

    /// Auto-promote eligible resources with eBPF compatibility checking
    ///
    /// Scans all access patterns and promotes those that meet the criteria.
    /// Only promotes patterns that have been analyzed and are eBPF-compatible.
    ///
    /// Returns the result containing:
    /// - Number of resources promoted
    /// - Number of resources skipped due to eBPF incompatibility
    /// - Number of resources that failed promotion
    pub fn auto_promote_with_stats(
        &self,
        controller: &mut EbpfController,
    ) -> Result<AutoPromotionResult> {
        let mut promoted = 0;
        let mut skipped = 0;
        let mut failed = 0;

        // Collect resources to promote (to avoid holding lock while promoting)
        let eligible: Vec<(String, AccessPattern)> = self
            .patterns
            .iter()
            .filter(|entry| self.should_promote(entry.key()))
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        info!("Auto-promotion scan: {} eligible patterns", eligible.len());

        for (resource, pattern) in eligible {
            // Check eBPF compatibility
            if let Some(false) = pattern.ebpf_promotable {
                debug!(
                    "Skipping '{}': eBPF-incompatible ({:?})",
                    resource, pattern.blocking_reasons
                );
                skipped += 1;
                continue;
            }

            // Attempt promotion
            match self.promote_to_ebpf_checked(&resource, controller, false) {
                Ok(_) => {
                    promoted += 1;
                }
                Err(e) => {
                    warn!("Failed to promote '{}': {}", resource, e);
                    failed += 1;
                }
            }
        }

        let result = AutoPromotionResult {
            promoted,
            skipped_incompatible: skipped,
            failed,
        };

        if promoted > 0 {
            info!(
                "Auto-promotion complete: {} promoted, {} skipped (incompatible), {} failed",
                promoted, skipped, failed
            );
        } else if skipped > 0 {
            info!(
                "Auto-promotion complete: {} patterns skipped due to eBPF incompatibility",
                skipped
            );
        }

        Ok(result)
    }

    /// Auto-promote eligible resources (backward compatible)
    ///
    /// Scans all access patterns and promotes those that meet the criteria.
    /// Returns the number of resources promoted.
    pub fn auto_promote(&self, controller: &mut EbpfController) -> Result<usize> {
        let result = self.auto_promote_with_stats(controller)?;
        Ok(result.promoted)
    }

    /// Analyze and auto-promote in one operation
    ///
    /// This method is useful when you have policy conditions available.
    /// It analyzes patterns for eBPF compatibility first, then auto-promotes
    /// eligible ones.
    ///
    /// # Arguments
    /// * `controller` - The eBPF controller
    /// * `conditions` - Map of resource paths to their policy conditions
    ///
    /// # Returns
    /// Returns the promotion result with statistics
    pub fn analyze_and_auto_promote(
        &self,
        controller: &mut EbpfController,
        conditions: &std::collections::HashMap<String, &policy_engine::reap::ReapCondition>,
    ) -> Result<AutoPromotionResult> {
        // First, analyze all patterns that have conditions
        let mut analyzed = 0;
        for (resource, condition) in conditions {
            if self.patterns.contains_key(resource) {
                self.analyze_pattern(resource, condition);
                analyzed += 1;
            }
        }

        info!("Analyzed {} patterns for eBPF compatibility", analyzed);

        // Then auto-promote with eBPF checks
        self.auto_promote_with_stats(controller)
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

        resources.sort_by_key(|b| std::cmp::Reverse(b.1));
        resources.truncate(n);
        resources
    }

    /// Get statistics about the learning engine
    pub fn get_stats(&self) -> LearningStats {
        let total_patterns = self.pattern_count();
        let promoted = self.promoted_count();
        let rejected = self.rejected_count.load(Ordering::Relaxed);

        let mut stable_count = 0;
        let mut unstable_count = 0;
        let mut eligible_count = 0;
        let mut analyzed_count = 0;
        let mut ebpf_compatible_count = 0;
        let mut ebpf_incompatible_count = 0;

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

            // Track eBPF compatibility analysis
            if let Some(promotable) = pattern.ebpf_promotable {
                analyzed_count += 1;
                if promotable {
                    ebpf_compatible_count += 1;
                } else {
                    ebpf_incompatible_count += 1;
                }
            }
        }

        LearningStats {
            total_patterns,
            promoted_patterns: promoted,
            rejected_patterns: rejected,
            stable_patterns: stable_count,
            unstable_patterns: unstable_count,
            eligible_for_promotion: eligible_count,
            analyzed_patterns: analyzed_count,
            ebpf_compatible_patterns: ebpf_compatible_count,
            ebpf_incompatible_patterns: ebpf_incompatible_count,
            promotion_threshold: self.promotion_threshold,
            stability_window: self.stability_window,
        }
    }

    /// Get patterns eligible for promotion
    pub fn get_eligible_patterns(&self) -> Vec<(String, AccessPattern)> {
        self.patterns
            .iter()
            .filter(|entry| self.should_promote(entry.key()))
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get patterns that are eBPF-compatible
    pub fn get_ebpf_compatible_patterns(&self) -> Vec<(String, AccessPattern)> {
        self.patterns
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .ebpf_promotable
                    .is_some_and(|promotable| promotable)
            })
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get patterns that are eBPF-incompatible
    pub fn get_ebpf_incompatible_patterns(&self) -> Vec<(String, AccessPattern)> {
        self.patterns
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .ebpf_promotable
                    .is_some_and(|promotable| !promotable)
            })
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Clear all learning data
    pub fn clear(&self) {
        self.patterns.clear();
        self.promoted.clear();
        self.promoted_count.store(0, Ordering::Relaxed);
        self.rejected_count.store(0, Ordering::Relaxed);
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

    /// Number of patterns rejected due to eBPF incompatibility
    pub rejected_patterns: usize,

    /// Number of stable patterns (consistent decisions)
    pub stable_patterns: usize,

    /// Number of unstable patterns (inconsistent decisions)
    pub unstable_patterns: usize,

    /// Number of patterns eligible for promotion
    pub eligible_for_promotion: usize,

    /// Number of patterns that have been analyzed for eBPF compatibility
    pub analyzed_patterns: usize,

    /// Number of analyzed patterns that are eBPF-compatible
    pub ebpf_compatible_patterns: usize,

    /// Number of analyzed patterns that are eBPF-incompatible
    pub ebpf_incompatible_patterns: usize,

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

    #[test]
    fn test_record_access_with_context() {
        let engine = LearningEngine::with_defaults();

        // Record access with policy context
        for _ in 0..10 {
            engine.record_access_with_context(
                "/api/users",
                PolicyAction::Allow,
                Some(1000),
                None,
                Some("admin_rule".to_string()),
            );
        }

        let pattern = engine.get_pattern("/api/users").unwrap();
        assert_eq!(pattern.count, 10);
        assert_eq!(pattern.policy_rule, Some("admin_rule".to_string()));
        assert_eq!(pattern.uid, Some(1000));
    }

    #[test]
    fn test_analyze_pattern_promotable() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::with_defaults();

        // Record accesses
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
        }

        // Analyze with simple condition (should be promotable)
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::Equal,
            right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
        };

        let promotable = engine.analyze_pattern("/api/users", &condition);
        assert!(promotable);

        // Check pattern was updated
        let pattern = engine.get_pattern("/api/users").unwrap();
        assert_eq!(pattern.ebpf_promotable, Some(true));
        assert!(pattern.complexity.is_some());
        assert!(pattern.estimated_latency_ns.is_some());
        assert!(!pattern.entity_lookups.is_empty());
    }

    #[test]
    fn test_analyze_pattern_not_promotable() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::with_defaults();

        // Record accesses
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
        }

        // Analyze with complex condition (too many entity lookups)
        let condition = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "department".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "clearance_level".to_string(),
                    index: None,
                }),
                op: Operator::GreaterEqual,
                right: ComparisonRight::Value(ReapValue::Integer(3)),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Resource,
                    attribute: "classification".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("confidential".to_string())),
            },
        ]);

        let promotable = engine.analyze_pattern("/api/users", &condition);
        assert!(!promotable);

        // Check pattern was updated with blocking reasons
        let pattern = engine.get_pattern("/api/users").unwrap();
        assert_eq!(pattern.ebpf_promotable, Some(false));
        assert!(!pattern.blocking_reasons.is_empty());
    }

    #[test]
    fn test_enhanced_stats() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::with_defaults();

        // Create 3 patterns
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
            engine.record_access("/api/posts", PolicyAction::Allow, None, None);
            engine.record_access("/api/comments", PolicyAction::Deny, None, None);
        }

        // Analyze two of them
        let simple_condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::Equal,
            right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
        };

        let complex_condition = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "department".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "clearance_level".to_string(),
                    index: None,
                }),
                op: Operator::GreaterEqual,
                right: ComparisonRight::Value(ReapValue::Integer(3)),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Resource,
                    attribute: "classification".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("confidential".to_string())),
            },
        ]);

        engine.analyze_pattern("/api/users", &simple_condition);
        engine.analyze_pattern("/api/posts", &complex_condition);

        // Get stats
        let stats = engine.get_stats();
        assert_eq!(stats.total_patterns, 3);
        assert_eq!(stats.stable_patterns, 3);
        assert_eq!(stats.analyzed_patterns, 2);
        assert_eq!(stats.ebpf_compatible_patterns, 1);
        assert_eq!(stats.ebpf_incompatible_patterns, 1);
    }

    #[test]
    fn test_get_ebpf_compatible_patterns() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::with_defaults();

        // Create patterns
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
            engine.record_access("/api/posts", PolicyAction::Allow, None, None);
        }

        // Analyze one as promotable
        let condition = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::Equal,
            right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
        };
        engine.analyze_pattern("/api/users", &condition);

        // Get eBPF-compatible patterns
        let compatible = engine.get_ebpf_compatible_patterns();
        assert_eq!(compatible.len(), 1);
        assert_eq!(compatible[0].0, "/api/users");
    }

    #[test]
    fn test_should_promote_with_ebpf_check() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::new(5, 5); // Low thresholds

        // Create pattern
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
        }

        // Before analysis, should be eligible (no eBPF check yet)
        assert!(engine.should_promote("/api/users"));

        // Analyze as not promotable
        let complex_condition = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "department".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "clearance_level".to_string(),
                    index: None,
                }),
                op: Operator::GreaterEqual,
                right: ComparisonRight::Value(ReapValue::Integer(3)),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Resource,
                    attribute: "classification".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("confidential".to_string())),
            },
        ]);
        engine.analyze_pattern("/api/users", &complex_condition);

        // After analysis, should NOT be eligible (eBPF incompatible)
        assert!(!engine.should_promote("/api/users"));
    }

    #[test]
    fn test_auto_promotion_result() {
        let result = AutoPromotionResult {
            promoted: 5,
            skipped_incompatible: 3,
            failed: 1,
        };

        assert_eq!(result.total(), 9);
        assert!((result.success_rate() - 55.555).abs() < 0.01);
    }

    #[test]
    fn test_promote_to_ebpf_checked_skip_incompatible() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::new(5, 5);

        // Create pattern
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
        }

        // Analyze as not promotable (too many lookups)
        let complex_condition = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "department".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "clearance_level".to_string(),
                    index: None,
                }),
                op: Operator::GreaterEqual,
                right: ComparisonRight::Value(ReapValue::Integer(3)),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Resource,
                    attribute: "classification".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("confidential".to_string())),
            },
        ]);
        engine.analyze_pattern("/api/users", &complex_condition);

        // Pattern should not be promotable
        let pattern = engine.get_pattern("/api/users").unwrap();
        assert_eq!(pattern.ebpf_promotable, Some(false));

        // Try to promote - should skip
        // (We can't test with real controller, but we can check the pattern state)
        assert!(!engine.should_promote("/api/users"));
    }

    #[test]
    fn test_promotion_stats() {
        use policy_engine::reap::{
            ComparisonLeft, ComparisonRight, Entity, EntityAttr, Operator, ReapCondition, ReapValue,
        };

        let engine = LearningEngine::new(5, 5);

        // Create multiple patterns
        for _ in 0..100 {
            engine.record_access("/api/users", PolicyAction::Allow, None, None);
            engine.record_access("/api/posts", PolicyAction::Allow, None, None);
            engine.record_access("/api/comments", PolicyAction::Deny, None, None);
        }

        // Analyze: one promotable, one not
        let simple = ReapCondition::Comparison {
            left: ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            op: Operator::Equal,
            right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
        };

        let complex = ReapCondition::And(vec![
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "role".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("admin".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "department".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("engineering".to_string())),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::User,
                    attribute: "clearance_level".to_string(),
                    index: None,
                }),
                op: Operator::GreaterEqual,
                right: ComparisonRight::Value(ReapValue::Integer(3)),
            },
            ReapCondition::Comparison {
                left: ComparisonLeft::EntityAttr(EntityAttr {
                    entity: Entity::Resource,
                    attribute: "classification".to_string(),
                    index: None,
                }),
                op: Operator::Equal,
                right: ComparisonRight::Value(ReapValue::String("confidential".to_string())),
            },
        ]);

        engine.analyze_pattern("/api/users", &simple);
        engine.analyze_pattern("/api/posts", &complex);

        // Check promotion eligibility
        assert!(engine.should_promote("/api/users")); // Simple, should promote
        assert!(!engine.should_promote("/api/posts")); // Complex, should not promote
        assert!(engine.should_promote("/api/comments")); // Not analyzed, but meets other criteria

        // Get stats
        let stats = engine.get_stats();
        assert_eq!(stats.ebpf_compatible_patterns, 1);
        assert_eq!(stats.ebpf_incompatible_patterns, 1);
    }
}
