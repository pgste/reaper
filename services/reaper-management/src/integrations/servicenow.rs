//! ServiceNow change-record validation (Plan 10 follow-up).
//!
//! When an environment's approval policy sets
//! `external_change_record: validated`, a promotion must reference a
//! ServiceNow change record (CHG number) that exists and is approved on the
//! configured instance. The lookup uses the Table API
//! (`GET /api/now/table/change_request?sysparm_query=number=...`) with basic
//! auth. Everything here fails closed: an unreachable instance or an
//! unexpected response blocks the promotion rather than letting it through.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::config::ServiceNowConfig;

/// Outcome of checking a change-record reference against ServiceNow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeRecordCheck {
    /// The record exists and its `approval` field is accepted.
    Valid,
    /// No record with that number exists.
    NotFound,
    /// The record exists but its `approval` value is not accepted
    /// (e.g. `requested`, `rejected`).
    NotApproved(String),
}

#[derive(Debug, Error)]
pub enum ServiceNowError {
    #[error("change-record reference '{0}' is not a valid identifier")]
    InvalidReference(String),
    #[error("ServiceNow request failed: {0}")]
    Http(String),
    #[error("ServiceNow returned status {0}")]
    Status(u16),
    #[error("ServiceNow response could not be parsed: {0}")]
    Parse(String),
}

#[derive(Debug, Deserialize)]
struct TableResponse {
    #[serde(default)]
    result: Vec<ChangeRecordRow>,
}

#[derive(Debug, Deserialize)]
struct ChangeRecordRow {
    #[serde(default)]
    approval: String,
}

/// Minimal ServiceNow Table API client for change-record validation.
pub struct ServiceNowClient {
    config: ServiceNowConfig,
    http: reqwest::Client,
}

impl ServiceNowClient {
    pub fn new(config: ServiceNowConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Only plain record identifiers are accepted (letters, digits, `-`/`_`,
    /// up to 64 chars) so the reference can never smuggle extra
    /// `sysparm_query` operators.
    pub fn is_valid_reference(reference: &str) -> bool {
        !reference.is_empty()
            && reference.len() <= 64
            && reference
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// Look up `reference` (e.g. `CHG0031337`) and report whether it is an
    /// approved change record.
    pub async fn validate_change_record(
        &self,
        reference: &str,
    ) -> Result<ChangeRecordCheck, ServiceNowError> {
        if !Self::is_valid_reference(reference) {
            return Err(ServiceNowError::InvalidReference(reference.to_string()));
        }

        let url = format!(
            "{}/api/now/table/change_request",
            self.config.base_url.trim_end_matches('/')
        );
        let mut request = self
            .http
            .get(&url)
            .query(&[
                ("sysparm_query", format!("number={reference}")),
                ("sysparm_fields", "number,approval".to_string()),
                ("sysparm_limit", "1".to_string()),
            ])
            .header("Accept", "application/json");
        if !self.config.username.is_empty() || self.config.api_token.is_some() {
            request = request.basic_auth(&self.config.username, self.config.api_token.as_deref());
        }

        let response = request
            .send()
            .await
            .map_err(|e| ServiceNowError::Http(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ServiceNowError::Status(status.as_u16()));
        }
        let body: TableResponse = response
            .json()
            .await
            .map_err(|e| ServiceNowError::Parse(e.to_string()))?;

        let Some(record) = body.result.first() else {
            return Ok(ChangeRecordCheck::NotFound);
        };
        let approval = record.approval.to_ascii_lowercase();
        if self
            .config
            .accepted_approvals
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&approval))
        {
            Ok(ChangeRecordCheck::Valid)
        } else {
            Ok(ChangeRecordCheck::NotApproved(record.approval.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_format_guard() {
        assert!(ServiceNowClient::is_valid_reference("CHG0031337"));
        assert!(ServiceNowClient::is_valid_reference("chg-123_a"));
        assert!(!ServiceNowClient::is_valid_reference(""));
        assert!(!ServiceNowClient::is_valid_reference("CHG^ORDERBYnumber"));
        assert!(!ServiceNowClient::is_valid_reference("CHG 123"));
        assert!(!ServiceNowClient::is_valid_reference(&"C".repeat(65)));
    }

    #[tokio::test]
    async fn validates_against_a_stub_instance() {
        use axum::{extract::Query, routing::get, Json, Router};
        use std::collections::HashMap;

        // Stub Table API: CHG0000001 approved, CHG0000002 requested,
        // anything else not found.
        let app = Router::new().route(
            "/api/now/table/change_request",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let number = q
                    .get("sysparm_query")
                    .and_then(|s| s.strip_prefix("number="))
                    .unwrap_or_default()
                    .to_string();
                let result = match number.as_str() {
                    "CHG0000001" => {
                        serde_json::json!([{"number": number, "approval": "approved"}])
                    }
                    "CHG0000002" => {
                        serde_json::json!([{"number": number, "approval": "requested"}])
                    }
                    _ => serde_json::json!([]),
                };
                Json(serde_json::json!({"result": result}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = ServiceNowClient::new(ServiceNowConfig::new(format!("http://{addr}")));
        assert_eq!(
            client.validate_change_record("CHG0000001").await.unwrap(),
            ChangeRecordCheck::Valid
        );
        assert_eq!(
            client.validate_change_record("CHG0000002").await.unwrap(),
            ChangeRecordCheck::NotApproved("requested".to_string())
        );
        assert_eq!(
            client.validate_change_record("CHG0000404").await.unwrap(),
            ChangeRecordCheck::NotFound
        );
        assert!(matches!(
            client.validate_change_record("CHG^EVIL").await,
            Err(ServiceNowError::InvalidReference(_))
        ));
    }
}
