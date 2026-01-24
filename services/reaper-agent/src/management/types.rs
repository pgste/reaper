//! Types for management plane communication
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent registration request
#[derive(Debug, Clone, Serialize)]
pub struct RegisterAgentRequest {
    /// Agent name
    pub name: String,
    /// Hostname where agent is running
    pub hostname: Option<String>,
    /// Agent version
    pub version: Option<String>,
    /// Agent labels for targeting
    #[serde(default)]
    pub labels: serde_json::Value,
}

/// Agent registration response
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterAgentResponse {
    /// Registered agent details
    pub agent: AgentInfo,
    /// JWT token for subsequent requests
    pub token: String,
    /// Token expiration time
    pub token_expires_at: DateTime<Utc>,
}

/// Agent information from management server
#[derive(Debug, Clone, Deserialize)]
pub struct AgentInfo {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub labels: serde_json::Value,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub registered_at: DateTime<Utc>,
}

/// Heartbeat request
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatRequest {
    /// Agent status
    pub status: Option<String>,
    /// Agent metrics
    pub metrics: Option<AgentMetrics>,
}

/// Agent metrics sent with heartbeat
#[derive(Debug, Clone, Serialize)]
pub struct AgentMetrics {
    /// Total requests processed
    pub requests_total: u64,
    /// Requests per second
    pub requests_per_second: f64,
    /// Average latency in microseconds
    pub avg_latency_us: f64,
    /// P50 latency in microseconds
    pub p50_latency_us: f64,
    /// P99 latency in microseconds
    pub p99_latency_us: f64,
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// CPU usage percentage (0-100)
    pub cpu_percent: f64,
    /// Total allow decisions
    pub decisions_allow: u64,
    /// Total deny decisions
    pub decisions_deny: u64,
    /// Agent uptime in seconds
    pub uptime_seconds: u64,
    /// Current bundle ID
    pub current_bundle_id: Option<Uuid>,
    /// Current bundle version
    pub current_bundle_version: Option<String>,
}

/// Heartbeat response
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub server_time: DateTime<Utc>,
}

/// Bundle information from management server
#[derive(Debug, Clone, Deserialize)]
pub struct BundleInfo {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub policy_count: i32,
    pub storage_key: Option<String>,
    pub checksum: Option<String>,
    pub compiled_size_bytes: Option<i64>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Bundle download result
#[derive(Debug)]
pub struct BundleDownload {
    /// Bundle binary data
    pub data: Vec<u8>,
    /// Bundle ID
    pub bundle_id: Uuid,
    /// SHA-256 checksum
    pub checksum: String,
}

/// Bundle content from management server (JSON format)
#[derive(Debug, Clone, Deserialize)]
pub struct ManagementBundle {
    pub version: i32,
    pub format: String,
    pub policies: Vec<ManagementBundlePolicy>,
    pub metadata: ManagementBundleMetadata,
}

/// Policy entry in management bundle
#[derive(Debug, Clone, Deserialize)]
pub struct ManagementBundlePolicy {
    pub id: String,
    pub version: i32,
    pub priority: i32,
    pub content: String,
    pub content_hash: String,
    pub language: String,
}

/// Metadata in management bundle
#[derive(Debug, Clone, Deserialize)]
pub struct ManagementBundleMetadata {
    pub created_at: String,
    pub policy_count: i32,
    pub include_debug: bool,
}

/// Management client error
#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error("Not configured: {0}")]
    NotConfigured(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Bundle not found")]
    BundleNotFound,

    #[error("Bundle checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Server error: {status} - {message}")]
    ServerError { status: u16, message: String },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not registered")]
    NotRegistered,
}

/// Result type for management operations
pub type ManagementResult<T> = Result<T, ManagementError>;
