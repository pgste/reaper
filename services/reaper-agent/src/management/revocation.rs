//! Agent-side revocation cache + enforcement (Plan 02, Phase B, step 4).
//!
//! Holds the last verified [`SignedRevocationList`] and checks each bundle at
//! load: a bundle whose bytes-digest or signing key id is on the list is
//! refused. The list is fetched by the sync loop on the normal cadence
//! (ADR-2: list-pull, not a per-load online check), so this adds no network
//! dependency to the load path — only a set lookup.
//!
//! Freshness: the list carries a monotonic `serial` (an older list is
//! rejected) and a `next_update` after which it is *stale*. Staleness policy
//! is configurable: `Monitor` keeps serving on the last-good list; `Enforce`
//! fails closed (refuses all loads) until a fresh list is fetched.

use std::collections::HashSet;

use parking_lot::RwLock;
use reaper_core::bundle_signing::VerifyingKey;
use reaper_core::config::RevocationStaleness;
use reaper_core::revocation::SignedRevocationList;
use tracing::{info, warn};

#[derive(Default)]
struct Cached {
    serial: u64,
    next_update: i64,
    hashes: HashSet<String>,
    key_ids: HashSet<String>,
    /// True once at least one valid list has been applied.
    loaded: bool,
}

/// Thread-safe revocation cache checked at bundle load.
pub struct RevocationStore {
    staleness: RevocationStaleness,
    cached: RwLock<Cached>,
}

impl RevocationStore {
    pub fn new(staleness: RevocationStaleness) -> Self {
        Self {
            staleness,
            cached: RwLock::new(Cached::default()),
        }
    }

    /// Verify and apply a freshly-fetched list. Rejects a list whose signature
    /// doesn't verify against the pinned key, or whose serial is not newer than
    /// the current one (replay of an old list). `key_id_pin` mirrors the bundle
    /// key-id pin.
    pub fn apply(
        &self,
        signed: &SignedRevocationList,
        key: &VerifyingKey,
        key_id_pin: Option<&str>,
    ) -> Result<(), String> {
        let list = signed
            .verify(key, key_id_pin)
            .map_err(|e| format!("revocation list signature invalid: {e}"))?;

        let mut cached = self.cached.write();
        if cached.loaded && list.serial < cached.serial {
            return Err(format!(
                "stale revocation list rejected: serial {} < current {}",
                list.serial, cached.serial
            ));
        }
        // Same serial with an already-loaded list: nothing changed.
        if cached.loaded && list.serial == cached.serial {
            cached.next_update = list.next_update;
            return Ok(());
        }
        cached.serial = list.serial;
        cached.next_update = list.next_update;
        cached.hashes = list
            .revoked_bundle_hashes
            .iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        cached.key_ids = list.revoked_key_ids.iter().cloned().collect();
        cached.loaded = true;
        info!(
            serial = list.serial,
            revoked_hashes = cached.hashes.len(),
            revoked_keys = cached.key_ids.len(),
            "Applied revocation list"
        );
        Ok(())
    }

    /// Load-time check for one bundle. `Err(reason)` refuses the load.
    ///
    /// - revoked hash or key id → refuse (the core purpose).
    /// - list stale AND `Enforce` → refuse all loads (fail closed).
    /// - list stale AND `Monitor` → allow, warn (fail open).
    /// - no list loaded yet → allow (nothing to enforce until first fetch;
    ///   `Enforce` does not brick a cold agent that hasn't fetched once).
    pub fn check(&self, bundle_sha256_hex: &str, key_id: &str, now: i64) -> Result<(), String> {
        let cached = self.cached.read();
        if !cached.loaded {
            return Ok(());
        }
        let hash_lc = bundle_sha256_hex.to_ascii_lowercase();
        if cached.key_ids.contains(key_id) {
            return Err(format!("signing key '{key_id}' is revoked"));
        }
        if cached.hashes.contains(&hash_lc) {
            return Err(format!("bundle hash {bundle_sha256_hex} is revoked"));
        }
        if cached.next_update != 0 && now > cached.next_update {
            match self.staleness {
                RevocationStaleness::Enforce => {
                    return Err(format!(
                        "revocation list is stale (next_update {} < now {}) and staleness mode \
                         is enforce: refusing to load",
                        cached.next_update, now
                    ));
                }
                RevocationStaleness::Monitor => {
                    warn!(
                        next_update = cached.next_update,
                        now, "Revocation list is stale (monitor mode: still serving last-good)"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaper_core::bundle_signing::SigningKey;
    use reaper_core::revocation::{RevocationList, SignedRevocationList};

    fn keypair() -> (SigningKey, VerifyingKey) {
        let sk = SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[5u8; 32])));
        let vk = VerifyingKey::from_hex(sk.algorithm(), &sk.public_key_hex()).unwrap();
        (sk, vk)
    }

    fn signed(
        sk: &SigningKey,
        serial: u64,
        next_update: i64,
        hashes: &[&str],
        keys: &[&str],
    ) -> SignedRevocationList {
        SignedRevocationList::sign(
            RevocationList {
                issued_at: "2026-01-01T00:00:00Z".into(),
                serial,
                next_update,
                revoked_bundle_hashes: hashes.iter().map(|s| s.to_string()).collect(),
                revoked_key_ids: keys.iter().map(|s| s.to_string()).collect(),
            },
            sk,
            "k1",
        )
    }

    #[test]
    fn revoked_hash_and_key_are_refused() {
        let (sk, vk) = keypair();
        let store = RevocationStore::new(RevocationStaleness::Monitor);
        store
            .apply(
                &signed(&sk, 1, 0, &["deadbeef"], &["leaked"]),
                &vk,
                Some("k1"),
            )
            .unwrap();

        assert!(store.check("deadbeef", "k1", 100).is_err(), "revoked hash");
        assert!(
            store.check("DEADBEEF", "k1", 100).is_err(),
            "case-insensitive"
        );
        assert!(store.check("aaaa", "leaked", 100).is_err(), "revoked key");
        assert!(store.check("aaaa", "good", 100).is_ok(), "clean bundle");
    }

    #[test]
    fn no_list_loaded_allows() {
        let store = RevocationStore::new(RevocationStaleness::Enforce);
        // Cold agent, never fetched: Enforce must not brick it.
        assert!(store.check("aaaa", "k1", 100).is_ok());
    }

    #[test]
    fn stale_list_enforce_fails_closed_monitor_serves() {
        let (sk, vk) = keypair();
        let list = signed(&sk, 1, 500, &["aa"], &[]);

        let enforce = RevocationStore::new(RevocationStaleness::Enforce);
        enforce.apply(&list, &vk, Some("k1")).unwrap();
        assert!(enforce.check("bb", "k1", 400).is_ok(), "fresh: allow");
        assert!(
            enforce.check("bb", "k1", 600).is_err(),
            "stale+enforce: refuse"
        );

        let monitor = RevocationStore::new(RevocationStaleness::Monitor);
        monitor.apply(&list, &vk, Some("k1")).unwrap();
        assert!(
            monitor.check("bb", "k1", 600).is_ok(),
            "stale+monitor: serve"
        );
        // ...but a revoked hash is still refused even when stale under monitor.
        assert!(monitor.check("aa", "k1", 600).is_err());
    }

    #[test]
    fn older_serial_is_rejected() {
        let (sk, vk) = keypair();
        let store = RevocationStore::new(RevocationStaleness::Monitor);
        store
            .apply(&signed(&sk, 5, 0, &["aa"], &[]), &vk, Some("k1"))
            .unwrap();
        // A replayed older list (serial 4) is refused...
        assert!(store
            .apply(&signed(&sk, 4, 0, &[], &[]), &vk, Some("k1"))
            .is_err());
        // ...and the revocation it carried stays in effect.
        assert!(store.check("aa", "k1", 100).is_err());
    }

    #[test]
    fn bad_signature_is_rejected() {
        let (sk, _) = keypair();
        let other =
            SigningKey::Ed25519(Box::new(ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])));
        let other_vk = VerifyingKey::from_hex(other.algorithm(), &other.public_key_hex()).unwrap();
        let store = RevocationStore::new(RevocationStaleness::Monitor);
        // List signed by sk, verified against the wrong key → rejected.
        assert!(store
            .apply(&signed(&sk, 1, 0, &["aa"], &[]), &other_vk, None)
            .is_err());
    }
}
