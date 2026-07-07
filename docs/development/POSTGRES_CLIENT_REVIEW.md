# PostgreSQL Client — Design & Principal Data Engineer Review

Status: **SHIPPED** — full suite green on SQLite and PostgreSQL 16.13
(196 lib + 28 integration + process-level data-plane E2E on both).

## What was built

One database client, two backends, **one query codebase**:

- `Database` wraps a single `sqlx::AnyPool`. Backend is selected at boot
  by `REAPER_DATABASE_TYPE` (`sqlite` default, `postgres` for production
  / self-hosted). No repository knows or cares which backend it runs on.
- Every SQL literal uses `$n` placeholders — the one placeholder syntax
  sqlx binds natively on **both** SQLite and PostgreSQL. Dynamically
  assembled queries (audit filters, partial UPDATEs, datastore list
  filters) build with `?` fragments and are normalized once at the end by
  `db::numbered_placeholders()`, so fragment order can never misnumber
  bindings.
- SQLite session tuning unchanged and applied per-connection via
  `after_connect`: WAL, `synchronous=NORMAL`, 5s `busy_timeout`,
  `foreign_keys=ON`.

### Bootstrap: standing up the database

The binary **is** the bootstrap tool. Point `REAPER_DATABASE_URL` at an
empty PostgreSQL database and start the server (or run any test):
migrations apply automatically, versioned and recorded.

```bash
createdb reaper
REAPER_DATABASE_TYPE=postgres \
REAPER_DATABASE_URL=postgres://reaper:…@db.internal:5432/reaper \
reaper-management
```

Hosted mode keeps the same behavior with zero user-visible database
(SQLite file); self-hosted mode is the two env vars above.

### Schema versioning tool — research and choice

Evaluated (2025-era landscape):

| Tool | Verdict |
|------|---------|
| **sqlx embedded migrator (chosen, custom-hardened)** | Migrations compiled into the binary (`include_str!`), applied in-transaction, recorded in `_reaper_migrations` with SHA-256 checksums. Zero external tooling for operators — the binary self-bootstraps any empty database. |
| refinery | Same embedded model, but a separate dependency wrapping the same ideas; our hardened runner is ~120 lines and keeps drift policy in our hands. |
| Atlas (ariga/atlas) | Best-in-class *declarative* schema-as-code with diffing/linting. Valuable as an optional CI layer later (lint PRs against the live schema), but a Go binary dependency is wrong as the *runtime* bootstrap for a self-hosted Rust product. |
| Flyway/Liquibase | JVM heritage — exactly the footprint Reaper exists to avoid. |

Policy encoded in the runner:

- **Append-only**: editing a shipped migration is a **hard startup
  error** (stored checksum ≠ compiled checksum → refuse to boot). Schema
  drift is surfaced, never silently absorbed.
- **Per-migration transactions**: PostgreSQL DDL is transactional; a
  failed migration leaves the database exactly at version N−1.
- Statement splitting respects single-quoted strings and strips `--`
  comments (unit-tested), so a `;` inside either can't bisect a statement.
- SQLite keeps its legacy idempotent runner — existing local databases
  upgrade exactly as before; no migration-table retrofit needed.

## Transactionality review

- **Outbox stays atomic on both backends.** All ADM mutations run
  `mutation + change_seq bump + adm_changes append` inside one
  `pool.begin()` transaction (`record_changes_in(&mut Transaction)`).
  This property was pinned by the process-level E2E and re-verified on
  PostgreSQL — a crash between statements cannot produce a change the
  replicas never hear about.
- **Sequence allocation**: `UPDATE datastores SET change_seq = change_seq
  + N … RETURNING change_seq` serializes writers **per datastore row**.
  On PostgreSQL this is a row lock held to commit — intentional: per-seq
  gaps/reorders would break replica contiguity checks. Cross-datastore
  writes don't contend. At control-plane rates (UI/API-driven, not
  1000 TPS) the measured cost is invisible; if a tenant ever needs more,
  the fix is batching (already supported — one bump covers N marks), not
  weaker ordering.
- **Publish** materializes + inserts the version row + compacts old
  changes in one transaction pinned to the current `change_seq`.
- **PostgreSQL default isolation (read committed) is sufficient**: every
  invariant that matters (seq monotonicity, outbox atomicity) is enforced
  by row locks and single-statement atomicity, not by snapshot
  assumptions. No SERIALIZABLE retry loops needed.

## Type-boundary rules (the cross-backend contract)

Flushed out by running the entire suite against real PostgreSQL — each of
these was a live bug SQLite's type coercion had been hiding:

1. **Booleans are INTEGER 0/1 end-to-end.** sqlx::Any surfaces SQLite
   INTEGER as i64 (decoding `bool` panics) and PostgreSQL rejects a bool
   param on an INTEGER column. Bind `as i64`, decode `i64 != 0`.
2. **No text params on numeric columns.** SQLite affinity coerces
   `'42'` → 42; PostgreSQL type-errors. Dynamic builders whose binding
   vectors are `Vec<String>` inline numeric literals instead (program
   constants only — no injection surface).
3. **Floating point columns are DOUBLE PRECISION**, never `REAL`
   (float4 on PG, undecodable into f64; SQLite gives DOUBLE PRECISION the
   same REAL affinity, so nothing changes there).
4. Timestamps stay RFC-3339 TEXT, IDs stay TEXT UUIDs — portable,
   index-friendly, and the decision-log path never sorts on them in SQL
   beyond lexicographic RFC-3339 (which is correct).

## Controls

- **Drift refusal** (checksummed migrations) — see above.
- **Ephemeral per-test databases**: `REAPER_TEST_DATABASE_URL` makes
  every test provision its own uniquely-named PostgreSQL database
  (`db::ephemeral_test_config`), preserving the isolation of per-tempdir
  SQLite files under full parallelism. Unset → SQLite, unchanged.
- **Both-backend gate**: the identical suite (196 lib + 28 integration +
  process E2E incl. kill/respawn self-heal) runs on both backends; PG run
  requires only a local `initdb` + the env var.
- **Connection pooling**: `max_connections` from config;
  `acquire_timeout` 10s on PostgreSQL so a saturated pool degrades with
  errors, not unbounded queueing.

## Performance (measured, release build, full HTTP stack)

| Metric | SQLite (WAL) | PostgreSQL 16 (local) |
|---|---|---|
| Entity save (authed API, tx mutation+outbox) p50 | 1.06 ms | 2.25 ms |
| Entity save p99 | 18.8 ms | **5.1 ms** |
| Authed read control p50 | 0.55 ms | 0.81 ms |
| Publish (materialize 500 entities + checksum + version row) | 7 ms | 11 ms |

Reading the numbers like a data engineer:

- PostgreSQL's p50 is higher (TCP round-trips + real per-commit fsync vs
  SQLite's in-process WAL append), but its **p99 is 3.7× better** —
  group commit smooths the tail that SQLite's checkpointing spikes.
  Production SLOs live at p99; PG is the better production story.
- Hot-path hygiene carried over: `api_keys.key_hash` is indexed and
  `last_used_at` writes are throttled (60s) and spawned off-path, so
  authed reads don't pay a write commit on either backend.
- pg_notify adds one fire-and-forget statement to publish only.

## Cross-instance eventing (PostgreSQL LISTEN/NOTIFY)

`events_pg.rs`: publish → `pg_notify('reaper_events', payload)`; every
instance LISTENs and re-broadcasts foreign notifications into its local
SSE channel (own-instance echoes skipped by instance id). Notifications
are **wake-up hints, never truth** — replicas still pull deltas by
sequence number, so a dropped notification costs latency, not data.
`PgListener` reconnects and re-LISTENs automatically.

## Known limitations / deliberate deferrals

- **PG-native types**: JSONB for `model`/`settings`/`document` columns,
  TIMESTAMPTZ, native UUID columns. Deferred: they'd fork the schema per
  backend and buy little at control-plane rates. Revisit if we ever query
  *into* documents server-side (JSONB + GIN would then win big).
- **`adm_changes` partitioning / retention**: publish already compacts
  below the snapshot floor; time-based partitioning only matters past
  ~10⁷ retained rows per datastore.
- **Ephemeral test databases are not dropped** after runs (local
  instances only; names are `reaper_test_<uuid>`). A janitor query is one
  line if it ever matters in CI.
- Agent-side delta wake-ups still ride SSE; LISTEN/NOTIFY only fans out
  between management instances (agents don't get DB credentials — by
  design).
