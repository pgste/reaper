//! Change request domain model (Plan 10 Phase B).
//!
//! A `ChangeRequest` is a pending env→env promotion: a bundle (and the source
//! environment's data version) pinned for deployment into a higher-tier
//! environment, gated by that environment's approval policy and change
//! windows. It stays `Pending` until enough distinct approvers sign off, then
//! `Approved` → the existing rollout machinery runs → `Applied`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Lifecycle status of a change request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeRequestStatus {
    Pending,
    Approved,
    Rejected,
    Applied,
    Cancelled,
}

impl ChangeRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Applied => "applied",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "approved" => Self::Approved,
            "rejected" => Self::Rejected,
            "applied" => Self::Applied,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// A governed env→env promotion request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangeRequest {
    pub id: Uuid,
    pub org_id: Uuid,
    pub from_env_id: Uuid,
    pub to_env_id: Uuid,
    pub bundle_id: Uuid,
    /// The source environment's data-plane version pinned at request time, so
    /// policy and data promote together (Plan 10 Step 7). `None` if the source
    /// namespace has no datastore.
    pub data_version: Option<i64>,
    /// Rollout strategy to use on apply (`None` = target-env/org default).
    pub strategy_id: Option<Uuid>,
    pub status: ChangeRequestStatus,
    pub requested_by: String,
    /// Set when the promotion is applied (the rollout that carried it).
    pub rollout_id: Option<Uuid>,
    /// Rejection / cancellation reason.
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// A recorded approver decision on a change request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangeApproval {
    pub id: Uuid,
    pub change_request_id: Uuid,
    pub approver_id: String,
    pub decision: ApprovalDecision,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

impl ApprovalDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "reject" => Self::Reject,
            _ => Self::Approve,
        }
    }
}

/// Input to create a change request (a promotion).
#[derive(Debug, Clone)]
pub struct CreateChangeRequest {
    pub from_env_id: Uuid,
    pub to_env_id: Uuid,
    pub bundle_id: Uuid,
    pub data_version: Option<i64>,
    pub strategy_id: Option<Uuid>,
    pub requested_by: String,
}
