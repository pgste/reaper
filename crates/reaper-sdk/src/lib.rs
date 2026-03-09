//! # Reaper SDK
//!
//! High-performance SDK for evaluating policies against the Reaper policy engine.
//!
//! ## Features
//!
//! - **HTTP Client**: Simple RESTful client for policy evaluation (1-2ms latency)
//! - **UDS Client**: Unix Domain Socket transport for same-host/pod deployments (~20-40% lower latency)
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
//!     // Create HTTP client (TCP)
//!     let client = ReaperClient::http("http://localhost:8080")?;
//!
//!     // Or create UDS client (lower latency for same-host)
//!     // let client = ReaperClient::unix("/var/run/reaper/agent.sock")?;
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
//! SDK Client  ──HTTP/UDS──>  Agent (8080 / socket)  ──>  PolicyEngine
//! ```

pub mod error;
pub mod http_client;
pub mod transport;
pub mod types;
pub mod uds_client;

pub use error::{ReaperError, Result};
pub use http_client::ReaperHttpClient;
pub use transport::Transport;
pub use types::{
    Decision, DeployBundleRequest, DeployBundleResponse, EntityData, PolicyRequest, PolicyResponse,
    Relationship, Source,
};
pub use uds_client::ReaperUdsClient;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// Internal transport dispatch.
enum ClientInner {
    Http(ReaperHttpClient),
    Unix(Box<ReaperUdsClient>),
}

/// Main SDK client supporting HTTP and Unix Domain Socket transports.
///
/// Use `ReaperClient::http()` for TCP connections (default) or
/// `ReaperClient::unix()` for same-host UDS connections.
/// The `ReaperClient::http()` signature is unchanged for backward compatibility.
pub struct ReaperClient {
    inner: ClientInner,
}

impl ReaperClient {
    /// Create a new HTTP-based client (backward compatible).
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
        Ok(Self {
            inner: ClientInner::Http(http_client),
        })
    }

    /// Create a new UDS-based client for same-host communication.
    ///
    /// # Arguments
    /// * `socket_path` - Path to the Unix socket file (e.g., "/var/run/reaper/agent.sock")
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::ReaperClient;
    ///
    /// let client = ReaperClient::unix("/var/run/reaper/agent.sock").unwrap();
    /// ```
    pub fn unix(socket_path: impl AsRef<Path>) -> Result<Self> {
        let uds_client = ReaperUdsClient::new(socket_path)?;
        Ok(Self {
            inner: ClientInner::Unix(Box::new(uds_client)),
        })
    }

    /// Create a client from a `Transport` configuration.
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::{ReaperClient, Transport};
    ///
    /// let client = ReaperClient::from_transport(Transport::http("http://localhost:8080")).unwrap();
    /// let client = ReaperClient::from_transport(Transport::unix("/var/run/reaper/agent.sock")).unwrap();
    /// ```
    pub fn from_transport(transport: Transport) -> Result<Self> {
        match transport {
            Transport::Http { endpoint } => Self::http(&endpoint),
            Transport::Unix { socket_path } => Self::unix(socket_path),
        }
    }

    /// Evaluate a policy request.
    ///
    /// # Arguments
    /// * `request` - The policy request to evaluate
    ///
    /// # Returns
    /// Policy decision (Allow/Deny) with latency information
    pub async fn evaluate(&self, request: PolicyRequest) -> Result<PolicyResponse> {
        match &self.inner {
            ClientInner::Http(client) => client.evaluate(request).await,
            ClientInner::Unix(client) => client.evaluate(request).await,
        }
    }

    /// Deploy a policy bundle to the agent.
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
        match &self.inner {
            ClientInner::Http(client) => client.deploy_bundle(bundle_bytes, version, force).await,
            ClientInner::Unix(client) => client.deploy_bundle(bundle_bytes, version, force).await,
        }
    }

    /// Check agent health.
    ///
    /// # Returns
    /// `Ok(())` if agent is healthy, error otherwise
    pub async fn health_check(&self) -> Result<()> {
        match &self.inner {
            ClientInner::Http(client) => client.health_check().await,
            ClientInner::Unix(client) => client.health_check().await,
        }
    }

    /// Send a POST request with a JSON body and deserialize the response.
    ///
    /// Generic transport method that lets consumers use their own request/response types
    /// while leveraging the SDK's transport layer (TCP or UDS).
    ///
    /// # Arguments
    /// * `path` - URL path (e.g., "/api/v1/messages")
    /// * `body` - Request body (must implement `Serialize`)
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::ReaperClient;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Serialize)]
    /// struct MyRequest { value: String }
    ///
    /// #[derive(Deserialize)]
    /// struct MyResponse { result: String }
    ///
    /// # async fn example() -> reaper_sdk::Result<()> {
    /// let client = ReaperClient::http("http://localhost:8080")?;
    /// let resp: MyResponse = client.post_json("/api/v1/custom", &MyRequest { value: "test".into() }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn post_json<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp> {
        match &self.inner {
            ClientInner::Http(client) => client.post_json(path, body).await,
            ClientInner::Unix(client) => client.post_json(path, body).await,
        }
    }

    /// Send a GET request and deserialize the response.
    ///
    /// Generic transport method that lets consumers use their own response types
    /// while leveraging the SDK's transport layer (TCP or UDS).
    ///
    /// # Arguments
    /// * `path` - URL path (e.g., "/api/v1/packages")
    pub async fn get_json<Resp: DeserializeOwned>(&self, path: &str) -> Result<Resp> {
        match &self.inner {
            ClientInner::Http(client) => client.get_json(path).await,
            ClientInner::Unix(client) => client.get_json(path).await,
        }
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

    #[test]
    fn test_unix_client_creation() {
        // Creating a UDS client should succeed even if the socket doesn't exist yet
        // (connection is lazy — it only fails when a request is made)
        let result = ReaperClient::unix("/tmp/nonexistent-reaper-test.sock");
        assert!(result.is_ok());
    }

    #[test]
    fn test_transport_http() {
        let transport = Transport::http("http://localhost:8080");
        let result = ReaperClient::from_transport(transport);
        assert!(result.is_ok());
    }

    #[test]
    fn test_transport_unix() {
        let transport = Transport::unix("/tmp/nonexistent-reaper-test.sock");
        let result = ReaperClient::from_transport(transport);
        assert!(result.is_ok());
    }
}
