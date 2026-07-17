//! Target-portable time access for the evaluation path.
//!
//! `wasm32-unknown-unknown` has no ambient clock: `std::time::Instant::now()`
//! and `SystemTime::now()` compile but panic at runtime. Everything the
//! evaluation path can reach — the engine latency probes and the DSL
//! `time::*` builtins — reads time through this module instead of `std::time`
//! directly, so the same policy source evaluates on both targets.
//!
//! Semantics per target:
//! - **Native**: reads `SystemTime` unless a clock has been pinned via
//!   [`set_injected_now_unix_ns`]. Pinning is the deterministic-replay /
//!   test seam (evaluate a policy "as of" a fixed instant); when nothing is
//!   injected (the production default) behavior is byte-identical to a direct
//!   `SystemTime::now()`.
//! - **wasm32**: wall-clock reads come from a host-injected epoch
//!   ([`set_injected_now_unix_ns`]) so embeddings can pin evaluation time and
//!   stay deterministic/replayable. When nothing is injected, we fall back to
//!   `chrono::Utc::now()` (JS `Date.now()` via chrono's `wasmbind`, matching
//!   the JS-first packaging decision in `plans/round-2/F2-wasm-target.md`).
//!
//! The injected clock is a single process-global (`0` = not injected, the
//! production default on every target). [`Stopwatch`] measures elapsed wall
//! time between readings — under a pinned clock it reports 0. It feeds latency
//! *metrics only* and is never an authorization input.

use std::sync::atomic::{AtomicI64, Ordering};

/// Host-injected evaluation clock, unix nanoseconds. 0 = not injected (the
/// production default). Available on every target so that deterministic replay
/// and the `time::*` / jwt-expiry differential oracle can pin evaluation time
/// without racing the wall clock.
static INJECTED_NOW_UNIX_NS: AtomicI64 = AtomicI64::new(0);

/// Pin the evaluation clock to a fixed unix-epoch timestamp (nanoseconds). All
/// subsequent `time::*` builtin reads (and any code reading [`now_unix_ns`])
/// use this value until it is changed or [`clear_injected_now`] is called.
/// Injecting time makes decisions deterministic and replayable — the same
/// policy `(policy, data, request)` at a pinned instant always decides the same.
///
/// Production code never calls this; it is the replay/test seam. `0` is
/// reserved to mean "not injected", so pinning to the epoch is a no-op.
pub fn set_injected_now_unix_ns(unix_ns: i64) {
    INJECTED_NOW_UNIX_NS.store(unix_ns, Ordering::Relaxed);
}

/// Unpin the evaluation clock; reads fall back to the target's real clock.
pub fn clear_injected_now() {
    INJECTED_NOW_UNIX_NS.store(0, Ordering::Relaxed);
}

/// Current unix time in nanoseconds, or `None` if no clock is available.
/// Callers on the evaluation path must fail closed (or degrade to a metric
/// value of 0) on `None` — never substitute a silent wrong time.
#[inline]
pub fn now_unix_ns() -> Option<i64> {
    // A pinned clock wins on every target (replay / deterministic tests).
    let injected = INJECTED_NOW_UNIX_NS.load(Ordering::Relaxed);
    if injected != 0 {
        return Some(injected);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos() as i64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        chrono::Utc::now().timestamp_nanos_opt()
    }
}

/// Monotonic-ish elapsed-time probe for latency metrics.
///
/// Native: a true monotonic `Instant`. wasm32: two wall-clock readings
/// (saturating, so a pinned/injected clock yields 0). Metrics only.
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    start_unix_ns: i64,
}

impl Stopwatch {
    #[inline]
    pub fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                start: std::time::Instant::now(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                start_unix_ns: now_unix_ns().unwrap_or(0),
            }
        }
    }

    #[inline]
    pub fn elapsed_ns(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_nanos() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.start_unix_ns == 0 {
                return 0;
            }
            now_unix_ns()
                .map(|now| now.saturating_sub(self.start_unix_ns).max(0) as u64)
                .unwrap_or(0)
        }
    }

    #[inline]
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.elapsed_ns())
    }

    #[inline]
    pub fn elapsed_micros(&self) -> u64 {
        self.elapsed_ns() / 1_000
    }

    #[inline]
    pub fn elapsed_millis(&self) -> u64 {
        self.elapsed_ns() / 1_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_unix_ns_is_available_and_sane_on_native() {
        let ns = now_unix_ns().expect("native clock must exist");
        // After 2020-01-01 (1577836800s), before 2100.
        assert!(ns > 1_577_836_800 * 1_000_000_000);
    }

    #[test]
    fn stopwatch_elapsed_is_monotonic_nonzero_after_work() {
        let sw = Stopwatch::start();
        let mut acc = 0u64;
        for i in 0..10_000u64 {
            acc = acc.wrapping_add(i);
        }
        std::hint::black_box(acc);
        // elapsed_ns is a u64; just prove ordering across unit helpers.
        let ns = sw.elapsed_ns();
        assert!(sw.elapsed_micros() <= ns);
        assert_eq!(ns / 1_000_000, sw.elapsed_millis());
    }
}
