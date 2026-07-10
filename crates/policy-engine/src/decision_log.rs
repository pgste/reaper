//! Decision Logging for Policy Evaluation
//!
//! Provides structured decision logging for audit, compliance, and observability.
//! Compatible with SIEM systems via NDJSON export format.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single decision log entry capturing all relevant context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogEntry {
    /// ISO 8601 timestamp
    pub timestamp: String,

    /// Unique decision ID (UUID)
    pub decision_id: String,

    /// OpenTelemetry trace ID for correlation (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// Principal (user) ID
    pub principal: String,

    /// Action being performed
    pub action: String,

    /// Resource being accessed
    pub resource: String,

    /// Additional context from the request
    #[serde(default)]
    pub context: HashMap<String, serde_json::Value>,

    /// Decision result: "allow", "deny", or "log"
    pub decision: String,

    /// Policy ID that was evaluated
    pub policy_id: String,

    /// Policy name
    pub policy_name: String,

    /// Policy version (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,

    /// Evaluation time in nanoseconds
    pub evaluation_time_ns: u64,

    /// Whether the result came from cache
    pub cache_hit: bool,

    /// Agent ID that processed the request (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Matched rule name (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,

    /// "Explain" snapshot: the resolved principal/resource entity attributes the
    /// decision branched on (e.g. `{"principal": {...}, "resource": {...}}`).
    /// Present only when the explain tier is enabled (heavier; opt-in, typically
    /// denies-only). Makes a decision reproducible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_data: Option<serde_json::Value>,

    /// Replayable-capture tier (Plan 04, step 7): the full resolved request as
    /// a self-contained blob — `{"principal", "action", "resource", "context"}`
    /// — so the counterfactual replay engine can re-evaluate this decision
    /// under a different policy/data version. Distinct from `context`, which is
    /// display-oriented and may be allowlisted/dropped; this snapshot is meant
    /// to be faithful. Protection still applies identically (mask/pseudonymize/
    /// encrypt): tenants that need BOTH privacy and full-fidelity replay use
    /// encryption (reversible by the tenant key holder at replay time), not
    /// masking/hashing (irreversible). Off unless the tier is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_input: Option<serde_json::Value>,

    /// Data-plane provenance: the datastore version this decision evaluated
    /// against (audits can pin exactly what data a decision saw).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_version: Option<i64>,

    /// Checksum of the data version (sha256:… as published by the control
    /// plane; verified by the agent on load).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_checksum: Option<String>,

    /// True when the agent's data exceeded its configured staleness budget
    /// at evaluation time (REAPER_DATA_MAX_STALENESS_SECS, mode=flag).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub data_stale: bool,

    /// Monotonic per-agent sequence number, assigned at capture time. It is the
    /// exact global ordering key across the sharded ring and the position in the
    /// tamper-evident hash chain (Plan 04). Strictly increasing; a gap in the
    /// durable stream signals a dropped or deleted record.
    #[serde(default)]
    pub seq: u64,

    /// Hash of the previous record in the durable stream. Empty on the in-memory
    /// query ring (the chain is a property of the durable audit artifact, not
    /// the ring); stamped by the writer thread. Plan 04.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev_hash: String,

    /// `sha256(canonical(record without hashes) || prev_hash)`. Together with
    /// `prev_hash` this forms a hash chain: any insertion, deletion, reordering,
    /// or mutation of the durable stream fails re-verification.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entry_hash: String,
}

/// A hash-chain verification failure, naming the record where it broke.
#[derive(Debug, Clone)]
pub struct ChainError {
    pub seq: u64,
    pub reason: String,
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "audit chain broken at seq {}: {}", self.seq, self.reason)
    }
}

impl std::error::Error for ChainError {}

/// Verify a run of decision records (in durable/write order) forms an intact
/// hash chain. Detects mutation (`entry_hash` mismatch), and insertion /
/// deletion / reordering (`prev_hash` mismatch), failing at the offending `seq`.
pub fn verify_chain(entries: &[DecisionLogEntry]) -> Result<(), ChainError> {
    verify_chain_from(entries, "")
}

/// Like [`verify_chain`] but the run is expected to link from `start_prev` (the
/// chain head just before the first entry) rather than the genesis empty hash.
/// Used to verify a checkpoint's sub-range, which starts mid-stream from the
/// previous checkpoint's `last_entry_hash`.
pub fn verify_chain_from(entries: &[DecisionLogEntry], start_prev: &str) -> Result<(), ChainError> {
    let mut prev = start_prev.to_string();
    for e in entries {
        if e.prev_hash != prev {
            return Err(ChainError {
                seq: e.seq,
                reason: "prev_hash does not link to the previous record \
                         (insertion, deletion, or reordering)"
                    .to_string(),
            });
        }
        let recomputed = e.compute_entry_hash(&e.prev_hash);
        if recomputed != e.entry_hash {
            return Err(ChainError {
                seq: e.seq,
                reason: "entry_hash mismatch (record was mutated)".to_string(),
            });
        }
        prev = e.entry_hash.clone();
    }
    Ok(())
}

/// Record-type discriminator for checkpoint lines in the NDJSON stream. Decision
/// records carry no `record_type`; the shipper routes on its presence/value.
pub const CHECKPOINT_RECORD_TYPE: &str = "checkpoint";

/// A signed checkpoint over a contiguous run of the durable decision stream
/// (Plan 04, step 3). It pins the covered `seq` range, the entry count, and the
/// chain head (`last_entry_hash`) at a point in time, signed with an agent
/// signing key. A verifier can then prove *completeness* of a range — no entry
/// silently dropped — without every intervening record being online, and detect
/// wall-clock rollback via the monotonic bounds.
///
/// Emitted as its own NDJSON line typed by [`CHECKPOINT_RECORD_TYPE`] so a
/// single stream carries both decisions and checkpoints (Vector routes them to
/// separate ClickHouse tables).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    /// Always [`CHECKPOINT_RECORD_TYPE`] — the shipper's routing discriminator.
    pub record_type: String,
    /// Per-writer-boot chain identity (a fresh UUID each agent/writer start), so
    /// checkpoints from different boots are never confused for one chain.
    pub chain_id: String,
    /// First `seq` covered by this checkpoint (inclusive).
    pub seq_start: u64,
    /// Last `seq` covered by this checkpoint (inclusive).
    pub seq_end: u64,
    /// Number of records in `[seq_start, seq_end]` (`seq_end - seq_start + 1`
    /// for a gapless range — a mismatch means the checkpoint's own range hides a
    /// drop).
    pub count: u64,
    /// Chain head *before* this range (the previous checkpoint's
    /// `last_entry_hash`, empty for the first checkpoint of a chain). The first
    /// covered record must link to it, so consecutive checkpoints chain
    /// end-to-end and no gap can hide between them.
    #[serde(default)]
    pub prev_hash: String,
    /// `entry_hash` of the last record in the covered range: the chain head the
    /// covered records must hash to.
    pub last_entry_hash: String,
    /// Monotonic clock (ns since writer start) at the first covered record.
    pub monotonic_start_ns: u64,
    /// Monotonic clock (ns since writer start) at checkpoint emission. Monotonic
    /// bounds only ever increase, so wall-clock rollback between checkpoints is
    /// detectable even if `wallclock` is tampered.
    pub monotonic_end_ns: u64,
    /// Wall-clock (RFC3339) at emission — for human/operational correlation.
    pub wallclock: String,
    /// Signing key id (for rotation / pinning). Empty ⇒ unsigned checkpoint.
    #[serde(default)]
    pub key_id: String,
    /// Signature algorithm (`ed25519-sha256` / `ecdsa-p256-sha256`). Empty ⇒
    /// unsigned.
    #[serde(default)]
    pub algorithm: String,
    /// Lowercase-hex signature over `canonical(checkpoint sans signature)`.
    /// Empty ⇒ unsigned (a loud warning was emitted at startup; completeness is
    /// still checkable from `count`/`last_entry_hash`, authenticity is not).
    #[serde(default)]
    pub signature: String,
}

impl Checkpoint {
    /// Build an unsigned checkpoint over `[seq_start, seq_end]`. Sign it with
    /// [`Checkpoint::sign`] before emitting when a key is configured.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: String,
        seq_start: u64,
        seq_end: u64,
        count: u64,
        prev_hash: String,
        last_entry_hash: String,
        monotonic_start_ns: u64,
        monotonic_end_ns: u64,
        wallclock: String,
    ) -> Self {
        Self {
            record_type: CHECKPOINT_RECORD_TYPE.to_string(),
            chain_id,
            seq_start,
            seq_end,
            count,
            prev_hash,
            last_entry_hash,
            monotonic_start_ns,
            monotonic_end_ns,
            wallclock,
            key_id: String::new(),
            algorithm: String::new(),
            signature: String::new(),
        }
    }

    /// Canonical bytes of the checkpoint with the `signature` field excluded —
    /// the message that gets signed. `key_id` and `algorithm` stay *inside* the
    /// signed message, so tampering with either breaks authenticity.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut clone = self.clone();
        clone.signature = String::new();
        let value = serde_json::to_value(&clone).unwrap_or(serde_json::Value::Null);
        serde_json::to_vec(&canonicalize(&value)).unwrap_or_default()
    }

    /// Sign this checkpoint in place with `key`/`key_id`, reusing the bundle
    /// signing primitive. Sets `algorithm` and `signature`.
    pub fn sign(&mut self, key: &reaper_core::bundle_signing::SigningKey, key_id: &str) {
        self.key_id = key_id.to_string();
        self.algorithm = key.algorithm().as_str().to_string();
        self.signature = String::new();
        let canonical = self.canonical_bytes();
        let envelope = reaper_core::bundle_signing::sign_bundle(&canonical, key, key_id);
        self.signature = envelope.signature;
    }

    /// Verify the checkpoint's signature with `vk`, optionally pinning
    /// `expected_key_id`. An unsigned checkpoint (`signature` empty) is rejected.
    pub fn verify_signature(
        &self,
        vk: &reaper_core::bundle_signing::VerifyingKey,
        expected_key_id: Option<&str>,
    ) -> Result<(), reaper_core::bundle_signing::SignatureError> {
        use reaper_core::bundle_signing::{BundleSignature, SignatureError};
        if self.signature.is_empty() {
            return Err(SignatureError::BadSignature);
        }
        let canonical = self.canonical_bytes();
        let envelope = BundleSignature {
            envelope_version: 1,
            algorithm: self.algorithm.clone(),
            key_id: self.key_id.clone(),
            bundle_id: String::new(),
            version: 0,
            not_before: 0,
            expires_at: 0,
            sha256: hex::encode(reaper_core::bundle_signing::sha256(&canonical)),
            signature: self.signature.clone(),
        };
        reaper_core::bundle_signing::verify_bundle(&canonical, &envelope, vk, expected_key_id)
    }
}

/// A checkpoint-verification failure, naming the offending `seq` when the break
/// is a specific covered record.
#[derive(Debug, Clone)]
pub struct CheckpointError {
    pub seq: Option<u64>,
    pub reason: String,
}

impl std::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.seq {
            Some(seq) => write!(
                f,
                "checkpoint verification failed at seq {seq}: {}",
                self.reason
            ),
            None => write!(f, "checkpoint verification failed: {}", self.reason),
        }
    }
}

impl std::error::Error for CheckpointError {}

/// Verify a signed checkpoint against the exact run of records it covers (in
/// durable/write order). All must hold (fail closed): the signature is valid
/// under `vk` (with `expected_key_id` pinned when given); `entries` form an
/// intact hash chain; they cover `[seq_start, seq_end]` gaplessly with the
/// claimed `count`; and the last record's `entry_hash` equals `last_entry_hash`.
/// A tampered, missing, inserted, or reordered record fails, naming the seq.
pub fn verify_checkpoint(
    checkpoint: &Checkpoint,
    entries: &[DecisionLogEntry],
    vk: &reaper_core::bundle_signing::VerifyingKey,
    expected_key_id: Option<&str>,
) -> Result<(), CheckpointError> {
    // 1. Authenticity of the checkpoint itself.
    checkpoint
        .verify_signature(vk, expected_key_id)
        .map_err(|e| CheckpointError {
            seq: None,
            reason: format!("signature invalid: {e}"),
        })?;

    // 2. The covered records must be an intact chain that links from the
    //    checkpoint's declared start hash (the previous checkpoint's head).
    verify_chain_from(entries, &checkpoint.prev_hash).map_err(|e| CheckpointError {
        seq: Some(e.seq),
        reason: e.reason,
    })?;

    // 3. Count must match — a checkpoint can't hide a drop inside its own range.
    if entries.len() as u64 != checkpoint.count {
        return Err(CheckpointError {
            seq: None,
            reason: format!(
                "count mismatch: checkpoint claims {} records, range has {}",
                checkpoint.count,
                entries.len()
            ),
        });
    }

    // 4. Range coverage: first/last seq must match the claimed bounds.
    match (entries.first(), entries.last()) {
        (Some(first), Some(last)) => {
            if first.seq != checkpoint.seq_start {
                return Err(CheckpointError {
                    seq: Some(first.seq),
                    reason: format!(
                        "seq_start mismatch: checkpoint claims {}, range starts at {}",
                        checkpoint.seq_start, first.seq
                    ),
                });
            }
            if last.seq != checkpoint.seq_end {
                return Err(CheckpointError {
                    seq: Some(last.seq),
                    reason: format!(
                        "seq_end mismatch: checkpoint claims {}, range ends at {}",
                        checkpoint.seq_end, last.seq
                    ),
                });
            }
            // 5. The chain head must match the covered records.
            if last.entry_hash != checkpoint.last_entry_hash {
                return Err(CheckpointError {
                    seq: Some(last.seq),
                    reason: "last_entry_hash does not match the covered records".to_string(),
                });
            }
        }
        _ => {
            if checkpoint.count != 0 {
                return Err(CheckpointError {
                    seq: None,
                    reason: "checkpoint claims records but none were supplied".to_string(),
                });
            }
        }
    }

    Ok(())
}

/// Recursively sort object keys so the same record always hashes to the same
/// bytes, regardless of `HashMap`/JSON iteration order.
fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize(&m[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(canonicalize).collect())
        }
        other => other.clone(),
    }
}

impl DecisionLogEntry {
    /// Create a new decision log entry with required fields
    pub fn new(
        principal: String,
        action: String,
        resource: String,
        decision: String,
        policy_id: String,
        policy_name: String,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            decision_id: uuid::Uuid::new_v4().to_string(),
            trace_id: None,
            principal,
            action,
            resource,
            context: HashMap::new(),
            decision,
            policy_id,
            policy_name,
            policy_version: None,
            evaluation_time_ns: 0,
            cache_hit: false,
            agent_id: None,
            matched_rule: None,
            input_data: None,
            replay_input: None,
            data_version: None,
            data_checksum: None,
            data_stale: false,
            seq: 0,
            prev_hash: String::new(),
            entry_hash: String::new(),
        }
    }

    /// Canonical bytes of this record with the hash fields excluded — the input
    /// that gets hashed. Object keys are sorted so it is byte-stable across runs
    /// and platforms.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut clone = self.clone();
        clone.prev_hash = String::new();
        clone.entry_hash = String::new();
        // to_value cannot fail for a plain serializable struct; fall back to an
        // empty object rather than panicking on the audit path.
        let value = serde_json::to_value(&clone).unwrap_or(serde_json::Value::Null);
        serde_json::to_vec(&canonicalize(&value)).unwrap_or_default()
    }

    /// `entry_hash = sha256(canonical_bytes || prev_hash)`.
    pub fn compute_entry_hash(&self, prev_hash: &str) -> String {
        let mut bytes = self.canonical_bytes();
        bytes.extend_from_slice(prev_hash.as_bytes());
        hex::encode(reaper_core::bundle_signing::sha256(&bytes))
    }

    /// Attach the "explain" input-data snapshot.
    pub fn with_input_data(mut self, input: serde_json::Value) -> Self {
        self.input_data = Some(input);
        self
    }

    /// Attach the replayable-capture snapshot (the full resolved request).
    pub fn with_replay_input(mut self, input: serde_json::Value) -> Self {
        self.replay_input = Some(input);
        self
    }

    /// Set the trace ID for OpenTelemetry correlation
    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the context
    pub fn with_context(mut self, context: HashMap<String, serde_json::Value>) -> Self {
        self.context = context;
        self
    }

    /// Set the policy version
    pub fn with_policy_version(mut self, version: String) -> Self {
        self.policy_version = Some(version);
        self
    }

    /// Set the evaluation time in nanoseconds
    pub fn with_evaluation_time_ns(mut self, ns: u64) -> Self {
        self.evaluation_time_ns = ns;
        self
    }

    /// Mark as a cache hit
    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the matched rule name
    pub fn with_matched_rule(mut self, rule: String) -> Self {
        self.matched_rule = Some(rule);
        self
    }

    /// Stamp data-plane sync provenance (version 0 = never synced: skipped).
    pub fn with_data_sync(mut self, version: i64, checksum: Option<String>, stale: bool) -> Self {
        if version > 0 {
            self.data_version = Some(version);
            self.data_checksum = checksum;
        }
        self.data_stale = stale;
        self
    }

    /// Convert to an NDJSON line.
    ///
    /// This runs on the background writer thread (never the eval hot path), but
    /// serialization speed still sets how fast the shipper drains the queue at
    /// high volume — so use SIMD `sonic-rs` on native targets (same serde struct,
    /// same output), falling back to `serde_json` on wasm where sonic-rs isn't
    /// available.
    pub fn to_ndjson(&self) -> Result<String, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            sonic_rs::to_string(self).map_err(|e| e.to_string())
        }
        #[cfg(target_arch = "wasm32")]
        {
            serde_json::to_string(self).map_err(|e| e.to_string())
        }
    }
}

/// Configuration for decision logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogConfig {
    /// Whether decision logging is enabled
    pub enabled: bool,

    /// Maximum entries in the buffer before oldest are dropped
    pub buffer_capacity: usize,

    /// Path to NDJSON file for persistent logging (optional)
    pub file_path: Option<String>,

    /// Emit each decision as an NDJSON line to stdout (container-native
    /// collection: a log agent — Vector/Fluent Bit/OTel Collector — scrapes
    /// stdout and ships to the central store). Can be combined with `file_path`.
    #[serde(default)]
    pub emit_stdout: bool,

    /// Flush interval in milliseconds (for file logging)
    pub flush_interval_ms: u64,

    /// Whether to log allow decisions (can be disabled to reduce volume)
    pub log_allows: bool,

    /// Whether to log deny decisions
    pub log_denies: bool,

    /// Fraction of *allow* decisions to keep, in [0.0, 1.0] (default 1.0 = all).
    /// Denies are never sampled — they're the security-relevant events. This is
    /// the cheapest volume-control knob: sampled-out allows are dropped before
    /// the log entry is even built. e.g. 0.01 keeps 1% of allows + 100% of denies.
    pub sample_allow_rate: f64,

    /// Whether to include context in logs (can be disabled for privacy)
    pub include_context: bool,

    /// "Explain" tier: snapshot the resolved principal/resource entity attributes
    /// the decision branched on into `input_data`, so a decision is reproducible.
    /// Off by default — it's heavier (DataStore lookups + JSON on the log path,
    /// never on the eval path). Combine with `input_data_denies_only` to pay it
    /// only where it matters most.
    #[serde(default)]
    pub include_input_data: bool,

    /// When `include_input_data` is on, capture the snapshot for denies only
    /// (default true) — denials are what you most need to explain, and this keeps
    /// the cost off the allow firehose.
    #[serde(default = "default_true")]
    pub input_data_denies_only: bool,

    /// Replayable-capture tier (Plan 04 step 7): snapshot the FULL resolved
    /// request into `replay_input` so decisions can be re-evaluated under a
    /// different policy/data version (counterfactual replay). Off by default —
    /// it stores the whole request context per captured decision. Protection
    /// (mask/hash/encrypt) applies to it exactly like everything else.
    #[serde(default)]
    pub include_replay_input: bool,

    /// When the replay tier is on, capture denies only (default FALSE, unlike
    /// the explain tier: replay's whole point is finding flips in BOTH
    /// directions — denies-only could never surface an allow→deny flip).
    #[serde(default)]
    pub replay_input_denies_only: bool,

    /// Number of ring shards. Each request thread maps to a stable shard, so
    /// concurrent producers take disjoint, uncontended locks. 0 = auto (detected
    /// parallelism, clamped 1..=64). Set to 1 to force a single shard
    /// (deterministic ordering, tests).
    #[serde(default)]
    pub capture_shards: usize,

    // ---- Data protection (masking / pseudonymization / encryption) ----
    // Applied once at capture, so the query API, file/stdout sinks, and exports
    // all see only protected data.
    /// Pseudonymize `principal` with HMAC-SHA-256 (requires `hash_salt`): the
    /// logged value becomes `sha256:<hex>` — stable across entries (joinable
    /// for investigations) but not reversible and not dictionary-attackable
    /// without the salt.
    #[serde(default)]
    pub hash_principal: bool,

    /// Secret HMAC key for `hash_principal`. Never serialized (won't appear in
    /// the `/decisions/stats` config echo or any export).
    #[serde(skip_serializing, default)]
    pub hash_salt: Option<String>,

    /// If set, only these request-context keys are kept; all others are dropped
    /// at capture. `None` keeps everything (subject to `mask_keys`).
    #[serde(default)]
    pub context_allowlist: Option<Vec<String>>,

    /// Keys to mask (value replaced with `"***"`) in the request context AND in
    /// the explain-tier `input_data` attribute maps. Case-insensitive.
    #[serde(default)]
    pub mask_keys: Vec<String>,

    /// Encrypt the explain-tier `input_data` snapshot at rest with AES-256-GCM
    /// (requires `encryption_key`). The logged value becomes an envelope
    /// `{"enc":"aes256gcm","nonce":...,"ciphertext":...}` that only the key
    /// holder (e.g. the control plane, per tenant) can open. Fail-closed:
    /// enabling this without a valid key makes buffer creation error —
    /// plaintext is never logged by mistake.
    #[serde(default)]
    pub encrypt_input_data: bool,

    /// 32-byte hex AES-256-GCM key for `encrypt_input_data`. Never serialized.
    #[serde(skip_serializing, default)]
    pub encryption_key: Option<String>,

    // ---- Signed checkpoints (Plan 04, step 3) ----
    /// Emit a signed checkpoint every N durable records (0 = disabled by count).
    /// Combine with `checkpoint_interval_secs` — whichever threshold trips first
    /// closes the window.
    #[serde(default)]
    pub checkpoint_every: usize,

    /// Emit a checkpoint at least every T seconds when records are pending
    /// (0 = disabled by time). Bounds how long an unattested tail can sit
    /// unproven on a low-traffic agent.
    #[serde(default)]
    pub checkpoint_interval_secs: u64,

    /// Hex private signing key for checkpoints (Ed25519 seed / P-256 scalar).
    /// Never serialized. Absent while checkpointing is on ⇒ unsigned checkpoints
    /// with a loud startup warning. Invalid key ⇒ fail closed at buffer creation.
    #[serde(skip_serializing, default)]
    pub checkpoint_signing_key: Option<String>,

    /// Key id stamped into checkpoints (for rotation / verifier pinning).
    #[serde(default)]
    pub checkpoint_key_id: Option<String>,

    /// Signature algorithm for checkpoints: `ed25519-sha256` (default) or
    /// `ecdsa-p256-sha256`.
    #[serde(default = "default_checkpoint_algorithm")]
    pub checkpoint_algorithm: String,

    // ---- Mandatory-audit (fail-closed) mode (Plan 04, step 4) ----
    /// When true, the audit trail is a hard requirement: sampling is forbidden,
    /// every decision must be logged, a durable sink and signed checkpoints are
    /// required, and a durable-sink loss at runtime is NOT silently dropped —
    /// the agent fails closed per `on_audit_unavailable`. Validated at startup
    /// (`validate()`), which rejects any conflicting config.
    #[serde(default)]
    pub audit_required: bool,

    /// What to do in mandatory mode when the durable sink cannot accept a record
    /// (writer queue saturated / file unwritable): fail eval closed (default) or
    /// block the log call for backpressure. Ignored unless `audit_required`.
    #[serde(default)]
    pub on_audit_unavailable: OnAuditUnavailable,
}

/// Fail-closed behavior for mandatory audit mode when the durable sink cannot
/// accept a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnAuditUnavailable {
    /// Latch the agent as audit-compromised: readiness flips to not-ready and
    /// evaluation fails closed (`503`), so no decision is served un-audited.
    /// Keeps the eval path non-blocking. This is the default.
    #[default]
    FailClosed,
    /// Block the (writer-thread-bound) log hand-off until the sink drains, so a
    /// record is never dropped. Trades tail latency under sink pressure for
    /// zero loss; never returns a wrong decision, only a slower one.
    Block,
}

impl OnAuditUnavailable {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().trim() {
            "fail_closed" | "fail-closed" | "unhealthy" | "503" => Some(Self::FailClosed),
            "block" | "backpressure" => Some(Self::Block),
            _ => None,
        }
    }
}

fn default_checkpoint_algorithm() -> String {
    reaper_core::bundle_signing::ALGORITHM.to_string()
}

fn default_true() -> bool {
    true
}

impl Default for DecisionLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_capacity: 10_000,
            file_path: None,
            emit_stdout: false,
            flush_interval_ms: 5_000,
            log_allows: true,
            log_denies: true,
            sample_allow_rate: 1.0,
            include_context: true,
            include_input_data: false,
            input_data_denies_only: true,
            include_replay_input: false,
            replay_input_denies_only: false,
            capture_shards: 0,
            hash_principal: false,
            hash_salt: None,
            context_allowlist: None,
            mask_keys: Vec::new(),
            encrypt_input_data: false,
            encryption_key: None,
            checkpoint_every: 0,
            checkpoint_interval_secs: 0,
            checkpoint_signing_key: None,
            checkpoint_key_id: None,
            checkpoint_algorithm: default_checkpoint_algorithm(),
            audit_required: false,
            on_audit_unavailable: OnAuditUnavailable::default(),
        }
    }
}

impl DecisionLogConfig {
    /// Create from environment variables
    pub fn from_env() -> Self {
        let mut config = Self {
            enabled: std::env::var("REAPER_DECISION_LOG_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            buffer_capacity: std::env::var("REAPER_DECISION_LOG_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000),
            file_path: std::env::var("REAPER_DECISION_LOG_FILE").ok(),
            emit_stdout: std::env::var("REAPER_DECISION_LOG_STDOUT")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            flush_interval_ms: std::env::var("REAPER_DECISION_LOG_FLUSH_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5_000),
            log_allows: std::env::var("REAPER_DECISION_LOG_ALLOWS")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            log_denies: std::env::var("REAPER_DECISION_LOG_DENIES")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            sample_allow_rate: std::env::var("REAPER_DECISION_LOG_SAMPLE_ALLOW_RATE")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .map(|r| r.clamp(0.0, 1.0))
                .unwrap_or(1.0),
            include_context: std::env::var("REAPER_DECISION_LOG_CONTEXT")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            include_input_data: std::env::var("REAPER_DECISION_LOG_INPUT_DATA")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            input_data_denies_only: std::env::var("REAPER_DECISION_LOG_INPUT_DATA_DENIES_ONLY")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
            include_replay_input: std::env::var("REAPER_DECISION_LOG_REPLAY_INPUT")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            replay_input_denies_only: std::env::var("REAPER_DECISION_LOG_REPLAY_INPUT_DENIES_ONLY")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            capture_shards: std::env::var("REAPER_DECISION_LOG_SHARDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            hash_principal: std::env::var("REAPER_DECISION_LOG_HASH_PRINCIPAL")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            hash_salt: std::env::var("REAPER_DECISION_LOG_HASH_SALT").ok(),
            context_allowlist: std::env::var("REAPER_DECISION_LOG_CONTEXT_ALLOWLIST")
                .ok()
                .map(|v| csv_list(&v)),
            mask_keys: std::env::var("REAPER_DECISION_LOG_MASK_KEYS")
                .ok()
                .map(|v| csv_list(&v))
                .unwrap_or_default(),
            encrypt_input_data: std::env::var("REAPER_DECISION_LOG_ENCRYPT_INPUT_DATA")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            encryption_key: std::env::var("REAPER_DECISION_LOG_ENCRYPTION_KEY").ok(),
            checkpoint_every: std::env::var("REAPER_DECISION_LOG_CHECKPOINT_EVERY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            checkpoint_interval_secs: std::env::var("REAPER_DECISION_LOG_CHECKPOINT_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            checkpoint_signing_key: std::env::var("REAPER_DECISION_LOG_CHECKPOINT_SIGNING_KEY")
                .ok(),
            checkpoint_key_id: std::env::var("REAPER_DECISION_LOG_CHECKPOINT_KEY_ID").ok(),
            checkpoint_algorithm: std::env::var("REAPER_DECISION_LOG_CHECKPOINT_ALGORITHM")
                .unwrap_or_else(|_| default_checkpoint_algorithm()),
            audit_required: false,
            on_audit_unavailable: std::env::var("REAPER_DECISION_LOG_ON_AUDIT_UNAVAILABLE")
                .ok()
                .and_then(|v| OnAuditUnavailable::parse(&v))
                .unwrap_or_default(),
        };

        // REAPER_DECISION_LOG_MODE: a one-word intent knob applied on top of
        // the fine-grained vars (mode wins — it's the explicit statement of
        // what must reach the store):
        //   full    -> EVERY decision ships: allows + denies, sampling forced
        //              off. The complete-audit mode for compliance/central
        //              ClickHouse capture.
        //   sampled -> denies always; allows kept at SAMPLE_ALLOW_RATE.
        //   denies  -> denies only (minimal volume).
        if let Ok(mode) = std::env::var("REAPER_DECISION_LOG_MODE") {
            config.apply_mode(&mode);
        }
        config
    }

    /// Apply a named capture mode preset (see `from_env`). Unknown modes are
    /// ignored (fine-grained settings stay as-is) with a warning.
    pub fn apply_mode(&mut self, mode: &str) {
        match mode.to_lowercase().trim() {
            "full" | "all" => {
                self.log_allows = true;
                self.log_denies = true;
                self.sample_allow_rate = 1.0;
            }
            "sampled" => {
                self.log_allows = true;
                self.log_denies = true;
                // sample_allow_rate stays as configured
            }
            "denies" | "denies-only" | "deny" => {
                self.log_allows = false;
                self.log_denies = true;
            }
            "mandatory" => {
                // Mandatory audit implies enabled + audit_required. It does NOT
                // silently relax sampling/log flags: `validate()` rejects a
                // conflicting explicit setting rather than masking it, so an
                // operator's wrong mental model surfaces as a startup error.
                self.enabled = true;
                self.audit_required = true;
            }
            "" => {}
            other => {
                tracing::warn!(
                    mode = other,
                    "unknown REAPER_DECISION_LOG_MODE (use full|sampled|denies|mandatory); ignoring"
                );
            }
        }
    }

    /// Validate the config (fail closed). Currently enforces the mandatory-audit
    /// invariants: no sampling, complete capture, a durable sink, and signed
    /// checkpoints. Returns a human-readable reason on the first violation.
    pub fn validate(&self) -> Result<(), String> {
        if self.audit_required {
            if !self.enabled {
                return Err("mandatory audit mode requires decision logging enabled".to_string());
            }
            if self.sample_allow_rate < 1.0 {
                return Err(format!(
                    "mandatory audit mode forbids allow sampling \
                     (sample_allow_rate = {}, must be 1.0)",
                    self.sample_allow_rate
                ));
            }
            if !self.log_allows || !self.log_denies {
                return Err(
                    "mandatory audit mode must log every decision (log_allows and log_denies \
                     must both be true)"
                        .to_string(),
                );
            }
            if self.file_path.is_none() && !self.emit_stdout {
                return Err("mandatory audit mode requires a durable sink (set \
                     REAPER_DECISION_LOG_FILE or REAPER_DECISION_LOG_STDOUT)"
                    .to_string());
            }
            if self.checkpoint_every == 0 && self.checkpoint_interval_secs == 0 {
                return Err("mandatory audit mode requires signed checkpoints (set \
                     REAPER_DECISION_LOG_CHECKPOINT_EVERY or _INTERVAL_SECS)"
                    .to_string());
            }
            match self.checkpoint_signing_key.as_deref() {
                Some(k) if !k.trim().is_empty() => {}
                _ => {
                    return Err("mandatory audit mode requires a checkpoint signing key \
                         (REAPER_DECISION_LOG_CHECKPOINT_SIGNING_KEY) so checkpoints are signed"
                        .to_string())
                }
            }
        }
        Ok(())
    }
}

/// Split a comma-separated env value into trimmed, non-empty items.
fn csv_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chained run of records exactly as the writer thread would.
    fn chained_run(n: u64) -> Vec<DecisionLogEntry> {
        let mut last = String::new();
        (0..n)
            .map(|i| {
                let mut e = DecisionLogEntry::new(
                    format!("user_{i}"),
                    "read".to_string(),
                    format!("/api/{i}"),
                    if i % 2 == 0 { "allow" } else { "deny" }.to_string(),
                    "policy_1".to_string(),
                    "p".to_string(),
                );
                e.seq = i;
                e.context.insert("z".into(), serde_json::json!(i));
                e.context.insert("a".into(), serde_json::json!("x"));
                e.prev_hash = last.clone();
                let h = e.compute_entry_hash(&last);
                e.entry_hash = h.clone();
                last = h;
                e
            })
            .collect()
    }

    #[test]
    fn test_hash_chain_verifies_and_detects_tampering() {
        let entries = chained_run(12);
        // Intact chain verifies. (canonical_bytes is deterministic despite the
        // HashMap context, or this would be flaky.)
        assert!(verify_chain(&entries).is_ok());

        // Mutation: flip a field in the middle → entry_hash mismatch at that seq.
        let mut mutated = entries.clone();
        mutated[5].decision = "allow".to_string();
        assert_eq!(verify_chain(&mutated).unwrap_err().seq, 5);

        // Mutation of the context (nested JSON) is also caught.
        let mut ctx = entries.clone();
        ctx[7].context.insert("a".into(), serde_json::json!("y"));
        assert_eq!(verify_chain(&ctx).unwrap_err().seq, 7);

        // Deletion: drop seq 5 → seq 6's prev_hash no longer links.
        let mut deleted = entries.clone();
        deleted.remove(5);
        assert_eq!(verify_chain(&deleted).unwrap_err().seq, 6);

        // Reordering: swap two records → prev_hash mismatch.
        let mut reordered = entries.clone();
        reordered.swap(5, 6);
        assert!(verify_chain(&reordered).is_err());

        // Insertion of a forged record breaks the following link.
        let mut inserted = entries.clone();
        let forged = inserted[3].clone();
        inserted.insert(4, forged);
        assert!(verify_chain(&inserted).is_err());
    }

    #[test]
    fn test_canonical_bytes_stable_across_context_order() {
        // Same logical record, context inserted in different orders → identical
        // canonical bytes and hash.
        let mut a = DecisionLogEntry::new(
            "u".into(),
            "read".into(),
            "/r".into(),
            "allow".into(),
            "p".into(),
            "n".into(),
        );
        let mut b = a.clone();
        b.decision_id = a.decision_id.clone();
        b.timestamp = a.timestamp.clone();
        for (k, v) in [("m", 1), ("a", 2), ("z", 3)] {
            a.context.insert(k.into(), serde_json::json!(v));
        }
        for (k, v) in [("z", 3), ("m", 1), ("a", 2)] {
            b.context.insert(k.into(), serde_json::json!(v));
        }
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
        assert_eq!(a.compute_entry_hash(""), b.compute_entry_hash(""));
    }

    #[test]
    fn test_decision_log_entry_creation() {
        let entry = DecisionLogEntry::new(
            "user_123".to_string(),
            "read".to_string(),
            "/api/data".to_string(),
            "allow".to_string(),
            "policy_456".to_string(),
            "data-access-policy".to_string(),
        );

        assert_eq!(entry.principal, "user_123");
        assert_eq!(entry.action, "read");
        assert_eq!(entry.resource, "/api/data");
        assert_eq!(entry.decision, "allow");
        assert!(!entry.decision_id.is_empty());
    }

    #[test]
    fn test_decision_log_entry_builder() {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "write".to_string(),
            "resource".to_string(),
            "deny".to_string(),
            "policy".to_string(),
            "policy-name".to_string(),
        )
        .with_evaluation_time_ns(500)
        .with_cache_hit(true)
        .with_agent_id("agent-1".to_string())
        .with_matched_rule("deny_rule".to_string());

        assert_eq!(entry.evaluation_time_ns, 500);
        assert!(entry.cache_hit);
        assert_eq!(entry.agent_id, Some("agent-1".to_string()));
        assert_eq!(entry.matched_rule, Some("deny_rule".to_string()));
    }

    #[test]
    fn test_decision_log_ndjson() {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "resource".to_string(),
            "allow".to_string(),
            "policy".to_string(),
            "test-policy".to_string(),
        );

        let json = entry.to_ndjson().unwrap();
        assert!(json.contains("\"principal\":\"user\""));
        assert!(json.contains("\"decision\":\"allow\""));
    }

    #[test]
    fn test_config_default() {
        let config = DecisionLogConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.buffer_capacity, 10_000);
        assert!(config.log_allows);
        assert!(config.log_denies);
    }

    #[test]
    fn test_mode_full_forces_complete_capture() {
        // Even with sampling configured down and allows off, mode=full wins:
        // every decision (allows included) must reach the store.
        let mut config = DecisionLogConfig {
            log_allows: false,
            sample_allow_rate: 0.01,
            ..Default::default()
        };
        config.apply_mode("full");
        assert!(config.log_allows);
        assert!(config.log_denies);
        assert_eq!(config.sample_allow_rate, 1.0);

        // "all" is an accepted alias.
        let mut config = DecisionLogConfig {
            sample_allow_rate: 0.5,
            ..Default::default()
        };
        config.apply_mode("ALL");
        assert_eq!(config.sample_allow_rate, 1.0);
    }

    #[test]
    fn test_mode_sampled_keeps_configured_rate() {
        let mut config = DecisionLogConfig {
            log_allows: false,
            sample_allow_rate: 0.25,
            ..Default::default()
        };
        config.apply_mode("sampled");
        assert!(config.log_allows, "sampled mode re-enables allows");
        assert_eq!(config.sample_allow_rate, 0.25, "rate untouched");
    }

    #[test]
    fn test_mode_denies_only() {
        let mut config = DecisionLogConfig::default();
        config.apply_mode("denies");
        assert!(!config.log_allows);
        assert!(config.log_denies);
    }

    // ---- Mandatory-audit mode (Plan 04 step 4) ----

    fn valid_mandatory() -> DecisionLogConfig {
        DecisionLogConfig {
            enabled: true,
            audit_required: true,
            emit_stdout: true,
            checkpoint_every: 100,
            checkpoint_signing_key: Some("07".repeat(32)),
            ..Default::default()
        }
    }

    #[test]
    fn test_mode_mandatory_sets_audit_required() {
        let mut config = DecisionLogConfig::default();
        config.apply_mode("mandatory");
        assert!(config.enabled);
        assert!(config.audit_required);
    }

    #[test]
    fn test_mandatory_validate_ok() {
        valid_mandatory().validate().unwrap();
    }

    #[test]
    fn test_mandatory_validate_rejects_sampling() {
        let mut c = valid_mandatory();
        c.sample_allow_rate = 0.5;
        assert!(c.validate().unwrap_err().contains("sampling"));
    }

    #[test]
    fn test_mandatory_validate_requires_complete_capture() {
        let mut c = valid_mandatory();
        c.log_allows = false;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_mandatory_validate_requires_sink_and_signed_checkpoints() {
        // No durable sink.
        let mut c = valid_mandatory();
        c.emit_stdout = false;
        c.file_path = None;
        assert!(c.validate().unwrap_err().contains("durable sink"));

        // No checkpoint trigger.
        let mut c = valid_mandatory();
        c.checkpoint_every = 0;
        c.checkpoint_interval_secs = 0;
        assert!(c.validate().unwrap_err().contains("checkpoints"));

        // No signing key.
        let mut c = valid_mandatory();
        c.checkpoint_signing_key = None;
        assert!(c.validate().unwrap_err().contains("signing key"));
    }

    #[test]
    fn test_non_mandatory_validate_is_lenient() {
        // Sampling is fine when audit isn't mandatory.
        let c = DecisionLogConfig {
            enabled: true,
            sample_allow_rate: 0.1,
            ..Default::default()
        };
        c.validate().unwrap();
    }

    #[test]
    fn test_on_audit_unavailable_parse() {
        assert_eq!(
            OnAuditUnavailable::parse("block"),
            Some(OnAuditUnavailable::Block)
        );
        assert_eq!(
            OnAuditUnavailable::parse("fail_closed"),
            Some(OnAuditUnavailable::FailClosed)
        );
        assert_eq!(OnAuditUnavailable::parse("nonsense"), None);
    }

    // ---- Signed checkpoints (Plan 04 step 3) ----

    fn test_signing_key() -> reaper_core::bundle_signing::SigningKey {
        use reaper_core::bundle_signing::{SigAlgorithm, SigningKey};
        SigningKey::from_hex(SigAlgorithm::Ed25519Sha256, &"07".repeat(32)).unwrap()
    }

    fn verifying_of(
        key: &reaper_core::bundle_signing::SigningKey,
    ) -> reaper_core::bundle_signing::VerifyingKey {
        reaper_core::bundle_signing::VerifyingKey::from_hex(key.algorithm(), &key.public_key_hex())
            .unwrap()
    }

    /// A signed checkpoint over the whole run.
    fn checkpoint_over(
        entries: &[DecisionLogEntry],
        key: &reaper_core::bundle_signing::SigningKey,
        key_id: &str,
    ) -> Checkpoint {
        let mut cp = Checkpoint::new(
            "chain-boot-1".to_string(),
            entries.first().unwrap().seq,
            entries.last().unwrap().seq,
            entries.len() as u64,
            entries.first().unwrap().prev_hash.clone(),
            entries.last().unwrap().entry_hash.clone(),
            0,
            1_000,
            "2026-01-01T00:00:00+00:00".to_string(),
        );
        cp.sign(key, key_id);
        cp
    }

    #[test]
    fn test_checkpoint_sign_verify_roundtrip() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(10);
        let cp = checkpoint_over(&entries, &key, "k1");

        // Signature alone verifies (key_id pinned).
        cp.verify_signature(&vk, Some("k1")).unwrap();
        // Full verification against the covered records passes.
        verify_checkpoint(&cp, &entries, &vk, Some("k1")).unwrap();
        assert_eq!(cp.record_type, CHECKPOINT_RECORD_TYPE);
        assert_eq!(cp.algorithm, reaper_core::bundle_signing::ALG_ED25519);
    }

    #[test]
    fn test_checkpoint_unsigned_is_rejected() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(3);
        // Never signed.
        let cp = Checkpoint::new(
            "c".to_string(),
            0,
            2,
            3,
            String::new(),
            entries.last().unwrap().entry_hash.clone(),
            0,
            1,
            "t".to_string(),
        );
        assert!(cp.verify_signature(&vk, None).is_err());
        assert!(verify_checkpoint(&cp, &entries, &vk, None).is_err());
    }

    #[test]
    fn test_checkpoint_key_id_pinning_enforced() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(4);
        let cp = checkpoint_over(&entries, &key, "prod-2026");
        // Pinning a different key id fails.
        assert!(cp.verify_signature(&vk, Some("prod-2025")).is_err());
        assert!(verify_checkpoint(&cp, &entries, &vk, Some("prod-2025")).is_err());
        // The right pin passes.
        verify_checkpoint(&cp, &entries, &vk, Some("prod-2026")).unwrap();
    }

    #[test]
    fn test_checkpoint_forged_signature_fails() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(6);
        let mut cp = checkpoint_over(&entries, &key, "k1");
        // Flip a metadata field the signature covers, without re-signing.
        cp.seq_end += 1;
        assert!(verify_checkpoint(&cp, &entries, &vk, Some("k1")).is_err());

        // Corrupt the signature bytes directly.
        let mut cp = checkpoint_over(&entries, &key, "k1");
        cp.signature = "00".repeat(64);
        assert!(cp.verify_signature(&vk, Some("k1")).is_err());
    }

    #[test]
    fn test_verify_checkpoint_detects_mutation_naming_seq() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(8);
        let cp = checkpoint_over(&entries, &key, "k1");

        // Mutate a covered record without re-chaining → chain break at that seq.
        let mut tampered = entries.clone();
        tampered[3].resource = "/api/hacked".to_string();
        let err = verify_checkpoint(&cp, &tampered, &vk, Some("k1")).unwrap_err();
        assert_eq!(err.seq, Some(3));
    }

    #[test]
    fn test_verify_checkpoint_detects_dropped_entry() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(8);
        let cp = checkpoint_over(&entries, &key, "k1");

        // Delete a record from the middle → prev_hash no longer links.
        let mut dropped = entries.clone();
        dropped.remove(4);
        let err = verify_checkpoint(&cp, &dropped, &vk, Some("k1")).unwrap_err();
        // The break is named at the record whose prev_hash no longer matches.
        assert_eq!(err.seq, Some(5));
    }

    #[test]
    fn test_verify_checkpoint_detects_hidden_tail_drop() {
        let key = test_signing_key();
        let vk = verifying_of(&key);
        let entries = chained_run(8);
        // Checkpoint claims 8 records, but only the first 7 are presented — an
        // intact prefix. Count + last_entry_hash must catch the missing tail.
        let cp = checkpoint_over(&entries, &key, "k1");
        let short = &entries[..7];
        let err = verify_checkpoint(&cp, short, &vk, Some("k1")).unwrap_err();
        assert!(err.reason.contains("count mismatch"), "{}", err.reason);
    }

    #[test]
    fn test_mode_unknown_is_ignored() {
        let mut config = DecisionLogConfig {
            sample_allow_rate: 0.5,
            ..Default::default()
        };
        config.apply_mode("bogus");
        assert_eq!(config.sample_allow_rate, 0.5);
        assert!(config.log_allows);
    }
}
