//! Unix Domain Socket HTTP client for the Reaper SDK.
//!
//! Uses hyper + tokio to send HTTP requests over a Unix socket.
//! This provides lower latency than TCP for same-host/pod communication.

use crate::error::{ReaperError, Result};
use crate::types::{DeployBundleRequest, DeployBundleResponse, PolicyRequest, PolicyResponse};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::UnixStream;
use tower::Service;
use tracing::{debug, error, info};

/// A connector that creates Unix socket connections for hyper.
///
/// Implements `tower::Service<hyper::Uri>` so it can be used as a hyper connector.
/// The URI authority is ignored — all connections go to the configured socket path.
#[derive(Clone)]
struct UnixConnector {
    socket_path: Arc<PathBuf>,
}

impl Service<hyper::Uri> for UnixConnector {
    type Response = hyper_util::rt::TokioIo<UnixStream>;
    type Error = std::io::Error;
    type Future =
        Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: hyper::Uri) -> Self::Future {
        let path = Arc::clone(&self.socket_path);
        Box::pin(async move {
            let stream = UnixStream::connect(&*path).await?;
            Ok(hyper_util::rt::TokioIo::new(stream))
        })
    }
}

/// UDS-based HTTP client for communicating with a Reaper Agent.
///
/// Mirrors the `ReaperHttpClient` API but uses Unix Domain Sockets
/// instead of TCP for lower-latency same-host IPC.
pub struct ReaperUdsClient {
    client: Client<UnixConnector, Full<Bytes>>,
    socket_path: PathBuf,
}

impl ReaperUdsClient {
    /// Create a new UDS client.
    ///
    /// # Arguments
    /// * `socket_path` - Path to the Unix socket file
    ///
    /// # Example
    /// ```no_run
    /// use reaper_sdk::uds_client::ReaperUdsClient;
    ///
    /// let client = ReaperUdsClient::new("/var/run/reaper/agent.sock").unwrap();
    /// ```
    pub fn new(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        let connector = UnixConnector {
            socket_path: Arc::new(socket_path.clone()),
        };

        let client = Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(128)
            .pool_idle_timeout(Duration::from_secs(30))
            .build(connector);

        info!("Created Reaper UDS client for {}", socket_path.display());

        Ok(Self {
            client,
            socket_path,
        })
    }

    /// Evaluate a policy request.
    pub async fn evaluate(&self, request: PolicyRequest) -> Result<PolicyResponse> {
        debug!(
            "Evaluating policy via UDS: policy_id={}, principal={}, action={}, resource={}",
            request.policy_id, request.principal, request.action, request.resource
        );

        let body = sonic_rs::to_vec(&request)
            .map_err(|e| ReaperError::Other(format!("Failed to serialize request: {}", e)))?;
        let response = self.send_request("POST", "/api/v1/messages", body).await?;
        let policy_response: PolicyResponse = sonic_rs::from_slice(&response)
            .map_err(|e| ReaperError::Other(format!("Failed to parse response: {}", e)))?;

        debug!(
            "Policy evaluation complete: decision={:?}, latency={}ns",
            policy_response.decision, policy_response.latency_ns
        );

        Ok(policy_response)
    }

    /// Deploy a policy bundle to the agent.
    pub async fn deploy_bundle(
        &self,
        bundle_bytes: &[u8],
        version: &str,
        force: bool,
    ) -> Result<DeployBundleResponse> {
        info!(
            "Deploying bundle via UDS: version={}, size={} bytes, force={}",
            version,
            bundle_bytes.len(),
            force
        );

        let request = DeployBundleRequest {
            bundle: bundle_bytes.to_vec(),
            version: version.to_string(),
            force,
        };

        let body = sonic_rs::to_vec(&request)
            .map_err(|e| ReaperError::Other(format!("Failed to serialize request: {}", e)))?;
        let response = self
            .send_request("POST", "/api/v1/bundles/deploy", body)
            .await?;
        let deploy_response: DeployBundleResponse = sonic_rs::from_slice(&response)
            .map_err(|e| ReaperError::Other(format!("Failed to parse response: {}", e)))?;

        info!(
            "Bundle deployed successfully: policy_id={}, version={}",
            deploy_response.policy_id, deploy_response.version
        );

        Ok(deploy_response)
    }

    /// Check agent health.
    pub async fn health_check(&self) -> Result<()> {
        debug!("Checking agent health via UDS");

        self.send_request("GET", "/health", Vec::new()).await?;

        debug!("Agent is healthy (UDS)");
        Ok(())
    }

    /// Send a POST request with a JSON body and deserialize the response.
    ///
    /// Generic transport method — lets consumers use their own types.
    pub async fn post_json<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp> {
        let json_bytes = sonic_rs::to_vec(body)
            .map_err(|e| ReaperError::Other(format!("Failed to serialize request: {}", e)))?;
        let response_bytes = self.send_request("POST", path, json_bytes).await?;
        sonic_rs::from_slice(&response_bytes).map_err(|e| {
            error!("Failed to parse response from {}: {}", path, e);
            ReaperError::Other(format!("Failed to parse response: {}", e))
        })
    }

    /// Send a GET request and deserialize the response.
    ///
    /// Generic transport method — lets consumers use their own types.
    pub async fn get_json<Resp: DeserializeOwned>(&self, path: &str) -> Result<Resp> {
        let response_bytes = self.send_request("GET", path, Vec::new()).await?;
        sonic_rs::from_slice(&response_bytes).map_err(|e| {
            error!("Failed to parse response from {}: {}", path, e);
            ReaperError::Other(format!("Failed to parse response: {}", e))
        })
    }

    /// Send an HTTP request over the Unix socket.
    ///
    /// Returns `Bytes` directly to avoid a `.to_vec()` copy on the response path.
    async fn send_request(&self, method: &str, path: &str, body: Vec<u8>) -> Result<Bytes> {
        // URI authority is ignored by our connector, but hyper requires a valid URI.
        let uri = format!("http://localhost{}", path);

        let req = Request::builder()
            .method(method)
            .uri(&uri)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| ReaperError::UnixSocketError(format!("Failed to build request: {}", e)))?;

        let resp = self.client.request(req).await.map_err(|e| {
            error!(
                "UDS request failed (socket={}): {}",
                self.socket_path.display(),
                e
            );
            ReaperError::UnixSocketError(format!("Request failed: {}", e))
        })?;

        let status = resp.status();
        let collected = resp.into_body().collect().await.map_err(|e| {
            ReaperError::UnixSocketError(format!("Failed to read response body: {}", e))
        })?;
        let resp_bytes = collected.to_bytes();

        if !status.is_success() {
            let error_text = String::from_utf8_lossy(&resp_bytes).to_string();
            error!("UDS HTTP {} error: {}", status, error_text);
            return Err(ReaperError::AgentError(format!(
                "HTTP {} error: {}",
                status, error_text
            )));
        }

        Ok(resp_bytes)
    }
}
