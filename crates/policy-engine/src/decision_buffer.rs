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

use crate::decision_log::{Checkpoint, DecisionLogConfig, DecisionLogEntry, OnAuditUnavailable};
use crate::decision_privacy::DataProtection;
use crossbeam_utils::CachePadded;
use parking_lot::RwLock;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

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
    /// Best-effort fire-and-forget entry (no acknowledgement).
    Entry(Arc<DecisionLogEntry>),
    /// Mandatory-audit entry that must be made durable (flush + fsync) before
    /// the decision is served. The writer replies `true` on the oneshot iff
    /// serialize + write + flush + fsync all succeeded, else `false`.
    EntryAck(Arc<DecisionLogEntry>, oneshot::Sender<bool>),
    Flush,
}

/// How long `log_durable` waits for the writer's durability acknowledgement
/// before treating the decision as durable-unavailable (fail closed). Bounds the
/// worst-case eval tail added by mandatory-audit mode.
const DURABLE_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Outcome of the shared capture preamble used by both `log` (best-effort) and
/// `log_durable` (mandatory). By the time it returns, filters, protection, seq
/// assignment, allow/deny counters, and the ring push have all been applied —
/// the caller only decides how to hand the `Arc` to the writer thread.
enum Prepared {
    /// Nothing to persist (logging disabled or filtered out by should-log). A
    /// durable caller treats this as success — there is nothing to make durable.
    Skipped,
    /// Data protection failed and the entry was discarded (fail closed). A
    /// durable caller must NOT serve the decision.
    Discarded,
    /// Entry captured into the ring; hand this `Arc` to the writer thread.
    Ready(Arc<DecisionLogEntry>),
}

/// Cross-boot continuity record (round-2 A3.2). The writer persists this as
/// checkpoints emit and on graceful shutdown, so the NEXT writer boot can read
/// the prior boot's terminal chain head and stamp it into its genesis checkpoint
/// — making a whole-boot deletion detectable via the dangling reference.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct ContinuityRecord {
    /// The prior boot's chain_id.
    chain_id: String,
    /// The prior boot's terminal chain head (last emitted checkpoint's
    /// `last_entry_hash`).
    last_head: String,
    /// The prior boot's last covered seq (informational).
    last_seq: u64,
}

/// Best-effort read of the prior boot's continuity record. A missing file is a
/// first-ever boot (`None`, no warning); a corrupt/unparseable file is tolerated
/// (`None`, with a warning) so a tampered/garbage file never blocks startup.
fn read_continuity(path: &Path) -> Option<ContinuityRecord> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<ContinuityRecord>(&raw) {
        Ok(rec) if !rec.chain_id.is_empty() && !rec.last_head.is_empty() => Some(rec),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "decision-log continuity file is corrupt; treating as no prior boot"
            );
            None
        }
    }
}

/// Best-effort rewrite of the continuity file (temp-file + rename, so a crash
/// mid-write can't corrupt it). Never panics — a failure is logged and ignored,
/// bounding the loss to at most the current checkpoint window.
fn write_continuity_best_effort(path: &Path, rec: &ContinuityRecord) {
    let json = match serde_json::to_string(rec) {
        Ok(j) => j,
        Err(_) => return,
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let tmp = path.with_extension("continuity.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_ok() {
        if std::fs::rename(&tmp, path).is_err() {
            // Rename failed (e.g. cross-device); fall back to a direct write.
            let _ = std::fs::write(path, json.as_bytes());
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        tracing::warn!(
            path = %path.display(),
            "decision-log continuity file write failed (best-effort; ignored)"
        );
    }
}

/// Running hash state for the durable decision stream. Lives only on the
/// single-threaded writer, so it needs no synchronization. Stamps each record's
/// `prev_hash`/`entry_hash` in write order — the tamper-evident chain a
/// regulator verifies over the NDJSON/ClickHouse artifact (Plan 04).
struct HashChain {
    /// Per-writer-boot chain identity, stamped into every record before it is
    /// hashed so the record is bound to this chain (round-2 A2). Matches the
    /// Checkpointer's `chain_id` for the same boot.
    chain_id: String,
    last_hash: String,
}

impl HashChain {
    fn new(chain_id: String) -> Self {
        Self {
            chain_id,
            last_hash: String::new(),
        }
    }

    /// Link `record` to the chain and advance it.
    fn stamp(&mut self, record: &mut DecisionLogEntry) {
        // Bind the chain identity into the record BEFORE hashing, so the
        // entry_hash covers it (a record cannot be moved to another chain
        // undetected).
        record.chain_id = self.chain_id.clone();
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
    /// Genesis anchor (round-2 A3.2): the PRIOR boot's chain_id + terminal chain
    /// head, read from the continuity file at startup (empty for a first-ever
    /// boot). Stamped onto this boot's FIRST checkpoint only.
    genesis_prev_chain_id: String,
    genesis_prev_chain_head: String,
    /// Whether this boot's genesis anchor has already been stamped onto a
    /// checkpoint (only the first checkpoint carries it).
    genesis_emitted: bool,
    /// Where to persist this boot's `{chain_id, last_head, last_seq}` as
    /// checkpoints emit, so the next boot can link to it. `None` disables it.
    continuity_path: Option<PathBuf>,
    // --- open-window state (None until the first record after a checkpoint) ---
    seq_start: Option<u64>,
    seq_end: u64,
    count: u64,
    monotonic_start_ns: u64,
    last_hash: String,
}

impl Checkpointer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        chain_id: String,
        signing: Option<(reaper_core::bundle_signing::SigningKey, String)>,
        every: usize,
        interval_secs: u64,
        genesis_prev_chain_id: String,
        genesis_prev_chain_head: String,
        continuity_path: Option<PathBuf>,
    ) -> Self {
        Self {
            chain_id,
            signing,
            every,
            interval: (interval_secs > 0).then(|| Duration::from_secs(interval_secs)),
            base: Instant::now(),
            prev_checkpoint_hash: String::new(),
            genesis_prev_chain_id,
            genesis_prev_chain_head,
            genesis_emitted: false,
            continuity_path,
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
        // Genesis anchor (round-2 A3.2): only this boot's FIRST checkpoint carries
        // the prior boot's chain_id + terminal head, signed into the checkpoint so
        // the linkage is authenticated. Subsequent checkpoints leave it empty
        // (serializes away → byte-identical to a pre-A3 checkpoint).
        if !self.genesis_emitted {
            checkpoint.prev_chain_id = self.genesis_prev_chain_id.clone();
            checkpoint.prev_chain_head = self.genesis_prev_chain_head.clone();
            self.genesis_emitted = true;
        }
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
        // Persist continuity so the next boot links to this head. Best-effort:
        // never panics the writer; a failure bounds loss to < one window.
        if let Some(ref path) = self.continuity_path {
            write_continuity_best_effort(
                path,
                &ContinuityRecord {
                    chain_id: self.chain_id.clone(),
                    last_head: self.last_hash.clone(),
                    last_seq: self.seq_end,
                },
            );
        }
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
    /// Optional low-latency streaming mirror (round-2 E1 slice 4): a non-blocking
    /// hand-off of the captured `Arc<DecisionLogEntry>` to an out-of-process
    /// consumer (the agent's SIEM streaming task). It is a **best-effort telemetry
    /// mirror, NOT the durable audit artifact**, so a saturated/closed consumer
    /// drops (counted) rather than ever blocking the writer's durability path.
    stream: Option<SyncSender<Arc<DecisionLogEntry>>>,
    /// Streamed records dropped because the consumer was full/gone. Shared with
    /// the buffer so it surfaces as the `stream_dropped` stat.
    stream_dropped: Arc<AtomicU64>,
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

    /// Mirror one captured entry to the streaming consumer, non-blocking. Never
    /// affects durability or audit health — a drop here is telemetry loss only.
    fn stream_entry(&self, entry: &Arc<DecisionLogEntry>) {
        if let Some(tx) = self.stream.as_ref() {
            if tx.try_send(Arc::clone(entry)).is_err() {
                let prior = self.stream_dropped.fetch_add(1, Ordering::Relaxed);
                if prior == 0 {
                    tracing::warn!(
                        "decision stream sink saturated or closed: dropping streamed records \
                         (durable file/stdout sinks are unaffected)"
                    );
                }
            }
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
    /// Records lost from the durable sink (writer queue saturated or a sink
    /// write error) — a durable audit loss, not just a query-ring eviction.
    pub writer_dropped: u64,
    /// Allow decisions dropped by sampling (`sample_allow_rate < 1.0`).
    pub sampled_out: u64,
    /// Records dropped from the optional low-latency streaming mirror (E1 slice
    /// 4) because the consumer was saturated/gone — telemetry loss only, never a
    /// durable audit loss.
    pub stream_dropped: u64,
    /// Mandatory-audit mode has latched audit-compromised (a durable loss
    /// occurred): the agent is failing eval closed. Always false when audit is
    /// not required.
    pub audit_compromised: bool,
}

/// One entry in a shard's ring: the global sequence number pins exact ordering
/// across shards; the `Arc` is shared with the writer thread (no deep clone).
type SeqEntry = (u64, Arc<DecisionLogEntry>);

/// Durable-audit health, shared between the producer path and the writer thread
/// (Plan 04 steps 4-5). A "durable loss" — the writer queue was saturated or a
/// sink write failed, so a record never reached the durable audit artifact — is
/// counted, alarms once (`tracing::error!`), and in mandatory mode latches the
/// agent audit-compromised so eval fails closed.
#[derive(Clone)]
struct AuditHealth {
    /// Mandatory-audit mode: a durable loss is fatal to serving un-audited.
    required: bool,
    /// Count of durable losses (surfaced as the `writer_dropped` stat/metric).
    durable_loss: Arc<AtomicU64>,
    /// One-shot latch so the alarm logs once, not per dropped record.
    alarmed: Arc<AtomicBool>,
    /// Latched true on the first durable loss while `required` — readiness flips
    /// not-ready and evaluation fails closed until the agent is restarted.
    compromised: Arc<AtomicBool>,
}

impl AuditHealth {
    fn new(required: bool) -> Self {
        Self {
            required,
            durable_loss: Arc::new(AtomicU64::new(0)),
            alarmed: Arc::new(AtomicBool::new(false)),
            compromised: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Record a durable audit loss (writer-queue drop or sink write error).
    fn note_loss(&self) {
        self.durable_loss.fetch_add(1, Ordering::Relaxed);
        if !self.alarmed.swap(true, Ordering::Relaxed) {
            tracing::error!(
                mandatory = self.required,
                "decision-log DURABLE audit loss: a decision record did not reach the durable \
                 sink (writer queue saturated or sink write error)"
            );
        }
        if self.required {
            self.compromised.store(true, Ordering::Relaxed);
        }
    }

    fn durable_loss(&self) -> u64 {
        self.durable_loss.load(Ordering::Relaxed)
    }

    fn is_healthy(&self) -> bool {
        !self.compromised.load(Ordering::Relaxed)
    }
}

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
    /// One-shot alarm latch for in-memory ring eviction (query-history loss).
    ring_alarmed: AtomicBool,
    flush_count: AtomicU64,
    allow_count: AtomicU64,
    deny_count: AtomicU64,

    /// Durable-audit health: durable-loss count + mandatory fail-closed latch,
    /// shared with the writer thread.
    audit: AuditHealth,

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

    /// Records dropped from the optional streaming mirror (E1 slice 4) because
    /// the consumer was saturated/gone. Telemetry loss only — never a durable
    /// audit loss. Shared with the writer thread's `WriterSinks`.
    stream_dropped: Arc<AtomicU64>,
}

impl DecisionBuffer {
    /// Create a new decision buffer with the given configuration.
    ///
    /// Fails closed on invalid data-protection config (hashing without a salt,
    /// encryption without a valid key) — the agent must not start logging
    /// unprotected data because a secret was missing.
    pub fn new(config: DecisionLogConfig) -> std::io::Result<Self> {
        Self::new_with_stream(config, None)
    }

    /// Like [`Self::new`], but also attaches an optional low-latency streaming
    /// mirror (round-2 E1 slice 4): every captured decision is handed
    /// non-blocking to `stream` (typically an out-of-process SIEM push task).
    /// The mirror is best-effort telemetry — it never gates durability, and a
    /// saturated/closed consumer drops (counted as `stream_dropped`). Passing
    /// `Some(..)` also starts the writer thread even when no file/stdout sink is
    /// configured, so streaming can run standalone.
    pub fn new_with_stream(
        config: DecisionLogConfig,
        stream: Option<SyncSender<Arc<DecisionLogEntry>>>,
    ) -> std::io::Result<Self> {
        // Fail closed on invalid config (e.g. mandatory-audit invariants).
        config
            .validate()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let protection = DataProtection::from_config(&config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // Durable-audit health, shared with the writer thread so a sink write
        // error there also flags the loss.
        let audit = AuditHealth::new(config.audit_required);

        let stream_dropped = Arc::new(AtomicU64::new(0));

        let writer_tx = if config.file_path.is_some() || config.emit_stdout || stream.is_some() {
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
            let mut sinks = WriterSinks {
                file,
                stdout,
                stream,
                stream_dropped: stream_dropped.clone(),
            };

            // One chain identity per writer boot (round-2 A2), shared by the
            // HashChain (stamped into every decision record) and the
            // Checkpointer (stamped into every checkpoint) — so a boot's
            // decisions and checkpoints carry the SAME chain_id and a verifier
            // can reconstruct the chain from the queryable store.
            let chain_id = uuid::Uuid::new_v4().to_string();

            // Signed-checkpoint emitter (Plan 04 step 3). Built here — with the
            // durable sink — so the signing key is validated (fail closed) before
            // the writer starts; moved into the writer thread which owns it.
            let mut checkpointer = build_checkpointer(&config, chain_id.clone())?;
            let tick = checkpointer.as_ref().and_then(|c| c.interval);

            let (tx, rx) = sync_channel::<WriterMsg>(WRITER_QUEUE_CAPACITY);
            let writer_audit = audit.clone();

            // Dedicated writer thread: it owns the sinks and does all
            // serialization + I/O. It drains the queue in batches and flushes
            // once per batch, so bursts amortize into few syscalls.
            std::thread::Builder::new()
                .name("decision-log-writer".to_string())
                .spawn(move || {
                    // Chain + checkpoint state live only here — the writer is
                    // single-threaded, so no synchronization is needed. The
                    // HashChain shares the boot's chain_id with the checkpointer.
                    let mut chain = HashChain::new(chain_id);
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
                        Self::handle_writer_msg(
                            &mut sinks,
                            &mut chain,
                            &mut checkpointer,
                            &writer_audit,
                            msg,
                        );
                        // Drain anything already queued, then flush once.
                        while let Ok(msg) = rx.try_recv() {
                            Self::handle_writer_msg(
                                &mut sinks,
                                &mut chain,
                                &mut checkpointer,
                                &writer_audit,
                                msg,
                            );
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
            ring_alarmed: AtomicBool::new(false),
            flush_count: AtomicU64::new(0),
            allow_count: AtomicU64::new(0),
            deny_count: AtomicU64::new(0),
            audit,
            sampled_out: AtomicU64::new(0),
            writer_tx,
            protection,
            stream_dropped,
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
    ///
    /// F1-s4: actor-carrying requests are DEFAULT-ON — for agentic traffic
    /// the allows are the dangerous decisions and explaining them must not
    /// require an opt-in that nobody set before the incident. Operators can
    /// switch this off with `input_data_actor_requests = false`
    /// (`REAPER_DECISION_LOG_INPUT_DATA_ACTOR_REQUESTS=false`).
    #[inline]
    pub fn should_capture_input(&self, is_allow: bool, has_actor: bool) -> bool {
        (self.config.include_input_data && (!self.config.input_data_denies_only || !is_allow))
            || (self.config.input_data_actor_requests && has_actor)
    }

    /// Whether the replayable-capture snapshot (the full resolved request)
    /// should be captured for this decision (Plan 04 step 7). Cheap boolean;
    /// the snapshot is built by the caller only when this returns true — the
    /// hot path pays nothing while the tier is off (default).
    #[inline]
    pub fn should_capture_replay(&self, is_allow: bool) -> bool {
        self.config.include_replay_input && (!self.config.replay_input_denies_only || !is_allow)
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
        audit: &AuditHealth,
        msg: WriterMsg,
    ) {
        match msg {
            WriterMsg::Entry(entry) => {
                let mut record = (*entry).clone();
                chain.stamp(&mut record);
                // A serialization or sink write failure means this record did
                // not reach the durable artifact — a durable audit loss.
                match record.to_ndjson() {
                    Ok(json) => {
                        let mut write_err = false;
                        if let Some(w) = sinks.file.as_mut() {
                            write_err |= writeln!(w, "{}", json).is_err();
                        }
                        if let Some(w) = sinks.stdout.as_mut() {
                            write_err |= writeln!(w, "{}", json).is_err();
                        }
                        if write_err {
                            audit.note_loss();
                        }
                    }
                    Err(_) => audit.note_loss(),
                }
                // Best-effort low-latency mirror (never gates durability).
                sinks.stream_entry(&entry);
                // Fold the just-stamped durable record into the checkpoint
                // window (may emit a signed checkpoint over the same sinks).
                if let Some(cp) = checkpointer.as_mut() {
                    cp.on_entry(record.seq, &record.entry_hash, sinks);
                }
            }
            WriterMsg::EntryAck(entry, ack) => {
                // Identical to Entry, but the record is made DURABLE (flush +
                // fsync of the file sink) and the outcome is acknowledged, so a
                // mandatory-audit decision is only served after it is on disk.
                let mut record = (*entry).clone();
                chain.stamp(&mut record);
                let durable = match record.to_ndjson() {
                    Ok(json) => Self::write_and_sync(sinks, &json),
                    Err(_) => false,
                };
                if !durable {
                    audit.note_loss();
                }
                sinks.stream_entry(&entry);
                // Ignore send errors: the receiver may have timed out and gone.
                let _ = ack.send(durable);
                // Fold into the checkpoint window exactly like Entry.
                if let Some(cp) = checkpointer.as_mut() {
                    cp.on_entry(record.seq, &record.entry_hash, sinks);
                }
            }
            WriterMsg::Flush => sinks.flush_all(),
        }
    }

    /// Write one NDJSON line to all configured sinks and make the FILE sink
    /// durable (flush its buffer, then `fsync`/`sync_data`). Returns `true` iff a
    /// file sink is present and the write + flush + fsync all succeeded — the
    /// basis for a durable-before-serve acknowledgement. stdout is mirrored
    /// best-effort but cannot be fsynced (a pipe/console), so it never
    /// contributes to the durability verdict; with no file sink configured,
    /// durability cannot be guaranteed and this returns `false`.
    fn write_and_sync(sinks: &mut WriterSinks, json: &str) -> bool {
        let file_ok = if let Some(w) = sinks.file.as_mut() {
            writeln!(w, "{}", json)
                .and_then(|_| w.flush())
                .and_then(|_| w.get_ref().sync_data())
                .is_ok()
        } else {
            // No durable file/WAL sink → durability cannot be guaranteed.
            false
        };
        if let Some(w) = sinks.stdout.as_mut() {
            let _ = writeln!(w, "{}", json).and_then(|_| w.flush());
        }
        file_ok
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
    pub fn log(&self, entry: DecisionLogEntry) {
        let arc = match self.prepare_entry(entry) {
            Prepared::Ready(arc) => arc,
            // Nothing to persist, or protection discarded it (already logged).
            Prepared::Skipped | Prepared::Discarded => return,
        };

        // Best-effort fire-and-forget hand-off to the background writer thread —
        // no JSON serialization and no write syscall on the request path. On a
        // saturated writer queue, drop-and-count (never block the reactor); in
        // mandatory `fail_closed` mode that latches the agent audit-compromised
        // so eval fails closed instead of silently losing the record.
        //
        // The `Block` branch is retained for the legacy `on_audit_unavailable =
        // block` policy, but it is NO LONGER reached from the async eval hot path:
        // mandatory-audit mode now uses `log_durable` (try_send + async ack), so
        // the reactor is never blocked by a synchronous `send` here.
        if let Some(ref tx) = self.writer_tx {
            let block = self.config.audit_required
                && self.config.on_audit_unavailable == OnAuditUnavailable::Block;
            if block {
                // A send error only happens if the writer thread is gone; that
                // is itself a durable loss.
                if tx.send(WriterMsg::Entry(arc)).is_err() {
                    self.audit.note_loss();
                }
            } else if tx.try_send(WriterMsg::Entry(arc)).is_err() {
                self.audit.note_loss();
            }
        }
    }

    /// Mandatory-audit capture: make the decision **durable before it is served**
    /// without blocking the async reactor. Returns `true` iff the record is on
    /// disk (or there was nothing to persist); `false` means durability could not
    /// be guaranteed and the caller MUST fail the decision closed.
    ///
    /// Non-blocking by construction: the entry is handed to the writer thread via
    /// `try_send` (never a blocking `send`), and the writer's fsync outcome is
    /// awaited on a `oneshot` under a bounded [`DURABLE_ACK_TIMEOUT`]. A full
    /// writer queue, a dropped writer, or a timeout are all durable-unavailable →
    /// `note_loss` + `false`.
    pub async fn log_durable(&self, entry: DecisionLogEntry) -> bool {
        let arc = match self.prepare_entry(entry) {
            Prepared::Ready(arc) => arc,
            // Disabled or filtered out — nothing to persist, so durability holds.
            Prepared::Skipped => return true,
            // Protection failed and the entry was discarded — fail closed.
            Prepared::Discarded => return false,
        };

        // No writer/durable sink configured → durability cannot be guaranteed.
        let Some(ref tx) = self.writer_tx else {
            self.audit.note_loss();
            return false;
        };

        let (ack_tx, ack_rx) = oneshot::channel::<bool>();
        // try_send ONLY — never block the reactor. A saturated bounded queue is a
        // durable-unavailable condition, not something to backpressure the worker.
        if tx.try_send(WriterMsg::EntryAck(arc, ack_tx)).is_err() {
            self.audit.note_loss();
            return false;
        }

        // Await the writer's durability verdict without blocking the reactor.
        match tokio::time::timeout(DURABLE_ACK_TIMEOUT, ack_rx).await {
            // Durable: on disk and fsynced.
            Ok(Ok(true)) => true,
            // Writer already called note_loss on the durable failure.
            Ok(Ok(false)) => false,
            // Receiver error (writer dropped the sender) or timeout: fail closed.
            Ok(Err(_)) | Err(_) => {
                self.audit.note_loss();
                false
            }
        }
    }

    /// Whether mandatory durable-before-serve is in effect. When true the eval
    /// path must use [`log_durable`](Self::log_durable) and fail closed on a
    /// non-durable result; when false the default best-effort
    /// [`log`](Self::log) fire-and-forget path is used (zero added latency).
    /// Independent of the `FailClosed`/`Block` policy — both mean "no silent
    /// loss", so both want durability before serving.
    #[inline]
    pub fn mandatory_durable(&self) -> bool {
        self.config.enabled && self.config.audit_required
    }

    /// Shared capture preamble for [`log`](Self::log) and
    /// [`log_durable`](Self::log_durable): apply the decision-type filter,
    /// context stripping, and data protection (fail closed on error), then assign
    /// the monotonic seq, bump allow/deny counters, and push the entry into this
    /// thread's ring shard. Returns the shared `Arc` so the caller can hand it to
    /// the writer thread. Keeping this single-sourced stops the best-effort and
    /// durable paths from drifting.
    fn prepare_entry(&self, mut entry: DecisionLogEntry) -> Prepared {
        if !self.config.enabled {
            return Prepared::Skipped;
        }

        // Check if we should log this decision type
        let is_allow = entry.decision == "allow";
        if is_allow && !self.config.log_allows {
            return Prepared::Skipped;
        }
        if !is_allow && !self.config.log_denies {
            return Prepared::Skipped;
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
                return Prepared::Discarded;
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

        // Push into this thread's shard (uncontended under concurrency). Ring
        // eviction is in-memory query-history loss (not a durable-audit loss —
        // the writer already has the record); count it and alarm once.
        let mut ring = self.shards[shard_index(self.shards.len())].write();
        if ring.len() >= self.shard_capacity {
            ring.pop_front();
            self.dropped_entries.fetch_add(1, Ordering::Relaxed);
            if !self.ring_alarmed.swap(true, Ordering::Relaxed) {
                tracing::error!(
                    "decision-log in-memory ring is evicting entries \
                     (buffer_capacity too small for query retention); durable sink unaffected"
                );
            }
        }
        ring.push_back((seq, arc.clone()));
        Prepared::Ready(arc)
    }

    /// Whether the durable audit trail is intact. In mandatory-audit mode a
    /// durable loss latches this false; readiness and eval read it to fail
    /// closed. Always true when audit is not required.
    #[inline]
    pub fn is_audit_healthy(&self) -> bool {
        self.audit.is_healthy()
    }

    /// Whether mandatory-audit (fail-closed) mode is on.
    #[inline]
    pub fn audit_required(&self) -> bool {
        self.audit.required
    }

    /// Test hook: simulate a durable-sink loss deterministically (the real path
    /// triggers this on a saturated writer queue or a sink write error, which
    /// can't be forced reliably in a unit test).
    #[cfg(test)]
    fn force_durable_loss_for_test(&self) {
        self.audit.note_loss();
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
            writer_dropped: self.audit.durable_loss(),
            sampled_out: self.sampled_out.load(Ordering::Relaxed),
            stream_dropped: self.stream_dropped.load(Ordering::Relaxed),
            audit_compromised: !self.audit.is_healthy(),
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
fn build_checkpointer(
    config: &DecisionLogConfig,
    chain_id: String,
) -> std::io::Result<Option<Checkpointer>> {
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

    // Cross-boot continuity (round-2 A3.2): read the PRIOR boot's terminal chain
    // head from the continuity file (if any) so this boot's genesis checkpoint
    // links to it. A missing/corrupt file → a first-ever (root) boot.
    let (genesis_prev_chain_id, genesis_prev_chain_head) = match config.continuity_path.as_deref() {
        Some(path) => match read_continuity(path) {
            Some(rec) => (rec.chain_id, rec.last_head),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    };

    Ok(Some(Checkpointer::new(
        chain_id,
        signing,
        config.checkpoint_every,
        config.checkpoint_interval_secs,
        genesis_prev_chain_id,
        genesis_prev_chain_head,
        config.continuity_path.clone(),
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

/// Sender half of the low-latency streaming mirror (E1 slice 4): the writer
/// thread hands each captured `Arc<DecisionLogEntry>` here non-blocking.
pub type DecisionStreamSender = SyncSender<Arc<DecisionLogEntry>>;
/// Receiver half — owned by the consumer (e.g. the agent's SIEM push task).
pub type DecisionStreamReceiver = std::sync::mpsc::Receiver<Arc<DecisionLogEntry>>;

/// Create a bounded streaming-mirror channel. `capacity` bounds how many entries
/// buffer before the writer starts dropping (telemetry loss, never a durable
/// loss) — size it for the consumer's push latency.
pub fn decision_stream_channel(capacity: usize) -> (DecisionStreamSender, DecisionStreamReceiver) {
    sync_channel(capacity)
}

/// Create a shared decision buffer from configuration
pub fn create_shared_buffer(config: DecisionLogConfig) -> std::io::Result<SharedDecisionBuffer> {
    Ok(Arc::new(DecisionBuffer::new(config)?))
}

/// Create a shared decision buffer with an attached streaming mirror (E1 slice
/// 4). Pair with [`decision_stream_channel`]; the returned receiver feeds the
/// out-of-process SIEM push consumer.
pub fn create_shared_buffer_with_stream(
    config: DecisionLogConfig,
    stream: DecisionStreamSender,
) -> std::io::Result<SharedDecisionBuffer> {
    Ok(Arc::new(DecisionBuffer::new_with_stream(
        config,
        Some(stream),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_log::PrivacyProfile;

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
    fn streaming_mirror_receives_captured_entries() {
        // The streaming mirror (E1 slice 4) hands every captured entry to the
        // consumer channel, off the durability path.
        let (tx, rx) = decision_stream_channel(16);
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new_with_stream(config, Some(tx)).unwrap();
        buffer.log(test_entry("deny"));
        buffer.log(test_entry("deny"));

        let mut got = Vec::new();
        for _ in 0..2 {
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(e) => got.push(e),
                Err(_) => break,
            }
        }
        assert_eq!(got.len(), 2, "both captured entries reach the stream");
        assert!(got.iter().all(|e| e.decision == "deny"));
    }

    #[test]
    fn streaming_mirror_drops_when_saturated_without_blocking() {
        // A saturated consumer must never block the writer's durability path —
        // excess records drop and surface as the `stream_dropped` stat.
        let (tx, _rx) = decision_stream_channel(1); // capacity 1; _rx never drains
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new_with_stream(config, Some(tx)).unwrap();
        for _ in 0..50 {
            buffer.log(test_entry("deny"));
        }
        let mut dropped = 0;
        for _ in 0..200 {
            dropped = buffer.stats().stream_dropped;
            if dropped > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            dropped > 0,
            "saturated stream must drop, not block the writer"
        );
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
            privacy_profile: Some(PrivacyProfile::Raw),
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
        // A2: every decision record carries that SAME chain_id, so the chain is
        // reconstructable from the store by chain_id.
        let boot_chain = &checkpoints[0].chain_id;
        assert!(!boot_chain.is_empty());
        assert!(
            decisions.iter().all(|d| &d.chain_id == boot_chain),
            "decisions must carry the writer boot's chain_id"
        );
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
    fn test_cross_boot_continuity_links_boots() {
        // Two writer boots sharing one archive file + one continuity file: boot 2
        // must read boot 1's terminal head from the continuity file and stamp it
        // into its genesis checkpoint, so verify_records links the boots.
        use crate::decision_log::{verify_records, Checkpoint, DecisionLogEntry, VerifyMode};
        use reaper_core::bundle_signing::{SigAlgorithm, SigningKey, VerifyingKey};

        let signing = SigningKey::from_hex(SigAlgorithm::Ed25519Sha256, &"07".repeat(32)).unwrap();
        let vk = VerifyingKey::from_hex(signing.algorithm(), &signing.public_key_hex()).unwrap();

        let pid = std::process::id();
        let archive =
            std::env::temp_dir().join(format!("reaper_declog_continuity_arc_{pid}.ndjson"));
        let continuity = std::env::temp_dir().join(format!("reaper_declog_continuity_{pid}.json"));
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_file(&continuity);

        let make_config = || DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            buffer_capacity: 100,
            file_path: Some(archive.to_string_lossy().to_string()),
            continuity_path: Some(continuity.clone()),
            checkpoint_every: 5,
            checkpoint_signing_key: Some("07".repeat(32)),
            checkpoint_key_id: Some("k1".to_string()),
            ..Default::default()
        };

        // ---- Boot 1 ----
        let boot1 = DecisionBuffer::new(make_config()).unwrap();
        for _ in 0..5 {
            boot1.log(test_entry("allow"));
        }
        boot1.flush().unwrap();
        // The count-triggered checkpoint writes continuity synchronously; poll it.
        let mut boot1_continuity = None;
        for _ in 0..300 {
            if let Some(rec) = read_continuity(&continuity) {
                boot1_continuity = Some(rec);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(boot1);
        let boot1_continuity = boot1_continuity.expect("boot 1 wrote continuity");
        assert!(!boot1_continuity.chain_id.is_empty());
        assert!(!boot1_continuity.last_head.is_empty());

        // ---- Boot 2 (fresh chain_id, reads boot 1's continuity) ----
        let boot2 = DecisionBuffer::new(make_config()).unwrap();
        for _ in 0..5 {
            boot2.log(test_entry("allow"));
        }
        boot2.flush().unwrap();
        // Poll until boot 2's genesis checkpoint (carrying prev_chain_id) lands.
        let mut have_genesis = false;
        for _ in 0..300 {
            let contents = std::fs::read_to_string(&archive).unwrap_or_default();
            have_genesis = contents.lines().any(|l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .and_then(|v| {
                        v.get("prev_chain_id")
                            .and_then(|p| p.as_str())
                            .map(|s| !s.is_empty())
                    })
                    .unwrap_or(false)
            });
            if have_genesis {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(boot2);
        assert!(have_genesis, "boot 2 must emit a genesis checkpoint");

        // Parse the whole archive (both boots) into decisions + checkpoints.
        let contents = std::fs::read_to_string(&archive).unwrap_or_default();
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_file(&continuity);
        let mut decisions: Vec<DecisionLogEntry> = Vec::new();
        let mut checkpoints: Vec<Checkpoint> = Vec::new();
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

        // Exactly one checkpoint carries the genesis anchor, pointing at boot 1.
        let genesis: Vec<&Checkpoint> = checkpoints
            .iter()
            .filter(|c| !c.prev_chain_id.is_empty())
            .collect();
        assert_eq!(
            genesis.len(),
            1,
            "only boot 2's first checkpoint is genesis"
        );
        assert_eq!(genesis[0].prev_chain_id, boot1_continuity.chain_id);
        assert_eq!(genesis[0].prev_chain_head, boot1_continuity.last_head);

        // End-to-end: the archive verifies and the boots are linked.
        let report = verify_records(
            decisions,
            &checkpoints,
            &[("k1".to_string(), vk)],
            VerifyMode::ByteExact,
        );
        assert!(report.ok, "violations: {:?}", report.violations);
        assert_eq!(report.boots_linked, 1);
    }

    #[test]
    fn test_invalid_checkpoint_key_fails_closed() {
        // A malformed signing key must prevent buffer creation, never silently
        // emit unsigned/garbage.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            emit_stdout: true,
            checkpoint_every: 5,
            checkpoint_signing_key: Some("nothex".to_string()),
            ..Default::default()
        };
        assert!(DecisionBuffer::new(config).is_err());
    }

    /// A fully valid mandatory-audit config (fsync-able file sink + signed
    /// checkpoints). Mandatory mode requires a file sink for durable-before-serve,
    /// so each call gets a unique temp path to avoid cross-test interference.
    fn mandatory_config() -> DecisionLogConfig {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "reaper_mandatory_cfg_{}_{}.ndjson",
            std::process::id(),
            n
        ));
        DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            audit_required: true,
            file_path: Some(path.to_string_lossy().into_owned()),
            checkpoint_every: 100,
            checkpoint_signing_key: Some("07".repeat(32)),
            checkpoint_key_id: Some("k1".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_mandatory_config_conflicts_fail_creation() {
        // Sampling conflicts with mandatory audit.
        let mut c = mandatory_config();
        c.sample_allow_rate = 0.5;
        assert!(DecisionBuffer::new(c).is_err());

        // No durable sink.
        let mut c = mandatory_config();
        c.emit_stdout = false;
        c.file_path = None;
        assert!(DecisionBuffer::new(c).is_err());

        // No checkpoint signing key.
        let mut c = mandatory_config();
        c.checkpoint_signing_key = None;
        assert!(DecisionBuffer::new(c).is_err());

        // The valid config creates fine and starts healthy.
        let buffer = DecisionBuffer::new(mandatory_config()).unwrap();
        assert!(buffer.audit_required());
        assert!(buffer.is_audit_healthy());
    }

    #[test]
    fn test_mandatory_durable_loss_latches_fail_closed() {
        let buffer = DecisionBuffer::new(mandatory_config()).unwrap();
        assert!(buffer.is_audit_healthy());

        // A durable loss latches the agent audit-compromised (fail closed).
        buffer.force_durable_loss_for_test();
        assert!(
            !buffer.is_audit_healthy(),
            "mandatory loss must fail closed"
        );
        let stats = buffer.stats();
        assert!(stats.audit_compromised);
        assert_eq!(stats.writer_dropped, 1);
    }

    #[test]
    fn test_non_mandatory_durable_loss_counts_but_stays_healthy() {
        // Without mandatory audit, a durable loss is counted/alarmed but does
        // not fail eval closed (best-effort observability tier).
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            emit_stdout: true,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.force_durable_loss_for_test();
        assert!(buffer.is_audit_healthy(), "non-mandatory stays healthy");
        assert_eq!(buffer.stats().writer_dropped, 1);
        assert!(!buffer.stats().audit_compromised);
    }

    #[test]
    fn test_mandatory_durable_reflects_config() {
        // Disabled (default) → not mandatory-durable.
        let buffer = DecisionBuffer::new(DecisionLogConfig::default()).unwrap();
        assert!(!buffer.mandatory_durable());

        // Enabled but audit not required → best-effort, not durable-before-serve.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(!buffer.mandatory_durable());

        // Enabled + audit_required → durable-before-serve.
        let buffer = DecisionBuffer::new(mandatory_config()).unwrap();
        assert!(buffer.mandatory_durable());
    }

    #[tokio::test]
    async fn test_log_durable_persists_and_acks_with_file_sink() {
        // With a writable file sink, log_durable writes + fsyncs the record and
        // only THEN acks true — so the record is on disk the instant it returns.
        let path = std::env::temp_dir().join(format!(
            "reaper_declog_durable_ok_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            file_path: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        let mut entry = test_entry("allow");
        entry.principal = "durable-alice".to_string();
        let durable = buffer.log_durable(entry).await;
        assert!(
            durable,
            "log_durable must ack true when the file sink fsyncs"
        );

        // The fsync completed before the ack, so no polling is needed.
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        assert!(
            contents.contains("durable-alice"),
            "record must be durably on disk before the ack: {contents:?}"
        );
        assert_eq!(buffer.stats().writer_dropped, 0);
        assert!(buffer.is_audit_healthy());
    }

    #[tokio::test]
    async fn test_log_durable_without_file_sink_fails_closed() {
        // A stdout-only sink cannot be fsynced, so durability can't be
        // guaranteed: log_durable must return false and count a durable loss.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            emit_stdout: true,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(
            !buffer.log_durable(test_entry("deny")).await,
            "no file sink → not durable"
        );
        assert_eq!(buffer.stats().writer_dropped, 1);
    }

    #[tokio::test]
    async fn test_log_durable_no_writer_fails_closed() {
        // No sink at all → no writer thread → durability impossible → false.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(!buffer.log_durable(test_entry("deny")).await);
        assert_eq!(buffer.stats().writer_dropped, 1);
    }

    #[tokio::test]
    async fn test_log_durable_disabled_returns_true_and_writes_nothing() {
        // Disabled logging has nothing to persist → durability trivially holds.
        let config = DecisionLogConfig {
            enabled: false,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(buffer.log_durable(test_entry("allow")).await);
        let stats = buffer.stats();
        assert_eq!(stats.total_entries, 0);
        assert_eq!(stats.writer_dropped, 0);
    }

    #[test]
    fn test_ring_eviction_is_not_a_durable_loss() {
        // In-memory ring eviction (dropped_entries) is query-history loss, not a
        // durable-audit loss: it must not flip audit health.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            buffer_capacity: 3,
            capture_shards: 1,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        for _ in 0..10 {
            buffer.log(test_entry("allow"));
        }
        let stats = buffer.stats();
        assert!(stats.dropped_entries >= 1, "ring evicted");
        assert_eq!(stats.writer_dropped, 0, "no durable loss");
        assert!(buffer.is_audit_healthy());
    }

    // ---- Replayable-capture tier (Plan 04 step 7) ----

    #[test]
    fn test_replay_gate_off_by_default() {
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(!buffer.should_capture_replay(true));
        assert!(!buffer.should_capture_replay(false));
    }

    #[test]
    fn test_replay_gate_on_captures_both_directions_by_default() {
        // Unlike the explain tier, replay defaults to BOTH decisions: flips
        // happen in both directions, denies-only could never surface an
        // allow→deny flip.
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            include_replay_input: true,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(buffer.should_capture_replay(true));
        assert!(buffer.should_capture_replay(false));

        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            include_replay_input: true,
            replay_input_denies_only: true,
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(!buffer.should_capture_replay(true));
        assert!(buffer.should_capture_replay(false));
    }

    #[test]
    fn test_replay_input_reaches_the_durable_sink_protected() {
        // End to end through the writer thread: the replay blob lands in the
        // NDJSON sink, with capture-time protection applied (masked key never
        // stored raw — the same guarantee the context/input_data sinks have).
        let path = std::env::temp_dir().join(format!(
            "reaper_declog_replay_test_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let config = DecisionLogConfig {
            enabled: true,
            file_path: Some(path.to_string_lossy().to_string()),
            include_replay_input: true,
            mask_keys: vec!["token".to_string()],
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();

        let mut entry = test_entry("deny");
        entry.replay_input = Some(serde_json::json!({
            "principal": "user",
            "action": "read",
            "resource": "resource",
            "context": {"token": "s3cr3t-replay", "region": "eu"}
        }));
        buffer.log(entry);
        buffer.flush().unwrap();

        let mut contents = String::new();
        for _ in 0..200 {
            contents = std::fs::read_to_string(&path).unwrap_or_default();
            if !contents.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&path);

        assert!(
            !contents.contains("s3cr3t-replay"),
            "masked value never raw"
        );
        let row: DecisionLogEntry = serde_json::from_str(contents.trim()).unwrap();
        let replay = row.replay_input.expect("replay blob persisted");
        assert_eq!(replay["context"]["token"], serde_json::json!("***"));
        assert_eq!(replay["context"]["region"], serde_json::json!("eu"));
        assert_eq!(replay["action"], serde_json::json!("read"));
        // The ring view carries it too (same Arc).
        let recent = buffer.get_recent(1);
        assert!(recent[0].replay_input.is_some());
    }

    #[test]
    fn test_replay_tier_off_stores_no_field() {
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        buffer.log(test_entry("deny"));
        let recent = buffer.get_recent(1);
        assert!(recent[0].replay_input.is_none());
        // And the NDJSON line omits the key entirely (skip_serializing_if).
        let json = recent[0].to_ndjson().unwrap();
        assert!(!json.contains("replay_input"));
    }
}
