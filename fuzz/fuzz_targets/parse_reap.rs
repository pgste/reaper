#![no_main]
//! Fuzz `ReapParser::parse` over arbitrary input.
//!
//! Property under test: parsing must be TOTAL — no input, however crafted
//! (deeply nested `(((…`, long `!!!…` runs, garbage bytes), may panic, abort,
//! or stack-overflow. A returned `Err` is the correct outcome for bad input.
//! This is the live acceptance test for Plan 05's DSL nesting-depth bound.

use libfuzzer_sys::fuzz_target;
use policy_engine::reap::ReapParser;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Discard the result — we only care that this never panics/aborts.
        let _ = ReapParser::parse(s);
    }
});
