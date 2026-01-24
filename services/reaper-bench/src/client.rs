//! HTTP client for communicating with Reaper Agent
//!
//! Supports mTLS authentication and HTTP/2 for optimal performance.

use reqwest::{Certificate, Client, Identity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::time::Duration;
use tracing::info;

/// Agent client wrapper
pub struct AgentClient {
    client: Client,
}

/// Policy evaluation request
#[derive(Debug, Clone, Serialize)]
pub struct PolicyRequest {
    pub policy_name: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<HashMap<String, String>>,
}

/// Policy evaluation response
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyResponse {
    pub decision: String,
    pub policy_id: Option<String>,
    pub policy_name: Option<String>,
    pub policy_version: Option<u64>,
    pub evaluation_time_microseconds: Option<f64>,
    pub total_time_microseconds: Option<f64>,
    pub matched_rule: Option<String>,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub cache_hit: bool,
}

/// Batch request for high-throughput mode
#[derive(Debug, Clone, Serialize)]
pub struct BatchRequest {
    pub policy_name: String,
    pub requests: Vec<BatchRequestItem>,
}

/// Individual request in a batch
#[derive(Debug, Clone, Serialize)]
pub struct BatchRequestItem {
    pub principal: String,
    pub action: String,
    pub resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<HashMap<String, String>>,
}

/// Batch response
#[derive(Debug, Clone, Deserialize)]
pub struct BatchResponse {
    pub total_requests: u32,
    pub successful: u32,
    pub failed: u32,
    pub total_time_microseconds: f64,
    pub results: Vec<BatchResultItem>,
}

/// Individual result in a batch
#[derive(Debug, Clone, Deserialize)]
pub struct BatchResultItem {
    pub decision: String,
    pub evaluation_time_microseconds: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

impl AgentClient {
    /// Create a new agent client
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Get the underlying reqwest client
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// Evaluate a single policy request (fast-messages endpoint)
    pub async fn evaluate(&self, url: &str, request: &PolicyRequest) -> anyhow::Result<PolicyResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/fast-messages", url))
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Agent returned error {}: {}", status, body);
        }

        Ok(response.json().await?)
    }

    /// Evaluate a batch of requests (batch-messages endpoint)
    pub async fn evaluate_batch(
        &self,
        url: &str,
        request: &BatchRequest,
    ) -> anyhow::Result<BatchResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/batch-messages", url))
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Agent returned error {}: {}", status, body);
        }

        Ok(response.json().await?)
    }

    /// Check agent health
    pub async fn health(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        let response = self
            .client
            .get(format!("{}/health", url))
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Agent health check failed: {}", response.status());
        }

        Ok(response.json().await?)
    }

    /// Load entity data into the agent's DataStore
    pub async fn load_data(&self, url: &str, data_json: &str) -> anyhow::Result<serde_json::Value> {
        let payload = serde_json::json!({
            "data": data_json
        });

        let response = self
            .client
            .post(format!("{}/api/v1/data", url))
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to load data {}: {}", status, body);
        }

        Ok(response.json().await?)
    }
}

/// Create an agent client with optional mTLS configuration
pub fn create_agent_client(
    ca_path: Option<&str>,
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> anyhow::Result<AgentClient> {
    let mut builder = Client::builder()
        .use_rustls_tls()
        .http2_prior_knowledge() // Force HTTP/2
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(30))
        .tcp_keepalive(Duration::from_secs(60))
        .timeout(Duration::from_secs(30));

    // Add CA certificate if provided
    if let Some(ca) = ca_path {
        info!("Loading CA certificate from: {}", ca);
        let ca_bytes = fs::read(ca)?;
        let ca_cert = Certificate::from_pem(&ca_bytes)?;
        builder = builder.add_root_certificate(ca_cert);
    }

    // Add client certificate and key for mTLS if both provided
    if let (Some(cert), Some(key)) = (cert_path, key_path) {
        info!("Loading client certificate from: {}", cert);
        info!("Loading client key from: {}", key);

        let cert_bytes = fs::read(cert)?;
        let key_bytes = fs::read(key)?;

        // Combine cert and key into identity
        let mut pem = cert_bytes.clone();
        pem.extend_from_slice(b"\n");
        pem.extend_from_slice(&key_bytes);

        let identity = Identity::from_pem(&pem)?;
        builder = builder.identity(identity);
    }

    // Allow invalid certificates in development
    if std::env::var("REAPER_TLS_INSECURE").is_ok() {
        builder = builder.danger_accept_invalid_certs(true);
    }

    let client = builder.build()?;
    Ok(AgentClient::new(client))
}
