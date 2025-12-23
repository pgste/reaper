//! # Reaper SDK
//!
//! High-performance SDK for evaluating policies against the Reaper policy engine.
//!
//! ## Features
//!
//! - **HTTP Client**: Simple RESTful client for policy evaluation (1-2ms latency)
//! - **Bundle Deployment**: Deploy policy bundles (.rbb format) with zero-downtime hot-reload
//! - **Connection Pooling**: Automatic connection reuse for high throughput
//! - **Type Safety**: Strongly-typed requests and responses
//!
//! ## Quick Start
//!
//! ```no_run
//! use reaper_sdk::{ReaperClient, PolicyRequest};
//! use std::collections::HashMap;
//!
//! #[tokio::main]
//! async fn main() -> reaper_sdk::Result<()> {
//!     // Create HTTP client
//!     let client = ReaperClient::http("http://localhost:8080")?;
//!
//!     // Evaluate a policy
//!     let request = PolicyRequest {
//!         policy_id: "my-policy".to_string(),
//!         principal: "user:alice".to_string(),
//!         action: "read".to_string(),
//!         resource: "/api/data".to_string(),
//!         context: HashMap::new(),
//!     };
//!
//!     let response = client.evaluate(request).await?;
//!     println!("Decision: {:?}", response.decision);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! The SDK communicates with a Reaper Agent running on localhost or a remote host.
//! The agent evaluates policies using a lock-free in-memory engine with sub-microsecond
//! latency for simple policies.
//!
//! ```text
//! SDK Client  ──HTTP──>  Agent (8080)  ──>  PolicyEngine  ──>  eBPF (optional)
//! ```

pub mod error;
pub mod http_client;
pub mod types;

pub use error::{ReaperError, Result};
pub use http_client::ReaperHttpClient;
pub use types::{
    Decision, DeployBundleRequest, DeployBundleResponse, EntityData, PolicyRequest, PolicyResponse,
    Relationship, Source,
};

/// Main SDK client (currently HTTP-only)
///
/// This is a convenience wrapper around `ReaperHttpClient`.
/// In the future, this will support multiple protocols (HTTP, UDP, Unix sockets).
pub struct ReaperClient {
    http_client: ReaperHttpClient,
}

impl ReaperClient {
    /// Create a new HTTP-based client
    ///
    /// # Arguments
    /// * `endpoint` - Base URL of the Reaper Agent (e.g., "http://localhost:8080")
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::ReaperClient;
    ///
    /// let client = ReaperClient::http("http://localhost:8080").unwrap();
    /// ```
    pub fn http(endpoint: &str) -> Result<Self> {
        let http_client = ReaperHttpClient::new(endpoint)?;
        Ok(Self { http_client })
    }

    /// Evaluate a policy request
    ///
    /// # Arguments
    /// * `request` - The policy request to evaluate
    ///
    /// # Returns
    /// Policy decision (Allow/Deny) with latency information
    pub async fn evaluate(&self, request: PolicyRequest) -> Result<PolicyResponse> {
        self.http_client.evaluate(request).await
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
    pub async fn deploy_bundle(
        &self,
        bundle_bytes: &[u8],
        version: &str,
        force: bool,
    ) -> Result<DeployBundleResponse> {
        self.http_client
            .deploy_bundle(bundle_bytes, version, force)
            .await
    }

    /// Check agent health
    ///
    /// # Returns
    /// `Ok(())` if agent is healthy, error otherwise
    pub async fn health_check(&self) -> Result<()> {
        self.http_client.health_check().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let result = ReaperClient::http("http://localhost:8080");
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_endpoint() {
        // Empty endpoint should still create client
        let result = ReaperClient::http("");
        assert!(result.is_ok());
    }
}
