# Data Distribution — Memory Profile & the Append-Only Interner

**Reproduce:** `cargo run -p policy-engine --example data_distribution_memory --release`
**Related:** `crates/policy-engine/examples/memory_volume_test.rs` (full-load RSS
vs store heap), `crates/policy-engine/src/data/interning.rs`.

## TL;DR

The DataStore stores data efficiently and the compiled bundle is a great
distribution artifact. The one thing to watch on the **primary data
distribution path** is that the string interner is **append-only**, so a
long-lived read-replica applying a high-churn delta stream can grow memory
unbounded even while its live entity count is flat.

## What was measured

Process RSS is a misleading proxy for stored data (it includes the transient
parsed-JSON DOM plus allocator arenas the OS never reclaims — a 5 KB file can
show ~200x RSS/file). All numbers below are the **actual live store heap**
(`DataStore::stats().estimated_memory_bytes`), which counts interned strings
once. Measured at 20k entities:

| Path | Live heap | Notes |
|---|---|---|
| A. Full hot load (JSON snapshot) | 3.69 MB | `deploy_data_version` path |
| B. Compiled data bundle | 3.69 MB | **identical** store; bundle bytes only **0.24x** the JSON (≈4x smaller on the wire) |
| C1. 200k delta upserts, **stable keys** | 3.69 MB (+0.00) | interner grew **0** — bounded ✅ |
| C2. 200k upsert+delete, **churning keys** | **98.29 MB** (+94.6) | live set back to 20k, interner grew 20k → **620k** ⚠️ |
| D. Atomic build-new-then-swap | 2x (7.37 MB peak) | old+new resident; agent avoids this by applying in place |

## Why C2 grows

`apply_data_deltas` → `DataLoader::upsert_entity_doc` / `delete_entity` mutate
the store **in place** (good — no 2x, no accumulation of the live set). But
every id / type / attribute-key / attribute-value / relation string is interned,
and the interner **never releases strings** (`interning.rs`: "Strings are never
deleted (append-only) for safety"). `delete_entity` removes the entity and its
index entries but leaves its strings interned forever.

So memory tracks the number of **distinct strings ever seen**, not the live set.
Workloads that mint fresh strings over time leak:
- short-lived entities with unique ids (sessions, requests, jobs),
- attributes set to ever-unique values (tokens, nonces, timestamps-as-strings,
  UUIDs, monotonic counters).

Workloads over a bounded key/value space (roles, departments, resource types —
the RBAC/ABAC common case) do **not** leak: C1 applied 200k upserts with zero
interner growth.

## Mitigations

1. **Snapshot rebuild resets the interner.** A full snapshot load builds a fresh
   store + fresh interner (B == A above). The existing compaction-floor →
   `snapshot_required` fallback already forces this for replicas that fall
   behind. Periodically forcing a snapshot rebuild (even when caught up) bounds
   the leak for always-online replicas. Cheapest fix; no code change to the hot
   path.
2. **Evictable interner.** Refcount interned strings (increment on entity
   insert, decrement on remove/overwrite) and drop at zero, or sweep unreferenced
   strings during change-log retention. Removes the leak entirely at the cost of
   refcount bookkeeping on the write path. Bigger change; would need its own arc
   with the differential suite as the safety net.

## Recommendation

For the RBAC/ABAC bounded-vocabulary workloads Reaper targets, C1 is the norm
and memory is bounded. Ship mitigation (1) as an operational guard now (document
a max delta-only uptime / periodic snapshot), and track (2) as a follow-up if a
high-cardinality-string workload appears. The `data_distribution_memory` example
is the regression guard: it asserts bundle≡JSON heap and stable-delta
boundedness, and reproduces the churn growth so the path can't silently stop
being exercised.
