//! SIEM connector delivery (round-2 E1, slice 3).
//!
//! Transport for pushing already-shaped decision records to a per-org SIEM
//! connector (Splunk HEC / generic HTTP). Generalised from
//! [`crate::webhook::WebhookDeliveryService`]: async `reqwest`, exponential-
//! backoff retries, 5xx/timeout retry classification, and per-connector
//! delivery-result tracking.
//!
//! Separation of concerns: record **shaping** lives in `policy-engine`
//! (`DecisionLogEntry::export` → NDJSON / OCSF / CEF), the **API** orchestrates
//! (reads history from the decision store, shapes, records the receipt), and this
//! service is purely the wire — it takes ready-made lines and delivers them.

use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use std::time::{Duration, Instant};

use policy_engine::ExportFormat;

use crate::db::repositories::{ConnectorType, SiemConnector};

type HmacSha256 = Hmac<Sha256>;

/// Delivery tuning (mirrors `WebhookConfig`).
#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    pub timeout_seconds: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 15,
            max_retries: 3,
            retry_delay_ms: 1000,
        }
    }
}

/// Outcome of a delivery attempt to one connector.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ConnectorDeliveryResult {
    pub success: bool,
    /// Number of records in the delivered batch.
    pub records: usize,
    /// Last HTTP status observed (absent on a transport error).
    pub status_code: Option<u16>,
    /// Error detail on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Delivers shaped decision records to SIEM connectors.
pub struct ConnectorDeliveryService {
    client: Client,
    config: ConnectorConfig,
}

impl ConnectorDeliveryService {
    pub fn new() -> Self {
        Self::with_config(ConnectorConfig::default())
    }

    pub fn with_config(config: ConnectorConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent("Reaper-SIEM/1.0")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client, config }
    }

    /// Build the HTTP body for a batch. Splunk HEC wraps each record as a
    /// `{"event": …}` object (JSON records inlined, CEF as a string); generic
    /// HTTP sends the lines newline-delimited.
    fn build_body(connector: &SiemConnector, lines: &[String]) -> String {
        match connector.connector_type {
            ConnectorType::SplunkHec => {
                let sourcetype = match connector.format {
                    ExportFormat::Cef => "cef",
                    _ => "_json",
                };
                lines
                    .iter()
                    .map(|line| match connector.format {
                        // JSON record: inline it as the event value.
                        ExportFormat::Ndjson | ExportFormat::Ocsf => {
                            format!("{{\"event\":{line},\"sourcetype\":\"{sourcetype}\"}}")
                        }
                        // CEF is a string; encode it as a JSON string event.
                        ExportFormat::Cef => {
                            let encoded =
                                serde_json::to_string(line).unwrap_or_else(|_| "\"\"".to_string());
                            format!("{{\"event\":{encoded},\"sourcetype\":\"{sourcetype}\"}}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            ConnectorType::Http => lines.join("\n"),
        }
    }

    fn content_type(connector: &SiemConnector) -> &'static str {
        match connector.connector_type {
            // HEC always speaks JSON, whatever the inner record format is.
            ConnectorType::SplunkHec => "application/json",
            ConnectorType::Http => match connector.format {
                ExportFormat::Cef => "text/plain",
                _ => "application/x-ndjson",
            },
        }
    }

    /// Deliver a batch of shaped records to `connector`, with retries. An empty
    /// batch is a no-op success (nothing to ship).
    pub async fn deliver(
        &self,
        connector: &SiemConnector,
        lines: &[String],
    ) -> ConnectorDeliveryResult {
        let start = Instant::now();
        if lines.is_empty() {
            return ConnectorDeliveryResult {
                success: true,
                records: 0,
                status_code: None,
                error: None,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }

        let body = Self::build_body(connector, lines);
        let content_type = Self::content_type(connector);

        // Generic HTTP: HMAC-sign the body when a secret is configured (mirrors
        // the webhook signature). Splunk HEC authenticates with its token header.
        let signature = match connector.connector_type {
            ConnectorType::Http => connector.secret.as_ref().map(|secret| {
                let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes())
                    .expect("HMAC accepts any key length");
                mac.update(body.as_bytes());
                format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
            }),
            ConnectorType::SplunkHec => None,
        };

        let mut last_error = None;
        let mut last_status = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let delay =
                    Duration::from_millis(self.config.retry_delay_ms * 2u64.pow(attempt - 1));
                tokio::time::sleep(delay).await;
            }

            let mut req = self
                .client
                .post(&connector.endpoint)
                .header("Content-Type", content_type);
            match connector.connector_type {
                ConnectorType::SplunkHec => {
                    if let Some(token) = &connector.secret {
                        req = req.header("Authorization", format!("Splunk {token}"));
                    }
                }
                ConnectorType::Http => {
                    if let Some(sig) = &signature {
                        req = req.header("X-Reaper-Signature", sig);
                    }
                }
            }

            match req.body(body.clone()).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    last_status = Some(status.as_u16());
                    if status.is_success() {
                        return ConnectorDeliveryResult {
                            success: true,
                            records: lines.len(),
                            status_code: Some(status.as_u16()),
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                        };
                    }
                    if status.is_server_error() && attempt < self.config.max_retries {
                        last_error = Some(format!("server error: {status}"));
                        continue;
                    }
                    last_error = Some(format!("HTTP {status}"));
                    break;
                }
                Err(e) => {
                    let retryable = e.is_connect() || e.is_timeout();
                    last_error = Some(e.to_string());
                    if retryable && attempt < self.config.max_retries {
                        continue;
                    }
                    break;
                }
            }
        }

        ConnectorDeliveryResult {
            success: false,
            records: lines.len(),
            status_code: last_status,
            error: last_error,
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }
}

impl Default for ConnectorDeliveryService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn connector(connector_type: ConnectorType, format: ExportFormat) -> SiemConnector {
        SiemConnector {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            name: "test".to_string(),
            connector_type,
            endpoint: "http://localhost:0".to_string(),
            secret: None,
            format,
            enabled: true,
            failure_count: 0,
            last_export_at: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn hec_wraps_json_records_as_events() {
        let c = connector(ConnectorType::SplunkHec, ExportFormat::Ocsf);
        let body = ConnectorDeliveryService::build_body(&c, &[r#"{"class_uid":3003}"#.to_string()]);
        assert_eq!(body, r#"{"event":{"class_uid":3003},"sourcetype":"_json"}"#);
    }

    #[test]
    fn hec_wraps_cef_as_string_event() {
        let c = connector(ConnectorType::SplunkHec, ExportFormat::Cef);
        let body = ConnectorDeliveryService::build_body(&c, &["CEF:0|Reaper|x".to_string()]);
        // The CEF string is JSON-encoded as the event value.
        assert_eq!(body, r#"{"event":"CEF:0|Reaper|x","sourcetype":"cef"}"#);
    }

    #[test]
    fn http_joins_lines_ndjson() {
        let c = connector(ConnectorType::Http, ExportFormat::Ndjson);
        let body = ConnectorDeliveryService::build_body(&c, &["a".to_string(), "b".to_string()]);
        assert_eq!(body, "a\nb");
        assert_eq!(
            ConnectorDeliveryService::content_type(&c),
            "application/x-ndjson"
        );
    }

    #[tokio::test]
    async fn empty_batch_is_a_noop_success() {
        let c = connector(ConnectorType::Http, ExportFormat::Ocsf);
        let r = ConnectorDeliveryService::new().deliver(&c, &[]).await;
        assert!(r.success);
        assert_eq!(r.records, 0);
        assert!(r.status_code.is_none());
    }
}
