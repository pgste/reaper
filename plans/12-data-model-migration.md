# Data-Model Migration Engine

> **STATUS: ✅ SHIPPED** — landed via PRs #45–#47 (2026-07-13) across phases 1–3.
> 1: closed typed `ModelTransform` set (9 ops: rename role/relation/attribute/
> entity-type, add/remove attribute/relation, retype) with `apply_to_model`
> precondition validation and mechanical inverses (`remove_attribute`
> declared irreversible); lossless-only coercion (ADR-4); pure record-level
> planner producing exact affected-row RecordOps and fail-closed
> PlanBlockers (un-coercible retype lists offenders; remove_relation over
> live tuples demands `delete_tuples: true`); `POST …/datastore/migrations/
> plan` dry-run materializes current vs proposed and loads BOTH into a real
> policy-engine DataStore, diffing grants + edges + traversal reachability —
> renames compared modulo the rename map, so a pure rename provably reports
> decision_neutral (mutation-free, proven by test). 2: atomic apply — one
> transaction covering record transforms, model update, model_version bump,
> append-only `adm_model_versions` history row, and outbox dirty markers;
> apply recomputes the plan server-side and publishes the post-migration
> data version; fleet converges via the EXISTING delta sync (delta ≡ rebuild
> across a migration proven byte-for-byte); the blind `PUT /model` overwrite
> replaced by a vocabulary-breaking guard; interner hygiene under a
> rename-of-500 verified O(schema). 3: `model_version` provenance on every
> published data version, threaded control plane → reaper-sync → agent →
> every DecisionLogEntry (Option, absent pre-migration — NDJSON compatible);
> rollback as an impact-checked FORWARD inverse migration (`POST …/
> migrations/{v}/rollback`) restoring the exact pre-migration checksum,
> refusing irreversible transforms with direction to the immutable
> pre-migration data version. Exotic reshapes stay export→transform→
> re-import per ADR-2; consistency tokens/zookies remain future work.

**Readiness gate:** Safe evolution of managed authorization data — currently a "botched model change = mass allow/deny incident" exposure with no defined path.
**Priority:** P1 (Product Architecture F6). Blocking before customers put production authorization data in the data plane at scale.
**Findings closed:** Product F6 (data fork has no model-migration engine; renaming a role / adding a relation / changing an attribute type has no defined path for existing records). Advances DATA_PLANE_PLAN §6 "temporal versioning & point-in-time restore" from "Later" toward a concrete design; complements the delta-sync (§8) and decision provenance (§7) already shipped.

---

## 1. Goal

Give the Authorization Data Model (ADM) a **typed, dry-runnable, reversible migration engine** so that changing the *shape* of the model — renaming a role, adding/removing a relation, retyping an attribute — has a defined, auditable transformation of every existing entity/binding/tuple record, with an **impact analysis before commit** ("N records, M edges affected; K principals gain/lose access"), a **backfill execution**, a **versioned model with rollback**, and coherence with **delta-sync distribution** and **decision-log provenance** so audit stays interpretable across the change.

Today a model change is a blind JSON overwrite (`update_model`, evidence below) that silently strands or breaks existing records. The engine replaces that with: *propose transforms → analyze impact → dry-run diff → apply atomically (records + model bump together) → publish a new data version → fleet converges via existing delta sync → decisions carry a model version so audit reads correctly before and after.*

**Non-goals:** changing the evaluator or the materialized bundle format (nothing in `crates/policy-engine/src/evaluators/` changes — the whole point of the ADM is that it compiles to the format the engine already consumes); consistency tokens/zookies (still future work); arbitrary user-scripted transforms (v1 is a closed, typed transform set).

---

## 2. Current state (evidence) — file:line

- **Model updates are a blind overwrite with zero record migration.** `services/reaper-management/src/api/datastore.rs:221-230` (`put_model`) accepts a `ModelDefinition` and calls `update_model`, which is `UPDATE datastores SET model = $1 ...` — `services/reaper-management/src/db/repositories/datastore.rs:143-158`. No validation of existing `adm_entities`/`adm_role_bindings`/`adm_tuples` against the new schema, no rename mapping, no backfill, no version guard, no dry-run. A rename or retype leaves every existing record referencing vocabulary that no longer matches.
- **The model IS versioned but only the schema JSON and a counter move.** `datastores` table: `model TEXT`, `current_version INTEGER`, `change_seq INTEGER` (`db/migrations/006_data_plane.sql:6-16`, `007_change_log.sql`). There is a `current_version` counter but no *model-version* history and no link between a model change and the records it should have transformed.
- **Write-time validation exists but only for new writes, not retroactively.** `domain/datastore.rs:96-142` (`validate_attributes`) rejects a mistyped/unknown attribute at entity write time. Nothing re-validates the *existing corpus* when the model changes — so tightening a type or removing an attribute passes silently and breaks at materialize/eval instead.
- **Records are relationally stored and individually addressable** (good raw material for a migration): `adm_entities` (`006:23-35`), `adm_role_bindings` (`006:40-53`, unique on `subject,role,scope`), `adm_tuples` (`006:56-70`, unique on `object,relation,subject`). Renames/retypes are bounded SQL updates over these.
- **Materialization is deterministic and checksum-stable** — a migration's effect is reproducible: `domain/datastore.rs:310-402` (`materialize`, BTreeMap/BTreeSet ordering → stable checksums), plus `materialize_one` (`:411-476`) for per-entity delta emission.
- **Delta sync is gap-proof and idempotent — the distribution substrate a migration must ride.** DATA_PLANE_PLAN §8: transactional outbox (`adm_changes`, `007_change_log.sql`), `GET …/datastore/changes?since=N`, agent `apply-deltas` with contiguity enforcement (409 → precise re-pull), snapshot/delta lineage pinned by `change_seq` on `adm_versions`. Engine primitives are all idempotent: `DataStore::upsert`/`remove_entity` (`crates/policy-engine/src/data/store.rs:343,353`), `RelationshipGraph::add_edge`/`remove_edge`/`detach_carried`/`detach` (`crates/policy-engine/src/data/relationships.rs:90,112,125,142`).
- **The interner is refcounted/evictable** — a mass rename that drops old strings must release them: `crates/policy-engine/src/data/interning.rs:134 intern_counted`, `:168 release`, `:214 lookup`, `:222 resolve`. A migration that renames a role from `editor`→`author` interns the new string and must release the old refcount on the last carrier.
- **Decision provenance already pins policy + data version** but not model version: `crates/policy-engine/src/decision_log.rs:46 policy_version`, `:72 data_version`, `:77 data_checksum`. There is no `model_version` field, so a decision made before vs after a model change is indistinguishable in audit beyond the data-version number.
- **DATA_PLANE_PLAN §6** explicitly defers "temporal versioning & point-in-time restore" to "Later" — this plan is the concrete design that closes F6 and starts on that deferred item.

---

## 3. Definition of Done — testable checkboxes

- [ ] A closed set of **typed transforms** exists and is serializable: `RenameRole`, `RenameRelation`, `RenameAttribute`, `AddRelation`, `RemoveRelation`, `RetypeAttribute`, `AddAttribute`, `RemoveAttribute`, `RenameEntityType`. Each declares its record-level effect and its inverse.
- [ ] `POST …/datastore/migrations/plan` returns a **dry-run diff + impact report** without mutating anything: counts of entities/bindings/tuples touched, edges added/removed, and **`{principals_gaining: K1, principals_losing: K2}`** computed by materializing the *proposed* state and diffing a decision/reachability set against current.
- [ ] Dry-run **impact correctness is proven**: for a battery of RBAC/ABAC/ReBAC fixtures, the reported gain/lose set exactly equals the set computed by evaluating a decision battery against before- and after-materialized stores (same discipline as the shipped `delta_sync_differential_tests`).
- [ ] `POST …/datastore/migrations/apply` executes the plan **atomically**: record transforms + `model` update + `model_version` bump + change-log append all commit in **one DB transaction** (transactional-outbox invariant preserved — no partial migration, no lost delta).
- [ ] A **retype that cannot be satisfied fails closed** in dry-run (e.g. `clearance: string→int` where a value is `"high"`) — reported as a coercion error listing offending records; apply is refused until resolved via an explicit coercion/default policy.
- [ ] The model is **version-historied**: a new `adm_model_versions` row per migration with the transform list, author, timestamp, and before/after model hashes; **rollback** re-applies the inverse transforms as a new forward migration (auditable, never a silent revert).
- [ ] Applying a migration **publishes a new data version** and the fleet **converges via the existing delta path** (or a snapshot when below the compaction floor) with **zero eval downtime** — proven by a load test on `/api/v1/check` during a rename.
- [ ] Every `DecisionLogEntry` carries **`model_version`** so audit can attribute a decision to the model shape in force; a query can bucket decisions by model version across a change.
- [ ] The **interner releases** strings orphaned by a mass rename (no unbounded growth) — verified by interner stats before/after a rename-of-N migration.
- [ ] `delta ≡ rebuild` still holds **across a migration**: an agent that applied the migration's deltas is byte-for-byte and decision-for-decision identical to one that fresh-loaded the post-migration snapshot.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

### Step 1 — Define the typed transform set
- **What:** A `ModelTransform` enum (serde) enumerating the closed transform set (see DoD). Each variant carries its parameters (`RenameRole{from,to}`, `RetypeAttribute{entity_type,name,from,to,coercion}`, `AddRelation{RelationDef}`, etc.). Implement two methods per transform: `apply_to_model(&ModelDefinition) -> Result<ModelDefinition>` and `inverse() -> Option<ModelTransform>`.
- **Where:** New `services/reaper-management/src/domain/migration.rs`, next to `domain/datastore.rs`. Reuse `ModelDefinition`, `RoleDef`, `RelationDef`, `AttributeDef`, `AttrType` from `domain/datastore.rs:18-143`.
- **Verify:** Unit tests: each transform produces the expected model; `apply(inverse(apply(x))) == x` for reversible transforms; irreversible ones (`RemoveAttribute`) declare `inverse() = None` and require an explicit backfill on rollback.

### Step 2 — Record-level transform planner (what records each transform touches)
- **What:** For each transform, compute the SQL/record-level change over `adm_entities`/`adm_role_bindings`/`adm_tuples`: e.g. `RenameRole{editor→author}` → `UPDATE adm_role_bindings SET role='author' WHERE role='editor'` scoped to the datastore; `RenameRelation` → update `adm_tuples.relation`; `RetypeAttribute` → coerce/validate each `adm_entities.attributes[name]`; `RemoveRelation` → delete matching tuples (or refuse if non-empty, per policy). Produce a `MigrationPlan { transforms, record_ops, coercion_errors }`.
- **Where:** `services/reaper-management/src/domain/migration.rs` (planner) + read helpers on `db/repositories/datastore.rs` (already has typed accessors for the record tables).
- **Verify:** Planner over fixtures returns exact affected-row counts; a retype with an un-coercible value populates `coercion_errors` and the plan is marked not-applyable.

### Step 3 — Dry-run diff + impact analysis (`plan` endpoint)
- **What:** Materialize the **current** records (`materialize`, `datastore.rs:310`) and the **proposed post-transform** records (transforms applied in-memory, not persisted) into two DataLoader documents, load each into a headless `policy-engine` `DataStore`, and run a **decision/reachability battery** (the datastore's own policies or a standard probe set) against both. Diff the allow-sets to produce `{principals_gaining, principals_losing, edges_added, edges_removed, records_touched}`. Return this plus a structural JSON diff of the two materialized docs.
- **Where:** New handler `plan_migration` in `services/reaper-management/src/api/datastore.rs` (route `POST /orgs/{o}/ns/{n}/datastore/migrations/plan`, alongside `publish` at `:64`). Impact engine reuses the same headless-engine approach the future replay service (Product D-E) will use — build it reusable.
- **Verify:** Impact-correctness test (DoD): reported gain/lose == brute-force decision-battery diff over before/after stores for RBAC/ABAC/ReBAC fixtures, including a rename that should be **decision-neutral** (report must show 0 gain/0 lose — a rename that changes access is a bug).

### Step 4 — Atomic apply (records + model + version + outbox in one tx)
- **What:** `apply_migration` runs, in a single DB transaction: (a) the record ops from step 2; (b) `update_model` to the post-transform model; (c) insert an `adm_model_versions` row (transforms, author, before/after hashes); (d) bump `current_version`; (e) append `adm_changes` dirty markers for every touched entity_id with fresh contiguous `seq` (the shipped transactional-outbox append, `datastore.rs:164+`). Commit or roll back together — no partial migration can be observed by a syncing agent.
- **Where:** `services/reaper-management/src/db/repositories/datastore.rs` (new `apply_migration` tx method; the outbox append helper already exists there per `007` D2 work); `api/datastore.rs` handler `apply_migration`. Replace the raw `update_model` call in `put_model` (`api/datastore.rs:221-230`) with a guard that **rejects a bare model overwrite that changes vocabulary** and directs callers to the migration endpoint (keeps additive-only edits allowed, blocks silent renames/retypes).
- **Verify:** Kill the process mid-apply (inject a panic after record ops, before commit); confirm the DB is unchanged (rollback) and no `adm_changes` rows leaked. Confirm a successful apply advances `change_seq` contiguously.

### Step 5 — Distribution: converge the fleet via existing delta sync
- **What:** No new distribution path. The migration's touched entities are emitted as deltas by `GET …/datastore/changes?since=N` (each via `materialize_one`, `datastore.rs:411`); agents `apply-deltas` idempotently (`DataStore::upsert`/`remove_entity`, `RelationshipGraph::remove_edge`, `store.rs:343,353`, `relationships.rs:112`). A large rename that exceeds the compaction floor triggers the existing `snapshot_required` full-sync. Publish pins the migration's `change_seq` on the new `adm_versions` row so snapshot/delta lineage stays exact.
- **Where:** No new code in the sync path — this step is *verification that the migration rides it correctly*, plus ensuring the migration apply calls the same `publish` flow (`api/datastore.rs:471`).
- **Verify:** `delta ≡ rebuild across migration` test (DoD): agent A applies the migration deltas incrementally; agent B fresh-loads the post-migration snapshot; assert identical store contents + identical decisions on the full battery, including redelivered deltas.

### Step 6 — Interner hygiene on rename
- **What:** Ensure a mass rename releases orphaned interned strings. On the agent, `upsert` already "cleans stale attribute-index entries + carried edges" (DATA_PLANE_PLAN §8) and the interner is refcounted (`interning.rs:134 intern_counted`, `:168 release`). Confirm the delta applier for a renamed role/relation/attribute releases the old string's refcount when the last carrier is upserted to the new value.
- **Where:** Verification against `crates/policy-engine/src/data/store.rs` (`upsert` path) and `interning.rs`; add a targeted test, no expected new product code if the refcount discipline already covers upsert.
- **Verify:** Interner stats (`interning.rs:249 stats`) before/after a rename-of-N: old string count drops to zero, no net growth.

### Step 7 — Model-version provenance in the decision log
- **What:** Add `model_version: Option<i64>` to `DecisionLogEntry` and a `with_model_version` setter mirroring `with_policy_version` (`decision_log.rs:46,137`) and the `data_version` handling (`:72,169`). The agent stamps the model version of the store snapshot it evaluated against (carried alongside `data_version`). Central decision query (`api/decisions.rs`) can then bucket/split decisions across a model change.
- **Where:** `crates/policy-engine/src/decision_log.rs` (field + setter + constructor default `:106-113`); agent decision-capture call site; optionally surface in `api/decisions.rs` facets.
- **Verify:** A decision recorded before a migration and one after carry different `model_version`; an audit query "how did decisions split across model v3→v4" returns coherent buckets. Backward-compat: field is `Option`, absent on never-migrated stores.

### Step 8 — Rollback as a forward inverse migration
- **What:** `POST …/datastore/migrations/{version}/rollback` composes the inverse transforms (step 1 `inverse()`) into a new forward `MigrationPlan`, runs the same dry-run + apply pipeline, and records a new `adm_model_versions` row (never mutates history). Irreversible transforms (`RemoveAttribute`) require the caller to supply a backfill/default in the rollback request.
- **Where:** `api/datastore.rs` handler + `domain/migration.rs` inverse composition.
- **Verify:** Apply `RenameRole{editor→author}`, then rollback; assert the store, materialized checksum, and decision battery return exactly to the pre-migration state, and the model-version history shows three rows (v_n, v_n+1 rename, v_n+2 inverse) — auditable, no silent revert.

---

## 5. Dependencies

- **Shipped and required:** the ADM record tables + versioning (`006_data_plane.sql`), the transactional outbox + delta sync (`007_change_log.sql`, DATA_PLANE_PLAN §8), the idempotent engine primitives (`store.rs`, `relationships.rs`), deterministic `materialize`/`materialize_one` (`domain/datastore.rs`), refcounted interner (`interning.rs`). This plan is "mostly assembly" on top of these.
- **New schema:** `adm_model_versions` table (append-only history) — one new migration file (`010_model_migrations.sql`), following the append-only PG-migration discipline in `connection.rs:28-39,206-278`.
- **Headless engine for impact analysis:** a reusable "load a materialized document into a throwaway `DataStore` and run a decision battery" helper — shared with the future decision-replay service (Product D-E), so build it once, reusable.
- **Adjacent (not blocking):** environment/promotion model (D-C) would gate *when* a migration is allowed to promote to prod; SSO (identity) would attribute the migration author. This plan records author id regardless.

---

## 6. Testing & verification (incl. migration dry-run correctness)

1. **Transform algebra unit tests:** `apply`, `inverse`, model-shape correctness (step 1).
2. **Planner row-count tests:** exact affected records per transform; un-coercible retype flagged (step 2).
3. **Dry-run impact correctness (the load-bearing test):** reported `{principals_gaining/losing, edges_added/removed}` == brute-force decision-battery diff over before/after materialized stores, for RBAC/ABAC/ReBAC fixtures; a pure rename reports **0 access change** (step 3). Same differential discipline that first caught the fail-open `!=` and the referential-cascade hole (DATA_PLANE_PLAN §8).
4. **Atomicity test:** panic injected mid-apply → DB unchanged, no orphaned `adm_changes` (step 4).
5. **`delta ≡ rebuild across migration`:** incremental-apply agent vs fresh-snapshot agent are identical in store + decisions, incl. redelivery (step 5) — extends the existing `delta_sync_differential_tests`.
6. **Interner hygiene:** old strings released to zero after rename-of-N (step 6).
7. **Provenance test:** decisions bucket correctly by `model_version` across a change; `Option` absent on never-migrated (step 7).
8. **Rollback round-trip:** rename → rollback returns store/checksum/decisions to pre-state; history shows three forward rows (step 8).
9. **Zero-downtime test:** load generator on `/api/v1/check` throughout a rename apply → convergence; no denied-by-migration responses.

---

## 7. Effort & phasing — S/M/L

- **Phase 1 (M) — Transforms + dry-run + impact (steps 1–3).** The differentiator and the safety net. Delivers "tell me who gains/loses access before I commit" with no persistence risk. Highest design care goes here (impact-correctness proof).
- **Phase 2 (M) — Atomic apply + distribution + interner (steps 4–6).** Wires the plan into the shipped transactional-outbox + delta-sync path; mostly assembly, the correctness gate is the `delta ≡ rebuild across migration` test. Includes replacing the blind `put_model` overwrite (`api/datastore.rs:221-230`) with a migration-routed guard.
- **Phase 3 (S–M) — Provenance + versioned rollback (steps 7–8).** Small additive field on `DecisionLogEntry`; rollback reuses the whole pipeline via inverse transforms.

Overall **L** for the arc (Product review rates F6 as L), front-loaded on Phase 1's correctness design. Each phase is independently shippable and independently valuable (dry-run alone is useful before apply exists).

---

## 8. Key decisions (ADR-style)

**ADR-1: Online vs offline migration.**
- *Context:* A model change touches records that agents are actively serving decisions against; the data plane is "eventually consistent with bounded staleness" (DATA_PLANE_PLAN §4.4), and agents fail-safe on the last good store.
- *Options:* (a) **Online** — apply atomically in the control-plane DB, publish, let the fleet converge via the existing gap-proof delta/snapshot sync while agents keep serving the pre-migration store until they swap atomically (same zero-downtime pattern as policy/data hot-swap). (b) **Offline** — freeze writes, drain, migrate, resume.
- *Decision:* **Online.** The apply is a single control-plane DB transaction (instantaneous, consistent source of truth); distribution is already atomic-swap + gap-proof, so no agent ever serves a half-migrated store. Brief bounded staleness during convergence is the same guarantee the data plane already documents and is acceptable for a shape change. Offline is unnecessary and would violate the zero-downtime promise.
- *Consequence:* No write-freeze; the only ordering guarantee is per-agent atomic swap, which the delta protocol already provides. Read-your-writes across a migration is out of scope (same as zookies — future).

**ADR-2: Typed closed transform set vs free-form migration scripts.**
- *Options:* (a) closed enum of typed transforms with known inverses and known record effects; (b) user-supplied transformation scripts/SQL.
- *Decision:* **Closed typed set for v1.** It makes dry-run impact analysis tractable and correct, makes inverses (rollback) mechanical, and keeps the type-strict contract enforceable. Free-form scripts defeat impact analysis and reintroduce the "mistyped value breaks at eval" hazard the ADM exists to prevent.
- *Consequence:* Some exotic reshape isn't expressible in v1; those are handled as export→transform→re-import (bulk import already exists, DATA_PLANE_PLAN §3.2). The closed set covers the F6-named cases (rename role, add relation, retype attribute).

**ADR-3: Rollback as a new forward inverse migration vs in-place revert.**
- *Decision:* **Forward inverse migration** — history is append-only (`adm_model_versions`), matching the append-only migration discipline already in `connection.rs`. Audit sees the rename and its undo as two events, never a rewritten past.

**ADR-4: Retype coercion policy.**
- *Decision:* A retype that can't be losslessly coerced (`string→int` with `"high"`) **fails closed in dry-run**, listing offending records; the operator supplies an explicit coercion/default or fixes the data first. Never silently drop or zero a value — a silent coercion in an authorization datastore is an access incident.

---

## 9. Risks & rollback

- **Risk: dry-run impact under-reports** (says "0 access change" when access actually changes) — the worst failure, because it green-lights a harmful migration. *Mitigation:* the impact number is computed by the *same* materialize + real-engine decision battery as production, and pinned by the differential test (step 3 DoD); a rename that isn't decision-neutral is treated as a test failure, not a feature. *Rollback:* if a shipped migration is found harmful, run the inverse migration (step 8) — the fleet re-converges via the same delta path.
- **Risk: partial apply** leaves records migrated but model not bumped (or vice versa), corrupting the outbox lineage. *Mitigation:* single-transaction apply (step 4) with the outbox append inside it — the shipped D2 invariant. *Rollback:* transaction rollback leaves the pre-migration state exactly intact.
- **Risk: a huge rename floods the change log** past the compaction floor. *Mitigation:* the existing `snapshot_required` → full-sync path handles it (DATA_PLANE_PLAN §8); no new machinery. *Rollback:* n/a — self-healing.
- **Risk: interner leak** on rename if old strings aren't released. *Mitigation:* step 6 verification against the refcounted interner; targeted test. *Rollback:* bounded by the interner's eviction, but the test gates it.
- **Risk: irreversible transform (`RemoveAttribute`) loses data that rollback can't restore.** *Mitigation:* require an explicit acknowledgement + optional pre-migration export; `inverse()` returns `None` and rollback demands a backfill. *Rollback:* restore the attribute values from the pre-migration data version (`adm_versions.document` is immutable) — the versioned document is the safety net, and ties into the DATA_PLANE_PLAN §6 PITR direction.
- **Risk: decisions straddling a migration are misattributed in audit.** *Mitigation:* `model_version` on every decision entry (step 7) makes the model shape explicit; combined with the existing `data_version`/`data_checksum`, an auditor can pin exactly which model + data a decision saw.
- **Overall rollback posture:** the control-plane apply is a single reversible transaction; the model history is append-only; every published data version is immutable; the fleet distribution path is idempotent and gap-proof — so any migration can be undone by a forward inverse migration with no data-plane surgery, and a mid-flight failure never leaves a half-migrated agent.
