//! HTTP client implementation for the Reaper SDK

use crate::error::{ReaperError, Result};
use crate::types::{DeployBundleRequest, DeployBundleResponse, PolicyRequest, PolicyResponse};
use reqwest::Client as HttpClient;
use std::time::Duration;
use tracing::{debug, error, info};

/// HTTP-based client for Reaper Agent
pub struct ReaperHttpClient {
    client: HttpClient,
    base_url: String,
}

impl ReaperHttpClient {
    /// Create a new HTTP client
    ///
    /// # Arguments
    /// * `endpoint` - Base URL of the Reaper Agent (e.g., "http://localhost:8080")
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::ReaperHttpClient;
    ///
    /// let client = ReaperHttpClient::new("http://localhost:8080").unwrap();
    /// ```
    pub fn new(endpoint: &str) -> Result<Self> {
        let client = HttpClient::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(|e| ReaperError::InvalidEndpoint(e.to_string()))?;

        // Ensure URL doesn't end with slash
        let base_url = endpoint.trim_end_matches('/').to_string();

        info!("Created Reaper HTTP client for {}", base_url);

        Ok(Self { client, base_url })
    }

    /// Evaluate a policy request
    ///
    /// # Arguments
    /// * `request` - The policy request to evaluate
    ///
    /// # Returns
    /// Policy decision (Allow/Deny) with latency information
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::{ReaperHttpClient, PolicyRequest};
    /// use std::collections::HashMap;
    ///
    /// # async fn example() -> reaper_sdk::Result<()> {
    /// let client = ReaperHttpClient::new("http://localhost:8080")?;
    ///
    /// let request = PolicyRequest {
    ///     policy_id: "my-policy".to_string(),
    ///     principal: "user:alice".to_string(),
    ///     action: "read".to_string(),
    ///     resource: "/api/data".to_string(),
    ///     context: HashMap::new(),
    /// };
    ///
    /// let response = client.evaluate(request).await?;
    /// println!("Decision: {:?}", response.decision);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn evaluate(&self, request: PolicyRequest) -> Result<PolicyResponse> {
        let url = format!("{}/api/v1/messages", self.base_url);

        debug!(
            "Evaluating policy: policy_id={}, principal={}, action={}, resource={}",
            request.policy_id, request.principal, request.action, request.resource
        );

        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("HTTP request failed: {}", e);
                ReaperError::HttpError(e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("HTTP {} error: {}", status, error_text);
            return Err(ReaperError::HttpStatus(status));
        }

        let policy_response: PolicyResponse = resp.json().await.map_err(|e| {
            error!("Failed to parse response: {}", e);
            ReaperError::HttpError(e)
        })?;

        debug!(
            "Policy evaluation complete: decision={:?}, latency={}ns",
            policy_response.decision, policy_response.latency_ns
        );

        Ok(policy_response)
    }

    /// Deploy a policy bundle to the agent
    ///
    /// # Arguments
    /// * `bundle_bytes` - Raw .rbb bundle bytes
    /// * `version` - Expected version string
    /// * `force` - Force deployment even if version check fails
    ///
    /// # Returns
    /// Deployment response with policy ID and version info
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::ReaperHttpClient;
    ///
    /// # async fn example() -> reaper_sdk::Result<()> {
    /// let client = ReaperHttpClient::new("http://localhost:8080")?;
    ///
    /// let bundle_bytes = std::fs::read("policy.rbb").unwrap();
    /// let response = client.deploy_bundle(&bundle_bytes, "1.0.0", false).await?;
    ///
    /// println!("Deployed policy: {}", response.policy_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn deploy_bundle(
        &self,
        bundle_bytes: &[u8],
        version: &str,
        force: bool,
    ) -> Result<DeployBundleResponse> {
        let url = format!("{}/api/v1/bundles/deploy", self.base_url);

        info!(
            "Deploying bundle: version={}, size={} bytes, force={}",
            version,
            bundle_bytes.len(),
            force
        );

        let request = DeployBundleRequest {
            bundle: bundle_bytes.to_vec(),
            version: version.to_string(),
            force,
        };

        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("HTTP request failed: {}", e);
                ReaperError::HttpError(e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("HTTP {} error: {}", status, error_text);
            return Err(ReaperError::BundleError(format!(
                "HTTP {} error: {}",
                status, error_text
            )));
        }

        let deploy_response: DeployBundleResponse = resp.json().await.map_err(|e| {
            error!("Failed to parse response: {}", e);
            ReaperError::HttpError(e)
        })?;

        info!(
            "Bundle deployed successfully: policy_id={}, version={}",
            deploy_response.policy_id, deploy_response.version
        );

        Ok(deploy_response)
    }

    /// Check agent health
    ///
    /// # Returns
    /// `Ok(())` if agent is healthy, error otherwise
    pub async fn health_check(&self) -> Result<()> {
        let url = format!("{}/health", self.base_url);

        debug!("Checking agent health");

        let resp = self.client.get(&url).send().await.map_err(|e| {
            error!("Health check failed: {}", e);
            ReaperError::HttpError(e)
        })?;

        if !resp.status().is_success() {
            return Err(ReaperError::AgentError(format!(
                "Agent unhealthy: HTTP {}",
                resp.status()
            )));
        }

        debug!("Agent is healthy");
        Ok(())
    }
}
