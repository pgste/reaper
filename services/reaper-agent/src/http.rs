//! Outbound HTTP client construction for the agent (round-3 Plan 06 §4.1, R3-01).
//!
//! Mirrors `reaper-management`'s helper: a bare `reqwest::Client::new()` has no
//! timeout, so a wedged upstream parks the awaiting task forever. Construct
//! agent-side clients here instead; a CI lint bans the bare constructor.

use std::time::Duration;

/// Connect-phase timeout applied to every agent client.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A `ClientBuilder` for a LONG-LIVED STREAMING request (e.g. the management
/// SSE channel). It sets a `connect_timeout` — the handshake must not hang —
/// but deliberately **no** total `.timeout()`: an SSE stream stays open
/// indefinitely, and a whole-request timeout would abort it mid-stream. Idle
/// handling for the stream itself lives in the SSE loop, not here.
pub fn streaming_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().connect_timeout(DEFAULT_CONNECT_TIMEOUT)
}
