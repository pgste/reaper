//! Webhook subscription domain types
//!
//! Outbound webhooks allow customers to receive notifications when events
//! occur (sync completed, bundle promoted, agent health changed, etc.).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Events that can trigger outbound webhooks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// Sync operation started
    SyncStarted,
    /// Sync operation completed successfully
    SyncCompleted,
    /// Sync operation failed
    SyncFailed,
    /// Bundle was compiled
    BundleCompiled,
    /// Bundle was promoted to production
    BundlePromoted,
    /// Rollout wave was completed
    RolloutWaveCompleted,
    /// Rollout was cancelled
    RolloutCancelled,
    /// Agent registered
    AgentRegistered,
    /// Agent became unhealthy
    AgentUnhealthy,
    /// Agent went offline
    AgentOffline,
    /// Policy validation failed
    PolicyValidationFailed,
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebhookEventType::SyncStarted => write!(f, "sync.started"),
            WebhookEventType::SyncCompleted => write!(f, "sync.completed"),
            WebhookEventType::SyncFailed => write!(f, "sync.failed"),
            WebhookEventType::BundleCompiled => write!(f, "bundle.compiled"),
            WebhookEventType::BundlePromoted => write!(f, "bundle.promoted"),
            WebhookEventType::RolloutWaveCompleted => write!(f, "rollout.wave_completed"),
            WebhookEventType::RolloutCancelled => write!(f, "rollout.cancelled"),
            WebhookEventType::AgentRegistered => write!(f, "agent.registered"),
            WebhookEventType::AgentUnhealthy => write!(f, "agent.unhealthy"),
            WebhookEventType::AgentOffline => write!(f, "agent.offline"),
            WebhookEventType::PolicyValidationFailed => write!(f, "policy.validation_failed"),
        }
    }
}

impl std::str::FromStr for WebhookEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sync.started" | "sync_started" => Ok(WebhookEventType::SyncStarted),
            "sync.completed" | "sync_completed" => Ok(WebhookEventType::SyncCompleted),
            "sync.failed" | "sync_failed" => Ok(WebhookEventType::SyncFailed),
            "bundle.compiled" | "bundle_compiled" => Ok(WebhookEventType::BundleCompiled),
            "bundle.promoted" | "bundle_promoted" => Ok(WebhookEventType::BundlePromoted),
            "rollout.wave_completed" | "rollout_wave_completed" => {
                Ok(WebhookEventType::RolloutWaveCompleted)
            }
            "rollout.cancelled" | "rollout_cancelled" => Ok(WebhookEventType::RolloutCancelled),
            "agent.registered" | "agent_registered" => Ok(WebhookEventType::AgentRegistered),
            "agent.unhealthy" | "agent_unhealthy" => Ok(WebhookEventType::AgentUnhealthy),
            "agent.offline" | "agent_offline" => Ok(WebhookEventType::AgentOffline),
            "policy.validation_failed" | "policy_validation_failed" => {
                Ok(WebhookEventType::PolicyValidationFailed)
            }
            _ => Err(format!("Unknown webhook event type: {}", s)),
        }
    }
}

/// Webhook subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing)]
    pub secret: Option<String>,
    pub events: Vec<WebhookEventType>,
    pub is_active: bool,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub failure_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create a webhook subscription
#[derive(Debug, Clone, Deserialize)]
pub struct CreateWebhookSubscription {
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    pub events: Vec<WebhookEventType>,
}

/// Request to update a webhook subscription
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWebhookSubscription {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub events: Option<Vec<WebhookEventType>>,
    pub is_active: Option<bool>,
}

/// Webhook delivery payload
#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    /// Unique ID for this delivery attempt
    pub delivery_id: Uuid,
    /// Event type that triggered this webhook
    pub event: String,
    /// Timestamp of the event
    pub timestamp: DateTime<Utc>,
    /// Organization ID
    pub org_id: Uuid,
    /// Organization slug
    pub org_slug: String,
    /// Event-specific data
    pub data: serde_json::Value,
}

/// Result of a webhook delivery attempt
#[derive(Debug, Clone)]
pub struct WebhookDeliveryResult {
    pub subscription_id: Uuid,
    pub delivery_id: Uuid,
    pub success: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub duration_ms: u64,
}
