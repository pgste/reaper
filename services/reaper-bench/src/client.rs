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

/// Batch response - matches agent's batch_evaluate_policy response format
#[derive(Debug, Clone, Deserialize)]
pub struct BatchResponse {
    pub policy_name: Option<String>,
    pub policy_id: Option<String>,
    pub request_count: u32,
    pub results: Vec<BatchResultItem>,
    pub summary: BatchSummary,
    pub agent_id: Option<String>,
}

/// Summary statistics from batch evaluation
#[derive(Debug, Clone, Deserialize)]
pub struct BatchSummary {
    pub allowed: u32,
    pub denied: u32,
    pub total_time_microseconds: f64,
    pub avg_time_microseconds: f64,
}

/// Individual result in a batch
#[derive(Debug, Clone, Deserialize)]
pub struct BatchResultItem {
    pub index: Option<u32>,
    pub decision: String,
    pub evaluation_time_microseconds: Option<f64>,
    #[serde(default)]
    pub cache_hit: bool,
    #[serde(default)]
    pub error: Option<String>,
}

// ============================================================================
// Package-related types
// ============================================================================

/// General evaluation request (for package and all policies evaluation)
#[derive(Debug, Clone, Serialize)]
pub struct EvaluateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    pub principal: String,
    pub action: String,
    pub resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<HashMap<String, String>>,
}

/// Package information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub policy_count: usize,
    pub policy_names: Vec<String>,
}

/// Denial information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenyInfo {
    pub policy_id: String,
    pub policy_name: String,
    pub package: String,
    pub matched_rule: Option<String>,
}

/// Response from package evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEvaluationResponse {
    pub package: String,
    pub decision: String,
    pub denied_by: Option<DenyInfo>,
    pub policies_evaluated: usize,
    pub total_evaluation_time_microseconds: f64,
}

/// Response from evaluating all policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllPoliciesEvaluationResponse {
    pub decision: String,
    pub denied_by: Option<DenyInfo>,
    pub policies_evaluated: usize,
    pub packages_evaluated: usize,
    pub total_evaluation_time_microseconds: f64,
}

/// List packages response
#[derive(Debug, Clone, Deserialize)]
pub struct ListPackagesResponse {
    pub packages: Vec<PackageInfo>,
    pub total: usize,
}

/// Package policies response
#[derive(Debug, Clone, Deserialize)]
pub struct PackagePoliciesResponse {
    pub package: String,
    pub policies: Vec<PolicyInfo>,
    pub total: usize,
}

/// Policy information
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyInfo {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub package: String,
    pub rules_count: usize,
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

    // ========================================================================
    // Package Evaluation Methods
    // ========================================================================

    /// List all packages
    pub async fn list_packages(&self, url: &str) -> anyhow::Result<Vec<PackageInfo>> {
        let response = self
            .client
            .get(format!("{}/api/v1/packages", url))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to list packages {}: {}", status, body);
        }

        let result: ListPackagesResponse = response.json().await?;
        Ok(result.packages)
    }

    /// Get policies in a specific package
    pub async fn get_package_policies(&self, url: &str, package: &str) -> anyhow::Result<PackagePoliciesResponse> {
        let response = self
            .client
            .get(format!("{}/api/v1/packages/{}/policies", url, package))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get package policies {}: {}", status, body);
        }

        Ok(response.json().await?)
    }

    /// Evaluate request against all policies in a specific package
    pub async fn evaluate_package(
        &self,
        url: &str,
        package: &str,
        request: &EvaluateRequest,
    ) -> anyhow::Result<PackageEvaluationResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/packages/{}/evaluate", url, package))
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Package evaluation failed {}: {}", status, body);
        }

        Ok(response.json().await?)
    }

    /// Evaluate request against ALL policies across ALL packages
    pub async fn evaluate_all(
        &self,
        url: &str,
        request: &EvaluateRequest,
    ) -> anyhow::Result<AllPoliciesEvaluationResponse> {
        let response = self
            .client
            .post(format!("{}/api/v1/evaluate", url))
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("All policies evaluation failed {}: {}", status, body);
        }

        Ok(response.json().await?)
    }

    /// Deploy a compiled policy to the agent
    pub async fn deploy_policy(
        &self,
        url: &str,
        policy_name: &str,
        policy_content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let response = self
            .client
            .post(format!("{}/api/v1/policies/compile", url))
            .json(&serde_json::json!({
                "policy_name": policy_name,
                "policy_content": policy_content
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Policy deployment failed {}: {}", status, body);
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
