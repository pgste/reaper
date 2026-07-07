//! Per-policy Prometheus metric-handle cache.
//!
//! `CounterVec::with_label_values` / `HistogramVec::with_label_values` are not
//! free: each call hashes the label-value slice and takes a read lock on the
//! metric vec's internal map to find (or create) the child metric. On the hot
//! evaluate path we touch two label vecs per request (`reaper_decisions_total`
//! keyed by `[decision, policy_name]` and `reaper_decision_duration_seconds`
//! keyed by `[policy_name]`), so that is two hashes + two locked lookups on
//! every single decision.
//!
//! The label values are effectively fixed per policy (a policy's name doesn't
//! change, and there are only three decision outcomes), so we resolve the child
//! handles **once per policy name** and cache the concrete `Counter`/`Histogram`
//! handles. The hot path then does a single `DashMap` lookup and calls `.inc()`
//! / `.observe()` directly on the cached child — no per-request label hashing of
//! the metric vecs.

use dashmap::DashMap;
use policy_engine::PolicyAction;
use prometheus::{Counter, Histogram};
use std::sync::Arc;

use crate::observability::{DECISIONS_TOTAL, DECISION_DURATION};

/// Pre-resolved metric child handles for a single policy.
pub struct PolicyMetricHandles {
    allow: Counter,
    deny: Counter,
    log: Counter,
    /// Decision-latency histogram for this policy.
    pub duration: Histogram,
}

impl PolicyMetricHandles {
    fn resolve(policy_name: &str) -> Self {
        // `with_label_values` returns an Arc-backed handle; resolving all three
        // decision outcomes + the duration histogram once amortizes the label
        // hashing across every future request for this policy.
        Self {
            allow: DECISIONS_TOTAL.with_label_values(&["allow", policy_name]),
            deny: DECISIONS_TOTAL.with_label_values(&["deny", policy_name]),
            log: DECISIONS_TOTAL.with_label_values(&["log", policy_name]),
            duration: DECISION_DURATION.with_label_values(&[policy_name]),
        }
    }

    /// The decision counter for `decision`.
    #[inline]
    pub fn counter(&self, decision: &PolicyAction) -> &Counter {
        match decision {
            PolicyAction::Allow => &self.allow,
            PolicyAction::Deny => &self.deny,
            PolicyAction::Log => &self.log,
        }
    }
}

/// Cache of per-policy metric handles keyed by policy name.
///
/// Lock-free reads on the hot path (`DashMap`), lazily populated on first sight
/// of a policy name. Cardinality is bounded by the number of deployed policies,
/// so this never grows unbounded (unlike per-resource labels, which is why
/// `resource` is deliberately not a metric label anywhere).
#[derive(Default)]
pub struct DecisionMetrics {
    by_policy: DashMap<String, Arc<PolicyMetricHandles>>,
}

impl DecisionMetrics {
    pub fn new() -> Self {
        Self {
            by_policy: DashMap::new(),
        }
    }

    /// Get (or lazily resolve) the cached metric handles for `policy_name`.
    #[inline]
    pub fn for_policy(&self, policy_name: &str) -> Arc<PolicyMetricHandles> {
        if let Some(handles) = self.by_policy.get(policy_name) {
            return handles.clone();
        }
        let handles = Arc::new(PolicyMetricHandles::resolve(policy_name));
        self.by_policy
            .insert(policy_name.to_string(), handles.clone());
        handles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_and_reuses_handles() {
        let metrics = DecisionMetrics::new();
        let a = metrics.for_policy("policy-a");
        let a2 = metrics.for_policy("policy-a");
        // Same cached Arc for the same policy name.
        assert!(Arc::ptr_eq(&a, &a2));

        let b = metrics.for_policy("policy-b");
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn counter_selects_by_decision() {
        let metrics = DecisionMetrics::new();
        let h = metrics.for_policy("sel");
        // Distinct child handles per outcome.
        let allow = h.counter(&PolicyAction::Allow);
        let deny = h.counter(&PolicyAction::Deny);
        allow.inc();
        deny.inc();
        assert_eq!(allow.get() as u64, 1);
        assert_eq!(deny.get() as u64, 1);
    }
}
