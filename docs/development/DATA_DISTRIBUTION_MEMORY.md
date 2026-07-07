# Data Distribution — Memory Profile & Bounded Churn

**Reproduce:** `cargo run -p policy-engine --example data_distribution_memory --release`
**Related:** `crates/policy-engine/examples/memory_volume_test.rs` (full-load RSS
vs store heap), `crates/policy-engine/src/data/interning.rs`,
`crates/policy-engine/src/data/store.rs`.

## TL;DR

The DataStore stores data efficiently, the compiled bundle is a great
distribution artifact, and — as of the reference-counted interner — a long-lived
read-replica applying a high-cardinality delta stream now holds **bounded**
memory: churned strings and index entries are reclaimed when the entity that
owned them is removed. Steady-state memory tracks the **live** set, not the total
number of distinct strings ever seen.

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
| C1. 200k delta upserts, **stable keys** | 3.69 MB (+0.00) | bounded ✅ |
| C2. 200k upsert+delete, **churning keys** | 3.69 MB (+0.00) | live set back to 20k; interner +3, heap flat ✅ |
| D. Atomic build-new-then-swap | 2x (7.37 MB peak) | old+new resident; agent avoids this by applying in place |

Before the fix, C2 grew to **98 MB** (interner 20k → 620k strings) — the memory
tracked distinct strings ever seen, not the live set.

## The two leaks that were fixed

`apply_data_deltas` → `DataLoader::upsert_entity_doc` / `delete_entity` mutate
the store **in place** (no 2x, no accumulation of the live set). Churn of unique
strings — short-lived entities with unique ids (sessions, requests, jobs),
attributes set to ever-unique values (tokens, nonces, UUIDs, counters) — used to
leak in two layers:

1. **The string interner was append-only.** `delete_entity` removed the entity
   but never its strings, so the interner grew forever.
2. **The attribute/composite indexes never pruned emptied entries.** `remove()`
   deleted the id from each `(key, value)` index set but left the now-empty set
   in the map, so a churn of unique attribute values grew the index maps without
   bound (this was ~half the leak).

## How it is fixed

### Reference-counted, pin-safe interner (`interning.rs`)

The interner now stores a refcount alongside each id in the same DashMap shard:

- `intern()` — the default — **pins** the string (`refs = u32::MAX`); it is never
  evicted. Used by policy compilation, entity types, and any caller that does not
  manage a reference lifecycle. **Pinning is the safety foundation:** a string a
  compiled policy literal references can never be evicted out from under it.
- `intern_counted()` increments a reference count. Used by the DataStore for the
  strings an entity **owns**: its id, parent, and string attribute values
  (high-cardinality). Pinned strings stay pinned (counting is a no-op on them).
- `release()` decrements and **evicts at zero**; it is a no-op on a pinned or
  already-evicted string. The DataStore calls it for exactly the strings an
  entity owned, once, when the entity is removed (`remove()` →
  `release_entity_strings`).

Safety and concurrency:
- Count and existence live in the **same shard lock**, so `intern_counted` can
  never revive a string that `release` is concurrently evicting.
- A live reference keeps the count ≥ 1, so a string in use cannot be evicted;
  `release` therefore always finds a valid `id → string` mapping.
- Ids are **monotonic** (never recycled), so even a hypothetical stale id can
  only ever resolve to `None`, never alias a different string.
- Bounded-vocabulary strings (types, attribute keys, relation names) are left
  pinned — they don't churn, so pinning them costs nothing and keeps the counted
  set small and easy to reason about.

`clear()` drops all non-pinned strings (`reset_counted`) so a snapshot
`clear()+reload` doesn't accumulate stale strings.

### Index pruning (`store.rs`)

`remove()` now prunes an index entry when its set becomes empty
(`DashMap::remove_if(&key, |_, set| set.is_empty())`, which re-checks under the
lock so a concurrent insert can't lose an entry). Covers the type, attribute, and
composite indexes.

## Correctness net

- `compiled_ast_equivalence_tests` (37) — compiled ≡ AST for every DSL function,
  now with eviction live.
- `delta_sync_differential_tests` — delta-applied store ≡ freshly-rebuilt store
  (exercises the exact evict-on-delete path).
- `differential_parity_tests`, `check_mode_differential_tests`.
- Interner unit tests: eviction at zero, refcount balance, **pin never evicted**,
  intern-after-count pins, `reset_counted` keeps pinned, evicted ids not reused.
- `data_distribution_memory` asserts stable **and** churning deltas stay bounded.

## Eval-path leaks — all fixed

The compiled evaluator represents strings only as interned ids, so anything it
reads from or produces per request had to be interned — and a plain `intern()`
pins forever. Three eval-path leak classes are now closed:

1. **Cross-entity `context.*` comparisons** (`context.token == resource.secret`)
   interned the request value. Now compared **by content** without interning: an
   already-interned value reuses its id; a novel one is carried as raw text and
   compared by content, matching the AST evaluator (which also removed a latent
   compiled-vs-AST divergence). Pinned by `context_interner_leak_tests`.

2. **Per-request entity lookups** (`mod.rs::evaluate`) interned the request
   principal and resource on every eval. This leaked novel resource ids
   (URL-path-style resources) and, worse, **pinned a loaded entity's id when it
   was used as a principal — defeating the data-plane's refcounted reclamation
   for that entity**. Now resolved via `lookup` (never `intern`): the principal
   must be an already-interned entity or the eval fails closed; a non-interned
   resource uses a sentinel id (it matches no literal by content and has no
   entity/edges).

3. **Result-producing methods** (`lower`/`upper`/`trim`/`split`/`replace`/`find`/
   `find_all` and comprehension transforms) intern the NEW strings they compute.
   Those live only for the one evaluation, so they are now interned via
   `intern_transient` — **counted** and recorded in a per-thread scratch frame —
   and released by a `ScratchGuard` when `evaluate()` returns (including on
   panic). Novel results are evicted; strings that are also owned by an entity or
   pinned as a literal are untouched (release is a balanced decrement / no-op).

Pinned by `eval_interner_bounding_tests` (high-cardinality resources, results,
and principal-stays-evictable) and by the `test_functions_10k` volume test, which
asserts **0 interner growth over 10k evals** of a policy producing string results
every eval. Latency is unchanged (the reclamation is dominated by the regex/alloc
work the methods already do).

## Residual (documented, not a leak under normal workloads)

- **ReBAC subjects** are pinned (interned via `add_relationship`), so churning a
  high-cardinality *relationship subject* space is not reclaimed until the next
  snapshot rebuild. Entity-attribute and entity-id churn (the common case) is
  fully reclaimed.
