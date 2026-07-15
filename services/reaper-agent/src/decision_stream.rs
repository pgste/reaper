//! Agent-side low-latency SIEM streaming sink (round-2 E1, slice 4).
//!
//! The decision buffer's writer thread mirrors every captured decision to a
//! bounded channel ([`policy_engine::decision_stream_channel`]); this module owns
//! the consumer. It batches records, shapes them (NDJSON / OCSF / CEF, via
//! `DecisionLogEntry::export`), and POSTs them straight to a configured HTTP
//! endpoint — bypassing the agent→Vector→ClickHouse→control-plane hop for
//! operators who need decisions in their SIEM with minimal latency.
//!
//! It is a **best-effort telemetry mirror, not the durable audit path**: the
//! writer never blocks on it (a saturated channel drops, counted as
//! `stream_dropped`), and delivery failures here never affect evaluation or the
//! durable file/WORM sinks. For a governed, per-tenant, authenticated push with
//! full history, use the control-plane connectors API (E1 slice 3) instead.
//!
//! The consumer runs on its own OS thread (the writer channel is a synchronous
//! `std::sync::mpsc`), driving async `reqwest` through a private current-thread
//! runtime — so it never touches the agent's request reactor.

use std::time::{Duration, Instant};

use policy_engine::{DecisionStreamReceiver, ExportFormat};
use tracing::{info, warn};

/// Streaming-sink configuration, from environment.
pub struct StreamSinkConfig {
    /// Destination URL (`REAPER_DECISION_STREAM_URL`).
    pub url: String,
    /// Record shape (`REAPER_DECISION_STREAM_FORMAT`: ndjson|ocsf|cef, default ocsf).
    pub format: ExportFormat,
    /// Optional bearer token (`REAPER_DECISION_STREAM_TOKEN`).
    pub token: Option<String>,
    /// Max records per POST (`REAPER_DECISION_STREAM_BATCH`, default 500).
    pub batch_max: usize,
    /// Max time to accumulate a batch before flushing
    /// (`REAPER_DECISION_STREAM_FLUSH_MS`, default 1000ms).
    pub flush_interval: Duration,
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl StreamSinkConfig {
    /// Read config; `None` (sink disabled) when `REAPER_DECISION_STREAM_URL` is
    /// unset or empty.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("REAPER_DECISION_STREAM_URL").ok()?;
        if url.trim().is_empty() {
            return None;
        }
        let format = std::env::var("REAPER_DECISION_STREAM_FORMAT")
            .ok()
            .and_then(|s| ExportFormat::parse(&s))
            .unwrap_or(ExportFormat::Ocsf);
        let token = std::env::var("REAPER_DECISION_STREAM_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let batch_max = env_usize("REAPER_DECISION_STREAM_BATCH", 500).clamp(1, 10_000);
        let flush_ms = env_usize("REAPER_DECISION_STREAM_FLUSH_MS", 1000).max(50) as u64;
        Some(Self {
            url: url.trim().to_string(),
            format,
            token,
            batch_max,
            flush_interval: Duration::from_millis(flush_ms),
        })
    }

    /// Bound on how many records may queue before the writer starts dropping
    /// (`REAPER_DECISION_STREAM_QUEUE`, default 10000).
    pub fn queue_capacity() -> usize {
        env_usize("REAPER_DECISION_STREAM_QUEUE", 10_000).clamp(1, 1_000_000)
    }

    fn content_type(&self) -> &'static str {
        match self.format {
            ExportFormat::Cef => "text/plain",
            _ => "application/x-ndjson",
        }
    }
}

/// Spawn the streaming-sink consumer on a dedicated OS thread. Best-effort: a
/// thread-spawn failure logs and disables streaming (never fatal).
pub fn spawn(config: StreamSinkConfig, rx: DecisionStreamReceiver) {
    if let Err(e) = std::thread::Builder::new()
        .name("decision-stream-sink".to_string())
        .spawn(move || run(config, rx))
    {
        warn!(error = %e, "failed to spawn decision-stream sink; streaming disabled");
    }
}

fn run(config: StreamSinkConfig, rx: DecisionStreamReceiver) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!(error = %e, "decision-stream sink: could not start runtime; streaming disabled");
            return;
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Reaper-Agent-Stream/1.0")
        .build()
        .unwrap_or_default();
    let content_type = config.content_type();
    info!(
        url = %config.url,
        format = config.format.as_str(),
        "decision-stream sink started"
    );

    loop {
        // Block until the first record (or exit when the writer is gone).
        let first = match rx.recv() {
            Ok(e) => e,
            Err(_) => break,
        };
        let mut batch = vec![first];
        let deadline = Instant::now() + config.flush_interval;
        while batch.len() < config.batch_max {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(e) => batch.push(e),
                Err(_) => break, // timeout or disconnect: flush what we have
            }
        }

        let lines: Vec<String> = batch
            .iter()
            .filter_map(|e| e.export(config.format).ok())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let body = lines.join("\n");
        runtime.block_on(push(&client, &config, body, content_type));
    }
    info!("decision-stream sink stopped (writer closed)");
}

/// Deliver one batch with a couple of retries on 5xx / transport errors.
async fn push(client: &reqwest::Client, config: &StreamSinkConfig, body: String, ct: &str) {
    for attempt in 0..=2u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
        }
        let mut req = client.post(&config.url).header("Content-Type", ct);
        if let Some(token) = &config.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        match req.body(body.clone()).send().await {
            Ok(resp) if resp.status().is_success() => return,
            Ok(resp) if resp.status().is_server_error() => continue,
            Ok(resp) => {
                warn!(
                    status = resp.status().as_u16(),
                    "decision-stream push rejected"
                );
                return;
            }
            Err(e) => {
                if e.is_timeout() || e.is_connect() {
                    continue;
                }
                warn!(error = %e, "decision-stream push failed");
                return;
            }
        }
    }
    warn!("decision-stream push exhausted retries; batch dropped");
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: these all mutate the same process-wide env vars, so keep them
    // sequential rather than risking a parallel-run race.
    #[test]
    fn from_env_gating_and_format() {
        let url = "REAPER_DECISION_STREAM_URL";
        let fmt = "REAPER_DECISION_STREAM_FORMAT";

        // Disabled without a URL.
        std::env::remove_var(url);
        assert!(StreamSinkConfig::from_env().is_none());

        // URL set, no format → defaults to OCSF (application/x-ndjson).
        std::env::set_var(url, "https://siem.example/ingest");
        std::env::remove_var(fmt);
        let cfg = StreamSinkConfig::from_env().unwrap();
        assert_eq!(cfg.format, ExportFormat::Ocsf);
        assert_eq!(cfg.content_type(), "application/x-ndjson");
        assert_eq!(cfg.batch_max, 500, "default batch");

        // Explicit CEF → text/plain.
        std::env::set_var(fmt, "cef");
        let cfg = StreamSinkConfig::from_env().unwrap();
        assert_eq!(cfg.format, ExportFormat::Cef);
        assert_eq!(cfg.content_type(), "text/plain");

        std::env::remove_var(url);
        std::env::remove_var(fmt);
    }
}
