//! SSE (Server-Sent Events) Client for real-time management events
//!
//! Provides instant notifications for:
//! - Bundle promotions (BundlePromoted)
//! - Data source refreshes (DataRefresh)
//! - Policy updates (PolicyUpdated)
//!
//! The SSE client automatically reconnects with exponential backoff
//! and gracefully degrades to poll-only mode if the server doesn't support SSE.

use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use futures::StreamExt;
use reqwest_eventsource::{Event, EventSource};

/// Management events received via SSE
#[derive(Debug, Clone)]
pub enum ManagementEvent {
    /// A bundle was promoted and is ready for deployment
    BundlePromoted {
        bundle_id: Uuid,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        version: String,
        download_url: String,
    },
    /// A data source was refreshed
    DataRefresh {
        source_id: Uuid,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        source_type: String,
    },
    /// A policy was updated
    PolicyUpdated {
        policy_id: Uuid,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        version: i32,
    },
    /// Keep-alive ping from server
    Ping { timestamp: String },
    /// SSE connection was established
    Connected,
    /// SSE connection was lost (will reconnect)
    Disconnected { error: Option<String> },
}

/// SSE client configuration
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// Base URL of the management server
    pub base_url: String,
    /// Organization slug
    pub org: String,
    /// Agent ID (obtained after registration)
    pub agent_id: Uuid,
    /// JWT token for authentication
    pub token: String,
    /// Initial reconnection delay (seconds)
    pub reconnect_initial_secs: u64,
    /// Maximum reconnection delay (seconds)
    pub reconnect_max_secs: u64,
}

/// SSE client for receiving real-time management events
pub struct SseClient {
    config: SseConfig,
    event_tx: mpsc::Sender<ManagementEvent>,
}

impl SseClient {
    /// Create a new SSE client
    pub fn new(config: SseConfig, event_tx: mpsc::Sender<ManagementEvent>) -> Self {
        Self { config, event_tx }
    }

    /// Run the SSE client until shutdown
    ///
    /// This method connects to the management server's SSE endpoint
    /// and processes events. It automatically reconnects on disconnect
    /// using exponential backoff.
    pub async fn run(&self, mut shutdown_rx: watch::Receiver<bool>) {
        let url = format!(
            "{}/orgs/{}/agents/{}/events",
            self.config.base_url, self.config.org, self.config.agent_id
        );

        info!(url = %url, "Starting SSE client");

        let mut reconnect_delay = Duration::from_secs(self.config.reconnect_initial_secs);
        let max_delay = Duration::from_secs(self.config.reconnect_max_secs);

        loop {
            // Check for shutdown before connecting
            if *shutdown_rx.borrow() {
                info!("SSE client shutting down");
                break;
            }

            match self.connect_and_process(&url, &mut shutdown_rx).await {
                Ok(SseResult::Shutdown) => {
                    info!("SSE client received shutdown signal");
                    break;
                }
                Ok(SseResult::Reconnect) => {
                    // Reset backoff on clean disconnect
                    reconnect_delay = Duration::from_secs(self.config.reconnect_initial_secs);
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        delay_secs = reconnect_delay.as_secs(),
                        "SSE connection error, reconnecting..."
                    );

                    // Notify about disconnection
                    let _ = self
                        .event_tx
                        .send(ManagementEvent::Disconnected {
                            error: Some(e.to_string()),
                        })
                        .await;

                    // Wait before reconnecting
                    tokio::select! {
                        _ = tokio::time::sleep(reconnect_delay) => {}
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                        }
                    }

                    // Exponential backoff
                    reconnect_delay = std::cmp::min(reconnect_delay * 2, max_delay);
                }
            }
        }
    }

    /// Connect to SSE endpoint and process events
    async fn connect_and_process(
        &self,
        url: &str,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<SseResult, SseError> {
        // Build request with authentication
        let request = reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .header("Accept", "text/event-stream");

        let mut event_source = EventSource::new(request)?;

        // Notify about connection
        let _ = self.event_tx.send(ManagementEvent::Connected).await;
        info!("SSE connection established");

        loop {
            tokio::select! {
                event = event_source.next() => {
                    match event {
                        Some(Ok(Event::Open)) => {
                            debug!("SSE stream opened");
                        }
                        Some(Ok(Event::Message(msg))) => {
                            if let Err(e) = self.handle_message(&msg.event, &msg.data).await {
                                warn!(error = %e, event = %msg.event, "Failed to handle SSE event");
                            }
                        }
                        Some(Err(e)) => {
                            return Err(SseError::Stream(e.to_string()));
                        }
                        None => {
                            debug!("SSE stream ended");
                            return Ok(SseResult::Reconnect);
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return Ok(SseResult::Shutdown);
                    }
                }
            }
        }
    }

    /// Handle a single SSE message
    async fn handle_message(&self, event_type: &str, data: &str) -> Result<(), SseError> {
        debug!(event_type = %event_type, data_len = data.len(), "Received SSE event");

        let event = match event_type {
            "bundle_promoted" => {
                let payload: BundlePromotedPayload =
                    serde_json::from_str(data).map_err(|e| SseError::Parse(e.to_string()))?;
                ManagementEvent::BundlePromoted {
                    bundle_id: payload.bundle_id,
                    org_id: payload.org_id,
                    namespace_id: payload.namespace_id,
                    version: payload.version,
                    download_url: payload.download_url,
                }
            }
            "data_refresh" => {
                let payload: DataRefreshPayload =
                    serde_json::from_str(data).map_err(|e| SseError::Parse(e.to_string()))?;
                ManagementEvent::DataRefresh {
                    source_id: payload.source_id,
                    org_id: payload.org_id,
                    namespace_id: payload.namespace_id,
                    source_type: payload.source_type,
                }
            }
            "policy_updated" => {
                let payload: PolicyUpdatedPayload =
                    serde_json::from_str(data).map_err(|e| SseError::Parse(e.to_string()))?;
                ManagementEvent::PolicyUpdated {
                    policy_id: payload.policy_id,
                    org_id: payload.org_id,
                    namespace_id: payload.namespace_id,
                    version: payload.version,
                }
            }
            "ping" => {
                let payload: PingPayload =
                    serde_json::from_str(data).map_err(|e| SseError::Parse(e.to_string()))?;
                ManagementEvent::Ping {
                    timestamp: payload.timestamp,
                }
            }
            _ => {
                debug!(event_type = %event_type, "Ignoring unknown SSE event type");
                return Ok(());
            }
        };

        self.event_tx
            .send(event)
            .await
            .map_err(|_| SseError::ChannelClosed)?;

        Ok(())
    }
}

/// Result of SSE connection attempt
enum SseResult {
    /// Shutdown was requested
    Shutdown,
    /// Connection closed cleanly, should reconnect
    Reconnect,
}

/// SSE client errors
#[derive(Debug, thiserror::Error)]
pub enum SseError {
    #[error("Failed to create event source: {0}")]
    EventSource(#[from] reqwest_eventsource::Error),

    #[error("Cannot clone request: {0}")]
    CannotCloneRequest(#[from] reqwest_eventsource::CannotCloneRequestError),

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Failed to parse event: {0}")]
    Parse(String),

    #[error("Event channel closed")]
    ChannelClosed,
}

// ============================================================================
// SSE Event Payloads
// ============================================================================

#[derive(Debug, serde::Deserialize)]
struct BundlePromotedPayload {
    bundle_id: Uuid,
    org_id: Uuid,
    namespace_id: Option<Uuid>,
    version: String,
    download_url: String,
}

#[derive(Debug, serde::Deserialize)]
struct DataRefreshPayload {
    source_id: Uuid,
    org_id: Uuid,
    namespace_id: Option<Uuid>,
    source_type: String,
}

#[derive(Debug, serde::Deserialize)]
struct PolicyUpdatedPayload {
    policy_id: Uuid,
    org_id: Uuid,
    namespace_id: Option<Uuid>,
    version: i32,
}

#[derive(Debug, serde::Deserialize)]
struct PingPayload {
    timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bundle_promoted() {
        let data = r#"{"bundle_id":"550e8400-e29b-41d4-a716-446655440000","org_id":"550e8400-e29b-41d4-a716-446655440001","namespace_id":null,"version":"1.2.3","download_url":"https://example.com/bundles/test"}"#;

        let payload: BundlePromotedPayload = serde_json::from_str(data).unwrap();
        assert_eq!(payload.version, "1.2.3");
        assert!(payload.namespace_id.is_none());
    }

    #[test]
    fn test_parse_data_refresh() {
        let data = r#"{"source_id":"550e8400-e29b-41d4-a716-446655440000","org_id":"550e8400-e29b-41d4-a716-446655440001","namespace_id":"550e8400-e29b-41d4-a716-446655440002","source_type":"api"}"#;

        let payload: DataRefreshPayload = serde_json::from_str(data).unwrap();
        assert_eq!(payload.source_type, "api");
        assert!(payload.namespace_id.is_some());
    }

    #[test]
    fn test_parse_ping() {
        let data = r#"{"timestamp":"2024-01-15T10:30:00Z"}"#;

        let payload: PingPayload = serde_json::from_str(data).unwrap();
        assert_eq!(payload.timestamp, "2024-01-15T10:30:00Z");
    }
}
