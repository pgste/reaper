//! Pluggable HTTP transport for the benchmark harness: TCP or Unix domain socket.
//!
//! Both engines expose the **identical** HTTP/1.1 JSON API over either transport:
//! - Reaper serves its axum router over a `UnixListener` when `REAPER_UDS_ENABLED=1`
//!   (see `services/reaper-agent/src/uds.rs`), in addition to TCP.
//! - OPA binds a socket with `opa run --server --addr unix:///path/to.sock`, and
//!   accepts multiple `--addr` flags so it can serve TCP *and* UDS at once.
//!
//! So a single [`Transport`] abstraction lets `main.rs` drive Reaper-vs-OPA over
//! TCP or over UDS with no change to the request/response logic — only the wire
//! transport differs, which is exactly the variable a UDS comparison isolates.
//!
//! `reqwest` has no Unix-socket transport, so the UDS path uses `hyper` directly
//! with a `tower::Service` connector that dials the configured socket (the same
//! pattern as `crates/reaper-sdk/src/uds_client.rs`).

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::net::UnixStream;
use tower::Service;

/// Per-request timeout, matching the TCP client's original `reqwest` timeout so
/// TCP and UDS runs fail-slow on the same budget.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// A hyper connector that dials a fixed Unix socket path, ignoring the URI
/// authority (which hyper still requires to be syntactically valid).
#[derive(Clone)]
pub(crate) struct UnixConnector {
    socket_path: Arc<PathBuf>,
}

impl Service<hyper::Uri> for UnixConnector {
    type Response = hyper_util::rt::TokioIo<UnixStream>;
    type Error = std::io::Error;
    type Future =
        Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
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

/// HTTP transport to a single engine endpoint.
///
/// Constructed from an endpoint string: an `http://host:port` base selects TCP;
/// a `unix:/absolute/path.sock` endpoint selects a Unix domain socket.
#[derive(Clone)]
pub enum Transport {
    Tcp {
        client: reqwest::Client,
        base: String,
    },
    Uds {
        // Boxed: the hyper client is far larger than the Tcp variant, and there
        // are only ever two Transport values in the process, so indirection here
        // costs nothing and keeps the enum compact (clippy::large_enum_variant).
        client: Box<Client<UnixConnector, Full<Bytes>>>,
        socket: PathBuf,
    },
}

impl Transport {
    /// Parse an endpoint string into a transport.
    ///
    /// - `unix:/run/reaper/agent.sock` → UDS on that socket path.
    /// - anything else (e.g. `http://localhost:8080`) → pooled TCP via reqwest.
    pub fn new(endpoint: &str) -> Result<Self> {
        if let Some(path) = endpoint.strip_prefix("unix:") {
            if path.is_empty() {
                anyhow::bail!("unix: endpoint '{endpoint}' has no socket path");
            }
            let socket = PathBuf::from(path);
            let connector = UnixConnector {
                socket_path: Arc::new(socket.clone()),
            };
            // Mirror reqwest's default keep-alive pooling so TCP and UDS both
            // amortise connection setup — otherwise UDS would pay a connect per
            // request and the comparison would be unfair to it.
            let client = Client::builder(TokioExecutor::new())
                .pool_max_idle_per_host(256)
                .pool_idle_timeout(Duration::from_secs(30))
                .build(connector);
            Ok(Self::Uds {
                client: Box::new(client),
                socket,
            })
        } else {
            Ok(Self::Tcp {
                client: reqwest::Client::new(),
                base: endpoint.trim_end_matches('/').to_string(),
            })
        }
    }

    /// Short transport tag for labels/logging: `"tcp"` or `"uds"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Tcp { .. } => "tcp",
            Self::Uds { .. } => "uds",
        }
    }

    /// Human-readable endpoint for progress output.
    pub fn endpoint(&self) -> String {
        match self {
            Self::Tcp { base, .. } => base.clone(),
            Self::Uds { socket, .. } => format!("unix:{}", socket.display()),
        }
    }

    /// GET `path`, returning the HTTP status code. Body is discarded — used for
    /// health checks where only reachability/status matters.
    pub async fn get_status(&self, path: &str) -> Result<u16> {
        match self {
            Self::Tcp { client, base } => {
                let resp = client
                    .get(format!("{base}{path}"))
                    .timeout(REQUEST_TIMEOUT)
                    .send()
                    .await?;
                Ok(resp.status().as_u16())
            }
            Self::Uds { client, .. } => {
                let (status, _body) = uds_request(client, "GET", path, Bytes::new()).await?;
                Ok(status)
            }
        }
    }

    /// POST `body` as JSON to `path`, returning `(status_code, parsed_json)`.
    ///
    /// Errors if the request fails or the response body is not valid JSON — the
    /// same failure semantics the harness had with `reqwest`'s `.json()`.
    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<(u16, serde_json::Value)> {
        match self {
            Self::Tcp { client, base } => {
                let resp = client
                    .post(format!("{base}{path}"))
                    .json(body)
                    .timeout(REQUEST_TIMEOUT)
                    .send()
                    .await?;
                let status = resp.status().as_u16();
                let json: serde_json::Value = resp.json().await?;
                Ok((status, json))
            }
            Self::Uds { client, .. } => {
                let payload = serde_json::to_vec(body)?;
                let (status, bytes) =
                    uds_request(client, "POST", path, Bytes::from(payload)).await?;
                let json: serde_json::Value = serde_json::from_slice(&bytes)
                    .context("UDS response body was not valid JSON")?;
                Ok((status, json))
            }
        }
    }
}

/// Issue one HTTP/1.1 request over the Unix socket and collect the full body.
async fn uds_request(
    client: &Client<UnixConnector, Full<Bytes>>,
    method: &str,
    path: &str,
    body: Bytes,
) -> Result<(u16, Bytes)> {
    // Authority is ignored by `UnixConnector`, but hyper requires a valid URI.
    let uri = format!("http://uds{path}");
    let req = Request::builder()
        .method(method)
        .uri(&uri)
        .header("content-type", "application/json")
        .body(Full::new(body))
        .context("failed to build UDS request")?;

    let resp = tokio::time::timeout(REQUEST_TIMEOUT, client.request(req))
        .await
        .context("UDS request timed out")?
        .context("UDS request failed")?;

    let status = resp.status().as_u16();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .context("failed to read UDS response body")?
        .to_bytes();
    Ok((status, bytes))
}

fn _assert_send_sync<T: Send + Sync>() {}

#[allow(dead_code)]
fn _transport_is_send_sync() {
    // The harness shares one Transport across many spawned tasks; keep it thread-safe.
    _assert_send_sync::<Transport>();
}
