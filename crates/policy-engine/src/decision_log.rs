//! Decision Logging for Policy Evaluation
//!
//! Provides structured decision logging for audit, compliance, and observability.
//! Compatible with SIEM systems via NDJSON export format.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single decision log entry capturing all relevant context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogEntry {
    /// ISO 8601 timestamp
    pub timestamp: String,

    /// Unique decision ID (UUID)
    pub decision_id: String,

    /// OpenTelemetry trace ID for correlation (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// Principal (user) ID
    pub principal: String,

    /// Action being performed
    pub action: String,

    /// Resource being accessed
    pub resource: String,

    /// Additional context from the request
    #[serde(default)]
    pub context: HashMap<String, serde_json::Value>,

    /// Decision result: "allow", "deny", or "log"
    pub decision: String,

    /// Policy ID that was evaluated
    pub policy_id: String,

    /// Policy name
    pub policy_name: String,

    /// Policy version (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,

    /// Evaluation time in nanoseconds
    pub evaluation_time_ns: u64,

    /// Whether the result came from cache
    pub cache_hit: bool,

    /// Agent ID that processed the request (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Matched rule name (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,
}

impl DecisionLogEntry {
    /// Create a new decision log entry with required fields
    pub fn new(
        principal: String,
        action: String,
        resource: String,
        decision: String,
        policy_id: String,
        policy_name: String,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            decision_id: uuid::Uuid::new_v4().to_string(),
            trace_id: None,
            principal,
            action,
            resource,
            context: HashMap::new(),
            decision,
            policy_id,
            policy_name,
            policy_version: None,
            evaluation_time_ns: 0,
            cache_hit: false,
            agent_id: None,
            matched_rule: None,
        }
    }

    /// Set the trace ID for OpenTelemetry correlation
    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the context
    pub fn with_context(mut self, context: HashMap<String, serde_json::Value>) -> Self {
        self.context = context;
        self
    }

    /// Set the policy version
    pub fn with_policy_version(mut self, version: String) -> Self {
        self.policy_version = Some(version);
        self
    }

    /// Set the evaluation time in nanoseconds
    pub fn with_evaluation_time_ns(mut self, ns: u64) -> Self {
        self.evaluation_time_ns = ns;
        self
    }

    /// Mark as a cache hit
    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the matched rule name
    pub fn with_matched_rule(mut self, rule: String) -> Self {
        self.matched_rule = Some(rule);
        self
    }

    /// Convert to NDJSON line (for file export)
    pub fn to_ndjson(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Configuration for decision logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogConfig {
    /// Whether decision logging is enabled
    pub enabled: bool,

    /// Maximum entries in the buffer before oldest are dropped
    pub buffer_capacity: usize,

    /// Path to NDJSON file for persistent logging (optional)
    pub file_path: Option<String>,

    /// Emit each decision as an NDJSON line to stdout (container-native
    /// collection: a log agent — Vector/Fluent Bit/OTel Collector — scrapes
    /// stdout and ships to the central store). Can be combined with `file_path`.
    #[serde(default)]
    pub emit_stdout: bool,

    /// Flush interval in milliseconds (for file logging)
    pub flush_interval_ms: u64,

    /// Whether to log allow decisions (can be disabled to reduce volume)
    pub log_allows: bool,

    /// Whether to log deny decisions
    pub log_denies: bool,

    /// Fraction of *allow* decisions to keep, in [0.0, 1.0] (default 1.0 = all).
    /// Denies are never sampled — they're the security-relevant events. This is
    /// the cheapest volume-control knob: sampled-out allows are dropped before
    /// the log entry is even built. e.g. 0.01 keeps 1% of allows + 100% of denies.
    pub sample_allow_rate: f64,

    /// Whether to include context in logs (can be disabled for privacy)
    pub include_context: bool,
}

impl Default for DecisionLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_capacity: 10_000,
            file_path: None,
            emit_stdout: false,
            flush_interval_ms: 5_000,
            log_allows: true,
            log_denies: true,
            sample_allow_rate: 1.0,
            include_context: true,
        }
    }
}

impl DecisionLogConfig {
    /// Create from environment variables
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("REAPER_DECISION_LOG_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            buffer_capacity: std::env::var("REAPER_DECISION_LOG_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000),
            file_path: std::env::var("REAPER_DECISION_LOG_FILE").ok(),
            emit_stdout: std::env::var("REAPER_DECISION_LOG_STDOUT")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            flush_interval_ms: std::env::var("REAPER_DECISION_LOG_FLUSH_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5_000),
            log_allows: std::env::var("REAPER_DECISION_LOG_ALLOWS")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            log_denies: std::env::var("REAPER_DECISION_LOG_DENIES")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            sample_allow_rate: std::env::var("REAPER_DECISION_LOG_SAMPLE_ALLOW_RATE")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .map(|r| r.clamp(0.0, 1.0))
                .unwrap_or(1.0),
            include_context: std::env::var("REAPER_DECISION_LOG_CONTEXT")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_log_entry_creation() {
        let entry = DecisionLogEntry::new(
            "user_123".to_string(),
            "read".to_string(),
            "/api/data".to_string(),
            "allow".to_string(),
            "policy_456".to_string(),
            "data-access-policy".to_string(),
        );

        assert_eq!(entry.principal, "user_123");
        assert_eq!(entry.action, "read");
        assert_eq!(entry.resource, "/api/data");
        assert_eq!(entry.decision, "allow");
        assert!(!entry.decision_id.is_empty());
    }

    #[test]
    fn test_decision_log_entry_builder() {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "write".to_string(),
            "resource".to_string(),
            "deny".to_string(),
            "policy".to_string(),
            "policy-name".to_string(),
        )
        .with_evaluation_time_ns(500)
        .with_cache_hit(true)
        .with_agent_id("agent-1".to_string())
        .with_matched_rule("deny_rule".to_string());

        assert_eq!(entry.evaluation_time_ns, 500);
        assert!(entry.cache_hit);
        assert_eq!(entry.agent_id, Some("agent-1".to_string()));
        assert_eq!(entry.matched_rule, Some("deny_rule".to_string()));
    }

    #[test]
    fn test_decision_log_ndjson() {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "resource".to_string(),
            "allow".to_string(),
            "policy".to_string(),
            "test-policy".to_string(),
        );

        let json = entry.to_ndjson().unwrap();
        assert!(json.contains("\"principal\":\"user\""));
        assert!(json.contains("\"decision\":\"allow\""));
    }

    #[test]
    fn test_config_default() {
        let config = DecisionLogConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.buffer_capacity, 10_000);
        assert!(config.log_allows);
        assert!(config.log_denies);
    }
}
