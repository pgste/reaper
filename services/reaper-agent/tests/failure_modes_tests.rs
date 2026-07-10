//! Failure-mode matrix — agent tier (Plan 05, Step 5).
//!
//! The data-plane staleness/require-sync gate. The audit-availability rows are
//! covered by `policy-engine`'s `decision_buffer` tests
//! (`test_mandatory_durable_loss_latches_fail_closed` and
//! `test_non_mandatory_durable_loss_counts_but_stays_healthy`), and the engine
//! deny-on-error / no-policy rows by `policy-engine`'s `failure_modes_tests` —
//! all cross-referenced from `docs/deployment/OPERATIONS_GUIDE.md`.

use std::sync::atomic::{AtomicI64, AtomicU64};

use parking_lot::RwLock;
use reaper_agent::state::{DataSyncState, StalenessMode};

fn sync_state(mode: StalenessMode, require_sync: bool, max_staleness_secs: u64) -> DataSyncState {
    DataSyncState {
        version: AtomicI64::new(0),
        checksum: RwLock::new(String::new()),
        last_synced_epoch: AtomicU64::new(0),
        applied_seq: AtomicI64::new(0),
        max_staleness_secs,
        mode,
        require_sync,
    }
}

/// Row: the data plane is armed but the first verified snapshot has not landed
/// ⇒ deny (fail closed). An empty replica must not answer as if it had data.
#[test]
fn data_gate_denies_before_first_sync() {
    let state = sync_state(StalenessMode::Monitor, /* require_sync */ true, 0);
    assert_eq!(state.deny_reason(), Some("awaiting_initial_data_sync"));
}

/// Row: once a verified snapshot has landed, the gate opens (serves normally).
#[test]
fn data_gate_opens_after_first_sync() {
    let state = sync_state(StalenessMode::Monitor, true, 0);
    state.record_sync(1, "sha256:abc".to_string());
    assert_eq!(state.deny_reason(), None, "a synced replica serves");
}

/// Row: in Enforce mode, exceeding the staleness budget ⇒ deny (fail closed);
/// stale data must not mint fresh allows.
#[test]
fn data_gate_denies_when_stale_in_enforce_mode() {
    let state = sync_state(StalenessMode::Enforce, false, /* max_staleness */ 1);
    // Record a sync far in the past so the budget is exceeded now.
    state.record_sync(1, "sha256:abc".to_string());
    state
        .last_synced_epoch
        .store(1, std::sync::atomic::Ordering::Release); // 1970 → very stale
    assert_eq!(state.deny_reason(), Some("data_staleness_exceeded"));
}

/// Control case: no gate armed ⇒ never denies on the data-plane account.
#[test]
fn data_gate_open_when_unarmed() {
    let state = sync_state(StalenessMode::Monitor, false, 0);
    assert_eq!(state.deny_reason(), None);
}
