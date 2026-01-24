//! Decision Buffer - Lock-free ring buffer for decision logging
//!
//! Provides a high-performance, thread-safe buffer for storing decision log entries
//! with minimal latency impact on policy evaluation.

use crate::decision_log::{DecisionLogConfig, DecisionLogEntry};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Statistics for the decision buffer
#[derive(Debug, Clone, Default)]
pub struct DecisionBufferStats {
    pub total_entries: u64,
    pub buffer_size: usize,
    pub buffer_capacity: usize,
    pub dropped_entries: u64,
    pub flush_count: u64,
    pub allow_count: u64,
    pub deny_count: u64,
}

/// A thread-safe ring buffer for decision log entries
pub struct DecisionBuffer {
    /// Configuration
    config: DecisionLogConfig,

    /// Ring buffer of entries (protected by RwLock for minimal contention)
    entries: RwLock<VecDeque<DecisionLogEntry>>,

    /// Statistics counters (atomic for lock-free updates)
    total_entries: AtomicU64,
    dropped_entries: AtomicU64,
    flush_count: AtomicU64,
    allow_count: AtomicU64,
    deny_count: AtomicU64,

    /// File writer for persistent logging (optional)
    file_writer: Option<RwLock<BufWriter<File>>>,
}

impl DecisionBuffer {
    /// Create a new decision buffer with the given configuration
    pub fn new(config: DecisionLogConfig) -> std::io::Result<Self> {
        let file_writer = if let Some(ref path) = config.file_path {
            // Ensure parent directory exists
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }

            let file = OpenOptions::new().create(true).append(true).open(path)?;

            Some(RwLock::new(BufWriter::new(file)))
        } else {
            None
        };

        Ok(Self {
            config,
            entries: RwLock::new(VecDeque::new()),
            total_entries: AtomicU64::new(0),
            dropped_entries: AtomicU64::new(0),
            flush_count: AtomicU64::new(0),
            allow_count: AtomicU64::new(0),
            deny_count: AtomicU64::new(0),
            file_writer,
        })
    }

    /// Create a new buffer with default configuration
    pub fn with_defaults() -> Self {
        Self::new(DecisionLogConfig::default()).expect("Default config should not fail")
    }

    /// Add a decision log entry to the buffer
    ///
    /// This operation is designed to have minimal latency impact (<100ns typical).
    /// If the buffer is full, the oldest entry is dropped.
    pub fn log(&self, mut entry: DecisionLogEntry) {
        if !self.config.enabled {
            return;
        }

        // Check if we should log this decision type
        let is_allow = entry.decision == "allow";
        if is_allow && !self.config.log_allows {
            return;
        }
        if !is_allow && !self.config.log_denies {
            return;
        }

        // Strip context if configured
        if !self.config.include_context {
            entry.context.clear();
        }

        // Update statistics
        self.total_entries.fetch_add(1, Ordering::Relaxed);
        if is_allow {
            self.allow_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.deny_count.fetch_add(1, Ordering::Relaxed);
        }

        // Write to file if configured
        if let Some(ref writer) = self.file_writer {
            if let Ok(json) = entry.to_ndjson() {
                let mut w = writer.write();
                let _ = writeln!(w, "{}", json);
            }
        }

        // Add to buffer (with potential drop if full)
        let mut entries = self.entries.write();
        if entries.len() >= self.config.buffer_capacity {
            entries.pop_front();
            self.dropped_entries.fetch_add(1, Ordering::Relaxed);
        }
        entries.push_back(entry);
    }

    /// Get recent decisions (most recent first)
    pub fn get_recent(&self, limit: usize) -> Vec<DecisionLogEntry> {
        let entries = self.entries.read();
        entries.iter().rev().take(limit).cloned().collect()
    }

    /// Get decisions with pagination
    pub fn get_page(&self, offset: usize, limit: usize) -> Vec<DecisionLogEntry> {
        let entries = self.entries.read();
        entries.iter().skip(offset).take(limit).cloned().collect()
    }

    /// Query decisions by filter
    pub fn query(&self, filter: DecisionFilter, limit: usize) -> Vec<DecisionLogEntry> {
        let entries = self.entries.read();
        entries
            .iter()
            .rev()
            .filter(|e| filter.matches(e))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get current buffer statistics
    pub fn stats(&self) -> DecisionBufferStats {
        let entries = self.entries.read();
        DecisionBufferStats {
            total_entries: self.total_entries.load(Ordering::Relaxed),
            buffer_size: entries.len(),
            buffer_capacity: self.config.buffer_capacity,
            dropped_entries: self.dropped_entries.load(Ordering::Relaxed),
            flush_count: self.flush_count.load(Ordering::Relaxed),
            allow_count: self.allow_count.load(Ordering::Relaxed),
            deny_count: self.deny_count.load(Ordering::Relaxed),
        }
    }

    /// Flush file buffer to disk
    pub fn flush(&self) -> std::io::Result<()> {
        if let Some(ref writer) = self.file_writer {
            let mut w = writer.write();
            w.flush()?;
            self.flush_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Clear the buffer
    pub fn clear(&self) {
        let mut entries = self.entries.write();
        entries.clear();
    }

    /// Export all entries as NDJSON
    pub fn export_ndjson(&self) -> String {
        let entries = self.entries.read();
        entries
            .iter()
            .filter_map(|e| e.to_ndjson().ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get the configuration
    pub fn config(&self) -> &DecisionLogConfig {
        &self.config
    }
}

/// Filter for querying decisions
#[derive(Debug, Clone, Default)]
pub struct DecisionFilter {
    pub principal: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub decision: Option<String>,
    pub policy_id: Option<String>,
    pub since: Option<String>, // ISO 8601 timestamp
}

impl DecisionFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_principal(mut self, principal: String) -> Self {
        self.principal = Some(principal);
        self
    }

    pub fn with_action(mut self, action: String) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_resource(mut self, resource: String) -> Self {
        self.resource = Some(resource);
        self
    }

    pub fn with_decision(mut self, decision: String) -> Self {
        self.decision = Some(decision);
        self
    }

    pub fn with_policy_id(mut self, policy_id: String) -> Self {
        self.policy_id = Some(policy_id);
        self
    }

    pub fn with_since(mut self, since: String) -> Self {
        self.since = Some(since);
        self
    }

    fn matches(&self, entry: &DecisionLogEntry) -> bool {
        if let Some(ref p) = self.principal {
            if &entry.principal != p {
                return false;
            }
        }
        if let Some(ref a) = self.action {
            if &entry.action != a {
                return false;
            }
        }
        if let Some(ref r) = self.resource {
            if &entry.resource != r {
                return false;
            }
        }
        if let Some(ref d) = self.decision {
            if &entry.decision != d {
                return false;
            }
        }
        if let Some(ref pid) = self.policy_id {
            if &entry.policy_id != pid {
                return false;
            }
        }
        if let Some(ref since) = self.since {
            if &entry.timestamp < since {
                return false;
            }
        }
        true
    }
}

/// Thread-safe handle to a decision buffer
pub type SharedDecisionBuffer = Arc<DecisionBuffer>;

/// Create a shared decision buffer from configuration
pub fn create_shared_buffer(config: DecisionLogConfig) -> std::io::Result<SharedDecisionBuffer> {
    Ok(Arc::new(DecisionBuffer::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(decision: &str) -> DecisionLogEntry {
        DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "resource".to_string(),
            decision.to_string(),
            "policy".to_string(),
            "test-policy".to_string(),
        )
    }

    #[test]
    fn test_buffer_basic_operations() {
        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 100,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();

        // Log some entries
        buffer.log(test_entry("allow"));
        buffer.log(test_entry("deny"));
        buffer.log(test_entry("allow"));

        let stats = buffer.stats();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.buffer_size, 3);
        assert_eq!(stats.allow_count, 2);
        assert_eq!(stats.deny_count, 1);
    }

    #[test]
    fn test_buffer_capacity_limit() {
        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 5,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();

        // Log more than capacity
        for i in 0..10 {
            let mut entry = test_entry("allow");
            entry.principal = format!("user_{}", i);
            buffer.log(entry);
        }

        let stats = buffer.stats();
        assert_eq!(stats.total_entries, 10);
        assert_eq!(stats.buffer_size, 5); // Capped at capacity
        assert_eq!(stats.dropped_entries, 5);

        // Recent entries should be the last 5
        let recent = buffer.get_recent(5);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].principal, "user_9"); // Most recent
        assert_eq!(recent[4].principal, "user_5"); // Oldest in buffer
    }

    #[test]
    fn test_buffer_disabled() {
        let config = DecisionLogConfig {
            enabled: false,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.log(test_entry("allow"));

        let stats = buffer.stats();
        assert_eq!(stats.total_entries, 0);
        assert_eq!(stats.buffer_size, 0);
    }

    #[test]
    fn test_buffer_filter_allows_only() {
        let config = DecisionLogConfig {
            enabled: true,
            log_allows: true,
            log_denies: false,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.log(test_entry("allow"));
        buffer.log(test_entry("deny"));

        let stats = buffer.stats();
        assert_eq!(stats.buffer_size, 1);
        assert_eq!(stats.allow_count, 1);
    }

    #[test]
    fn test_buffer_query() {
        let config = DecisionLogConfig {
            enabled: true,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();

        let mut entry1 = test_entry("allow");
        entry1.principal = "alice".to_string();
        buffer.log(entry1);

        let mut entry2 = test_entry("deny");
        entry2.principal = "bob".to_string();
        buffer.log(entry2);

        let mut entry3 = test_entry("allow");
        entry3.principal = "alice".to_string();
        buffer.log(entry3);

        // Query by principal
        let filter = DecisionFilter::new().with_principal("alice".to_string());
        let results = buffer.query(filter, 10);
        assert_eq!(results.len(), 2);

        // Query by decision
        let filter = DecisionFilter::new().with_decision("deny".to_string());
        let results = buffer.query(filter, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].principal, "bob");
    }

    #[test]
    fn test_buffer_ndjson_export() {
        let config = DecisionLogConfig {
            enabled: true,
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.log(test_entry("allow"));
        buffer.log(test_entry("deny"));

        let ndjson = buffer.export_ndjson();
        let lines: Vec<&str> = ndjson.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"decision\":\"allow\""));
        assert!(lines[1].contains("\"decision\":\"deny\""));
    }
}
