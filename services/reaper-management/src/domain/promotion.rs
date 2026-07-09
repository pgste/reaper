//! Governed promotion change requests (Plan 02, Phase B, step 5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether a change request promotes a bundle forward or rolls back to a
/// previously-recorded bundle. Both flow through the same two-person approval
/// and the same agent-side verified load path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Promote,
    Rollback,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Promote => "promote",
            ChangeKind::Rollback => "rollback",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "promote" => Some(ChangeKind::Promote),
            "rollback" => Some(ChangeKind::Rollback),
            _ => None,
        }
    }
}

/// Lifecycle of a promotion change request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    /// Opened by the requester; awaiting a second, distinct approver.
    Pending,
    /// Approved by a distinct principal and the promotion applied.
    Executed,
    /// Rejected (or superseded) — never applied.
    Rejected,
}

impl ChangeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeStatus::Pending => "pending",
            ChangeStatus::Executed => "executed",
            ChangeStatus::Rejected => "rejected",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ChangeStatus::Pending),
            "executed" => Some(ChangeStatus::Executed),
            "rejected" => Some(ChangeStatus::Rejected),
            _ => None,
        }
    }
}

/// A promotion (or rollback) awaiting / having received two-person approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionChangeRequest {
    pub id: Uuid,
    pub org_id: Uuid,
    pub bundle_id: Uuid,
    /// Bundle version string pinned at request time (audit trail).
    pub bundle_version: Option<String>,
    pub kind: ChangeKind,
    pub status: ChangeStatus,
    /// Actor id (user / api-key / agent) that opened the request.
    pub requester_id: String,
    /// Distinct actor id that approved and executed it.
    pub approver_id: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}
