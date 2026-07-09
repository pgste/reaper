//! Decision Buffer - Sharded ring buffer for decision logging
//!
//! Provides a high-performance, thread-safe buffer for storing decision log
//! entries with minimal latency impact on policy evaluation.
//!
//! ## Capture path (per-thread sharded, inline)
//!
//! The retention ring is split into N cache-padded shards, each a small
//! `RwLock<VecDeque>`. Every request thread maps to a stable shard, so under
//! concurrency producers take *disjoint, uncontended* locks — no shared lock,
//! no cross-core cache-line bouncing. Within a shard the push is inline (the
//! same cheap uncontended `parking_lot` acquire the old single-ring design paid),
//! so the single-thread cost doesn't regress and entries are queryable
//! immediately.
//!
//! Two designs were benchmarked before landing on this one:
//! - single `RwLock<VecDeque>` (original): 734 ns/op at 1 thread but collapses
//!   to 0.72M ops/s aggregate at 4 producer threads (lock convoy);
//! - lock-free `ArrayQueue` shards + background drain thread: 3.0M ops/s at 4
//!   threads, but the drain thread frees producer allocations cross-thread,
//!   contending the malloc arena against *every* allocation on the request
//!   path (+3.4µs per request in the full-handler bench at 1 thread).
//!
//! Sharding the ring itself keeps the inline push (same-thread alloc/free, no
//! second thread) *and* removes the shared lock. Global ordering across shards
//! is preserved exactly via a per-entry sequence number; queries merge shards
//! by sequence (queries are rare — the eval path is what matters).
//!
//! File/stdout serialization + I/O stay on the dedicated writer thread, fed an
//! `Arc` (no deep clone) — never the request path.

use crate::decision_log::{Checkpoint, DecisionLogConfig, DecisionLogEntry};
use crate::decision_privacy::DataProtection;
use crossbeam_utils::CachePadded;
use parking_lot::RwLock;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Global seed source so each thread's sampling PRNG starts distinct without an
/// RNG syscall or a time source on the hot path.
static SAMPLE_SEED: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

/// Monotonic per-thread tag used to map each thread to a capture shard. Assigned
/// once per thread on first use, so a given thread always hits the same shard.
static SHARD_TAG_SEQ: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static SAMPLE_RNG: Cell<u64> = Cell::new(seed_thread());
    static SHARD_TAG: u64 = SHARD_TAG_SEQ.fetch_add(1, Ordering::Relaxed);
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

/// Map the current thread to one of `n` shards (stable per thread).
#[inline]
fn shard_index(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    SHARD_TAG.with(|t| (*t as usize) % n)
}

/// Bound on the background file-writer queue. When full, entries are dropped
/// (and counted) rather than blocking the request thread — the in-memory ring
/// buffer still retains them for the query API.
const WRITER_QUEUE_CAPACITY: usize = 65_536;

/// Upper bound on auto-detected shard count.
const MAX_AUTO_SHARDS: usize = 64;

/// Message to the background decision-log writer.
enum WriterMsg {
    Entry(Arc<DecisionLogEntry>),
    Flush,
}

/// Running hash state for the durable decision stream. Lives only on the
/// single-threaded writer, so it needs no synchronization. Stamps each record's
/// `prev_hash`/`entry_hash` in write order — the tamper-evident chain a
/// regulator verifies over the NDJSON/ClickHouse artifact (Plan 04).
struct HashChain {
    last_hash: String,
}

impl HashChain {
    fn new() -> Self {
        Self {
            last_hash: String::new(),
        }
    }

    /// Link `record` to the chain and advance it.
    fn stamp(&mut self, record: &mut DecisionLogEntry) {
        record.prev_hash = self.last_hash.clone();
        let hash = record.compute_entry_hash(&self.last_hash);
        record.entry_hash = hash.clone();
        self.last_hash = hash;
    }
}

/// Emits signed checkpoints over contiguous runs of the durable stream (Plan 04,
/// step 3). Lives only on the writer thread (single-threaded, no sync). A window
/// opens on the first record after the previous checkpoint and closes when the
/// count threshold trips, the time threshold fires, or the writer shuts down.
struct Checkpointer {
    /// Per-boot chain identity, stamped into every checkpoint from this writer.
    chain_id: String,
    /// Signing key + key id. `None` ⇒ unsigned checkpoints (warned at startup).
    signing: Option<(reaper_core::bundle_signing::SigningKey, String)>,
    /// Close the window after this many records (0 = no count trigger).
    every: usize,
    /// Close the window at least this often when records are pending.
    interval: Option<Duration>,
    /// Monotonic base captured at writer start; checkpoint bounds are ns offsets
    /// from it, so they only ever increase (wall-clock rollback is detectable).
    base: Instant,
    /// Chain head as of the last emitted checkpoint (the next window's start
    /// hash). Empty before the first checkpoint. Threads the chain across
    /// checkpoints so a verifier can prove no gap hides between two of them.
    prev_checkpoint_hash: String,
    // --- open-window state (None until the first record after a checkpoint) ---
    seq_start: Option<u64>,
    seq_end: u64,
    count: u64,
    monotonic_start_ns: u64,
    last_hash: String,
}

impl Checkpointer {
    fn new(
        signing: Option<(reaper_core::bundle_signing::SigningKey, String)>,
        every: usize,
        interval_secs: u64,
    ) -> Self {
        Self {
            chain_id: uuid::Uuid::new_v4().to_string(),
            signing,
            every,
            interval: (interval_secs > 0).then(|| Duration::from_secs(interval_secs)),
            base: Instant::now(),
            prev_checkpoint_hash: String::new(),
            seq_start: None,
            seq_end: 0,
            count: 0,
            monotonic_start_ns: 0,
            last_hash: String::new(),
        }
    }

    /// Fold a just-stamped durable record into the open window; emit if the
    /// count threshold trips.
    fn on_entry(&mut self, seq: u64, entry_hash: &str, sinks: &mut WriterSinks) {
        if self.seq_start.is_none() {
            self.seq_start = Some(seq);
            self.monotonic_start_ns = self.base.elapsed().as_nanos() as u64;
        }
        self.seq_end = seq;
        self.count += 1;
        self.last_hash = entry_hash.to_string();
        if self.every > 0 && self.count >= self.every as u64 {
            self.emit(sinks);
        }
    }

    /// Time/shutdown trigger: emit a checkpoint if records are pending.
    fn flush_window(&mut self, sinks: &mut WriterSinks) {
        if self.count > 0 {
            self.emit(sinks);
        }
    }

    /// Build, sign, write, and reset the current window.
    fn emit(&mut self, sinks: &mut WriterSinks) {
        let Some(seq_start) = self.seq_start else {
            return;
        };
        let monotonic_end_ns = self.base.elapsed().as_nanos() as u64;
        let mut checkpoint = Checkpoint::new(
            self.chain_id.clone(),
            seq_start,
            self.seq_end,
            self.count,
            self.prev_checkpoint_hash.clone(),
            self.last_hash.clone(),
            self.monotonic_start_ns,
            monotonic_end_ns,
            chrono::Utc::now().to_rfc3339(),
        );
        if let Some((ref key, ref key_id)) = self.signing {
            checkpoint.sign(key, key_id);
        }
        if let Ok(json) = serde_json::to_string(&checkpoint) {
            if let Some(w) = sinks.file.as_mut() {
                let _ = writeln!(w, "{}", json);
            }
            if let Some(w) = sinks.stdout.as_mut() {
                let _ = writeln!(w, "{}", json);
            }
        }
        // The chain head of this window becomes the next window's start hash.
        self.prev_checkpoint_hash = self.last_hash.clone();
        self.seq_start = None;
        self.count = 0;
    }
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

/// One entry in a shard's ring: the global sequence number pins exact ordering
/// across shards; the `Arc` is shared with the writer thread (no deep clone).
type SeqEntry = (u64, Arc<DecisionLogEntry>);

/// A thread-safe ring buffer for decision log entries with per-thread sharded
/// capture (see module docs for the design rationale).
pub struct DecisionBuffer {
    /// Configuration
    config: DecisionLogConfig,

    /// Ring shards. Each request thread maps to a stable shard, so concurrent
    /// producers take disjoint locks. Cache-padded so neighbouring shards'
    /// lock words don't false-share.
    shards: Box<[CachePadded<RwLock<VecDeque<SeqEntry>>>]>,

    /// Per-shard retention cap (total capacity split across shards).
    shard_capacity: usize,

    /// Global sequence counter: total intake count AND the per-entry ordering
    /// key merged on at query time.
    seq: AtomicU64,

    /// Statistics counters (atomic for lock-free updates)
    dropped_entries: AtomicU64,
    flush_count: AtomicU64,
    allow_count: AtomicU64,
    deny_count: AtomicU64,

    /// Entries dropped because the background writer queue was full.
    writer_dropped: AtomicU64,

    /// Allow decisions dropped by sampling (`sample_allow_rate < 1.0`).
    sampled_out: AtomicU64,

    /// Sender to the background file-writer thread (None if no sinks configured).
    /// File serialization and the write syscall happen on that thread, never on
    /// the request path.
    writer_tx: Option<SyncSender<WriterMsg>>,

    /// Capture-time data protection (masking / pseudonymization / encryption).
    /// Applied before an entry reaches the ring or the writer, so every
    /// downstream view sees only protected data. None = nothing configured.
    protection: Option<DataProtection>,
}

impl DecisionBuffer {
    /// Create a new decision buffer with the given configuration.
    ///
    /// Fails closed on invalid data-protection config (hashing without a salt,
    /// encryption without a valid key) — the agent must not start logging
    /// unprotected data because a secret was missing.
    pub fn new(config: DecisionLogConfig) -> std::io::Result<Self> {
        let protection = DataProtection::from_config(&config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

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

            // Signed-checkpoint emitter (Plan 04 step 3). Built here — with the
            // durable sink — so the signing key is validated (fail closed) before
            // the writer starts; moved into the writer thread which owns it.
            let mut checkpointer = build_checkpointer(&config)?;
            let tick = checkpointer.as_ref().and_then(|c| c.interval);

            let (tx, rx) = sync_channel::<WriterMsg>(WRITER_QUEUE_CAPACITY);

            // Dedicated writer thread: it owns the sinks and does all
            // serialization + I/O. It drains the queue in batches and flushes
            // once per batch, so bursts amortize into few syscalls.
            std::thread::Builder::new()
                .name("decision-log-writer".to_string())
                .spawn(move || {
                    // Chain + checkpoint state live only here — the writer is
                    // single-threaded, so no synchronization is needed.
                    let mut chain = HashChain::new();
                    loop {
                        // With a time-based checkpoint interval, block only up to
                        // the interval so a low-traffic tail still gets attested;
                        // otherwise block indefinitely for the next message.
                        let msg = match tick {
                            Some(dt) => match rx.recv_timeout(dt) {
                                Ok(m) => m,
                                Err(RecvTimeoutError::Timeout) => {
                                    if let Some(cp) = checkpointer.as_mut() {
                                        cp.flush_window(&mut sinks);
                                    }
                                    sinks.flush_all();
                                    continue;
                                }
                                Err(RecvTimeoutError::Disconnected) => break,
                            },
                            None => match rx.recv() {
                                Ok(m) => m,
                                Err(_) => break,
                            },
                        };
                        Self::handle_writer_msg(&mut sinks, &mut chain, &mut checkpointer, msg);
                        // Drain anything already queued, then flush once.
                        while let Ok(msg) = rx.try_recv() {
                            Self::handle_writer_msg(&mut sinks, &mut chain, &mut checkpointer, msg);
                        }
                        sinks.flush_all();
                    }
                    // Channel closed (buffer dropped): attest the pending tail,
                    // then final flush.
                    if let Some(cp) = checkpointer.as_mut() {
                        cp.flush_window(&mut sinks);
                    }
                    sinks.flush_all();
                })?;

            Some(tx)
        } else {
            if config.checkpoint_every > 0 || config.checkpoint_interval_secs > 0 {
                tracing::warn!(
                    "decision-log checkpoints are configured but no durable sink \
                     (REAPER_DECISION_LOG_FILE / _STDOUT) is set — no checkpoints will be emitted"
                );
            }
            None
        };

        // Shard fan-out only matters when logging is enabled; a disabled buffer
        // keeps a single empty shard.
        let n_shards = if config.enabled {
            resolve_shards(&config)
        } else {
            1
        };
        // Split total capacity across shards (rounded up so N shards never
        // retain fewer entries than configured).
        let shard_capacity = config.buffer_capacity.div_ceil(n_shards).max(1);
        let shards: Box<[CachePadded<RwLock<VecDeque<SeqEntry>>>]> = (0..n_shards)
            .map(|_| CachePadded::new(RwLock::new(VecDeque::new())))
            .collect();

        Ok(Self {
            config,
            shards,
            shard_capacity,
            seq: AtomicU64::new(0),
            dropped_entries: AtomicU64::new(0),
            flush_count: AtomicU64::new(0),
            allow_count: AtomicU64::new(0),
            deny_count: AtomicU64::new(0),
            writer_dropped: AtomicU64::new(0),
            sampled_out: AtomicU64::new(0),
            writer_tx,
            protection,
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

    /// Whether the "explain" input-data snapshot should be captured for this
    /// decision. Cheap boolean check; the (heavier) snapshot itself is only done
    /// by the caller when this returns true. When the explain tier is off
    /// (default) this is always false → zero extra work.
    #[inline]
    pub fn should_capture_input(&self, is_allow: bool) -> bool {
        self.config.include_input_data && (!self.config.input_data_denies_only || !is_allow)
    }

    /// Serialize a message once and fan it out to all configured sinks, on the
    /// background writer thread.
    ///
    /// The writer is the single serialization point, so it is where the
    /// tamper-evident hash chain is stamped: each durable record is linked to
    /// the previous one in write order. The shared in-ring `Arc` is left
    /// hash-free (the chain describes the durable audit artifact, not the query
    /// ring), so we chain over a cheap writer-thread clone — off the hot path.
    fn handle_writer_msg(
        sinks: &mut WriterSinks,
        chain: &mut HashChain,
        checkpointer: &mut Option<Checkpointer>,
        msg: WriterMsg,
    ) {
        match msg {
            WriterMsg::Entry(entry) => {
                let mut record = (*entry).clone();
                chain.stamp(&mut record);
                if let Ok(json) = record.to_ndjson() {
                    if let Some(w) = sinks.file.as_mut() {
                        let _ = writeln!(w, "{}", json);
                    }
                    if let Some(w) = sinks.stdout.as_mut() {
                        let _ = writeln!(w, "{}", json);
                    }
                }
                // Fold the just-stamped durable record into the checkpoint
                // window (may emit a signed checkpoint over the same sinks).
                if let Some(cp) = checkpointer.as_mut() {
                    cp.on_entry(record.seq, &record.entry_hash, sinks);
                }
            }
            WriterMsg::Flush => sinks.flush_all(),
        }
    }

    /// Create a new buffer with default configuration
    pub fn with_defaults() -> Self {
        Self::new(DecisionLogConfig::default()).expect("Default config should not fail")
    }

    /// Add a decision log entry to the buffer.
    ///
    /// The request path does: filter checks, stat counters, an `Arc` hand-off to
    /// the writer thread (no JSON, no I/O, no deep clone), and one push into
    /// this thread's *own* ring shard — an uncontended lock under concurrency,
    /// since threads map to disjoint shards. If the shard is full the oldest
    /// entry is dropped (counted), same-thread, so allocation and free stay on
    /// the same malloc arena.
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

        // Data protection (masking / pseudonymization / encryption), applied
        // before the entry reaches the ring or the writer so no downstream view
        // ever sees raw values. Fail closed: if protection errors (e.g.
        // encryption failure), the entry is discarded, never logged raw.
        if let Some(ref protection) = self.protection {
            if let Err(e) = protection.apply(&mut entry) {
                tracing::error!(error = %e, "decision-log protection failed; entry discarded");
                return;
            }
        }

        // Update statistics. The sequence number doubles as the total-intake
        // counter and the exact global ordering key across shards.
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        // Persist the monotonic counter into the record itself, so the durable
        // audit stream (NDJSON / ClickHouse) carries it — the ordering key and
        // hash-chain position survive past the in-memory ring.
        entry.seq = seq;
        if is_allow {
            self.allow_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.deny_count.fetch_add(1, Ordering::Relaxed);
        }

        let arc = Arc::new(entry);

        // Hand file persistence to the background writer thread — no JSON
        // serialization and no write syscall on the request path. The writer
        // shares the Arc (no deep clone). If its queue is saturated the entry
        // is dropped there (and counted) rather than blocking the request.
        if let Some(ref tx) = self.writer_tx {
            if tx.try_send(WriterMsg::Entry(arc.clone())).is_err() {
                self.writer_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Push into this thread's shard (uncontended under concurrency).
        let mut ring = self.shards[shard_index(self.shards.len())].write();
        if ring.len() >= self.shard_capacity {
            ring.pop_front();
            self.dropped_entries.fetch_add(1, Ordering::Relaxed);
        }
        ring.push_back((seq, arc));
    }

    /// Collect `(seq, entry)` pairs from every shard that pass `keep`, sorted
    /// newest-first (descending sequence). Query-path helper — the merge cost
    /// lives here, never on the capture path.
    fn collect_sorted_desc<F: Fn(&DecisionLogEntry) -> bool>(&self, keep: F) -> Vec<SeqEntry> {
        let mut all: Vec<SeqEntry> = Vec::new();
        for shard in self.shards.iter() {
            let ring = shard.read();
            all.extend(
                ring.iter()
                    .filter(|(_, e)| keep(e))
                    .map(|(s, e)| (*s, e.clone())),
            );
        }
        all.sort_unstable_by_key(|b| std::cmp::Reverse(b.0));
        all
    }

    /// Get recent decisions (most recent first)
    pub fn get_recent(&self, limit: usize) -> Vec<DecisionLogEntry> {
        self.collect_sorted_desc(|_| true)
            .into_iter()
            .take(limit)
            .map(|(_, e)| (*e).clone())
            .collect()
    }

    /// Find a single decision by its `decision_id` (most recent match). Scans
    /// the in-memory ring — for older decisions, query the central store.
    pub fn find_by_decision_id(&self, decision_id: &str) -> Option<DecisionLogEntry> {
        let mut best: Option<SeqEntry> = None;
        for shard in self.shards.iter() {
            let ring = shard.read();
            if let Some((s, e)) = ring
                .iter()
                .rev()
                .find(|(_, e)| e.decision_id == decision_id)
            {
                if best.as_ref().is_none_or(|(bs, _)| *s > *bs) {
                    best = Some((*s, e.clone()));
                }
            }
        }
        best.map(|(_, e)| (*e).clone())
    }

    /// Get decisions with pagination (oldest-first ordering, matching the
    /// original single-ring behaviour).
    pub fn get_page(&self, offset: usize, limit: usize) -> Vec<DecisionLogEntry> {
        let mut all = self.collect_sorted_desc(|_| true);
        all.reverse(); // ascending (oldest first)
        all.into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, e)| (*e).clone())
            .collect()
    }

    /// Query decisions by filter (most recent first)
    pub fn query(&self, filter: DecisionFilter, limit: usize) -> Vec<DecisionLogEntry> {
        self.collect_sorted_desc(|e| filter.matches(e))
            .into_iter()
            .take(limit)
            .map(|(_, e)| (*e).clone())
            .collect()
    }

    /// Get current buffer statistics
    pub fn stats(&self) -> DecisionBufferStats {
        let buffer_size = self.shards.iter().map(|s| s.read().len()).sum();
        DecisionBufferStats {
            total_entries: self.seq.load(Ordering::Relaxed),
            buffer_size,
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
        for shard in self.shards.iter() {
            shard.write().clear();
        }
    }

    /// Export all entries as NDJSON (oldest first)
    pub fn export_ndjson(&self) -> String {
        let mut all = self.collect_sorted_desc(|_| true);
        all.reverse(); // ascending (oldest first)
        all.iter()
            .filter_map(|(_, e)| e.to_ndjson().ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get the configuration
    pub fn config(&self) -> &DecisionLogConfig {
        &self.config
    }
}

/// Build the checkpoint emitter from config (Plan 04 step 3), or `None` when no
/// checkpoint trigger is configured. Fails closed if a signing key is present
/// but invalid — the agent must not start emitting checkpoints under a bad key.
/// With triggers set but no key, warns and returns an unsigned emitter.
fn build_checkpointer(config: &DecisionLogConfig) -> std::io::Result<Option<Checkpointer>> {
    use reaper_core::bundle_signing::{SigAlgorithm, SigningKey};

    if config.checkpoint_every == 0 && config.checkpoint_interval_secs == 0 {
        return Ok(None);
    }

    let signing = match config.checkpoint_signing_key.as_deref() {
        Some(hex) if !hex.trim().is_empty() => {
            let alg = SigAlgorithm::parse(&config.checkpoint_algorithm).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("checkpoint signing algorithm: {e}"),
                )
            })?;
            let key = SigningKey::from_hex(alg, hex).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("checkpoint signing key: {e}"),
                )
            })?;
            let key_id = config
                .checkpoint_key_id
                .clone()
                .unwrap_or_else(|| "default".to_string());
            Some((key, key_id))
        }
        _ => {
            tracing::warn!(
                "decision-log checkpoints enabled without \
                 REAPER_DECISION_LOG_CHECKPOINT_SIGNING_KEY — emitting UNSIGNED checkpoints \
                 (completeness provable from count/last_entry_hash, authenticity is not)"
            );
            None
        }
    };

    Ok(Some(Checkpointer::new(
        signing,
        config.checkpoint_every,
        config.checkpoint_interval_secs,
    )))
}

/// Resolve the configured shard count, applying the auto-detect default and
/// clamping to a sane range.
fn resolve_shards(config: &DecisionLogConfig) -> usize {
    if config.capture_shards > 0 {
        return config.capture_shards.min(MAX_AUTO_SHARDS);
    }
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4)
        .clamp(1, MAX_AUTO_SHARDS)
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
    fn test_protection_applies_to_ring_and_file_sink() {
        // With protection configured, neither the query ring nor the file sink
        // may ever contain the raw principal or masked values — protection is
        // applied once at capture, upstream of both.
        let path = std::env::temp_dir().join(format!(
            "reaper_declog_prot_test_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let config = DecisionLogConfig {
            enabled: true,
            file_path: Some(path.to_string_lossy().to_string()),
            hash_principal: true,
            hash_salt: Some("test-salt".to_string()),
            mask_keys: vec!["token".to_string()],
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        let mut entry = test_entry("deny");
        entry.principal = "alice@example.com".to_string();
        entry.context.insert(
            "token".to_string(),
            serde_json::Value::String("s3cr3t".to_string()),
        );
        buffer.log(entry);
        buffer.flush().unwrap();

        // Ring view is protected.
        let recent = buffer.get_recent(1);
        assert!(recent[0].principal.starts_with("sha256:"));
        assert_eq!(recent[0].context["token"], serde_json::json!("***"));

        // File sink is protected too (written async by the writer thread).
        let mut contents = String::new();
        for _ in 0..200 {
            contents = std::fs::read_to_string(&path).unwrap_or_default();
            if !contents.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&path);
        assert!(!contents.contains("alice@example.com"));
        assert!(!contents.contains("s3cr3t"));
        assert!(contents.contains("sha256:"));
    }

    #[test]
    fn test_protection_misconfig_fails_buffer_creation() {
        // Fail closed: a missing secret must prevent startup, not silently log raw.
        let config = DecisionLogConfig {
            enabled: true,
            hash_principal: true,
            hash_salt: None,
            ..Default::default()
        };
        assert!(DecisionBuffer::new(config).is_err());
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
            capture_shards: 1, // single shard → exact global eviction order
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

    #[test]
    fn test_multi_shard_ordering_is_global() {
        // Entries logged from one thread land in one shard; entries from many
        // threads land in many shards — the sequence number must still yield
        // exact global ordering in queries.
        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 100_000,
            capture_shards: 8,
            ..Default::default()
        };
        let buffer = Arc::new(DecisionBuffer::new(config).unwrap());

        let threads = 8;
        let per_thread = 2_000;
        let mut handles = Vec::new();
        for t in 0..threads {
            let b = Arc::clone(&buffer);
            handles.push(std::thread::spawn(move || {
                for i in 0..per_thread {
                    let mut e = test_entry("allow");
                    e.principal = format!("t{t}_{i}");
                    b.log(e);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let expected = threads * per_thread;
        let stats = buffer.stats();
        assert_eq!(stats.total_entries as usize, expected);
        assert_eq!(stats.buffer_size, expected, "no loss under capacity");
        assert_eq!(stats.dropped_entries, 0);

        // get_recent returns every entry exactly once, newest-first by seq, and
        // pagination over the full set is disjoint + complete.
        let all = buffer.get_recent(expected + 10);
        assert_eq!(all.len(), expected);
        let unique: std::collections::HashSet<_> =
            all.iter().map(|e| e.principal.clone()).collect();
        assert_eq!(unique.len(), expected, "each entry appears exactly once");

        let page1 = buffer.get_page(0, expected / 2);
        let page2 = buffer.get_page(expected / 2, expected);
        assert_eq!(page1.len() + page2.len(), expected);
    }

    #[test]
    fn test_seq_is_persisted_into_entries() {
        // Every captured entry must carry its monotonic seq in the record (not
        // just the in-ring tuple), so the durable audit stream keeps the
        // ordering key and hash-chain position.
        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 10_000,
            capture_shards: 4,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        let n: usize = 500;
        for i in 0..n {
            let mut e = test_entry("allow");
            e.principal = format!("p{i}");
            buffer.log(e);
        }

        let all = buffer.get_recent(n + 10);
        assert_eq!(all.len(), n);

        // Seqs cover [0, n) exactly once — unique and contiguous (no gaps).
        let mut seqs: Vec<u64> = all.iter().map(|e| e.seq).collect();
        // As returned they are newest-first, i.e. strictly descending.
        assert!(
            seqs.windows(2).all(|w| w[0] > w[1]),
            "get_recent is strictly descending by seq"
        );
        seqs.sort_unstable();
        assert_eq!(
            seqs,
            (0..n as u64).collect::<Vec<_>>(),
            "contiguous unique seq"
        );

        // NDJSON round-trips seq.
        let json = all[0].to_ndjson().unwrap();
        let back: DecisionLogEntry = serde_json::from_str(json.trim()).unwrap();
        assert_eq!(back.seq, all[0].seq);
    }

    #[test]
    fn test_find_by_decision_id_across_shards() {
        let config = DecisionLogConfig {
            enabled: true,
            capture_shards: 4,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        let mut target = test_entry("deny");
        target.decision_id = "wanted-id".to_string();
        buffer.log(test_entry("allow"));
        buffer.log(target);
        buffer.log(test_entry("allow"));

        let found = buffer.find_by_decision_id("wanted-id").expect("found");
        assert_eq!(found.decision, "deny");
        assert!(buffer.find_by_decision_id("missing").is_none());
    }

    #[test]
    fn test_writer_emits_signed_checkpoints_that_verify() {
        use crate::decision_log::{verify_checkpoint, Checkpoint, DecisionLogEntry};
        use reaper_core::bundle_signing::{SigAlgorithm, SigningKey, VerifyingKey};

        let signing = SigningKey::from_hex(SigAlgorithm::Ed25519Sha256, &"07".repeat(32)).unwrap();
        let vk = VerifyingKey::from_hex(signing.algorithm(), &signing.public_key_hex()).unwrap();

        let path = std::env::temp_dir().join(format!(
            "reaper_declog_ckpt_test_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let config = DecisionLogConfig {
            enabled: true,
            buffer_capacity: 100,
            file_path: Some(path.to_string_lossy().to_string()),
            // A checkpoint every 5 durable records, signed.
            checkpoint_every: 5,
            checkpoint_signing_key: Some("07".repeat(32)),
            checkpoint_key_id: Some("k1".to_string()),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        for _ in 0..10 {
            buffer.log(test_entry("allow"));
        }
        buffer.flush().unwrap();

        // Poll until both checkpoint lines land (async writer thread).
        let mut decisions: Vec<DecisionLogEntry> = Vec::new();
        let mut checkpoints: Vec<Checkpoint> = Vec::new();
        for _ in 0..200 {
            decisions.clear();
            checkpoints.clear();
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            for line in contents.lines() {
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("record_type").and_then(|r| r.as_str()) == Some("checkpoint") {
                    checkpoints.push(serde_json::from_value(v).unwrap());
                } else {
                    decisions.push(serde_json::from_value(v).unwrap());
                }
            }
            if checkpoints.len() >= 2 && decisions.len() >= 10 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&path);

        assert_eq!(checkpoints.len(), 2, "a checkpoint every 5 of 10 records");
        assert_eq!(decisions.len(), 10);

        // Both checkpoints share one chain id (single writer boot).
        assert_eq!(checkpoints[0].chain_id, checkpoints[1].chain_id);
        // Contiguous coverage: [0,4] then [5,9].
        assert_eq!((checkpoints[0].seq_start, checkpoints[0].seq_end), (0, 4));
        assert_eq!((checkpoints[1].seq_start, checkpoints[1].seq_end), (5, 9));

        // Each checkpoint's signature verifies (key id pinned), and it proves the
        // covered records hash-chain to its head.
        for cp in &checkpoints {
            cp.verify_signature(&vk, Some("k1")).unwrap();
            let covered: Vec<DecisionLogEntry> = decisions
                .iter()
                .filter(|e| e.seq >= cp.seq_start && e.seq <= cp.seq_end)
                .cloned()
                .collect();
            verify_checkpoint(cp, &covered, &vk, Some("k1")).unwrap();
        }
    }

    #[test]
    fn test_invalid_checkpoint_key_fails_closed() {
        // A malformed signing key must prevent buffer creation, never silently
        // emit unsigned/garbage.
        let config = DecisionLogConfig {
            enabled: true,
            emit_stdout: true,
            checkpoint_every: 5,
            checkpoint_signing_key: Some("nothex".to_string()),
            ..Default::default()
        };
        assert!(DecisionBuffer::new(config).is_err());
    }
}
