//! HTTP/UDS client for communicating with Reaper Agent
//!
//! Uses the Reaper SDK for transport (TCP or Unix Domain Socket).
//! Supports mTLS authentication via reqwest and UDS for same-host/pod deployments.

// Benchmark harness: variants are wired up per-run, so not all paths/fields
// are referenced in every build.
#![allow(dead_code)]

use reaper_sdk::ReaperClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// Agent client wrapper using the Reaper SDK's transport layer.
///
/// Supports both TCP (HTTP/HTTPS) and Unix Domain Socket connections.
pub struct AgentClient {
    client: ReaperClient,
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
    pub decision_id: Option<String>,
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
    /// Request identifier for correlation (required by agent)
    pub id: String,
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
    /// Create a new agent client wrapping a ReaperClient.
    pub fn new(client: ReaperClient) -> Self {
        Self { client }
    }

    /// Get a reference to the underlying ReaperClient.
    pub fn sdk_client(&self) -> &ReaperClient {
        &self.client
    }

    /// Evaluate a single policy request (fast-messages endpoint)
    pub async fn evaluate(
        &self,
        _url: &str,
        request: &PolicyRequest,
    ) -> anyhow::Result<PolicyResponse> {
        self.client
            .post_json("/api/v1/fast-messages", request)
            .await
            .map_err(|e| anyhow::anyhow!("Agent evaluate error: {}", e))
    }

    /// Evaluate a batch of requests (batch-messages endpoint)
    pub async fn evaluate_batch(
        &self,
        _url: &str,
        request: &BatchRequest,
    ) -> anyhow::Result<BatchResponse> {
        self.client
            .post_json("/api/v1/batch-messages", request)
            .await
            .map_err(|e| anyhow::anyhow!("Agent batch evaluate error: {}", e))
    }

    /// Check agent health
    pub async fn health(&self, _url: &str) -> anyhow::Result<serde_json::Value> {
        self.client
            .get_json("/health")
            .await
            .map_err(|e| anyhow::anyhow!("Agent health check failed: {}", e))
    }

    /// Load entity data into the agent's DataStore
    pub async fn load_data(
        &self,
        _url: &str,
        data_json: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let payload = serde_json::json!({
            "data": data_json
        });

        self.client
            .post_json("/api/v1/data", &payload)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load data: {}", e))
    }

    // ========================================================================
    // Package Evaluation Methods
    // ========================================================================

    /// List all packages
    pub async fn list_packages(&self, _url: &str) -> anyhow::Result<Vec<PackageInfo>> {
        let result: ListPackagesResponse = self
            .client
            .get_json("/api/v1/packages")
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list packages: {}", e))?;
        Ok(result.packages)
    }

    /// Get policies in a specific package
    pub async fn get_package_policies(
        &self,
        _url: &str,
        package: &str,
    ) -> anyhow::Result<PackagePoliciesResponse> {
        self.client
            .get_json(&format!("/api/v1/packages/{}/policies", package))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get package policies: {}", e))
    }

    /// Evaluate request against all policies in a specific package
    pub async fn evaluate_package(
        &self,
        _url: &str,
        package: &str,
        request: &EvaluateRequest,
    ) -> anyhow::Result<PackageEvaluationResponse> {
        self.client
            .post_json(&format!("/api/v1/packages/{}/evaluate", package), request)
            .await
            .map_err(|e| anyhow::anyhow!("Package evaluation failed: {}", e))
    }

    /// Evaluate request against ALL policies across ALL packages
    pub async fn evaluate_all(
        &self,
        _url: &str,
        request: &EvaluateRequest,
    ) -> anyhow::Result<AllPoliciesEvaluationResponse> {
        self.client
            .post_json("/api/v1/evaluate", request)
            .await
            .map_err(|e| anyhow::anyhow!("All policies evaluation failed: {}", e))
    }

    /// Deploy a compiled policy to the agent
    pub async fn deploy_policy(
        &self,
        _url: &str,
        policy_name: &str,
        policy_content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.client
            .post_json(
                "/api/v1/policies/compile",
                &serde_json::json!({
                    "policy_name": policy_name,
                    "policy_content": policy_content
                }),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Policy deployment failed: {}", e))
    }
}

/// Create an agent client with optional mTLS and/or UDS transport.
///
/// If `uds_path` is provided, connects via Unix Domain Socket (ignores TLS settings).
/// Otherwise, connects via HTTP/HTTPS with optional mTLS.
pub fn create_agent_client(
    agent_url: &str,
    uds_path: Option<&str>,
    _ca_path: Option<&str>,
    _cert_path: Option<&str>,
    _key_path: Option<&str>,
) -> anyhow::Result<AgentClient> {
    let client = if let Some(socket_path) = uds_path {
        info!("Creating UDS client: {}", socket_path);
        if !Path::new(socket_path).exists() {
            info!(
                "Note: socket file does not exist yet (agent may not be running): {}",
                socket_path
            );
        }
        ReaperClient::unix(socket_path)
            .map_err(|e| anyhow::anyhow!("Failed to create UDS client: {}", e))?
    } else {
        info!("Creating HTTP client: {}", agent_url);
        ReaperClient::http(agent_url)
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?
    };

    Ok(AgentClient::new(client))
}
