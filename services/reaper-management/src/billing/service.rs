//! Billing service
//!
//! Handles subscription billing integration. The current implementation provides
//! the domain types and plan limits infrastructure. Full Stripe integration can
//! be enabled by setting STRIPE_SECRET_KEY and implementing the API calls.
//!
//! For production Stripe integration, you would:
//! 1. Create Stripe products and prices in the Stripe dashboard
//! 2. Configure price IDs via environment variables
//! 3. Implement webhook handlers for subscription lifecycle events

use chrono::Utc;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::db::Database;
use crate::domain::billing::{
    BillingSummary, CheckoutSessionResponse, PlanTier, PortalSessionResponse, Subscription,
    SubscriptionStatus,
};
use crate::domain::organization::Organization;

/// Billing service errors
#[derive(Debug, Error)]
pub enum BillingError {
    #[error("Stripe error: {0}")]
    Stripe(String),
    #[error("Database error: {0}")]
    Database(#[from] crate::db::DatabaseError),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Subscription not found")]
    NotFound,
    #[error("Invalid plan tier: {0}")]
    InvalidPlan(String),
    #[error("Billing not configured")]
    NotConfigured,
}

/// Billing configuration
#[derive(Debug, Clone)]
pub struct BillingConfig {
    /// Stripe secret key
    pub stripe_secret_key: String,
    /// Stripe webhook signing secret
    pub stripe_webhook_secret: String,
    /// Stripe price IDs for each tier
    pub price_ids: StripePriceIds,
    /// Base URL for redirects
    pub base_url: String,
}

/// Stripe price IDs for each plan tier
#[derive(Debug, Clone, Default)]
pub struct StripePriceIds {
    pub starter_monthly: String,
    pub starter_yearly: String,
    pub professional_monthly: String,
    pub professional_yearly: String,
    pub enterprise_monthly: String,
    pub enterprise_yearly: String,
}

impl BillingConfig {
    /// Create config from environment variables
    pub fn from_env() -> Result<Self, BillingError> {
        let stripe_secret_key = std::env::var("STRIPE_SECRET_KEY")
            .map_err(|_| BillingError::Config("STRIPE_SECRET_KEY not set".to_string()))?;

        let stripe_webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET")
            .map_err(|_| BillingError::Config("STRIPE_WEBHOOK_SECRET not set".to_string()))?;

        let base_url =
            std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_string());

        // Price IDs can be configured via environment or use defaults
        let price_ids = StripePriceIds {
            starter_monthly: std::env::var("STRIPE_PRICE_STARTER_MONTHLY")
                .unwrap_or_else(|_| "price_starter_monthly".to_string()),
            starter_yearly: std::env::var("STRIPE_PRICE_STARTER_YEARLY")
                .unwrap_or_else(|_| "price_starter_yearly".to_string()),
            professional_monthly: std::env::var("STRIPE_PRICE_PROFESSIONAL_MONTHLY")
                .unwrap_or_else(|_| "price_professional_monthly".to_string()),
            professional_yearly: std::env::var("STRIPE_PRICE_PROFESSIONAL_YEARLY")
                .unwrap_or_else(|_| "price_professional_yearly".to_string()),
            enterprise_monthly: std::env::var("STRIPE_PRICE_ENTERPRISE_MONTHLY")
                .unwrap_or_else(|_| "price_enterprise_monthly".to_string()),
            enterprise_yearly: std::env::var("STRIPE_PRICE_ENTERPRISE_YEARLY")
                .unwrap_or_else(|_| "price_enterprise_yearly".to_string()),
        };

        Ok(Self {
            stripe_secret_key,
            stripe_webhook_secret,
            price_ids,
            base_url,
        })
    }

    /// Get price ID for a plan tier (monthly by default)
    pub fn get_price_id(&self, tier: PlanTier, yearly: bool) -> Option<&str> {
        match (tier, yearly) {
            (PlanTier::Free, _) => None, // Free tier has no price
            (PlanTier::Starter, false) => Some(&self.price_ids.starter_monthly),
            (PlanTier::Starter, true) => Some(&self.price_ids.starter_yearly),
            (PlanTier::Professional, false) => Some(&self.price_ids.professional_monthly),
            (PlanTier::Professional, true) => Some(&self.price_ids.professional_yearly),
            (PlanTier::Enterprise, false) => Some(&self.price_ids.enterprise_monthly),
            (PlanTier::Enterprise, true) => Some(&self.price_ids.enterprise_yearly),
        }
    }
}

/// Billing service
pub struct BillingService {
    #[allow(dead_code)]
    db: Arc<Database>,
    config: Option<BillingConfig>,
}

impl BillingService {
    /// Create a new billing service with Stripe configuration
    pub fn new(db: Arc<Database>, config: BillingConfig) -> Self {
        Self {
            db,
            config: Some(config),
        }
    }

    /// Create a billing service without Stripe (for testing or when not configured)
    pub fn disabled(db: Arc<Database>) -> Self {
        Self { db, config: None }
    }

    /// Check if billing is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    fn config(&self) -> Result<&BillingConfig, BillingError> {
        self.config.as_ref().ok_or(BillingError::NotConfigured)
    }

    /// Create or get a Stripe customer for an organization
    ///
    /// In production, this would call the Stripe API to create a customer.
    /// For now, returns a placeholder customer ID.
    pub async fn get_or_create_customer(
        &self,
        org_id: Uuid,
        org_name: &str,
        _email: &str,
    ) -> Result<String, BillingError> {
        let _config = self.config()?;

        // TODO: Implement actual Stripe customer creation using stripe crate
        // let client = stripe::Client::new(&config.stripe_secret_key);
        // let params = stripe::CreateCustomer { ... };
        // let customer = stripe::Customer::create(&client, params).await?;

        info!(org_id = %org_id, org_name = %org_name, "Would create Stripe customer");

        // Return placeholder - in production, return customer.id
        Ok(format!("cus_placeholder_{}", org_id))
    }

    /// Create a checkout session for upgrading/subscribing
    ///
    /// In production, this would create a Stripe checkout session.
    /// Returns a placeholder response for now.
    pub async fn create_checkout_session(
        &self,
        customer_id: &str,
        org_id: Uuid,
        plan_tier: PlanTier,
        yearly: bool,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<CheckoutSessionResponse, BillingError> {
        let config = self.config()?;

        let price_id = config.get_price_id(plan_tier, yearly).ok_or_else(|| {
            BillingError::InvalidPlan(format!("No price for tier: {}", plan_tier))
        })?;

        debug!(
            customer_id = %customer_id,
            org_id = %org_id,
            plan_tier = %plan_tier,
            price_id = %price_id,
            success_url = %success_url,
            cancel_url = %cancel_url,
            "Would create checkout session"
        );

        // TODO: Implement actual Stripe checkout session creation
        // let client = stripe::Client::new(&config.stripe_secret_key);
        // let params = stripe::CreateCheckoutSession { ... };
        // let session = stripe::CheckoutSession::create(&client, params).await?;

        // Return placeholder - in production, return actual session
        Ok(CheckoutSessionResponse {
            session_id: format!("cs_placeholder_{}", Uuid::new_v4()),
            url: format!(
                "{}/checkout/placeholder?org={}&tier={}",
                config.base_url, org_id, plan_tier
            ),
        })
    }

    /// Create a billing portal session for managing subscription
    ///
    /// In production, this would create a Stripe billing portal session.
    pub async fn create_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> Result<PortalSessionResponse, BillingError> {
        let config = self.config()?;

        debug!(
            customer_id = %customer_id,
            return_url = %return_url,
            "Would create billing portal session"
        );

        // TODO: Implement actual Stripe billing portal session creation
        // let client = stripe::Client::new(&config.stripe_secret_key);
        // let params = stripe::CreateBillingPortalSession { ... };
        // let session = stripe::BillingPortalSession::create(&client, params).await?;

        // Return placeholder
        Ok(PortalSessionResponse {
            url: format!(
                "{}/billing/portal?customer={}",
                config.base_url, customer_id
            ),
        })
    }

    /// Create a free tier subscription for a new organization
    pub fn create_free_subscription(&self, org_id: Uuid) -> Subscription {
        Subscription {
            id: Uuid::new_v4(),
            org_id,
            stripe_customer_id: String::new(),
            stripe_subscription_id: None,
            plan_tier: PlanTier::Free,
            status: SubscriptionStatus::Active,
            current_period_start: Some(Utc::now()),
            current_period_end: None, // Free tier doesn't expire
            trial_end: None,
            cancel_at_period_end: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Get billing summary for an organization, with **real** usage counts and
    /// the org's effective plan limits (round-2 E4). The plan tier is resolved
    /// from `Organization.settings.plan_tier`; usage is counted from the DB.
    pub async fn get_billing_summary(
        &self,
        org: &Organization,
    ) -> Result<BillingSummary, BillingError> {
        let limits = crate::quota::effective_limits(org);
        let usage = crate::quota::count_usage(&self.db, org.id).await?;

        // Overage detection (unchanged): a limit of 0 or -1 means unlimited.
        let mut exceeded_limits = Vec::new();
        if limits.max_agents > 0 && usage.active_agents > limits.max_agents {
            exceeded_limits.push("agents".to_string());
        }
        if limits.max_policies > 0 && usage.policy_count > limits.max_policies {
            exceeded_limits.push("policies".to_string());
        }
        if limits.max_users > 0 && usage.user_count > limits.max_users {
            exceeded_limits.push("users".to_string());
        }
        if limits.max_storage_bytes > 0 && usage.storage_bytes > limits.max_storage_bytes {
            exceeded_limits.push("storage".to_string());
        }

        Ok(BillingSummary {
            subscription: None,
            limits,
            usage,
            within_limits: exceeded_limits.is_empty(),
            exceeded_limits,
        })
    }

    /// Handle Stripe webhook event
    ///
    /// In production, this would verify the webhook signature and process events.
    pub async fn handle_webhook_event(
        &self,
        payload: &str,
        signature: &str,
    ) -> Result<(), BillingError> {
        let _config = self.config()?;

        // TODO: Implement actual Stripe webhook verification and handling
        // let event = stripe::Webhook::construct_event(payload, signature, &config.stripe_webhook_secret)?;
        // match event.type_ {
        //     stripe::EventType::CustomerSubscriptionCreated => { ... }
        //     stripe::EventType::CustomerSubscriptionUpdated => { ... }
        //     stripe::EventType::CustomerSubscriptionDeleted => { ... }
        //     stripe::EventType::InvoicePaymentSucceeded => { ... }
        //     stripe::EventType::InvoicePaymentFailed => { ... }
        //     _ => {}
        // }

        warn!(
            payload_len = payload.len(),
            signature_len = signature.len(),
            "Webhook handling not fully implemented - would process event"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::billing::PlanLimits;

    #[test]
    fn test_plan_limits_free() {
        let limits = PlanLimits::free();
        assert_eq!(limits.max_agents, 2);
        assert_eq!(limits.max_policies, 10);
        assert!(!limits.sso_enabled);
    }

    #[test]
    fn test_plan_limits_professional() {
        let limits = PlanLimits::professional();
        assert_eq!(limits.max_agents, 50);
        assert!(limits.sso_enabled);
        assert!(limits.audit_logs_enabled);
    }

    #[test]
    fn test_plan_limits_enterprise() {
        let limits = PlanLimits::enterprise();
        assert_eq!(limits.max_agents, -1); // Unlimited
        assert!(limits.sso_enabled);
    }

    #[test]
    fn test_billing_config_price_ids() {
        let config = BillingConfig {
            stripe_secret_key: "sk_test_xxx".to_string(),
            stripe_webhook_secret: "whsec_xxx".to_string(),
            price_ids: StripePriceIds {
                starter_monthly: "price_starter_m".to_string(),
                starter_yearly: "price_starter_y".to_string(),
                professional_monthly: "price_pro_m".to_string(),
                professional_yearly: "price_pro_y".to_string(),
                enterprise_monthly: "price_ent_m".to_string(),
                enterprise_yearly: "price_ent_y".to_string(),
            },
            base_url: "http://localhost:8081".to_string(),
        };

        assert_eq!(
            config.get_price_id(PlanTier::Starter, false),
            Some("price_starter_m")
        );
        assert_eq!(
            config.get_price_id(PlanTier::Professional, true),
            Some("price_pro_y")
        );
        assert_eq!(config.get_price_id(PlanTier::Free, false), None);
    }

    #[test]
    fn test_free_subscription() {
        let db = Arc::new(Database::new_mock());
        let service = BillingService::disabled(db);

        let org_id = Uuid::new_v4();
        let sub = service.create_free_subscription(org_id);

        assert_eq!(sub.org_id, org_id);
        assert_eq!(sub.plan_tier, PlanTier::Free);
        assert_eq!(sub.status, SubscriptionStatus::Active);
    }
}
