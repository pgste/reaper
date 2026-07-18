//! Outbound HTTP client construction (round-3 Plan 06 §4.1, R3-01).
//!
//! Every `reqwest::Client` in this service is built through this module so no
//! outbound call can hang the awaiting task on a wedged upstream. A bare
//! `reqwest::Client::new()` has **no timeout** — a stalled peer parks the
//! caller forever — and the old `builder…build().unwrap_or_else(|_|
//! Client::new())` pattern silently dropped the timeout on a builder error.
//! Both are now banned by a CI lint; construct clients here instead.

use std::time::Duration;

/// Connect-phase timeout applied to every client. The DNS+TCP+TLS handshake
/// must complete within this or the call errors fast.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Whole-request timeout for a general outbound call (send → full response).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A `ClientBuilder` pre-configured with the connect timeout and a total-request
/// `timeout`. Callers that need extra hardening (e.g. `.redirect(Policy::none())`
/// for SSRF-guarded fetches, or a `.user_agent(...)`) chain onto this and call
/// `.build()` — the build error must be propagated, never swallowed into a
/// no-timeout client.
pub fn http_client_builder(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(timeout)
}

/// Build a client with an explicit total-request `timeout`. Errors propagate.
pub fn http_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    http_client_builder(timeout).build()
}

/// Build a client with the workspace default request timeout.
pub fn http_client_default() -> reqwest::Result<reqwest::Client> {
    http_client(DEFAULT_REQUEST_TIMEOUT)
}

/// Build a client from a pre-configured `builder` for an INFALLIBLE constructor
/// (one that returns `Self`, not `Result`, so it cannot propagate the build
/// error — e.g. the sync syncers, built inside the infallible `AppState::new`).
///
/// `ClientBuilder::build()` fails only when the TLS backend cannot initialise —
/// a fatal, whole-process condition under which no client works. The old code
/// swallowed that into `reqwest::Client::new()`, a client with **no timeout**;
/// this instead logs loudly and falls back to `Client::default()`. The
/// difference that matters: on the REACHABLE path the returned client always
/// carries the builder's timeout, and the sole no-timeout fallback lives here,
/// logged and centralised, never scattered across call sites.
pub fn build_or_default(builder: reqwest::ClientBuilder) -> reqwest::Client {
    builder.build().unwrap_or_else(|e| {
        tracing::error!(
            error = %e,
            "reqwest client build failed (TLS backend init?); falling back to a \
             default client — outbound calls on this client have NO timeout"
        );
        reqwest::Client::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Instant;

    /// The whole point of the helper: a request to a peer that accepts the
    /// connection but never responds must ERROR at the timeout, not hang the
    /// task forever (round-3 Plan 06 §4.1 DoD, R3-01).
    #[tokio::test]
    async fn client_times_out_on_a_black_hole_peer() {
        // A listener that accepts connections, reads the request, and then
        // deliberately never writes a response.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_secs(30)); // black hole
            }
        });

        let client = http_client(Duration::from_millis(300)).unwrap();
        let start = Instant::now();
        let result = client.get(format!("http://{addr}/")).send().await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "a request to a black-hole peer must error, not hang"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "must fail near the 300ms timeout; took {elapsed:?}"
        );
    }
}
