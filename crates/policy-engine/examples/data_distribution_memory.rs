//! Memory profile of the PRIMARY DATA DISTRIBUTION path.
//!
//! Exercises the three ways data reaches an agent's DataStore and reports the
//! ACTUAL live store heap (not process RSS) plus interner growth at each step:
//!
//!   A. Full hot load (snapshot as JSON)   — deploy_data_version path
//!   B. Snapshot as a compiled data bundle — compact wire/disk artifact
//!   C. Delta stream (upsert/delete)        — apply_data_deltas path
//!         C1. stable keys   — churn over a bounded key/value space
//!         C2. churning keys — every delta introduces fresh unique strings
//!
//! The point of C is to show where steady-state memory is BOUNDED vs where it
//! GROWS: deltas mutate the store in place (live heap tracks the live set), but
//! the string interner is append-only, so churn through ever-unique strings
//! grows interner memory unbounded even while the live entity count is flat.

use policy_engine::{DataBundle, DataLoader, DataStore};
use serde_json::json;

#[cfg(target_os = "linux")]
fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("VmHWM:")
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|kb| kb.parse::<usize>().ok())
            })
        })
        .map(|kb| kb as f64 / 1024.0)
        .unwrap_or(0.0)
}
#[cfg(not(target_os = "linux"))]
fn peak_rss_mb() -> f64 {
    0.0
}

fn report(label: &str, store: &DataStore) {
    let st = store.stats();
    println!(
        "  {:<34} entities={:>7}  interned={:>8}  heap={:>7.2} MB",
        label,
        st.total_entities,
        st.interner_stats.unique_strings,
        st.estimated_memory_bytes as f64 / 1_048_576.0,
    );
}

/// Build a JSON snapshot document of `n` entities (the full-load payload).
fn snapshot_json(n: usize) -> String {
    let mut entities = Vec::with_capacity(n);
    for i in 0..n {
        entities.push(json!({
            "id": format!("user_{i}"),
            "type": "User",
            "attributes": {
                "role": if i % 3 == 0 { "admin" } else { "viewer" },
                "department": format!("dept_{}", i % 20),
                "clearance": (i % 5) as i64,
                "active": true,
            }
        }));
    }
    serde_json::to_string(&json!({ "entities": entities })).unwrap()
}

fn main() {
    const N: usize = 20_000;
    println!("PRIMARY DATA DISTRIBUTION — memory profile ({N} entities)\n");

    // ── A. Full hot load (snapshot as JSON) ────────────────────────────────
    println!("A. Full hot load (JSON snapshot -> deploy_data_version):");
    let snapshot = snapshot_json(N);
    println!(
        "   snapshot JSON payload: {:.2} MB",
        snapshot.len() as f64 / 1_048_576.0
    );
    let store_a = DataStore::new();
    DataLoader::new(store_a.clone())
        .load_json(&snapshot)
        .unwrap();
    report("after full load", &store_a);
    let heap_a = store_a.stats().estimated_memory_bytes;

    // ── B. Snapshot as a compiled data bundle ──────────────────────────────
    println!("\nB. Snapshot as compiled data bundle (compact distribution artifact):");
    let bundle_bytes = store_a
        .to_bundle("dist".into(), "1".into())
        .to_bytes()
        .unwrap();
    println!(
        "   bundle bytes: {:.2} MB ({:.2}x the JSON snapshot)",
        bundle_bytes.len() as f64 / 1_048_576.0,
        bundle_bytes.len() as f64 / snapshot.len() as f64
    );
    let store_b = DataBundle::from_bytes(&bundle_bytes)
        .unwrap()
        .load_into_store()
        .unwrap();
    report("after bundle load", &store_b);
    let heap_b = store_b.stats().estimated_memory_bytes;
    println!(
        "   bundle-load heap matches JSON-load heap: {} (Δ {:.0} KB)",
        (heap_a as i64 - heap_b as i64).unsigned_abs() < (heap_a as u64 / 20),
        (heap_a as i64 - heap_b as i64).abs() as f64 / 1024.0
    );

    // ── C1. Delta stream, STABLE keys (bounded churn) ──────────────────────
    println!("\nC1. Delta stream — STABLE keys (re-upsert existing entities):");
    let store_c1 = DataStore::new();
    let loader_c1 = DataLoader::new(store_c1.clone());
    loader_c1.load_json(&snapshot_json(N)).unwrap();
    report("baseline", &store_c1);
    let heap_c1_start = store_c1.stats().estimated_memory_bytes;
    let interned_c1_start = store_c1.stats().interner_stats.unique_strings;
    // Apply 10x the dataset in delta upserts, but only over the EXISTING keys
    // and the EXISTING value space — nothing new to intern.
    for round in 0..10 {
        for i in 0..N {
            let _ = loader_c1.upsert_entity_doc(&json!({
                "id": format!("user_{i}"),
                "type": "User",
                "attributes": {
                    "role": if (i + round) % 3 == 0 { "admin" } else { "viewer" },
                    "department": format!("dept_{}", i % 20),
                    "clearance": (i % 5) as i64,
                    "active": round % 2 == 0,
                }
            }));
        }
    }
    report("after 10x delta upserts", &store_c1);
    let heap_c1_end = store_c1.stats().estimated_memory_bytes;
    let interned_c1_end = store_c1.stats().interner_stats.unique_strings;
    println!(
        "   -> {} delta upserts applied; interner grew {} strings; heap {:+.2} MB",
        10 * N,
        interned_c1_end - interned_c1_start,
        (heap_c1_end as f64 - heap_c1_start as f64) / 1_048_576.0
    );
    println!("   VERDICT: steady-state memory BOUNDED (heap tracks live set).");

    // ── C2. Delta stream, CHURNING keys (unbounded unique strings) ─────────
    println!("\nC2. Delta stream — CHURNING keys (upsert then delete, unique each time):");
    let store_c2 = DataStore::new();
    let loader_c2 = DataLoader::new(store_c2.clone());
    loader_c2.load_json(&snapshot_json(N)).unwrap();
    let heap_c2_start = store_c2.stats().estimated_memory_bytes;
    let interned_c2_start = store_c2.stats().interner_stats.unique_strings;
    report("baseline", &store_c2);
    // Simulate a long-lived replica: repeatedly upsert a short-lived entity
    // with a globally-unique id + unique attribute value, then delete it. The
    // live set returns to N every cycle, but each cycle mints fresh strings.
    let churn = 10 * N;
    for k in 0..churn {
        let uniq = format!("ephemeral_{k}");
        let _ = loader_c2.upsert_entity_doc(&json!({
            "id": uniq,
            "type": "Session",
            "attributes": { "token": format!("tok_{k}"), "nonce": format!("n_{k}") }
        }));
        loader_c2.delete_entity(&uniq);
    }
    report("after 10x upsert+delete churn", &store_c2);
    let heap_c2_end = store_c2.stats().estimated_memory_bytes;
    let interned_c2_end = store_c2.stats().interner_stats.unique_strings;
    println!(
        "   -> live entities back to {} but interner grew {} -> {} (+{} strings)",
        store_c2.stats().total_entities,
        interned_c2_start,
        interned_c2_end,
        interned_c2_end - interned_c2_start
    );
    println!(
        "   -> heap {:+.2} MB despite flat live set  <-- APPEND-ONLY INTERNER GROWTH",
        (heap_c2_end as f64 - heap_c2_start as f64) / 1_048_576.0
    );
    println!(
        "   VERDICT: interner is append-only; churn leaks until a snapshot rebuild resets it."
    );

    // ── D. Atomic swap transient (build-new-then-swap) ─────────────────────
    // The agent currently applies data IN PLACE (Arc<DataStore>, DashMap
    // inserts) so it does NOT pay this; a build-new-then-swap deploy (for true
    // atomic zero-partial-visibility) holds old + new simultaneously = ~2x.
    println!("\nD. Atomic swap transient (if a deploy built a new store and swapped):");
    let store_d = DataBundle::from_bytes(&bundle_bytes)
        .unwrap()
        .load_into_store()
        .unwrap();
    let heap_d = store_d.stats().estimated_memory_bytes;
    println!(
        "   old ({:.2} MB) + new ({:.2} MB) both resident = {:.2} MB peak (~2x live set)",
        heap_a as f64 / 1_048_576.0,
        heap_d as f64 / 1_048_576.0,
        (heap_a + heap_d) as f64 / 1_048_576.0
    );
    println!("   NOTE: agent applies in place (no 2x), trading atomicity for footprint.");

    println!("\nPeak RSS over run: {:.1} MB", peak_rss_mb());

    // ── Invariants (regression guards) ─────────────────────────────────────
    // Bundle load must reproduce the exact same live store as the JSON load.
    assert!(
        (heap_a as i64 - heap_b as i64).unsigned_abs() < (heap_a as u64 / 20),
        "bundle-load heap ({heap_b}) must match JSON-load heap ({heap_a}) within 5%"
    );
    // Stable-key delta churn must not grow the store at all.
    assert_eq!(
        interned_c1_end, interned_c1_start,
        "stable-key deltas must not intern new strings"
    );
    assert!(
        (heap_c1_end as i64 - heap_c1_start as i64).unsigned_abs() < (heap_c1_start as u64 / 20),
        "stable-key delta heap must stay bounded (start={heap_c1_start}, end={heap_c1_end})"
    );
    // Churning-key deltas are EXPECTED to grow the interner (documents the
    // append-only leak); assert the demonstration actually reproduced it so the
    // example can't silently stop exercising the path.
    assert!(
        interned_c2_end > interned_c2_start * 2,
        "churn demo should show interner growth (start={interned_c2_start}, end={interned_c2_end})"
    );
    println!(
        "\n✅ invariants hold (bundle≡JSON heap, stable deltas bounded, churn growth reproduced)"
    );
}
