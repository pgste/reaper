//! Bundle domain model
//!
//! Bundles are compiled policy packages with promotion workflow.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bundle status in the promotion workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleStatus {
    /// Initial state, policies being selected
    Draft,
    /// .rpp bundle generated and stored
    Compiled,
    /// Ready for testing, not yet live
    Staged,
    /// Live - agents notified via SSE
    Promoted,
    /// Marked for removal
    Deprecated,
}

impl Default for BundleStatus {
    fn default() -> Self {
        Self::Draft
    }
}

impl std::fmt::Display for BundleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Compiled => write!(f, "compiled"),
            Self::Staged => write!(f, "staged"),
            Self::Promoted => write!(f, "promoted"),
            Self::Deprecated => write!(f, "deprecated"),
        }
    }
}

impl std::str::FromStr for BundleStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "draft" => Ok(Self::Draft),
            "compiled" => Ok(Self::Compiled),
            "staged" => Ok(Self::Staged),
            "promoted" => Ok(Self::Promoted),
            "deprecated" => Ok(Self::Deprecated),
            _ => Err(format!("Unknown bundle status: {}", s)),
        }
    }
}

/// Bundle entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub status: BundleStatus,
    pub storage_key: Option<String>,
    pub size_bytes: Option<i64>,
    pub checksum: Option<String>,
    pub policy_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Update bundle metadata
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateBundle {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Bundle-Policy association
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundlePolicy {
    pub bundle_id: Uuid,
    pub policy_id: Uuid,
    pub policy_version: i32,
    pub priority: i32,
}

/// Promotion history record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundlePromotion {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub from_status: BundleStatus,
    pub to_status: BundleStatus,
    pub promoted_by: Option<String>,
    pub promoted_at: DateTime<Utc>,
    pub notes: Option<String>,
}

/// Input for creating a new bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBundle {
    pub name: String,
    pub description: Option<String>,
    /// Policy IDs to include in the bundle
    #[serde(default)]
    pub policy_ids: Vec<Uuid>,
}

/// Request for promoting a bundle
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromotionRequest {
    pub notes: Option<String>,
    /// Target specific agents (None = all agents in org)
    pub target_agents: Option<Vec<Uuid>>,
    /// If true, notify only without auto-deploy
    #[serde(default)]
    pub notify_only: bool,
}

impl Bundle {
    /// Check if bundle can be compiled
    pub fn can_compile(&self) -> bool {
        self.status == BundleStatus::Draft
    }

    /// Check if bundle can be staged
    pub fn can_stage(&self) -> bool {
        self.status == BundleStatus::Compiled
    }

    /// Check if bundle can be promoted
    pub fn can_promote(&self) -> bool {
        self.status == BundleStatus::Staged
    }

    /// Check if bundle can be deprecated
    pub fn can_deprecate(&self) -> bool {
        matches!(self.status, BundleStatus::Promoted | BundleStatus::Staged)
    }

    /// Get the next valid status transitions
    pub fn valid_transitions(&self) -> Vec<BundleStatus> {
        match self.status {
            BundleStatus::Draft => vec![BundleStatus::Compiled],
            BundleStatus::Compiled => vec![BundleStatus::Staged],
            BundleStatus::Staged => vec![BundleStatus::Promoted, BundleStatus::Deprecated],
            BundleStatus::Promoted => vec![BundleStatus::Deprecated],
            BundleStatus::Deprecated => vec![],
        }
    }
}

impl BundleStatus {
    /// Check if this is a terminal status
    pub fn is_terminal(&self) -> bool {
        *self == BundleStatus::Deprecated
    }

    /// Check if bundle is live
    pub fn is_live(&self) -> bool {
        *self == BundleStatus::Promoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_status_parsing() {
        assert_eq!("draft".parse::<BundleStatus>().unwrap(), BundleStatus::Draft);
        assert_eq!("promoted".parse::<BundleStatus>().unwrap(), BundleStatus::Promoted);
        assert!("invalid".parse::<BundleStatus>().is_err());
    }

    #[test]
    fn test_bundle_transitions() {
        let draft_bundle = Bundle {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "test".to_string(),
            description: None,
            status: BundleStatus::Draft,
            storage_key: None,
            size_bytes: None,
            checksum: None,
            policy_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(draft_bundle.can_compile());
        assert!(!draft_bundle.can_stage());
        assert!(!draft_bundle.can_promote());

        let compiled_bundle = Bundle {
            status: BundleStatus::Compiled,
            ..draft_bundle.clone()
        };
        assert!(!compiled_bundle.can_compile());
        assert!(compiled_bundle.can_stage());

        let staged_bundle = Bundle {
            status: BundleStatus::Staged,
            ..draft_bundle
        };
        assert!(staged_bundle.can_promote());
        assert!(staged_bundle.can_deprecate());
    }
}
