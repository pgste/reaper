//! Audit-completeness invariants (round-3 Plan 05 §4.4, Testing T7).
//!
//! The audit hash-chain tamper-evidence is unit-tested in `decision_log.rs`.
//! This suite pins the *accounting* invariant compliance actually rests on:
//! **a served decision is never silently lost.** Every decision handed to the
//! buffer is either retained in the query ring OR counted — as an evicted
//! `dropped_entries`, a `sampled_out` allow, or (durable sink) a
//! `writer_dropped` — but `total_entries` always equals the number served, and
//! the sum of retained + counted-loss reconciles. A regression that dropped a
//! record without counting it (the "served but not recorded" bug) breaks a
//! reconciliation assert here.
//!
//! These drive the real `DecisionBuffer` (the mechanism the agent handler calls
//! via `buffer.log()` under `buffer.should_log()`), so they exercise the exact
//! accounting the handler relies on, deterministically (single shard, no writer
//! thread), without standing up an agent process.

use policy_engine::decision_log::DecisionLogConfig;
use policy_engine::{create_shared_buffer, DecisionLogEntry, PrivacyProfile, SharedDecisionBuffer};

fn entry(i: u64, allow: bool) -> DecisionLogEntry {
    DecisionLogEntry::new(
        format!("user_{i}"),
        "read".to_string(),
        format!("/api/{i}"),
        if allow { "allow" } else { "deny" }.to_string(),
        "policy_1".to_string(),
        "p".to_string(),
    )
}

/// A deterministic in-memory buffer: single shard (stable ordering), no file
/// sink (no background writer thread), the given ring capacity.
fn buffer(capacity: usize) -> SharedDecisionBuffer {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: capacity,
        file_path: None,
        flush_interval_ms: 1000,
        capture_shards: 1,
        privacy_profile: Some(PrivacyProfile::Raw),
        ..Default::default()
    };
    create_shared_buffer(config).expect("buffer")
}

/// Log `entry` only if the buffer's sampling gate admits it, mirroring the agent
/// handler's `if buffer.should_log(is_allow) { buffer.log(entry) }`. Returns
/// whether it was admitted.
fn served(buffer: &SharedDecisionBuffer, i: u64, allow: bool) -> bool {
    if buffer.should_log(allow) {
        buffer.log(entry(i, allow));
        true
    } else {
        false
    }
}

#[test]
fn every_served_decision_is_recorded_when_ring_is_large_enough() {
    let n: u64 = 500;
    let buf = buffer(n as usize + 10); // capacity > n: nothing evicted
    let mut allows = 0u64;
    for i in 0..n {
        let allow = i % 3 == 0;
        assert!(served(&buf, i, allow), "default config logs every decision");
        allows += u64::from(allow);
    }

    let s = buf.stats();
    // Served == recorded: total_entries counts every push, the ring holds them
    // all, nothing dropped.
    assert_eq!(s.total_entries, n, "every served decision is counted");
    assert_eq!(s.buffer_size, n as usize, "ring holds all N (capacity > N)");
    assert_eq!(s.dropped_entries, 0, "nothing evicted when capacity > N");
    assert_eq!(
        s.allow_count + s.deny_count,
        n,
        "allow+deny counters reconcile to N"
    );
    assert_eq!(s.allow_count, allows, "allow counter matches served allows");
}

#[test]
fn saturating_the_ring_counts_drops_never_loses_silently() {
    // Capacity strictly less than the number served: the oldest are evicted, and
    // every eviction MUST be counted in dropped_entries — never a silent loss.
    let capacity = 64usize;
    let n: u64 = 1000;
    let buf = buffer(capacity);
    for i in 0..n {
        served(&buf, i, i % 2 == 0);
    }

    let s = buf.stats();
    assert_eq!(s.total_entries, n, "every served decision is still counted");
    assert!(
        s.buffer_size <= capacity,
        "ring never exceeds capacity ({} > {capacity})",
        s.buffer_size
    );
    // The reconciliation that proves no silent loss: retained + evicted == served.
    assert_eq!(
        s.buffer_size as u64 + s.dropped_entries,
        n,
        "retained ({}) + dropped ({}) must equal served ({n}) — no silent loss",
        s.buffer_size,
        s.dropped_entries
    );
    assert!(s.dropped_entries > 0, "saturation must have evicted some");
}

#[test]
fn sampling_allows_is_counted_not_a_silent_loss() {
    // sample_allow_rate = 0 drops every ALLOW at the gate (denies are never
    // sampled). A sampled-out allow is not "unrecorded and lost" — it is counted
    // in sampled_out, so served == recorded + sampled_out still reconciles.
    let n: u64 = 300;
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: n as usize + 10,
        file_path: None,
        flush_interval_ms: 1000,
        capture_shards: 1,
        privacy_profile: Some(PrivacyProfile::Raw),
        sample_allow_rate: 0.0, // drop all allows at the gate
        ..Default::default()
    };
    let buf = create_shared_buffer(config).expect("buffer");

    let mut denies = 0u64;
    let mut admitted_allows = 0u64;
    let mut gated_allows = 0u64;
    for i in 0..n {
        let allow = i % 2 == 0;
        let admitted = served(&buf, i, allow);
        if allow {
            if admitted {
                admitted_allows += 1;
            } else {
                gated_allows += 1;
            }
        } else {
            denies += 1;
            assert!(admitted, "denies are NEVER sampled out");
        }
    }

    let s = buf.stats();
    assert_eq!(admitted_allows, 0, "sample_allow_rate=0 admits no allows");
    assert!(gated_allows > 0, "some allows were gated");
    // Denies all recorded.
    assert_eq!(s.deny_count, denies, "every deny is recorded");
    assert_eq!(s.total_entries, denies, "only denies reached the ring");
    // The security-relevant reconciliation: every served decision is accounted —
    // recorded (total_entries) + gated-at-sampling (sampled_out) == served.
    assert_eq!(
        s.total_entries + s.sampled_out,
        n,
        "recorded ({}) + sampled_out ({}) must equal served ({n})",
        s.total_entries,
        s.sampled_out
    );
}

#[test]
fn disabled_capture_records_nothing_and_still_reconciles() {
    // Capture off (enabled=false): should_log admits nothing, so no decision is
    // recorded. The invariant still holds trivially — served decisions are all
    // "not captured", none silently lost into a half-recorded state.
    let config = DecisionLogConfig {
        enabled: false,
        capture_shards: 1,
        privacy_profile: Some(PrivacyProfile::Raw),
        ..Default::default()
    };
    let buf = create_shared_buffer(config).expect("buffer");

    let n: u64 = 100;
    let mut admitted = 0u64;
    for i in 0..n {
        if served(&buf, i, i % 2 == 0) {
            admitted += 1;
        }
    }
    assert_eq!(admitted, 0, "disabled capture admits nothing");

    let s = buf.stats();
    assert_eq!(s.total_entries, 0, "nothing recorded when capture is off");
    assert_eq!(s.dropped_entries, 0, "no drops (nothing was ever admitted)");
}
