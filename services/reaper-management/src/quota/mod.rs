//! Multi-tenant plan-quota enforcement (round-2 E4).
//!
//! Plan limits were advisory: `UsageMetrics` was hardcoded to 0 and nothing was
//! enforced. This module wires **real** per-org usage counts and enforces the
//! plan limits at the create paths (agent register, policy create).
//!
//! The org's plan is persisted in `Organization.settings.plan_tier` (the
//! already-supported settings JSON, no new table); per-org numeric overrides
//! (`max_agents`, `max_policies`, …) in the same blob let an enterprise deal
//! raise a single limit without changing tier. A limit of `-1` is unlimited.

use uuid::Uuid;

use crate::api::error::ApiError;
use crate::db::repositories::{
    AgentRepository, BundleRepository, PolicyRepository, TeamRepository,
};
use crate::db::{Database, DatabaseError};
use crate::domain::billing::{PlanLimits, PlanTier, UsageMetrics};
use crate::domain::organization::Organization;

/// The org's plan tier from `settings.plan_tier` (default [`PlanTier::Free`]).
pub fn plan_tier_of(org: &Organization) -> PlanTier {
    org.get_setting::<String>("plan_tier")
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

/// Effective limits: the tier's limits with any per-org numeric overrides from
/// `settings` applied — so a custom enterprise limit needs no schema change.
pub fn effective_limits(org: &Organization) -> PlanLimits {
    let mut limits = PlanLimits::for_tier(plan_tier_of(org));
    let override_i64 = |key: &str| org.get_setting::<i64>(key);
    if let Some(v) = override_i64("max_agents") {
        limits.max_agents = v;
    }
    if let Some(v) = override_i64("max_policies") {
        limits.max_policies = v;
    }
    if let Some(v) = override_i64("max_users") {
        limits.max_users = v;
    }
    if let Some(v) = override_i64("max_storage_bytes") {
        limits.max_storage_bytes = v;
    }
    if let Some(v) = override_i64("max_evaluations_per_month") {
        limits.max_evaluations_per_month = v;
    }
    limits
}

/// Count real usage for an org across the control-plane-visible dimensions.
/// `policy_evaluations` and `storage_bytes` are not tracked here (they live on
/// the agent data plane) and stay 0.
pub async fn count_usage(db: &Database, org_id: Uuid) -> Result<UsageMetrics, DatabaseError> {
    let now = chrono::Utc::now();
    Ok(UsageMetrics {
        org_id,
        period_start: now,
        period_end: now,
        active_agents: AgentRepository::new(db).count_by_org(org_id).await?,
        policy_evaluations: 0,
        policy_count: PolicyRepository::new(db).count_by_org(org_id).await?,
        bundle_count: BundleRepository::new(db).count_by_org(org_id).await?,
        user_count: TeamRepository::new(db).count_by_org(org_id).await?,
        storage_bytes: 0,
    })
}

/// A quota dimension enforced before creating a resource.
#[derive(Debug, Clone, Copy)]
pub enum Dimension {
    Agents,
    Policies,
}

impl Dimension {
    fn name(self) -> &'static str {
        match self {
            Dimension::Agents => "agents",
            Dimension::Policies => "policies",
        }
    }
}

/// True when `current` already meets or exceeds a finite `limit`. A negative
/// limit (`-1`) is unlimited; 0 is treated as unlimited too, matching the
/// existing `get_billing_summary` overage logic (`limit > 0`).
fn at_or_over(limit: i64, current: i64) -> bool {
    limit > 0 && current >= limit
}

/// Refuse when creating one more of `dim` would exceed the org's plan limit.
/// Returns `Err(ApiError::QuotaExceeded)` (402) on refusal, or a DB error if a
/// count fails (fail-closed: a broken count must not silently admit).
pub async fn enforce_can_add(
    db: &Database,
    org: &Organization,
    dim: Dimension,
) -> Result<(), ApiError> {
    let limits = effective_limits(org);
    let (limit, current) = match dim {
        Dimension::Agents => (
            limits.max_agents,
            AgentRepository::new(db).count_by_org(org.id).await?,
        ),
        Dimension::Policies => (
            limits.max_policies,
            PolicyRepository::new(db).count_by_org(org.id).await?,
        ),
    };
    if at_or_over(limit, current) {
        return Err(ApiError::QuotaExceeded(format!(
            "{} quota reached: {}/{} on the {} plan — upgrade the plan or raise the limit to add more",
            dim.name(),
            current,
            limit,
            plan_tier_of(org),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn org_with(settings: serde_json::Value) -> Organization {
        Organization {
            id: Uuid::new_v4(),
            name: "t".into(),
            slug: "t".into(),
            display_name: None,
            description: None,
            settings,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn plan_tier_defaults_to_free_and_parses_settings() {
        assert_eq!(plan_tier_of(&org_with(json!({}))), PlanTier::Free);
        assert_eq!(
            plan_tier_of(&org_with(json!({ "plan_tier": "professional" }))),
            PlanTier::Professional
        );
        // Unknown tier falls back to Free (fail-safe to the tightest limits).
        assert_eq!(
            plan_tier_of(&org_with(json!({ "plan_tier": "bogus" }))),
            PlanTier::Free
        );
    }

    #[test]
    fn effective_limits_apply_per_org_overrides() {
        // Starter tier, but this org negotiated 25 agents.
        let org = org_with(json!({ "plan_tier": "starter", "max_agents": 25 }));
        let limits = effective_limits(&org);
        assert_eq!(limits.tier, PlanTier::Starter);
        assert_eq!(limits.max_agents, 25, "override wins");
        assert_eq!(limits.max_policies, 50, "un-overridden tier value kept");
    }

    #[test]
    fn at_or_over_treats_negative_and_zero_as_unlimited() {
        assert!(!at_or_over(-1, 1_000_000), "enterprise unlimited");
        assert!(!at_or_over(0, 1_000_000), "0 = unlimited");
        assert!(!at_or_over(10, 9), "under the cap");
        assert!(at_or_over(10, 10), "at the cap: adding one more exceeds");
        assert!(at_or_over(10, 11), "already over");
    }
}
