//! Agent domain model
//!
//! Agents are the policy enforcement points that connect to the management server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentStatus {
    /// Agent has registered but not yet connected
    #[default]
    Pending,
    /// Agent is actively connected
    Active,
    /// Agent has disconnected gracefully
    Inactive,
    /// Agent has been manually disabled
    Disabled,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active => write!(f, "active"),
            Self::Inactive => write!(f, "inactive"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            "disabled" => Ok(Self::Disabled),
            _ => Err(format!("Unknown agent status: {}", s)),
        }
    }
}

/// Agent entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub version: Option<String>,
    pub status: AgentStatus,
    pub labels: serde_json::Value,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub registered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Agent-Bundle deployment record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBundle {
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub deployed_at: DateTime<Utc>,
    pub deployment_status: DeploymentStatus,
}

/// Deployment status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DeploymentStatus {
    #[default]
    Pending,
    Deployed,
    Failed,
}

/// Input for registering a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAgent {
    pub name: String,
    pub hostname: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub labels: serde_json::Value,
}

/// Agent heartbeat payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHeartbeat {
    pub status: AgentStatus,
    pub active_bundle_id: Option<Uuid>,
    pub metrics: Option<AgentMetrics>,
}

/// Agent performance metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentMetrics {
    /// Total requests processed
    pub requests_total: u64,
    /// Current requests per second
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
    /// Total eval-errors: served requests that could not be evaluated as
    /// intended. Decision-quality signal for auto-rollback (round-3 Plan 03),
    /// distinct from a legitimate deny.
    pub eval_errors: u64,
    /// Agent uptime in seconds
    pub uptime_seconds: u64,
    /// Current bundle ID
    pub current_bundle_id: Option<Uuid>,
    /// Current bundle version
    pub current_bundle_version: Option<String>,
    /// Data-plane replica state reported by the agent (two-way sync
    /// visibility: which datastore version it serves, where it is in the
    /// change stream, and whether its staleness budget is exceeded).
    #[serde(default)]
    pub data_version: Option<i64>,
    #[serde(default)]
    pub data_applied_seq: Option<i64>,
    #[serde(default)]
    pub data_stale: Option<bool>,
}

/// Aggregate runtime decision quality across an org's agents, used by the
/// decision-quality auto-rollback arm (round-3 Plan 03).
#[derive(Debug, Clone, Default)]
pub struct OrgDecisionMetrics {
    pub eval_errors: u64,
    pub decisions_allow: u64,
    pub decisions_deny: u64,
    /// Worst (max) p99 eval latency across the fleet, microseconds.
    pub p99_latency_us: f64,
}

impl OrgDecisionMetrics {
    /// Served decisions observed (allow + deny + eval-error).
    pub fn total_decisions(&self) -> u64 {
        self.eval_errors + self.decisions_allow + self.decisions_deny
    }

    /// Eval-error rate as a percentage of all served decisions.
    pub fn eval_error_rate(&self) -> f64 {
        let total = self.total_decisions();
        if total == 0 {
            0.0
        } else {
            self.eval_errors as f64 / total as f64 * 100.0
        }
    }

    /// Deny rate as a percentage of allow+deny (eval-errors excluded — they are
    /// a separate signal, not a policy deny).
    pub fn denial_rate(&self) -> f64 {
        let ad = self.decisions_allow + self.decisions_deny;
        if ad == 0 {
            0.0
        } else {
            self.decisions_deny as f64 / ad as f64 * 100.0
        }
    }
}

impl Agent {
    /// Check if agent is healthy (heartbeat within threshold)
    pub fn is_healthy(&self, threshold_seconds: i64) -> bool {
        if self.status == AgentStatus::Disabled {
            return false;
        }

        match self.last_heartbeat_at {
            Some(last_heartbeat) => {
                let elapsed = Utc::now().signed_duration_since(last_heartbeat);
                elapsed.num_seconds() < threshold_seconds
            }
            None => false,
        }
    }

    /// Check if agent can receive deployments
    pub fn can_deploy(&self) -> bool {
        matches!(self.status, AgentStatus::Active | AgentStatus::Pending)
    }

    /// Get a label value
    pub fn get_label(&self, key: &str) -> Option<&serde_json::Value> {
        self.labels.get(key)
    }

    /// Check if agent has a specific label
    pub fn has_label(&self, key: &str, value: &str) -> bool {
        self.labels
            .get(key)
            .and_then(|v| v.as_str())
            .map(|v| v == value)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_status_parsing() {
        assert_eq!(
            "active".parse::<AgentStatus>().unwrap(),
            AgentStatus::Active
        );
        assert_eq!(
            "inactive".parse::<AgentStatus>().unwrap(),
            AgentStatus::Inactive
        );
        assert!("unknown".parse::<AgentStatus>().is_err());
    }

    #[test]
    fn test_agent_health_check() {
        let mut agent = Agent {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "test-agent".to_string(),
            hostname: Some("localhost".to_string()),
            ip_address: None,
            version: Some("1.0.0".to_string()),
            status: AgentStatus::Active,
            labels: serde_json::json!({}),
            last_heartbeat_at: Some(Utc::now()),
            registered_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(agent.is_healthy(60));

        // Simulate old heartbeat
        agent.last_heartbeat_at = Some(Utc::now() - chrono::Duration::seconds(120));
        assert!(!agent.is_healthy(60));

        // Disabled agent is never healthy
        agent.status = AgentStatus::Disabled;
        agent.last_heartbeat_at = Some(Utc::now());
        assert!(!agent.is_healthy(60));
    }

    #[test]
    fn test_agent_labels() {
        let agent = Agent {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "test-agent".to_string(),
            hostname: None,
            ip_address: None,
            version: None,
            status: AgentStatus::Active,
            labels: serde_json::json!({
                "environment": "production",
                "region": "us-east-1"
            }),
            last_heartbeat_at: None,
            registered_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(agent.has_label("environment", "production"));
        assert!(!agent.has_label("environment", "staging"));
        assert!(!agent.has_label("nonexistent", "value"));
    }
}
