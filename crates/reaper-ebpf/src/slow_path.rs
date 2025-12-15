//! Slow Path Handler - Consumes eBPF events and evaluates complex policies
//!
//! When the eBPF fast path cannot make a decision (no matching simple rule),
//! it sends an event via the ring buffer. This handler:
//! 1. Polls the ring buffer for events
//! 2. Evaluates them using the full PolicyEngine (Cedar, Reaper DSL)
//! 3. Records access patterns for learning
//! 4. Auto-promotes frequently accessed paths to eBPF
//!
//! This creates a self-optimizing system where hot paths automatically
//! move to the <100ns eBPF fast path over time.

use crate::controller::EbpfController;
use crate::learning::LearningEngine;
use crate::types::PolicyEvent;
use anyhow::{Context, Result};
use policy_engine::PolicyEngine;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Slow path handler - consumes eBPF events and evaluates complex policies
///
/// TODO: This will be fully implemented once the eBPF program is compiled
/// and we can properly handle RingBuf lifetimes
pub struct SlowPathHandler {
    /// Full policy engine (Cedar, Reaper DSL, etc.)
    #[allow(dead_code)]
    policy_engine: Arc<PolicyEngine>,

    /// Learning engine for auto-promotion
    learning_engine: Arc<LearningEngine>,

    /// eBPF controller (for promoting policies)
    controller: Arc<RwLock<EbpfController>>,

    /// Whether to auto-promote eligible policies
    auto_promote_enabled: bool,

    /// Auto-promote interval (check every N seconds)
    auto_promote_interval: Duration,

    /// Statistics
    events_processed: Arc<std::sync::atomic::AtomicU64>,
    events_errors: Arc<std::sync::atomic::AtomicU64>,
}

impl SlowPathHandler {
    /// Create a new slow path handler
    ///
    /// # Arguments
    /// * `policy_engine` - Full PolicyEngine for complex policy evaluation
    /// * `learning_engine` - LearningEngine for tracking access patterns
    /// * `controller` - EbpfController for promoting policies
    pub fn new(
        policy_engine: Arc<PolicyEngine>,
        learning_engine: Arc<LearningEngine>,
        controller: Arc<RwLock<EbpfController>>,
    ) -> Self {
        Self {
            policy_engine,
            learning_engine,
            controller,
            auto_promote_enabled: true,
            auto_promote_interval: Duration::from_secs(60), // Check every minute
            events_processed: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            events_errors: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Enable or disable auto-promotion
    pub fn set_auto_promote(&mut self, enabled: bool) {
        self.auto_promote_enabled = enabled;
        info!(
            "Auto-promotion: {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Set auto-promotion check interval
    pub fn set_auto_promote_interval(&mut self, interval: Duration) {
        self.auto_promote_interval = interval;
        info!("Auto-promotion interval: {:?}", interval);
    }

    /// Run the slow path handler (blocking)
    ///
    /// This method blocks indefinitely, polling the ring buffer and processing events.
    /// It should be run in a background task:
    ///
    /// ```no_run
    /// tokio::spawn(async move {
    ///     handler.run().await.expect("Slow path handler failed");
    /// });
    /// ```
    pub async fn run(mut self) -> Result<()> {
        info!("Starting slow path handler...");

        // Spawn auto-promotion task
        if self.auto_promote_enabled {
            let controller = Arc::clone(&self.controller);
            let learning_engine = Arc::clone(&self.learning_engine);
            let interval = self.auto_promote_interval;

            tokio::spawn(async move {
                Self::auto_promote_task(controller, learning_engine, interval).await;
            });
        }

        // Main event loop
        loop {
            // Poll ring buffer
            match self.poll_events().await {
                Ok(count) => {
                    if count > 0 {
                        debug!("Processed {} events", count);
                    }
                }
                Err(e) => {
                    error!("Error polling events: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }

            // Small sleep to avoid busy-waiting
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Poll ring buffer for events (async)
    ///
    /// TODO: Implement once RingBuf is available
    async fn poll_events(&mut self) -> Result<usize> {
        // Placeholder - will be implemented with actual RingBuf
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(0)
    }

    /// Handle a single event from eBPF
    #[allow(dead_code)]
    async fn handle_event(&self, event_bytes: &[u8]) -> Result<()> {
        // Parse event
        let event = self.parse_event(event_bytes)?;

        debug!(
            "Slow path event: pid={} uid={} path={} action={}",
            event.pid,
            event.uid,
            event.path_str(),
            event.action
        );

        // Convert to PolicyRequest
        let request = event.to_policy_request();

        // Evaluate using full PolicyEngine
        let start = std::time::Instant::now();

        // TODO: Need to get policy ID from somewhere
        // For now, use a placeholder. In production, this would come from:
        // 1. Event metadata
        // 2. Policy lookup by resource
        // 3. Default policy
        let policy_id = uuid::Uuid::nil(); // Placeholder

        let decision = self
            .policy_engine
            .evaluate(&policy_id, &request)
            .context("Failed to evaluate policy")?;

        let latency = start.elapsed();

        info!(
            "Slow path decision: {} → {:?} ({:.2}µs)",
            request.resource,
            decision.decision,
            latency.as_micros() as f64 / 1.0
        );

        // Record access for learning
        self.learning_engine.record_access(
            &request.resource,
            decision.decision.clone(),
            Some(event.uid),
            Some(event.gid),
        );

        // Check if should promote immediately (optional)
        if self.auto_promote_enabled && self.learning_engine.should_promote(&request.resource) {
            debug!(
                "Resource eligible for immediate promotion: {}",
                request.resource
            );
            // Will be promoted in next auto-promote cycle
        }

        Ok(())
    }

    /// Parse PolicyEvent from raw bytes
    #[allow(dead_code)]
    fn parse_event(&self, bytes: &[u8]) -> Result<PolicyEvent> {
        if bytes.len() < std::mem::size_of::<PolicyEvent>() {
            anyhow::bail!(
                "Event bytes too short: {} < {}",
                bytes.len(),
                std::mem::size_of::<PolicyEvent>()
            );
        }

        // SAFETY: We've verified the length above
        // PolicyEvent is #[repr(C)] so layout is guaranteed
        let event = unsafe { std::ptr::read(bytes.as_ptr() as *const PolicyEvent) };

        Ok(event)
    }

    /// Auto-promotion background task
    async fn auto_promote_task(
        controller: Arc<RwLock<EbpfController>>,
        learning_engine: Arc<LearningEngine>,
        interval: Duration,
    ) {
        info!("Starting auto-promotion task (interval: {:?})", interval);

        loop {
            tokio::time::sleep(interval).await;

            debug!("Running auto-promotion check...");

            // Get write lock on controller
            match controller.write().await.auto_promote(&learning_engine) {
                Ok(count) => {
                    if count > 0 {
                        info!("Auto-promoted {} resources to eBPF", count);
                    }
                }
                Err(e) => {
                    error!("Auto-promotion error: {}", e);
                }
            }
        }
    }

    /// Get statistics
    pub fn get_stats(&self) -> SlowPathStats {
        SlowPathStats {
            events_processed: self
                .events_processed
                .load(std::sync::atomic::Ordering::Relaxed),
            events_errors: self
                .events_errors
                .load(std::sync::atomic::Ordering::Relaxed),
            auto_promote_enabled: self.auto_promote_enabled,
        }
    }
}

// Add auto_promote method to EbpfController (via extension trait)
trait EbpfControllerExt {
    fn auto_promote(&mut self, learning_engine: &LearningEngine) -> Result<usize>;
}

impl EbpfControllerExt for EbpfController {
    fn auto_promote(&mut self, learning_engine: &LearningEngine) -> Result<usize> {
        learning_engine.auto_promote(self)
    }
}

/// Statistics about slow path handler
#[derive(Debug, Clone)]
pub struct SlowPathStats {
    /// Total events processed
    pub events_processed: u64,

    /// Total events with errors
    pub events_errors: u64,

    /// Whether auto-promotion is enabled
    pub auto_promote_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_event_size() {
        // Ensure PolicyEvent has correct size for ring buffer
        let size = std::mem::size_of::<PolicyEvent>();
        assert!(size > 0);
        assert!(size <= 1024); // Reasonable size for ring buffer
    }
}
