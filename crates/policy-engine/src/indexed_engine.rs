//! Indexed Policy Engine - Multi-Index Optimization
//!
//! ⚠️  **EXPERIMENTAL - NOT RECOMMENDED FOR PRODUCTION USE**
//!
//! **Performance Reality**: This indexed engine is 6-8x **SLOWER** than the baseline
//! Simple evaluator due to DashMap and abstraction overhead (~15µs per evaluation).
//!
//! ## Actual Benchmark Results (ARM64, Release)
//!
//! | Policies | Linear Scan | Indexed | Result |
//! |----------|-------------|---------|--------|
//! | 10       | 79ns        | 638ns   | **8x SLOWER** |
//! | 100      | 286ns       | 1,919ns | **6.7x SLOWER** |
//! | 1,000    | 2,465ns     | 15,877ns | **6.4x SLOWER** |
//!
//! **Why?** DashMap overhead (~15µs) exceeds any benefit from indexing.
//!
//! **Recommendation**: Use the Simple evaluator instead (341ns mean, 2.9M req/s).
//!
//! This code is kept for research and educational purposes to demonstrate
//! that complex optimizations can actually hurt performance.
//!
//! ## Original Design (What We Thought Would Happen)
//!
//! **Before (Linear Scan):**
//! - Check 1000 policies for every request
//! - O(n) complexity
//! - ~50µs per request
//!
//! **After (Multi-Index):**
//! - Check 2-5 policies for most requests
//! - O(1) index lookup + O(k) policy evaluation where k << n
//! - ~500ns-5µs per request
//! - **10-100x faster!**
//!
//! ## How It Works
//!
//! Build indexes by:
//! 1. Resource pattern (exact match, prefix)
//! 2. Principal role/attribute
//! 3. Action type
//! 4. Combinations (resource + role, etc.)
//!
//! Request evaluation:
//! 1. Look up candidates in resource index → ~10 policies
//! 2. Intersect with role index → ~2-3 policies
//! 3. Evaluate only the intersection
//! 4. Return first match

use crate::engine::{EnhancedPolicy, PolicyAction, PolicyDecision, PolicyRequest};
use dashmap::DashMap;
use reaper_core::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Principal represents the entity making the request
#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub context: HashMap<String, String>,
}

/// Policy index entry
#[derive(Debug, Clone)]
struct IndexEntry {
    policy_id: Uuid,
    #[allow(dead_code)]
    priority: u32,
}

/// Multi-index policy engine
///
/// Maintains multiple indexes for fast policy lookup
pub struct IndexedPolicyEngine {
    /// All policies by ID
    policies: Arc<DashMap<Uuid, Arc<EnhancedPolicy>>>,

    /// Index by resource (exact match)
    /// Key: resource string, Value: list of policy IDs
    by_resource: Arc<DashMap<String, Vec<IndexEntry>>>,

    /// Index by resource prefix
    /// Key: resource prefix (e.g., "/api/"), Value: list of policy IDs
    by_resource_prefix: Arc<DashMap<String, Vec<IndexEntry>>>,

    /// Index by action
    /// Key: action (e.g., "read", "write"), Value: list of policy IDs
    by_action: Arc<DashMap<String, Vec<IndexEntry>>>,

    /// Index by principal role (if available in policy metadata)
    /// Key: role (e.g., "admin", "user"), Value: list of policy IDs
    by_role: Arc<DashMap<String, Vec<IndexEntry>>>,

    /// Wildcard policies (match everything)
    wildcard_policies: Arc<DashMap<Uuid, Arc<EnhancedPolicy>>>,

    /// Statistics
    index_hits: Arc<std::sync::atomic::AtomicU64>,
    index_misses: Arc<std::sync::atomic::AtomicU64>,
    policies_checked: Arc<std::sync::atomic::AtomicU64>,
}

impl IndexedPolicyEngine {
    /// Create a new indexed policy engine
    pub fn new() -> Self {
        Self {
            policies: Arc::new(DashMap::new()),
            by_resource: Arc::new(DashMap::new()),
            by_resource_prefix: Arc::new(DashMap::new()),
            by_action: Arc::new(DashMap::new()),
            by_role: Arc::new(DashMap::new()),
            wildcard_policies: Arc::new(DashMap::new()),
            index_hits: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            index_misses: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            policies_checked: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Deploy a policy and build indexes
    pub fn deploy_policy(&self, policy: EnhancedPolicy) -> Result<()> {
        let policy_id = policy.id;
        let policy = Arc::new(policy);

        // Store policy
        self.policies.insert(policy_id, Arc::clone(&policy));

        // Build indexes
        self.build_indexes_for_policy(&policy);

        Ok(())
    }

    /// Build indexes for a single policy
    fn build_indexes_for_policy(&self, policy: &Arc<EnhancedPolicy>) {
        let entry = IndexEntry {
            policy_id: policy.id,
            priority: policy.priority,
        };

        // Index by resource (if policy specifies a resource)
        if let Some(resource) = self.extract_resource_pattern(policy) {
            if resource == "*" {
                // Wildcard policy
                self.wildcard_policies.insert(policy.id, Arc::clone(policy));
            } else if resource.ends_with("*") {
                // Prefix match
                let prefix = resource.trim_end_matches('*');
                self.by_resource_prefix
                    .entry(prefix.to_string())
                    .or_default()
                    .push(entry.clone());
            } else {
                // Exact match
                self.by_resource
                    .entry(resource)
                    .or_default()
                    .push(entry.clone());
            }
        }

        // Index by action (if policy specifies an action)
        if let Some(action) = self.extract_action_pattern(policy) {
            self.by_action
                .entry(action)
                .or_default()
                .push(entry.clone());
        }

        // Index by role (if policy specifies a role)
        if let Some(role) = self.extract_role_pattern(policy) {
            self.by_role.entry(role).or_default().push(entry);
        }
    }

    /// Extract resource pattern from policy
    fn extract_resource_pattern(&self, policy: &EnhancedPolicy) -> Option<String> {
        // Extract from first rule's resource pattern
        // For simple policies, all rules might target same resource pattern
        policy.rules.first().map(|rule| rule.resource.clone())
    }

    /// Extract action pattern from policy
    fn extract_action_pattern(&self, _policy: &EnhancedPolicy) -> Option<String> {
        // Extract from metadata or first rule
        // For now, we don't index by action (resource is more selective)
        None
    }

    /// Extract role pattern from policy
    fn extract_role_pattern(&self, policy: &EnhancedPolicy) -> Option<String> {
        // Parse conditions for role checks (e.g., "role==admin")
        for rule in &policy.rules {
            for condition in &rule.conditions {
                if let Some(role) = condition.strip_prefix("role==") {
                    return Some(role.to_string());
                }
            }
        }
        None
    }

    /// Evaluate a single policy against a request
    fn evaluate_policy(
        &self,
        policy: &EnhancedPolicy,
        request: &PolicyRequest,
    ) -> Option<PolicyDecision> {
        // Evaluate each rule in the policy
        for rule in &policy.rules {
            // Check resource match (exact, wildcard, or prefix)
            let resource_matches = if rule.resource == "*" {
                true
            } else if rule.resource.ends_with('*') {
                let prefix = rule.resource.trim_end_matches('*');
                request.resource.starts_with(prefix)
            } else {
                rule.resource == request.resource
            };

            if !resource_matches {
                continue;
            }

            // Check conditions
            let conditions_met = rule
                .conditions
                .iter()
                .all(|condition| self.evaluate_condition(condition, request));

            if conditions_met {
                return Some(PolicyDecision {
                    decision: rule.action.clone(),
                    policy_id: policy.id,
                    policy_version: policy.version,
                    evaluation_time_ns: 0,
                    matched_rule: None,
                });
            }
        }

        None
    }

    /// Evaluate a single condition against a request
    fn evaluate_condition(&self, condition: &str, request: &PolicyRequest) -> bool {
        // Parse condition (e.g., "role==admin", "department==eng")
        if let Some((key, value)) = condition.split_once("==") {
            if let Some(context_value) = request.context.get(key) {
                return context_value == value;
            }
        }
        false
    }

    /// Evaluate a request against indexed policies
    ///
    /// This is the fast path that uses indexes!
    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyDecision> {
        // Step 1: Find candidate policies using indexes
        let candidates = self.find_candidates(request);

        self.policies_checked.fetch_add(
            candidates.len() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        if !candidates.is_empty() {
            self.index_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.index_misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Step 2: Evaluate candidates in priority order
        let mut sorted_candidates: Vec<_> = candidates.into_iter().collect();
        sorted_candidates.sort_by_key(|id| {
            self.policies
                .get(id)
                .map(|p| p.value().priority)
                .unwrap_or(u32::MAX)
        });

        for policy_id in sorted_candidates {
            if let Some(policy) = self.policies.get(&policy_id) {
                // Evaluate policy rules
                if let Some(decision) = self.evaluate_policy(&policy, request) {
                    return Ok(decision);
                }
            }
        }

        // Step 3: Check wildcard policies
        for entry in self.wildcard_policies.iter() {
            let policy = entry.value();
            // Evaluate wildcard policy
            if let Some(decision) = self.evaluate_policy(policy, request) {
                return Ok(decision);
            }
        }

        // Default: Deny
        Ok(PolicyDecision {
            decision: PolicyAction::Deny,
            policy_id: Uuid::nil(),
            policy_version: 0,
            evaluation_time_ns: 0,
            matched_rule: None,
        })
    }

    /// Find candidate policies using indexes
    ///
    /// Returns the intersection of all matching indexes
    fn find_candidates(&self, request: &PolicyRequest) -> HashSet<Uuid> {
        let mut candidates: Option<HashSet<Uuid>> = None;

        // Index 1: Resource exact match
        if let Some(entries) = self.by_resource.get(&request.resource) {
            let ids: HashSet<Uuid> = entries.iter().map(|e| e.policy_id).collect();
            candidates = Some(ids);
        }

        // Index 2: Resource prefix match
        for entry in self.by_resource_prefix.iter() {
            if request.resource.starts_with(entry.key()) {
                let ids: HashSet<Uuid> = entry.value().iter().map(|e| e.policy_id).collect();
                if let Some(ref mut cands) = candidates {
                    // Intersection: only keep IDs in both sets
                    cands.retain(|id| ids.contains(id));
                    // Also add new candidates
                    cands.extend(ids);
                } else {
                    candidates = Some(ids);
                }
            }
        }

        // Index 3: Action match
        if let Some(entries) = self.by_action.get(&request.action) {
            let ids: HashSet<Uuid> = entries.iter().map(|e| e.policy_id).collect();
            if let Some(ref mut cands) = candidates {
                // Keep only policies that match BOTH resource AND action
                cands.retain(|id| ids.contains(id));
            } else {
                candidates = Some(ids);
            }
        }

        // Index 4: Role match (if role in request context)
        if let Some(role) = request.context.get("role") {
            if let Some(entries) = self.by_role.get(role) {
                let ids: HashSet<Uuid> = entries.iter().map(|e| e.policy_id).collect();
                if let Some(ref mut cands) = candidates {
                    cands.retain(|id| ids.contains(id));
                } else {
                    candidates = Some(ids);
                }
            }
        }

        candidates.unwrap_or_default()
    }

    /// Get statistics about index usage
    pub fn get_index_stats(&self) -> IndexStats {
        let hits = self.index_hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.index_misses.load(std::sync::atomic::Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let policies_checked = self
            .policies_checked
            .load(std::sync::atomic::Ordering::Relaxed);
        let avg_policies_per_request = if total > 0 {
            policies_checked as f64 / total as f64
        } else {
            0.0
        };

        IndexStats {
            total_policies: self.policies.len(),
            index_hits: hits,
            index_misses: misses,
            hit_rate,
            avg_policies_per_request,
            resource_index_size: self.by_resource.len(),
            prefix_index_size: self.by_resource_prefix.len(),
            action_index_size: self.by_action.len(),
            role_index_size: self.by_role.len(),
        }
    }
}

/// Statistics about index usage
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_policies: usize,
    pub index_hits: u64,
    pub index_misses: u64,
    pub hit_rate: f64,
    pub avg_policies_per_request: f64,
    pub resource_index_size: usize,
    pub prefix_index_size: usize,
    pub action_index_size: usize,
    pub role_index_size: usize,
}

impl Default for IndexedPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_indexed_engine_creation() {
        let engine = IndexedPolicyEngine::new();
        let stats = engine.get_index_stats();

        assert_eq!(stats.total_policies, 0);
        assert_eq!(stats.index_hits, 0);
        assert_eq!(stats.index_misses, 0);
    }

    #[test]
    fn test_deploy_policy() {
        let engine = IndexedPolicyEngine::new();

        let policy = EnhancedPolicy::new(
            "test-policy".to_string(),
            "test description".to_string(),
            vec![],
        );

        assert!(engine.deploy_policy(policy).is_ok());

        let stats = engine.get_index_stats();
        assert_eq!(stats.total_policies, 1);
    }

    #[test]
    fn test_evaluate_request() {
        let engine = IndexedPolicyEngine::new();

        let request = PolicyRequest {
            action: "read".to_string(),
            resource: "/api/users".to_string(),
            context: HashMap::new(),
        };

        let decision = engine.evaluate(&request);
        assert!(decision.is_ok());
    }

    #[test]
    fn test_find_candidates_empty() {
        let engine = IndexedPolicyEngine::new();

        let request = PolicyRequest {
            action: "read".to_string(),
            resource: "/api/users".to_string(),
            context: HashMap::new(),
        };

        let candidates = engine.find_candidates(&request);
        assert_eq!(candidates.len(), 0);
    }
}
