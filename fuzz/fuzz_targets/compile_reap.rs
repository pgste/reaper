#![no_main]
//! Fuzz the full `parse → compile_policy` pipeline over arbitrary input.
//!
//! Property under test: compilation of any successfully-parsed policy must be
//! total — the recursive compile walk must not panic, abort, or stack-overflow,
//! even on adversarially-shaped (but grammatically valid) trees. Complements
//! `parse_reap` by exercising the compiler's own recursion (Plan 05 depth guard,
//! defense-in-depth path).

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use policy_engine::reap::{compile_policy, ReapParser};
use policy_engine::DataStore;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(policy) = ReapParser::parse(s) {
            let _ = compile_policy(policy, Arc::new(DataStore::new()));
        }
    }
});
