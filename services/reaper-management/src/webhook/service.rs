//! Webhook delivery service
//!
//! Sends outbound webhooks to customer endpoints with HMAC signing.

use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::db::repositories::WebhookRepository;
use crate::db::Database;
use crate::domain::webhook::{
    WebhookDeliveryResult, WebhookEventType, WebhookPayload, WebhookSubscription,
};

type HmacSha256 = Hmac<Sha256>;

/// Configuration for webhook delivery
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Timeout for each delivery attempt
    pub timeout_seconds: u64,
    /// Number of retry attempts
    pub max_retries: u32,
    /// Base delay between retries (exponential backoff)
    pub retry_delay_ms: u64,
    /// Maximum number of consecutive failures before disabling
    pub max_failures: i32,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 10,
            max_retries: 3,
            retry_delay_ms: 1000,
            max_failures: 10,
        }
    }
}

/// Service for delivering webhooks
pub struct WebhookDeliveryService {
    db: Arc<Database>,
    client: Client,
    config: WebhookConfig,
}

impl WebhookDeliveryService {
    /// Create a new webhook delivery service
    pub fn new(db: Arc<Database>) -> Self {
        Self::with_config(db, WebhookConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(db: Arc<Database>, config: WebhookConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent("Reaper-Webhook/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self { db, client, config }
    }

    /// Deliver a webhook event to all subscribed endpoints for an organization
    pub async fn deliver_event(
        &self,
        org_id: Uuid,
        org_slug: &str,
        event_type: WebhookEventType,
        data: serde_json::Value,
    ) -> Vec<WebhookDeliveryResult> {
        let repo = WebhookRepository::new(&self.db);
        let subscriptions = match repo.list_by_event(org_id, event_type).await {
            Ok(subs) => subs,
            Err(e) => {
                error!(org_id = %org_id, event = %event_type, error = %e, "Failed to fetch webhook subscriptions");
                return vec![];
            }
        };

        if subscriptions.is_empty() {
            debug!(org_id = %org_id, event = %event_type, "No webhook subscriptions for event");
            return vec![];
        }

        info!(
            org_id = %org_id,
            event = %event_type,
            subscription_count = subscriptions.len(),
            "Delivering webhook to subscribers"
        );

        let mut results = Vec::with_capacity(subscriptions.len());

        for subscription in subscriptions {
            // Skip if subscription has too many failures
            if subscription.failure_count >= self.config.max_failures {
                warn!(
                    subscription_id = %subscription.id,
                    failure_count = subscription.failure_count,
                    "Skipping webhook - too many failures"
                );
                continue;
            }

            let delivery_id = Uuid::new_v4();
            let payload = WebhookPayload {
                delivery_id,
                event: event_type.to_string(),
                timestamp: Utc::now(),
                org_id,
                org_slug: org_slug.to_string(),
                data: data.clone(),
            };

            let result = self.deliver_to_subscription(&subscription, payload).await;

            // Record the delivery result
            if let Err(e) = repo.record_trigger(subscription.id, result.success).await {
                error!(
                    subscription_id = %subscription.id,
                    error = %e,
                    "Failed to record webhook trigger"
                );
            }

            results.push(result);
        }

        results
    }

    /// Deliver a webhook to a single subscription with retries
    async fn deliver_to_subscription(
        &self,
        subscription: &WebhookSubscription,
        payload: WebhookPayload,
    ) -> WebhookDeliveryResult {
        let start = Instant::now();
        let delivery_id = payload.delivery_id;

        // Serialize payload
        let body = match serde_json::to_string(&payload) {
            Ok(b) => b,
            Err(e) => {
                return WebhookDeliveryResult {
                    subscription_id: subscription.id,
                    delivery_id,
                    success: false,
                    status_code: None,
                    error: Some(format!("Failed to serialize payload: {}", e)),
                    duration_ms: start.elapsed().as_millis() as u64,
                };
            }
        };

        // Calculate HMAC signature if secret is configured
        let signature = subscription.secret.as_ref().map(|secret| {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                .expect("HMAC can take key of any size");
            mac.update(body.as_bytes());
            let result = mac.finalize();
            format!("sha256={}", hex::encode(result.into_bytes()))
        });

        // Attempt delivery with retries
        let mut last_error = None;
        let mut last_status_code = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let delay =
                    Duration::from_millis(self.config.retry_delay_ms * 2u64.pow(attempt - 1));
                tokio::time::sleep(delay).await;
                debug!(
                    subscription_id = %subscription.id,
                    attempt = attempt,
                    "Retrying webhook delivery"
                );
            }

            let mut request = self
                .client
                .post(&subscription.url)
                .header("Content-Type", "application/json")
                .header("X-Reaper-Delivery", delivery_id.to_string())
                .header("X-Reaper-Event", payload.event.clone());

            if let Some(ref sig) = signature {
                request = request.header("X-Reaper-Signature", sig);
            }

            match request.body(body.clone()).send().await {
                Ok(response) => {
                    let status = response.status();
                    last_status_code = Some(status.as_u16());

                    if status.is_success() {
                        info!(
                            subscription_id = %subscription.id,
                            url = %subscription.url,
                            status = status.as_u16(),
                            duration_ms = start.elapsed().as_millis(),
                            "Webhook delivered successfully"
                        );
                        return WebhookDeliveryResult {
                            subscription_id: subscription.id,
                            delivery_id,
                            success: true,
                            status_code: Some(status.as_u16()),
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                        };
                    }

                    // Non-success status - may retry for 5xx errors
                    if status.is_server_error() && attempt < self.config.max_retries {
                        last_error = Some(format!("Server error: {}", status));
                        continue;
                    }

                    // Client error or final attempt
                    last_error = Some(format!("HTTP {}", status));
                    break;
                }
                Err(e) => {
                    last_error = Some(e.to_string());

                    // Connection errors may be retried
                    if (e.is_connect() || e.is_timeout()) && attempt < self.config.max_retries {
                        continue;
                    }

                    break;
                }
            }
        }

        warn!(
            subscription_id = %subscription.id,
            url = %subscription.url,
            error = ?last_error,
            duration_ms = start.elapsed().as_millis(),
            "Webhook delivery failed"
        );

        WebhookDeliveryResult {
            subscription_id: subscription.id,
            delivery_id,
            success: false,
            status_code: last_status_code,
            error: last_error,
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Test a webhook subscription by sending a test payload
    pub async fn test_webhook(
        &self,
        subscription: &WebhookSubscription,
        org_slug: &str,
    ) -> WebhookDeliveryResult {
        let payload = WebhookPayload {
            delivery_id: Uuid::new_v4(),
            event: "test".to_string(),
            timestamp: Utc::now(),
            org_id: subscription.org_id,
            org_slug: org_slug.to_string(),
            data: serde_json::json!({
                "message": "This is a test webhook from Reaper",
                "subscription_name": subscription.name,
            }),
        };

        self.deliver_to_subscription(subscription, payload).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_signature() {
        let secret = "test-secret";
        let body = r#"{"event":"test"}"#;

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(body.as_bytes());
        let result = mac.finalize();
        let signature = format!("sha256={}", hex::encode(result.into_bytes()));

        // Signature should be consistent
        assert!(signature.starts_with("sha256="));
        assert_eq!(signature.len(), 7 + 64); // "sha256=" + 64 hex chars
    }
}
