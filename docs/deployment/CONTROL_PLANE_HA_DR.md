# Control-Plane HA / DR

How to run the Reaper control plane — the `reaper-management` service and its
PostgreSQL source-of-truth — so it survives the loss of a node, an
availability zone, or the database itself, with numeric recovery targets.

This document covers **data durability** (database HA, backup, point-in-time
restore, restore verification — §§2–6), **control-plane redundancy** (§7),
and the **DR game-day** (§8). The zero-downtime upgrade procedure lives in
the companion `FLEET_UPGRADE_RUNBOOK.md`.

> Scope note: the **agents** are deliberately out of scope. They are stateless
> and fail safe — when the control plane is down they keep serving decisions
> from the last synced bundle (see `DATA_PLANE_PLAN.md` §5/§7: cold-start gate
> `REAPER_DATA_REQUIRE_SYNC`, staleness budget
> `REAPER_DATA_MAX_STALENESS_SECS`). Control-plane loss degrades *management*,
> never *authorization*.

---

## 1. Recovery targets

| Target | Value | Measured by |
|---|---|---|
| Automatic primary→standby failover | **≤ 60 s** | game-day: kill primary, time to writable |
| RPO (worst-case data loss, total primary loss) | **≤ 5 min** | WAL archive cadence + restore test |
| RTO (full control-plane restore into a clean environment) | **≤ 30 min** | game-day wall clock |
| Backup / WAL retention | **≥ 7 days** | PITR to any timestamp in window |
| Backup restore verification | **nightly, automated** | restore-check CronJob (§5) |

Backups must be **encrypted at rest** (platform-side on the managed path;
SSE / encrypted buckets on the self-hosted path) and restore-verified on a
schedule — a backup that has not been restored is not a backup.

---

## 2. ADR: managed vs self-hosted PostgreSQL HA

- **Context.** The shipped `deploy/kubernetes/postgres.yaml` is a
  single-replica StatefulSet with no WAL archiving and no backup — a single
  point of failure over the policy/audit source-of-truth. The data-plane plan
  already names managed Postgres as the canonical production store.
- **Options.**
  - **(a) Managed HA Postgres** — RDS/Aurora Multi-AZ, Cloud SQL HA, Azure
    Flexible Server zone-redundant. Automatic failover, PITR, encrypted
    backups and cross-AZ redundancy are platform features; near-zero
    operational surface; per-hour cost and cloud lock-in of the DB tier.
  - **(b) Self-hosted operator cluster** — CloudNativePG streaming-replication
    cluster (1 primary + 2 standbys), WAL archiving to object storage.
    Portable and air-gap capable; you own failover and backup correctness.
- **Decision.** **Managed HA is the recommended default production path.**
  The CloudNativePG cluster (`deploy/kubernetes/postgres-cnpg.yaml`) is the
  supported portable/self-hosted path. Both present the **same connection
  contract** to the application — a single primary URL (plus, later, an
  optional read-replica URL) — so there is no code fork in
  `services/reaper-management/src/db/connection.rs`.
- **Consequence.** Fastest route to the RPO/RTO targets for regulated buyers;
  self-hosting stays first-class, mirroring how Reaper already ships both
  SQLite (dev) and Postgres (prod).

The old single-replica `postgres.yaml` remains **for dev/demo only** and says
so in its header.

---

## 3. Path A — managed HA Postgres (recommended)

1. **Provision** a multi-AZ / zone-redundant instance:
   - AWS: RDS for PostgreSQL Multi-AZ (or Aurora PostgreSQL), automated
     backups **on**, retention ≥ 7 days, deletion protection on, storage
     encryption on (KMS), preferably a cross-region snapshot copy rule.
   - GCP: Cloud SQL for PostgreSQL with HA (regional) configuration, PITR
     (WAL archiving) enabled, automated backups ≥ 7 days.
   - Azure: Database for PostgreSQL Flexible Server, zone-redundant HA,
     PITR window ≥ 7 days.
2. **Wire the URL.** The application takes one URL; failover hides behind the
   platform's stable endpoint/DNS:
   - Helm: set `postgresql.enabled: false` and `externalDatabase.url` (or
     provide `management.secrets.existingSecret` containing
     `REAPER_DATABASE_URL`).
   - Kustomize: replace the `DATABASE_URL` in `postgres-secrets` with the
     managed endpoint and drop `postgres.yaml` from `resources`.
3. **Failover behavior.** Platform failover is typically 30–60 s of refused
   connections/DNS cutover. The management pool retries acquisition; brief
   5xx on *write* endpoints during the window is expected and bounded (pool
   hardening lands in Plan 11 Phase B). Agents are unaffected.
4. **PITR restore (managed).** Use the platform's restore-to-timestamp into a
   **new** instance, then point `DATABASE_URL` at it (never restore over the
   primary in place). Measure wall-clock for the RTO record.

---

## 4. Path B — self-hosted CloudNativePG cluster (portable)

Prerequisite: install the CloudNativePG operator (any current release):

```bash
kubectl apply --server-side -f \
  https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.24/releases/cnpg-1.24.1.yaml
```

Then apply the cluster + backup manifests:

```bash
kubectl apply -f deploy/kubernetes/postgres-cnpg.yaml
```

What `postgres-cnpg.yaml` gives you:

- **3-instance cluster** (`reaper-pg`): 1 primary + 2 streaming standbys,
  spread across nodes/zones via pod anti-affinity, automatic failover and
  self-healing handled by the operator. Failover is typically **10–40 s**.
- **Stable Services** managed by the operator:
  - `reaper-pg-rw` — always the current primary (**use this in
    `DATABASE_URL`**),
  - `reaper-pg-ro` — round-robin over standbys (future read-scaling),
  - `reaper-pg-r` — any instance.
- **Continuous WAL archiving + base backups** to S3-compatible object storage
  (`barmanObjectStore`), retention 14 days, with a nightly `ScheduledBackup`.
  WAL is shipped continuously (archived at least every 5 minutes via
  `archive_timeout`), which is what bounds the RPO at ≤ 5 min even if the
  whole cluster and its PVCs are lost.
- **Credentials** in the `reaper-pg-app` secret created by the operator
  (or supply your own via `bootstrap.initdb.secret`).

Point the app at it (kustomize example):

```yaml
DATABASE_URL: "postgres://reaper:<password>@reaper-pg-rw:5432/reaper_management"
```

### Failover test (Definition-of-Done: ≤ 60 s)

```bash
kubectl delete pod reaper-pg-1   # current primary
# watch promotion:
kubectl get cluster reaper-pg -w
# time from delete → "Cluster in healthy state" with a new primary; assert ≤ 60s.
```

The management API must recover writes with only transient errors and no
process restarts.

### PITR restore (self-hosted)

Restores go into a **new** cluster bootstrapped from the object store —
never in-place:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: reaper-pg-restore
spec:
  instances: 1
  bootstrap:
    recovery:
      source: reaper-pg
      recoveryTarget:
        targetTime: "2026-07-12 09:30:00+00"   # any timestamp in retention
  externalClusters:
    - name: reaper-pg
      barmanObjectStore:
        # same destinationPath/endpoint/credentials as the live cluster
        destinationPath: s3://reaper-pg-backups/
        endpointURL: https://<s3-endpoint>
        s3Credentials:
          accessKeyId:
            name: reaper-pg-backup-creds
            key: ACCESS_KEY_ID
          secretAccessKey:
            name: reaper-pg-backup-creds
            key: SECRET_ACCESS_KEY
```

Verification procedure (run this at least once, and in every game-day):

1. `INSERT` a known marker row; note the timestamp T.
2. Restore to T−1min → marker **absent**. Restore to T+1min → marker
   **present**.
3. Record: wall-clock of the restore (**RTO**) and the write-loss window
   (**RPO**).

---

## 5. Automated restore verification

`deploy/kubernetes/postgres-restore-check.yaml` is a nightly CronJob (03:30)
that proves the latest backup actually restores:

1. creates a throwaway single-instance CNPG cluster in the
   `reaper-restore-check` namespace, bootstrapped by `recovery` from the live
   cluster's object store (latest backup + WAL replay);
2. waits for the cluster to become healthy (bounded at 20 min — comfortably
   inside the 30 min RTO target, and the job fails loudly if exceeded);
3. runs a smoke query through the restored primary (row counts over
   `organizations` / `policies`, plus `SELECT 1`);
4. tears the throwaway cluster down, leaving a log line
   `restore-check: OK (restore=<seconds>s)` for alerting.

Alert on Job failure (`kube_job_status_failed` on
`reaper-pg-restore-check`) — a failed restore-check means the backups are
not trustworthy and MUST page someone, not sit in a dashboard.

On the managed path, use the platform's equivalent (e.g. AWS Backup restore
testing plans, or a scheduled pipeline that restores the latest snapshot to a
scratch instance and runs the same smoke query).

---

## 6. Connection contract (no code fork)

Both paths present the application with:

- **one primary URL** (`REAPER_DATABASE_URL` / helm `externalDatabase.url`) —
  the managed stable endpoint or the operator's `-rw` Service;
- *(Phase B)* an optional **read-replica URL** — the managed reader endpoint
  or the `-ro` Service — plus pool-level failover tolerance
  (health-check-on-acquire, bounded retry) in
  `services/reaper-management/src/db/connection.rs`.

The SQLite dev path is untouched.

---

## 7. Control-plane redundancy (Phase B)

The management service is safe to run at **N ≥ 2 replicas**:

- **Failover-aware pool** (`db/connection.rs`): Postgres connections are
  health-checked before hand-out (`test_before_acquire`), recycled on a
  30-minute lifetime / 10-minute idle timeout so nothing stays pinned to a
  demoted primary, acquire fails fast (5 s), and the initial connect retries
  with bounded backoff (1→16 s, ≈31 s total) so a replica booting mid-failover
  doesn't crash-loop. Optionally set `database.replica_url`
  (`REAPER_DATABASE_REPLICA_URL`) to the managed reader endpoint or the CNPG
  `-ro` Service — it opens a read pool (`Database::any_read_pool`) for future
  read-scaling; writes always go to the primary URL.
- **Migrations under concurrent boot**: `run_pg_migrations` takes a
  transaction-scoped Postgres advisory lock (`advisory_keys::MIGRATIONS`), so
  N replicas starting simultaneously against one database queue and exactly
  one applies each pending migration — the checksum drift guard is unchanged.
  The lock releases automatically on commit, drop, or process death.
  *Drill:* start 3 replicas against an empty database; one migrates, none
  error.
- **Background singletons**: the change-log retention sweeper elects a
  per-tick leader with `pg_try_advisory_xact_lock`
  (`advisory_keys::CHANGE_LOG_SWEEP`); non-leaders skip the tick. The prune
  itself is idempotent delete-by-range, so this removes redundant work, not a
  correctness hazard. On SQLite (single-process dev) the lock is reported
  `Unsupported` and the sweep just runs.
- **Sessions are DB-backed** (the `sessions` table), so login on replica A
  and an authenticated call on replica B work. *Drill:* run 2+ replicas
  behind the Service and exercise exactly that.
- **Zero-gap rollouts**: the Helm Deployment ships
  `maxUnavailable: 0 / maxSurge: 1` (`management.updateStrategy`) and a
  default **soft pod anti-affinity** spreading replicas across nodes
  (override with `management.affinity`). Combined with the existing HPA
  (min 2) and PDB (`minAvailable: 1`), a node drain cannot evict the last
  replica. *Drill:* 2-minute API load test while deleting one replica —
  zero failed requests.
- **The RWO PVC caveat**: `management.persistence` uses a `ReadWriteOnce`
  PVC that forces all replicas onto one node — the chart prints a NOTES
  warning when it detects multi-replica + RWO. For real node-loss redundancy
  set `persistence.enabled: false` and use shared bundle storage
  (`management.config.storageType: s3`), or a `ReadWriteMany` storage class.
  Postgres remains the source of truth; the volume only holds compiled
  bundle blobs.

## 8. DR game-day (quarterly)

A rehearsed, measured recovery — run at least quarterly and after any major
topology change. The fleet-upgrade happy path lives in
`FLEET_UPGRADE_RUNBOOK.md`; this section is the failure half. Each scenario
records **planned vs actual** numbers against §1 and files remediations for
any miss.

> Run scenarios 2–4 in a staging environment that mirrors production
> topology. Scenario 1 is safe to run in production if you have ≥ 2
> standbys and want a real answer.

**Scenario 1 — primary loss (target: failover ≤ 60 s).**
Kill the current Postgres primary (`kubectl delete pod reaper-pg-1`, or the
managed platform's failover test). Measure: time until a standby is
promoted and the management API accepts writes again. Expected: agents are
unaffected throughout (they serve from memory); management writes see only
transient errors that the hardened pool rides out.

**Scenario 2 — total database loss (target: RPO ≤ 5 min, RTO ≤ 30 min).**
Write a known marker row and note the time. Destroy the cluster including
its PVCs (managed path: treat the instance as lost). Restore from the
object-store backup into a clean cluster (§4 PITR procedure), point a fresh
`DATABASE_URL` at it, restart management. Measure: wall-clock to a serving
control plane (RTO) and the write-loss window around the marker (RPO).

**Scenario 3 — total control-plane loss (target: RTO ≤ 30 min; zero
authorization downtime).** Delete the management Deployment AND the
database. Re-deploy from Helm/manifests, restore the DB per scenario 2,
verify agents re-register and re-sync. A load generator on an agent must
show **zero outage-denied decisions** for the entire scenario — this is
the proof that control-plane loss degrades management, never
authorization.

**Scenario 4 — replica kill under load (target: zero failed API
requests).** 2-minute load test against the management API while deleting
one replica and draining its node. The PDB must hold the last replica; the
Service must shed to survivors without a failed request.

**Record for every run:**

| Scenario | Target | Actual | Pass | Remediation |
|---|---|---|---|---|
| 1 failover | ≤ 60 s | | | |
| 2 RPO / RTO | ≤ 5 min / ≤ 30 min | | | |
| 3 RTO / eval downtime | ≤ 30 min / 0 | | | |
| 4 failed requests | 0 | | | |

File every miss as a tracked issue before closing the game-day.
