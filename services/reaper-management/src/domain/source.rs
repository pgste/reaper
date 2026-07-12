//! Policy source domain model
//!
//! Defines sources from which policies can be fetched (Git repos, external APIs).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Policy source type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SourceType {
    /// Git repository source
    #[default]
    Git,
    /// External HTTP API source
    Api,
    /// S3 bucket source
    S3,
    /// Bundle URL source (webhook-triggered)
    BundleUrl,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "git"),
            Self::Api => write!(f, "api"),
            Self::S3 => write!(f, "s3"),
            Self::BundleUrl => write!(f, "bundle_url"),
        }
    }
}

impl std::str::FromStr for SourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "git" => Ok(Self::Git),
            "api" => Ok(Self::Api),
            "s3" => Ok(Self::S3),
            "bundle_url" | "bundleurl" => Ok(Self::BundleUrl),
            _ => Err(format!("Unknown source type: {}", s)),
        }
    }
}

/// Sync status for a policy source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SyncStatus {
    /// Never synced
    #[default]
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

    // --- Provenance / hardening (Plan 09 Phase B) ------------------------
    /// Provider driving this source ("github" | "gitlab"). Set when the source
    /// was created via an App/OAuth flow; used to resolve webhook handlers and
    /// to mint installation tokens.
    #[serde(default)]
    pub provider: Option<String>,
    /// GitHub App installation id. When set, cloning mints a short-lived
    /// installation token at sync time instead of using `password` — no
    /// long-lived credential is stored in `url` or `password` (Plan 09 Step 6).
    #[serde(default)]
    pub installation_id: Option<String>,
    /// "owner/repo" — how a webhook payload identifies this source, and what
    /// the installation-token clone URL is built from.
    #[serde(default)]
    pub repo_full_name: Option<String>,
    /// Require the tracked HEAD commit to carry a trusted signature (Plan 09
    /// Step 5). When true, an unsigned/untrusted HEAD fails the sync closed.
    #[serde(default)]
    pub require_signed_commits: bool,
    /// Trusted SSH signer public keys (`ssh-ed25519 AAAA… comment`) checked
    /// when `require_signed_commits` is set.
    #[serde(default)]
    pub trusted_signing_keys: Vec<String>,
    /// How UI/API edits to a policy backed by this source are reconciled with
    /// git (Plan 09 Step 9, ADR-3). Default `commit_back`.
    #[serde(default)]
    pub conflict_mode: ConflictMode,
}

/// Git↔UI conflict model for a git-backed source (Plan 09 Step 9 / ADR-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    /// Recommended default: a UI/API edit opens a branch + commit on the source
    /// repo; deployed state is only ever changed by the sync path materializing
    /// a commit, so there is exactly one lineage and drift is impossible.
    #[default]
    CommitBack,
    /// Git is authoritative: UI/API edits to a git-backed policy are rejected.
    ReadOnly,
    /// Escape hatch (discouraged): the UI/API edit is applied directly to the
    /// deployed state and git is left behind — guarantees eventual divergence,
    /// so a drift event is emitted. Offered, not recommended.
    LastWriterWins,
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
            provider: None,
            installation_id: None,
            repo_full_name: None,
            require_signed_commits: false,
            trusted_signing_keys: Vec::new(),
            conflict_mode: ConflictMode::default(),
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

/// S3 source configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    /// S3 bucket name
    pub bucket: String,
    /// AWS region (e.g., "us-east-1")
    pub region: String,
    /// Prefix within bucket (folder path)
    #[serde(default)]
    pub prefix: Option<String>,
    /// File patterns to include (glob)
    #[serde(default = "default_s3_patterns")]
    pub patterns: Vec<String>,
    /// AWS access key ID (optional if using IAM role)
    #[serde(default)]
    pub access_key_id: Option<String>,
    /// AWS secret access key (optional if using IAM role)
    #[serde(default)]
    pub secret_access_key: Option<String>,
    /// Use IAM role for authentication (default: true)
    #[serde(default = "default_use_iam_role")]
    pub use_iam_role: bool,
    /// Custom S3 endpoint URL (for S3-compatible services)
    #[serde(default)]
    pub endpoint_url: Option<String>,
    /// Whether to fetch .rbb bundles directly (skip compilation)
    #[serde(default)]
    pub bundle_mode: bool,
}

fn default_s3_patterns() -> Vec<String> {
    vec!["**/*.reap".to_string(), "**/*.yaml".to_string()]
}

fn default_use_iam_role() -> bool {
    true
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            region: "us-east-1".to_string(),
            prefix: None,
            patterns: default_s3_patterns(),
            access_key_id: None,
            secret_access_key: None,
            use_iam_role: true,
            endpoint_url: None,
            bundle_mode: false,
        }
    }
}

/// Bundle URL source configuration (webhook-triggered)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleUrlConfig {
    /// Base URL for bundle downloads (optional, if bundles always come from webhooks)
    #[serde(default)]
    pub base_url: Option<String>,
    /// Expected checksum algorithm ("sha256", "md5")
    #[serde(default = "default_checksum_algorithm")]
    pub checksum_algorithm: String,
    /// Webhook secret for HMAC validation
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Authentication header name for bundle download
    #[serde(default)]
    pub auth_header: Option<String>,
    /// Authentication token for bundle download
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Timeout for bundle download in seconds
    #[serde(default = "default_download_timeout")]
    pub download_timeout_secs: u32,
    /// Whether to verify bundle checksum
    #[serde(default = "default_verify_checksum")]
    pub verify_checksum: bool,
}

fn default_checksum_algorithm() -> String {
    "sha256".to_string()
}

fn default_download_timeout() -> u32 {
    60
}

fn default_verify_checksum() -> bool {
    true
}

impl Default for BundleUrlConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            checksum_algorithm: default_checksum_algorithm(),
            webhook_secret: None,
            auth_header: None,
            auth_token: None,
            download_timeout_secs: default_download_timeout(),
            verify_checksum: true,
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

    /// Get S3 configuration if this is an S3 source
    pub fn s3_config(&self) -> Option<S3Config> {
        if self.source_type == SourceType::S3 {
            serde_json::from_value(self.config.clone()).ok()
        } else {
            None
        }
    }

    /// Get Bundle URL configuration if this is a BundleUrl source
    pub fn bundle_url_config(&self) -> Option<BundleUrlConfig> {
        if self.source_type == SourceType::BundleUrl {
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
        assert_eq!("s3".parse::<SourceType>().unwrap(), SourceType::S3);
        assert_eq!(
            "bundle_url".parse::<SourceType>().unwrap(),
            SourceType::BundleUrl
        );
        assert_eq!(
            "bundleurl".parse::<SourceType>().unwrap(),
            SourceType::BundleUrl
        );
        assert!("invalid".parse::<SourceType>().is_err());
    }

    #[test]
    fn test_s3_config_default() {
        let config = S3Config::default();
        assert_eq!(config.region, "us-east-1");
        assert!(config.use_iam_role);
        assert!(!config.bundle_mode);
        assert!(config.patterns.contains(&"**/*.reap".to_string()));
    }

    #[test]
    fn test_bundle_url_config_default() {
        let config = BundleUrlConfig::default();
        assert_eq!(config.checksum_algorithm, "sha256");
        assert_eq!(config.download_timeout_secs, 60);
        assert!(config.verify_checksum);
    }

    #[test]
    fn test_sync_status_parsing() {
        assert_eq!(
            "pending".parse::<SyncStatus>().unwrap(),
            SyncStatus::Pending
        );
        assert_eq!(
            "success".parse::<SyncStatus>().unwrap(),
            SyncStatus::Success
        );
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
