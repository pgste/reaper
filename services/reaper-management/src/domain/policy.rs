//! Policy domain model
//!
//! Policies are the core authorization rules managed by the system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Policy language type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum PolicyLanguage {
    Simple,
    Cedar,
    #[default]
    Reaper,
}

impl std::fmt::Display for PolicyLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple => write!(f, "simple"),
            Self::Cedar => write!(f, "cedar"),
            Self::Reaper => write!(f, "reaper"),
        }
    }
}

impl std::str::FromStr for PolicyLanguage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "simple" => Ok(Self::Simple),
            "cedar" => Ok(Self::Cedar),
            "reaper" | "reap" => Ok(Self::Reaper),
            _ => Err(format!("Unknown policy language: {}", s)),
        }
    }
}

/// Policy entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Uuid,
    pub org_id: Uuid,
    pub team_id: Option<Uuid>,
    pub source_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub language: PolicyLanguage,
    pub source_path: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Policy version - immutable record of policy content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersion {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub version: i32,
    pub content: String,
    pub content_hash: String,
    pub source_commit: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a new policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePolicy {
    pub name: String,
    pub description: Option<String>,
    pub team_id: Option<Uuid>,
    pub source_id: Option<Uuid>,
    #[serde(default)]
    pub language: PolicyLanguage,
    pub source_path: Option<String>,
    pub content: String,
}

/// Input for updating a policy
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatePolicy {
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    /// If provided, creates a new version
    pub content: Option<String>,
}

impl Policy {
    /// Check if policy belongs to a team
    pub fn is_team_policy(&self) -> bool {
        self.team_id.is_some()
    }

    /// Check if policy is sourced from external source
    pub fn is_sourced(&self) -> bool {
        self.source_id.is_some()
    }
}

impl PolicyVersion {
    /// Compute content hash using SHA-256
    pub fn compute_hash(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_language_parsing() {
        assert_eq!(
            "simple".parse::<PolicyLanguage>().unwrap(),
            PolicyLanguage::Simple
        );
        assert_eq!(
            "cedar".parse::<PolicyLanguage>().unwrap(),
            PolicyLanguage::Cedar
        );
        assert_eq!(
            "reaper".parse::<PolicyLanguage>().unwrap(),
            PolicyLanguage::Reaper
        );
        assert_eq!(
            "reap".parse::<PolicyLanguage>().unwrap(),
            PolicyLanguage::Reaper
        );
        assert!("unknown".parse::<PolicyLanguage>().is_err());
    }

    #[test]
    fn test_content_hash() {
        let hash1 = PolicyVersion::compute_hash("policy content");
        let hash2 = PolicyVersion::compute_hash("policy content");
        let hash3 = PolicyVersion::compute_hash("different content");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA-256 produces 64 hex chars
    }
}
