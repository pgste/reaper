//! Policy source domain model
//!
//! Defines sources from which policies can be fetched (Git repos, external APIs).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Policy source type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Git repository source
    Git,
    /// External HTTP API source
    Api,
}

impl Default for SourceType {
    fn default() -> Self {
        Self::Git
    }
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "git"),
            Self::Api => write!(f, "api"),
        }
    }
}

impl std::str::FromStr for SourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "git" => Ok(Self::Git),
            "api" => Ok(Self::Api),
            _ => Err(format!("Unknown source type: {}", s)),
        }
    }
}

/// Sync status for a policy source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    /// Never synced
    Pending,
    /// Currently syncing
    Syncing,
    /// Last sync succeeded
    Success,
    /// Last sync failed
    Failed,
    /// Source is disabled
    Disabled,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Syncing => write!(f, "syncing"),
            Self::Success => write!(f, "success"),
            Self::Failed => write!(f, "failed"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

impl std::str::FromStr for SyncStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "syncing" => Ok(Self::Syncing),
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "disabled" => Ok(Self::Disabled),
            _ => Err(format!("Unknown sync status: {}", s)),
        }
    }
}

/// Git source configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Repository URL (https or ssh)
    pub url: String,
    /// Branch to track (default: main)
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Path within repo to policy files
    #[serde(default)]
    pub path: Option<String>,
    /// SSH private key (for ssh:// URLs)
    #[serde(default)]
    pub ssh_key: Option<String>,
    /// HTTP basic auth username
    #[serde(default)]
    pub username: Option<String>,
    /// HTTP basic auth password/token
    #[serde(default)]
    pub password: Option<String>,
    /// File patterns to include (glob)
    #[serde(default = "default_patterns")]
    pub patterns: Vec<String>,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_patterns() -> Vec<String> {
    vec!["**/*.reap".to_string(), "**/*.yaml".to_string()]
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            branch: default_branch(),
            path: None,
            ssh_key: None,
            username: None,
            password: None,
            patterns: default_patterns(),
        }
    }
}

/// API source configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// API endpoint URL
    pub url: String,
    /// HTTP method (GET, POST)
    #[serde(default = "default_method")]
    pub method: String,
    /// Request headers
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    /// Request body (for POST)
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// JSONPath to extract policies from response
    #[serde(default)]
    pub jsonpath: Option<String>,
    /// API key header name
    #[serde(default)]
    pub api_key_header: Option<String>,
    /// API key value
    #[serde(default)]
    pub api_key: Option<String>,
    /// Response format (json, yaml)
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_format() -> String {
    "json".to_string()
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: default_method(),
            headers: std::collections::HashMap::new(),
            body: None,
            jsonpath: None,
            api_key_header: None,
            api_key: None,
            format: default_format(),
        }
    }
}

/// Policy source entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySource {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_type: SourceType,
    /// Source-specific configuration (GitConfig or ApiConfig as JSON)
    pub config: serde_json::Value,
    /// Sync interval in seconds (0 = manual only)
    pub sync_interval_secs: u32,
    pub sync_status: SyncStatus,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_sync_error: Option<String>,
    pub last_sync_commit: Option<String>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PolicySource {
    /// Get Git configuration if this is a Git source
    pub fn git_config(&self) -> Option<GitConfig> {
        if self.source_type == SourceType::Git {
            serde_json::from_value(self.config.clone()).ok()
        } else {
            None
        }
    }

    /// Get API configuration if this is an API source
    pub fn api_config(&self) -> Option<ApiConfig> {
        if self.source_type == SourceType::Api {
            serde_json::from_value(self.config.clone()).ok()
        } else {
            None
        }
    }

    /// Check if sync is due
    pub fn is_sync_due(&self) -> bool {
        if !self.is_enabled || self.sync_interval_secs == 0 {
            return false;
        }

        match self.last_sync_at {
            Some(last_sync) => {
                let elapsed = Utc::now().signed_duration_since(last_sync);
                elapsed.num_seconds() >= self.sync_interval_secs as i64
            }
            None => true,
        }
    }

    /// Check if source can be synced
    pub fn can_sync(&self) -> bool {
        self.is_enabled && self.sync_status != SyncStatus::Syncing
    }
}

/// Input for creating a policy source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePolicySource {
    pub name: String,
    pub description: Option<String>,
    pub source_type: SourceType,
    pub config: serde_json::Value,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u32,
}

fn default_sync_interval() -> u32 {
    300 // 5 minutes
}

/// Input for updating a policy source
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatePolicySource {
    pub name: Option<String>,
    pub description: Option<String>,
    pub config: Option<serde_json::Value>,
    pub sync_interval_secs: Option<u32>,
    pub is_enabled: Option<bool>,
}

/// Sync result from a policy source
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub source_id: Uuid,
    pub success: bool,
    pub policies_found: usize,
    pub policies_updated: usize,
    pub policies_created: usize,
    pub commit: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_type_parsing() {
        assert_eq!("git".parse::<SourceType>().unwrap(), SourceType::Git);
        assert_eq!("api".parse::<SourceType>().unwrap(), SourceType::Api);
        assert!("invalid".parse::<SourceType>().is_err());
    }

    #[test]
    fn test_sync_status_parsing() {
        assert_eq!("pending".parse::<SyncStatus>().unwrap(), SyncStatus::Pending);
        assert_eq!("success".parse::<SyncStatus>().unwrap(), SyncStatus::Success);
        assert_eq!("failed".parse::<SyncStatus>().unwrap(), SyncStatus::Failed);
    }

    #[test]
    fn test_git_config_default() {
        let config = GitConfig::default();
        assert_eq!(config.branch, "main");
        assert!(config.patterns.contains(&"**/*.reap".to_string()));
    }

    #[test]
    fn test_is_sync_due() {
        let mut source = PolicySource {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "test".to_string(),
            description: None,
            source_type: SourceType::Git,
            config: serde_json::json!({}),
            sync_interval_secs: 60,
            sync_status: SyncStatus::Success,
            last_sync_at: Some(Utc::now() - chrono::Duration::seconds(120)),
            last_sync_error: None,
            last_sync_commit: None,
            is_enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Sync is due (120 seconds elapsed, interval is 60)
        assert!(source.is_sync_due());

        // Recent sync
        source.last_sync_at = Some(Utc::now());
        assert!(!source.is_sync_due());

        // Disabled source
        source.is_enabled = false;
        source.last_sync_at = Some(Utc::now() - chrono::Duration::seconds(120));
        assert!(!source.is_sync_due());
    }
}
