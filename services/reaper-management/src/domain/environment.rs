//! Environment domain model (Plan 10).
//!
//! An `Environment` is a first-class lifecycle object (dev < staging < prod)
//! layered *over* a namespace — the namespace stays the deployment/data scope
//! unit; the environment adds ordering, an approval policy, and change
//! windows/freezes so promotion between environments is a governed transition
//! (ADR-1). One environment binds exactly one namespace.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::scopes::Scope;

/// A deployment environment bound one-to-one to a namespace.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Environment {
    pub id: Uuid,
    pub org_id: Uuid,
    /// Human-readable name (unique per org), e.g. "production".
    pub name: String,
    /// Lifecycle ordering: promotion is allowed only to a strictly higher
    /// tier (dev=0 < staging=10 < prod=20). Values are arbitrary ints so tiers
    /// can be inserted between existing ones.
    pub tier_order: i32,
    /// The namespace this environment deploys into (unique across envs).
    pub namespace_id: Uuid,
    /// Optional data-plane binding (datastore id) carried on promotion so
    /// policy and data move together (Plan 10 Step 7).
    pub data_plane_ref: Option<String>,
    /// Approval policy gating promotions *into* this environment.
    pub approval_policy: ApprovalPolicy,
    /// Change windows / freeze periods restricting *when* a promotion applies.
    pub change_windows: ChangeWindows,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Environment {
    /// Whether a bundle in `self` may be promoted **to** `target`. Promotion is
    /// strictly upward — a higher tier_order — so prod→dev or same-tier moves
    /// are rejected (Plan 10 Step 5).
    pub fn can_promote_to(&self, target: &Environment) -> bool {
        target.tier_order > self.tier_order
    }
}

/// Approval policy gating promotion into an environment (Plan 10 Step 3).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApprovalPolicy {
    /// Minimum number of approvals required. 0 = auto-approve (e.g. dev).
    #[serde(default)]
    pub min_approvers: u8,
    /// If true, an approval by the requester does not count toward the
    /// minimum (two-person control).
    #[serde(default = "default_true")]
    pub distinct_from_requester: bool,
    /// Scopes an approver must hold for their approval to count.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub required_scopes: Vec<Scope>,
    /// If true, every deployment into this environment's namespace must go
    /// through the governed promotion path (change record + approvals) — a
    /// direct rollout is rejected with 409. Platform `admin` keys bypass as
    /// an audited break-glass. Default false, so small deployments keep the
    /// frictionless direct path unless an org opts in.
    #[serde(default)]
    pub require_change_record: bool,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            min_approvers: 0,
            distinct_from_requester: true,
            required_scopes: Vec::new(),
            require_change_record: false,
        }
    }
}

/// Outcome of evaluating an approval policy against the recorded approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Enough qualifying, distinct approvals — the change may proceed.
    Satisfied,
    /// Not yet enough approvals.
    Pending { have: usize, need: u8 },
}

impl ApprovalPolicy {
    /// Evaluate the policy given the requester and the set of approver ids that
    /// have approved (each already verified to hold `required_scopes` by the
    /// caller). Distinct approver identities are counted; the requester is
    /// excluded when `distinct_from_requester`.
    pub fn evaluate(&self, requester: Uuid, approver_ids: &[Uuid]) -> ApprovalOutcome {
        use std::collections::BTreeSet;
        let mut distinct: BTreeSet<Uuid> = approver_ids.iter().copied().collect();
        if self.distinct_from_requester {
            distinct.remove(&requester);
        }
        let have = distinct.len();
        if have >= self.min_approvers as usize {
            ApprovalOutcome::Satisfied
        } else {
            ApprovalOutcome::Pending {
                have,
                need: self.min_approvers,
            }
        }
    }
}

/// Change windows / freeze periods for an environment (Plan 10 Step 3).
///
/// `freeze_periods` are absolute [start, end) intervals during which no
/// promotion may apply (e.g. a holiday code freeze). Kept deliberately simple
/// (absolute intervals) — recurring weekly windows can be layered later.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ChangeWindows {
    #[serde(default)]
    pub freeze_periods: Vec<FreezePeriod>,
}

/// An absolute freeze interval `[start, end)`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FreezePeriod {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Whether a change is permitted at instant `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowDecision {
    Allowed,
    InFreeze { reason: Option<String> },
}

impl ChangeWindows {
    /// Is a change allowed at `now`? Blocked if `now` falls in any freeze
    /// period `[start, end)`.
    pub fn is_change_allowed(&self, now: DateTime<Utc>) -> WindowDecision {
        for f in &self.freeze_periods {
            if now >= f.start && now < f.end {
                return WindowDecision::InFreeze {
                    reason: f.reason.clone(),
                };
            }
        }
        WindowDecision::Allowed
    }
}

fn default_true() -> bool {
    true
}

/// Input for creating an environment.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEnvironment {
    pub name: String,
    pub tier_order: i32,
    pub namespace_id: Uuid,
    #[serde(default)]
    pub data_plane_ref: Option<String>,
    #[serde(default)]
    pub approval_policy: ApprovalPolicy,
    #[serde(default)]
    pub change_windows: ChangeWindows,
}

/// Input for updating an environment.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateEnvironment {
    pub name: Option<String>,
    pub tier_order: Option<i32>,
    pub data_plane_ref: Option<String>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub change_windows: Option<ChangeWindows>,
    pub is_active: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn env(name: &str, tier: i32) -> Environment {
        Environment {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: name.to_string(),
            tier_order: tier,
            namespace_id: Uuid::new_v4(),
            data_plane_ref: None,
            approval_policy: ApprovalPolicy::default(),
            change_windows: ChangeWindows::default(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn promotion_is_strictly_upward() {
        let dev = env("dev", 0);
        let staging = env("staging", 10);
        let prod = env("prod", 20);
        assert!(dev.can_promote_to(&staging));
        assert!(staging.can_promote_to(&prod));
        assert!(dev.can_promote_to(&prod));
        // Downward and same-tier are rejected.
        assert!(!prod.can_promote_to(&dev));
        assert!(!prod.can_promote_to(&staging));
        assert!(!staging.can_promote_to(&env("staging2", 10)));
    }

    #[test]
    fn approval_two_person_distinct_from_requester() {
        let policy = ApprovalPolicy {
            min_approvers: 2,
            distinct_from_requester: true,
            ..Default::default()
        };
        let requester = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        // No approvals yet.
        assert_eq!(
            policy.evaluate(requester, &[]),
            ApprovalOutcome::Pending { have: 0, need: 2 }
        );
        // The requester approving themselves doesn't count.
        assert_eq!(
            policy.evaluate(requester, &[requester]),
            ApprovalOutcome::Pending { have: 0, need: 2 }
        );
        // One distinct approver: still short.
        assert_eq!(
            policy.evaluate(requester, &[a]),
            ApprovalOutcome::Pending { have: 1, need: 2 }
        );
        // Duplicate approvals from the same person count once.
        assert_eq!(
            policy.evaluate(requester, &[a, a]),
            ApprovalOutcome::Pending { have: 1, need: 2 }
        );
        // Two distinct approvers (neither the requester): satisfied.
        assert_eq!(
            policy.evaluate(requester, &[a, b]),
            ApprovalOutcome::Satisfied
        );
        // Requester + one other still only counts the other.
        assert_eq!(
            policy.evaluate(requester, &[requester, a]),
            ApprovalOutcome::Pending { have: 1, need: 2 }
        );
    }

    #[test]
    fn approval_self_approve_allowed_when_not_distinct() {
        let policy = ApprovalPolicy {
            min_approvers: 1,
            distinct_from_requester: false,
            ..Default::default()
        };
        let requester = Uuid::new_v4();
        assert_eq!(
            policy.evaluate(requester, &[requester]),
            ApprovalOutcome::Satisfied
        );
    }

    #[test]
    fn require_change_record_defaults_off_for_stored_policies() {
        // Environments created before the flag existed have approval_policy
        // JSON without it — they must keep the frictionless direct-rollout
        // default rather than suddenly enforcing the promotion path.
        let policy: ApprovalPolicy =
            serde_json::from_str(r#"{"min_approvers": 2, "distinct_from_requester": true}"#)
                .unwrap();
        assert!(!policy.require_change_record);
        assert!(!ApprovalPolicy::default().require_change_record);

        let opted_in: ApprovalPolicy =
            serde_json::from_str(r#"{"require_change_record": true}"#).unwrap();
        assert!(opted_in.require_change_record);
    }

    #[test]
    fn zero_approvers_is_auto_satisfied() {
        let policy = ApprovalPolicy::default(); // min_approvers = 0
        assert_eq!(
            policy.evaluate(Uuid::new_v4(), &[]),
            ApprovalOutcome::Satisfied
        );
    }

    #[test]
    fn freeze_period_blocks_only_within_interval() {
        let start = Utc.with_ymd_and_hms(2026, 12, 24, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 12, 26, 0, 0, 0).unwrap();
        let windows = ChangeWindows {
            freeze_periods: vec![FreezePeriod {
                start,
                end,
                reason: Some("holiday freeze".to_string()),
            }],
        };

        let before = Utc.with_ymd_and_hms(2026, 12, 23, 23, 59, 0).unwrap();
        let during = Utc.with_ymd_and_hms(2026, 12, 25, 12, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 12, 26, 0, 0, 0).unwrap(); // end is exclusive

        assert_eq!(windows.is_change_allowed(before), WindowDecision::Allowed);
        assert!(matches!(
            windows.is_change_allowed(during),
            WindowDecision::InFreeze { .. }
        ));
        assert_eq!(windows.is_change_allowed(after), WindowDecision::Allowed);
    }
}
