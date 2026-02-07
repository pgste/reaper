//! Billing domain types
//!
//! Types for Stripe-based subscription billing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Billing plan tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanTier {
    /// Free tier - limited features
    Free,
    /// Starter tier - small teams
    Starter,
    /// Professional tier - growing teams
    Professional,
    /// Enterprise tier - custom pricing
    Enterprise,
}

impl Default for PlanTier {
    fn default() -> Self {
        Self::Free
    }
}

impl std::fmt::Display for PlanTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanTier::Free => write!(f, "free"),
            PlanTier::Starter => write!(f, "starter"),
            PlanTier::Professional => write!(f, "professional"),
            PlanTier::Enterprise => write!(f, "enterprise"),
        }
    }
}

impl std::str::FromStr for PlanTier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "free" => Ok(PlanTier::Free),
            "starter" => Ok(PlanTier::Starter),
            "professional" | "pro" => Ok(PlanTier::Professional),
            "enterprise" => Ok(PlanTier::Enterprise),
            _ => Err(format!("Unknown plan tier: {}", s)),
        }
    }
}

/// Subscription status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    /// Active subscription
    Active,
    /// Trial period
    Trialing,
    /// Past due payment
    PastDue,
    /// Subscription cancelled
    Canceled,
    /// Subscription ended
    Ended,
    /// Payment incomplete
    Incomplete,
    /// Paused subscription
    Paused,
}

impl Default for SubscriptionStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl std::fmt::Display for SubscriptionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubscriptionStatus::Active => write!(f, "active"),
            SubscriptionStatus::Trialing => write!(f, "trialing"),
            SubscriptionStatus::PastDue => write!(f, "past_due"),
            SubscriptionStatus::Canceled => write!(f, "canceled"),
            SubscriptionStatus::Ended => write!(f, "ended"),
            SubscriptionStatus::Incomplete => write!(f, "incomplete"),
            SubscriptionStatus::Paused => write!(f, "paused"),
        }
    }
}

/// Billing subscription for an organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub org_id: Uuid,
    /// Stripe customer ID
    pub stripe_customer_id: String,
    /// Stripe subscription ID
    pub stripe_subscription_id: Option<String>,
    /// Current plan tier
    pub plan_tier: PlanTier,
    /// Subscription status
    pub status: SubscriptionStatus,
    /// Current billing period start
    pub current_period_start: Option<DateTime<Utc>>,
    /// Current billing period end
    pub current_period_end: Option<DateTime<Utc>>,
    /// Trial end date (if applicable)
    pub trial_end: Option<DateTime<Utc>>,
    /// Whether subscription will cancel at period end
    pub cancel_at_period_end: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Usage metrics for billing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageMetrics {
    /// Organization ID
    pub org_id: Uuid,
    /// Billing period start
    pub period_start: DateTime<Utc>,
    /// Billing period end
    pub period_end: DateTime<Utc>,
    /// Number of active agents
    pub active_agents: i64,
    /// Total policy evaluations
    pub policy_evaluations: i64,
    /// Number of policies
    pub policy_count: i64,
    /// Number of bundles
    pub bundle_count: i64,
    /// Number of active users
    pub user_count: i64,
    /// Storage used in bytes
    pub storage_bytes: i64,
}

/// Plan limits for a tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanLimits {
    pub tier: PlanTier,
    /// Maximum number of agents (-1 for unlimited)
    pub max_agents: i64,
    /// Maximum number of policies (-1 for unlimited)
    pub max_policies: i64,
    /// Maximum number of users (-1 for unlimited)
    pub max_users: i64,
    /// Maximum storage in bytes (-1 for unlimited)
    pub max_storage_bytes: i64,
    /// Policy evaluations per month (-1 for unlimited)
    pub max_evaluations_per_month: i64,
    /// Whether SSO is available
    pub sso_enabled: bool,
    /// Whether audit logs are available
    pub audit_logs_enabled: bool,
    /// Data retention days
    pub data_retention_days: i32,
}

impl PlanLimits {
    /// Get limits for the free tier
    pub fn free() -> Self {
        Self {
            tier: PlanTier::Free,
            max_agents: 2,
            max_policies: 10,
            max_users: 3,
            max_storage_bytes: 100 * 1024 * 1024, // 100MB
            max_evaluations_per_month: 10_000,
            sso_enabled: false,
            audit_logs_enabled: false,
            data_retention_days: 7,
        }
    }

    /// Get limits for the starter tier
    pub fn starter() -> Self {
        Self {
            tier: PlanTier::Starter,
            max_agents: 10,
            max_policies: 50,
            max_users: 10,
            max_storage_bytes: 1024 * 1024 * 1024, // 1GB
            max_evaluations_per_month: 100_000,
            sso_enabled: false,
            audit_logs_enabled: true,
            data_retention_days: 30,
        }
    }

    /// Get limits for the professional tier
    pub fn professional() -> Self {
        Self {
            tier: PlanTier::Professional,
            max_agents: 50,
            max_policies: 200,
            max_users: 50,
            max_storage_bytes: 10 * 1024 * 1024 * 1024, // 10GB
            max_evaluations_per_month: 1_000_000,
            sso_enabled: true,
            audit_logs_enabled: true,
            data_retention_days: 90,
        }
    }

    /// Get limits for the enterprise tier
    pub fn enterprise() -> Self {
        Self {
            tier: PlanTier::Enterprise,
            max_agents: -1,                // Unlimited
            max_policies: -1,              // Unlimited
            max_users: -1,                 // Unlimited
            max_storage_bytes: -1,         // Unlimited
            max_evaluations_per_month: -1, // Unlimited
            sso_enabled: true,
            audit_logs_enabled: true,
            data_retention_days: 365,
        }
    }

    /// Get limits for a given tier
    pub fn for_tier(tier: PlanTier) -> Self {
        match tier {
            PlanTier::Free => Self::free(),
            PlanTier::Starter => Self::starter(),
            PlanTier::Professional => Self::professional(),
            PlanTier::Enterprise => Self::enterprise(),
        }
    }
}

/// Request to create a checkout session
#[derive(Debug, Clone, Deserialize)]
pub struct CreateCheckoutRequest {
    /// Plan to subscribe to
    pub plan_tier: PlanTier,
    /// URL to redirect to on success
    pub success_url: String,
    /// URL to redirect to on cancellation
    pub cancel_url: String,
}

/// Request to update subscription
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSubscriptionRequest {
    /// New plan tier
    pub plan_tier: Option<PlanTier>,
    /// Whether to cancel at period end
    pub cancel_at_period_end: Option<bool>,
}

/// Checkout session response
#[derive(Debug, Clone, Serialize)]
pub struct CheckoutSessionResponse {
    /// Stripe checkout session ID
    pub session_id: String,
    /// URL to redirect user to
    pub url: String,
}

/// Portal session response
#[derive(Debug, Clone, Serialize)]
pub struct PortalSessionResponse {
    /// URL to Stripe customer portal
    pub url: String,
}

/// Billing summary for an organization
#[derive(Debug, Clone, Serialize)]
pub struct BillingSummary {
    /// Current subscription
    pub subscription: Option<Subscription>,
    /// Current plan limits
    pub limits: PlanLimits,
    /// Current usage
    pub usage: UsageMetrics,
    /// Whether org is within limits
    pub within_limits: bool,
    /// Limits that are exceeded
    pub exceeded_limits: Vec<String>,
}
