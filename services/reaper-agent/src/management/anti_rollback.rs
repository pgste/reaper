//! Persisted anti-rollback floor for signed bundles (Plan 02, Phase B, step 3).
//!
//! The v2 signature envelope carries a monotonic `version` per bundle lineage
//! (`bundle_id`). This store remembers the highest version this agent has
//! applied for each lineage and **rejects a strictly older one**, so a
//! compromised store/CDN/proxy cannot replay a genuinely-signed but superseded
//! bundle. The floor is persisted to disk, so a downgrade is still refused
//! after a process restart.
//!
//! Semantics:
//! - `incoming < floor`  → reject (rollback).
//! - `incoming == floor` → allow (idempotent re-apply / at-least-once
//!   redelivery must not break).
//! - `incoming > floor`  → allow and raise the floor.
//!
//! `force` bypasses the rejection (emergency downgrade) but never lowers the
//! floor: the floor is `max(floor, applied)`, so the documented way back is to
//! re-sign the older content as a *new higher* version.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use tracing::{error, warn};

/// A rollback rejection: the incoming version is below the persisted floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackRejected {
    pub bundle_id: String,
    pub incoming: u64,
    pub floor: u64,
}

impl std::fmt::Display for RollbackRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "anti-rollback: bundle {} version {} is older than the highest applied version {} \
             (re-sign the content as a newer version, or force to override)",
            self.bundle_id, self.incoming, self.floor
        )
    }
}

/// Lineage → highest-applied version, cached in memory and persisted to disk.
pub struct AntiRollbackStore {
    /// `None` = non-persistent (in-memory only; used by standalone agents and
    /// tests). Disk-backed stores write atomically on every raise.
    path: Option<PathBuf>,
    floors: Mutex<HashMap<String, u64>>,
}

impl AntiRollbackStore {
    /// In-memory only — no persistence across restart.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            floors: Mutex::new(HashMap::new()),
        }
    }

    /// Disk-backed store at `path`, loading any existing floors. A corrupt or
    /// unreadable file is logged and treated as empty (fail *safe*: an empty
    /// floor never wrongly rejects a current bundle; the first apply re-seeds
    /// it — and a downgrade attempted in that window is still caught by the
    /// signature's validity window and, once revocation lands, the CRL).
    pub fn persistent(path: PathBuf) -> Self {
        let floors = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<HashMap<String, u64>>(&bytes) {
                Ok(map) => map,
                Err(e) => {
                    error!(path = %path.display(), error = %e,
                        "anti-rollback floor file is corrupt; starting from empty");
                    HashMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                error!(path = %path.display(), error = %e,
                    "cannot read anti-rollback floor file; starting from empty");
                HashMap::new()
            }
        };
        Self {
            path: Some(path),
            floors: Mutex::new(floors),
        }
    }

    /// Current floor for a lineage (0 if never applied). Exposed for
    /// diagnostics/tests; `admit` is the enforcement entry point.
    #[allow(dead_code)]
    pub fn floor(&self, bundle_id: &str) -> u64 {
        self.floors.lock().get(bundle_id).copied().unwrap_or(0)
    }

    /// Check the incoming version against the floor and, if it may apply, raise
    /// the floor and persist. `force` skips the rejection but still raises.
    ///
    /// A `bundle_id` of `""` (legacy v1 envelope, version 0) is not tracked —
    /// anti-rollback requires the v2 lineage id; such bundles pass through
    /// (they are already rejected upstream when `require_envelope_v2`).
    pub fn admit(
        &self,
        bundle_id: &str,
        incoming: u64,
        force: bool,
    ) -> Result<(), RollbackRejected> {
        if bundle_id.is_empty() {
            return Ok(());
        }
        let mut floors = self.floors.lock();
        let floor = floors.get(bundle_id).copied().unwrap_or(0);
        if incoming < floor && !force {
            return Err(RollbackRejected {
                bundle_id: bundle_id.to_string(),
                incoming,
                floor,
            });
        }
        let new_floor = floor.max(incoming);
        if new_floor != floor {
            floors.insert(bundle_id.to_string(), new_floor);
            let snapshot = floors.clone();
            drop(floors);
            self.persist(&snapshot);
        }
        Ok(())
    }

    /// Atomic write: temp file in the same directory, then rename over the
    /// target so a crash mid-write never leaves a truncated floor file.
    fn persist(&self, floors: &HashMap<String, u64>) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(json) = serde_json::to_vec(floors) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, path)) {
            warn!(path = %path.display(), error = %e,
                "failed to persist anti-rollback floor (in-memory floor still enforced)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_strictly_older_allows_equal_and_newer() {
        let s = AntiRollbackStore::in_memory();
        assert!(s.admit("b", 5, false).is_ok());
        // idempotent re-apply of the current version is fine
        assert!(s.admit("b", 5, false).is_ok());
        // newer raises the floor
        assert!(s.admit("b", 7, false).is_ok());
        assert_eq!(s.floor("b"), 7);
        // strictly older is rejected
        let err = s.admit("b", 6, false).unwrap_err();
        assert_eq!(err.floor, 7);
        assert_eq!(err.incoming, 6);
        // a different lineage is independent
        assert!(s.admit("other", 1, false).is_ok());
    }

    #[test]
    fn force_overrides_but_keeps_the_floor() {
        let s = AntiRollbackStore::in_memory();
        assert!(s.admit("b", 10, false).is_ok());
        // force applies an older version...
        assert!(s.admit("b", 3, true).is_ok());
        // ...but the floor stays at 10, so a normal older push is still refused
        assert!(s.admit("b", 5, false).is_err());
        assert_eq!(s.floor("b"), 10);
    }

    #[test]
    fn empty_bundle_id_is_not_tracked() {
        let s = AntiRollbackStore::in_memory();
        assert!(s.admit("", 0, false).is_ok());
        assert!(s.admit("", 0, false).is_ok());
    }

    #[test]
    fn floor_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("anti_rollback.json");
        {
            let s = AntiRollbackStore::persistent(path.clone());
            assert!(s.admit("b", 42, false).is_ok());
        }
        // Reopen: the floor survived the "restart".
        let s2 = AntiRollbackStore::persistent(path);
        assert_eq!(s2.floor("b"), 42);
        assert!(s2.admit("b", 41, false).is_err(), "downgrade after restart");
        assert!(s2.admit("b", 42, false).is_ok(), "idempotent after restart");
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("anti_rollback.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let s = AntiRollbackStore::persistent(path);
        assert_eq!(s.floor("b"), 0);
        assert!(s.admit("b", 1, false).is_ok());
    }
}
