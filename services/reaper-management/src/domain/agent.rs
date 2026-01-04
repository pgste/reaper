//! Agent domain model
//!
//! Agents are the policy enforcement points that connect to the management server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent has registered but not yet connected
    Pending,
    /// Agent is actively connected
    Active,
    /// Agent has disconnected gracefully
    Inactive,
    /// Agent has been manually disabled
    Disabled,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Pending
    }
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
pub enum DeploymentStatus {
    Pending,
    Deployed,
    Failed,
}

impl Default for DeploymentStatus {
    fn default() -> Self {
        Self::Pending
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetrics {
    pub requests_total: u64,
    pub requests_per_second: f64,
    pub avg_latency_us: f64,
    pub p99_latency_us: f64,
    pub memory_mb: f64,
    pub cpu_percent: f64,
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
        assert_eq!("active".parse::<AgentStatus>().unwrap(), AgentStatus::Active);
        assert_eq!("inactive".parse::<AgentStatus>().unwrap(), AgentStatus::Inactive);
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
