//! Decision Buffer - Lock-free ring buffer for decision logging
//!
//! Provides a high-performance, thread-safe buffer for storing decision log entries
//! with minimal latency impact on policy evaluation.

use crate::decision_log::{DecisionLogConfig, DecisionLogEntry};
use parking_lot::RwLock;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;

/// Global seed source so each thread's sampling PRNG starts distinct without an
/// RNG syscall or a time source on the hot path.
static SAMPLE_SEED: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

thread_local! {
    static SAMPLE_RNG: Cell<u64> = Cell::new(seed_thread());
}

/// Distinct non-zero per-thread seed via a SplitMix64 step off the global counter.
fn seed_thread() -> u64 {
    let mut z = SAMPLE_SEED.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) | 1
}

/// A uniform sample in `[0.0, 1.0)` from a thread-local xorshift64 (a few ns, no
/// shared state, no syscall) — used for deny-priority allow sampling.
#[inline]
fn sample_unit() -> f64 {
    SAMPLE_RNG.with(|c| {
        let mut x = c.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        c.set(x);
        // Top 53 bits → f64 in [0, 1).
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Bound on the background file-writer queue. When full, entries are dropped
/// (and counted) rather than blocking the request thread — the in-memory ring
/// buffer still retains them for the query API.
const WRITER_QUEUE_CAPACITY: usize = 65_536;

/// Message to the background decision-log writer.
enum WriterMsg {
    Entry(Box<DecisionLogEntry>),
    Flush,
}

/// Output sinks owned by the background writer thread. NDJSON is serialized once
/// and fanned out to whichever sinks are configured (file and/or stdout). All
/// serialization + I/O happens here, never on the request path.
struct WriterSinks {
    file: Option<BufWriter<std::fs::File>>,
    stdout: Option<BufWriter<std::io::Stdout>>,
}

impl WriterSinks {
    fn flush_all(&mut self) {
        if let Some(w) = self.file.as_mut() {
            let _ = w.flush();
        }
        if let Some(w) = self.stdout.as_mut() {
            let _ = w.flush();
        }
    }
}

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
    /// Entries dropped because the background file-writer queue was full.
    pub writer_dropped: u64,
    /// Allow decisions dropped by sampling (`sample_allow_rate < 1.0`).
    pub sampled_out: u64,
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

    /// Entries dropped because the background writer queue was full.
    writer_dropped: AtomicU64,

    /// Allow decisions dropped by sampling (`sample_allow_rate < 1.0`).
    sampled_out: AtomicU64,

    /// Sender to the background file-writer thread (None if no file configured).
    /// File serialization and the write syscall happen on that thread, never on
    /// the request path.
    writer_tx: Option<SyncSender<WriterMsg>>,
}

impl DecisionBuffer {
    /// Create a new decision buffer with the given configuration
    pub fn new(config: DecisionLogConfig) -> std::io::Result<Self> {
        let writer_tx = if config.file_path.is_some() || config.emit_stdout {
            let file = if let Some(ref path) = config.file_path {
                // Ensure parent directory exists
                if let Some(parent) = Path::new(path).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Some(BufWriter::new(
                    OpenOptions::new().create(true).append(true).open(path)?,
                ))
            } else {
                None
            };
            let stdout = if config.emit_stdout {
                Some(BufWriter::new(std::io::stdout()))
            } else {
                None
            };
            let mut sinks = WriterSinks { file, stdout };

            let (tx, rx) = sync_channel::<WriterMsg>(WRITER_QUEUE_CAPACITY);

            // Dedicated writer thread: it owns the sinks and does all
            // serialization + I/O. It drains the queue in batches and flushes
            // once per batch, so bursts amortize into few syscalls.
            std::thread::Builder::new()
                .name("decision-log-writer".to_string())
                .spawn(move || {
                    while let Ok(msg) = rx.recv() {
                        Self::handle_writer_msg(&mut sinks, msg);
                        // Drain anything already queued, then flush once.
                        while let Ok(msg) = rx.try_recv() {
                            Self::handle_writer_msg(&mut sinks, msg);
                        }
                        sinks.flush_all();
                    }
                    // Channel closed (buffer dropped): final flush.
                    sinks.flush_all();
                })?;

            Some(tx)
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
            writer_dropped: AtomicU64::new(0),
            sampled_out: AtomicU64::new(0),
            writer_tx,
        })
    }

    /// Cheap pre-check the request path calls BEFORE building a `DecisionLogEntry`,
    /// so sampled-out or disabled decisions cost nothing (no allocation, no
    /// formatting). Returns true if this decision should be captured.
    ///
    /// Deny-priority sampling: denies are always kept (security-relevant);
    /// allows are kept with probability `sample_allow_rate` using a thread-local
    /// PRNG (a few ns, no shared state, no syscall).
    #[inline]
    pub fn should_log(&self, is_allow: bool) -> bool {
        if !self.config.enabled {
            return false;
        }
        if is_allow {
            if !self.config.log_allows {
                return false;
            }
            let rate = self.config.sample_allow_rate;
            if rate < 1.0 && (rate <= 0.0 || sample_unit() >= rate) {
                self.sampled_out.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        } else if !self.config.log_denies {
            return false;
        }
        true
    }

    /// Serialize a message once and fan it out to all configured sinks, on the
    /// background writer thread.
    fn handle_writer_msg(sinks: &mut WriterSinks, msg: WriterMsg) {
        match msg {
            WriterMsg::Entry(entry) => {
                if let Ok(json) = entry.to_ndjson() {
                    if let Some(w) = sinks.file.as_mut() {
                        let _ = writeln!(w, "{}", json);
                    }
                    if let Some(w) = sinks.stdout.as_mut() {
                        let _ = writeln!(w, "{}", json);
                    }
                }
            }
            WriterMsg::Flush => sinks.flush_all(),
        }
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

        // Hand file persistence to the background writer thread — no JSON
        // serialization and no write syscall on the request path. A clone is
        // sent to the writer while the original stays in the in-memory ring for
        // the query API. If the writer queue is saturated the entry is dropped
        // (and counted) rather than blocking the request.
        if let Some(ref tx) = self.writer_tx {
            if tx
                .try_send(WriterMsg::Entry(Box::new(entry.clone())))
                .is_err()
            {
                self.writer_dropped.fetch_add(1, Ordering::Relaxed);
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
            writer_dropped: self.writer_dropped.load(Ordering::Relaxed),
            sampled_out: self.sampled_out.load(Ordering::Relaxed),
        }
    }

    /// Request a flush of the file buffer to disk.
    ///
    /// The write is performed on the background writer thread, so this signals a
    /// flush rather than performing it synchronously (best-effort).
    pub fn flush(&self) -> std::io::Result<()> {
        if let Some(ref tx) = self.writer_tx {
            let _ = tx.try_send(WriterMsg::Flush);
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
    fn test_should_log_disabled() {
        let buffer = DecisionBuffer::new(DecisionLogConfig::default()).unwrap(); // disabled
        assert!(!buffer.should_log(true));
        assert!(!buffer.should_log(false));
    }

    #[test]
    fn test_should_log_deny_priority_sampling() {
        // Keep 0% of allows, but denies must always pass.
        let config = DecisionLogConfig {
            enabled: true,
            sample_allow_rate: 0.0,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        for _ in 0..1000 {
            assert!(buffer.should_log(false), "denies must never be sampled out");
            assert!(!buffer.should_log(true), "allows sampled out at rate 0.0");
        }
        assert_eq!(buffer.stats().sampled_out, 1000);
    }

    #[test]
    fn test_should_log_full_rate_keeps_all() {
        let config = DecisionLogConfig {
            enabled: true,
            sample_allow_rate: 1.0,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        for _ in 0..1000 {
            assert!(buffer.should_log(true));
        }
        assert_eq!(buffer.stats().sampled_out, 0);
    }

    #[test]
    fn test_should_log_partial_sampling_is_approximate() {
        let config = DecisionLogConfig {
            enabled: true,
            sample_allow_rate: 0.25,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        let n = 20_000;
        let kept = (0..n).filter(|_| buffer.should_log(true)).count();
        // ~25% kept; generous bounds to avoid flakiness.
        assert!(
            (3_000..7_000).contains(&kept),
            "expected ~5000 kept, got {kept}"
        );
        assert_eq!(buffer.stats().sampled_out as usize, n - kept);
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
    fn test_file_writer_persists_entries_async() {
        // Entries are serialized and written on the background writer thread;
        // verify they actually reach the file (polling, since it is async).
        let path =
            std::env::temp_dir().join(format!("reaper_declog_test_{}.ndjson", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 100,
            file_path: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };

        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.log(test_entry("allow"));
        buffer.log(test_entry("deny"));
        buffer.flush().unwrap();

        let mut contents = String::new();
        for _ in 0..200 {
            contents = std::fs::read_to_string(&path).unwrap_or_default();
            if contents.lines().count() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            contents.lines().count(),
            2,
            "both entries should be persisted to file by the writer thread"
        );
        assert!(contents.contains("\"decision\":\"allow\""));
        assert!(contents.contains("\"decision\":\"deny\""));
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
