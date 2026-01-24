//! Graceful shutdown module
//!
//! Provides coordinated shutdown for the management server.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Shutdown signal that can be cloned and shared across tasks
#[derive(Clone)]
pub struct ShutdownSignal {
    /// Sender to notify all receivers of shutdown
    shutdown_tx: broadcast::Sender<()>,
    /// Flag indicating shutdown has been initiated
    is_shutting_down: Arc<AtomicBool>,
    /// Counter for in-flight requests
    in_flight_requests: Arc<AtomicU64>,
}

impl ShutdownSignal {
    /// Create a new shutdown signal
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            shutdown_tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
            in_flight_requests: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Subscribe to shutdown notifications
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Check if shutdown has been initiated
    pub fn is_shutting_down(&self) -> bool {
        self.is_shutting_down.load(Ordering::SeqCst)
    }

    /// Initiate shutdown
    pub fn shutdown(&self) {
        self.is_shutting_down.store(true, Ordering::SeqCst);
        let _ = self.shutdown_tx.send(());
    }

    /// Increment in-flight request counter
    pub fn request_started(&self) {
        self.in_flight_requests.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement in-flight request counter
    pub fn request_finished(&self) {
        self.in_flight_requests.fetch_sub(1, Ordering::SeqCst);
    }

    /// Get current in-flight request count
    pub fn in_flight_count(&self) -> u64 {
        self.in_flight_requests.load(Ordering::SeqCst)
    }

    /// Wait for all in-flight requests to complete with timeout
    pub async fn wait_for_requests(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            let count = self.in_flight_count();
            if count == 0 {
                return true;
            }

            if start.elapsed() >= timeout {
                warn!(
                    "Shutdown timeout reached with {} requests still in-flight",
                    count
                );
                return false;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for graceful shutdown
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Timeout for waiting for in-flight requests
    pub timeout: Duration,
    /// Whether to force shutdown after timeout
    pub force_after_timeout: bool,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            force_after_timeout: true,
        }
    }
}

/// Wait for shutdown signal from OS (SIGTERM, SIGINT)
pub async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received CTRL+C, initiating graceful shutdown...");
        }
        _ = terminate => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }
}

/// Perform graceful shutdown with the given configuration
pub async fn graceful_shutdown(
    shutdown_signal: &ShutdownSignal,
    config: &ShutdownConfig,
) -> bool {
    info!("Initiating graceful shutdown...");
    shutdown_signal.shutdown();

    let in_flight = shutdown_signal.in_flight_count();
    if in_flight > 0 {
        info!(
            "Waiting for {} in-flight requests (timeout: {:?})",
            in_flight, config.timeout
        );
    }

    let success = shutdown_signal.wait_for_requests(config.timeout).await;

    if success {
        info!("All in-flight requests completed, shutdown complete");
    } else if config.force_after_timeout {
        warn!("Forcing shutdown after timeout");
    }

    success
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_signal() {
        let signal = ShutdownSignal::new();

        assert!(!signal.is_shutting_down());
        assert_eq!(signal.in_flight_count(), 0);

        signal.request_started();
        assert_eq!(signal.in_flight_count(), 1);

        signal.request_finished();
        assert_eq!(signal.in_flight_count(), 0);

        signal.shutdown();
        assert!(signal.is_shutting_down());
    }

    #[tokio::test]
    async fn test_wait_for_requests() {
        let signal = ShutdownSignal::new();

        // With no requests, should complete immediately
        let success = signal.wait_for_requests(Duration::from_millis(100)).await;
        assert!(success);

        // With requests, simulate completion
        signal.request_started();
        let signal_clone = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            signal_clone.request_finished();
        });

        let success = signal.wait_for_requests(Duration::from_millis(200)).await;
        assert!(success);
    }
}
