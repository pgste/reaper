# Control-Plane HA/DR & Fleet Upgrades

> **STATUS: ✅ SHIPPED** — landed via PRs #42–#44 (2026-07-12) across phases A–C.
> A (data durability): `docs/deployment/CONTROL_PLANE_HA_DR.md` with numeric
> targets (failover ≤60s, RPO ≤5min, RTO ≤30min, retention ≥7d) and the
> managed-vs-self-hosted ADR; CloudNativePG 3-instance cluster manifest with
> continuous WAL archiving (archive_timeout bounds RPO) + nightly
> ScheduledBackup; automated nightly restore-check CronJob that restores the
> latest backup into a throwaway cluster and smoke-queries it (fails loudly);
> old single-replica postgres.yaml demoted to DEV/DEMO ONLY; helm
> externalDatabase.url wired (was referenced but missing). B (redundancy):
> failover-aware pool (test_before_acquire, 30m lifetime/10m idle, 5s
> acquire, bounded connect backoff, optional REAPER_DATABASE_REPLICA_URL
> read pool), advisory-locked migrations under N concurrent replicas,
> change-log sweeper per-tick leader election via pg_try_advisory_xact_lock,
> zero-gap Helm rollouts (maxUnavailable:0/maxSurge:1, default soft pod
> anti-affinity, NOTES warning on multi-replica+RWO persistence). C:
> `FLEET_UPGRADE_RUNBOOK.md` (pre-flight durability checkpoint, migrate →
> roll plane → roll agents wave-wise behind the REQUIRE_SYNC readiness gate →
> confirmed-convergence completion, rollback paths) and the quarterly DR
> game-day script (§8: primary loss, total DB loss, total plane loss,
> replica-kill under load, with a planned-vs-actual record table). The
> runtime drills (failover timing, PITR restore, game-day numbers) are
> written as procedures to execute in a k8s environment — the exit
> checklist's measured RPO/RTO boxes tick on the first game-day run.

**Readiness gate:** Enterprise deployability / operational resilience (SS2/21, DORA) — currently a hard rejection at a bank architecture review board.
**Priority:** P1 (Product Architecture F5). Non-blocking for a design-partner PoC, blocking for a regulated production deployment.
**Findings closed:** Product F5 (no control-plane HA/DR/backup posture; single Postgres; no RPO/RTO; no fleet-upgrade-without-downtime runbook). Synthesis "Honourable mentions" — control-plane DR/HA/backup. Partially hardens the availability theme (§ availability) on the *plane* side only (the agent/eval-path `panic="abort"` items are out of scope here).

---

## 1. Goal

Make the Reaper **control plane** (the `reaper-management` service + its PostgreSQL source-of-truth) survive the loss of a node, an availability zone, or a bad database, and make it possible to **upgrade the whole fleet — control plane and agents — with zero authorization-evaluation downtime**, with all of it documented against numeric RPO/RTO targets and rehearsed in a DR game-day.

Concretely, close four gaps:
1. **Database HA + backup + PITR** — replace the single `replicas: 1` StatefulSet Postgres with a managed or replicated HA topology plus continuous archiving and point-in-time restore.
2. **Control-plane redundancy** — run the stateless management service as multiple replicas behind the HPA/PDB that already exist, and remove the last stateful assumptions (local PVC, in-process session/state).
3. **Fleet-upgrade runbook** — a documented, tested procedure that leans on the agent's already-correct zero-downtime hot-swap and confirmed-convergence rollout so agents never stop serving decisions during an upgrade.
4. **DR game-day/test plan** — a rehearsed, measured recovery of the control plane from backup, proving the RPO/RTO numbers.

**Non-goals:** multi-region active/active control plane (documented as future work); the eval-plane availability items (`panic="abort"`, DSL recursion) tracked separately; agent HA (agents are already stateless and fail-safe — they keep serving the last bundle when the plane is down, per DATA_PLANE_PLAN §5 and the cold-start/staleness gates).

---

## 2. Current state (evidence) — file:line

- **Single database pool, no replica/failover awareness.** `services/reaper-management/src/db/connection.rs:140-153` (`new_postgres`) builds one `AnyPoolOptions` pool against one `config.url` with `max_connections` and a 10s acquire timeout — no read-replica URL, no primary/standby split, no retry-on-failover. The pool is `Option<AnyPool>` on a `Clone` struct (`connection.rs:61-65`), shared per process.
- **Postgres ships as a single-replica StatefulSet with no backup.** `deploy/kubernetes/postgres.yaml:49-51` (`replicas: 1`), a single 20Gi `ReadWriteOnce` PVC (`:114-125`), `storageClassName: standard`, no WAL archiving, no `pg_basebackup`/`pgBackRest`/CronJob, and a hard-coded placeholder password (`:35` `change-this-password-in-production`). Liveness/readiness are only `pg_isready` (`:87-110`).
- **Helm ships one Postgres via the Bitnami subchart, no HA.** `deploy/helm/reaper/values.yaml:346-363` (`postgresql.enabled: true`, `primary.persistence 20Gi`) — no `readReplicas`, no `architecture: replication`, no backup values, password auto-generated with no rotation story.
- **Management deployment has a local PVC and mounts it read-write.** `deploy/helm/reaper/templates/management-deployment.yaml:60-62,83-90` mounts `management-...-storage` at `/var/lib/reaper/storage`; `management-pvc.yaml:10-18` is `ReadWriteOnce`. A `ReadWriteOnce` PVC pins the pod to one node and blocks clean multi-replica scale-out unless that storage is unused in Postgres mode. `values.yaml:75-95` shows `persistence` enabled.
- **HPA/PDB already exist but are not exercised for HA.** `management-hpa.yaml:1-34` (CPU/memory autoscaling, gated on `management.autoscaling.enabled`), `management-pdb.yaml:1-16` (`minAvailable`), `values.yaml:16` (`replicaCount: 2`), `:55-64` (autoscaling min/max, `minAvailable: 1`). The redundancy scaffolding is present but the service must be verified truly stateless (sessions, background sweepers) for it to be safe.
- **Background singletons in the management process.** `main.rs:144-173` spawns a change-log retention sweeper (per Product F2 review). Any such singleton must be leader-elected or idempotent before running N management replicas.
- **PG migrations run at startup, in-process.** `connection.rs:206-278` (`run_pg_migrations`) applies embedded migrations transactionally with a checksum drift guard — good, but N replicas booting concurrently will race the migrator; needs an advisory lock or a pre-deploy migration Job (see step 5).
- **Agents already fail safe when the plane is down.** DATA_PLANE_PLAN.md §5 + §7: agents serve the last bundle, poll fallback exists, cold-start gate (`REAPER_DATA_REQUIRE_SYNC`) and staleness budget (`REAPER_DATA_MAX_STALENESS_SECS`) are operator-tunable. Product review F5 confirms "the agents fail safe if the plane is down." This is the lever the fleet-upgrade runbook stands on.
- **No DR documentation.** No file under `docs/deployment/` covers HA/DR/backup/RPO/RTO or a fleet-upgrade runbook (Product F5 absence check).

---

## 3. Definition of Done — testable checkboxes (with RPO/RTO numbers)

**Database HA / backup / PITR**
- [ ] Postgres runs in an HA topology (managed multi-AZ, or a replicated operator cluster) with an automatic primary→standby failover measured at **≤ 60s** in a game-day.
- [ ] Continuous WAL archiving is enabled; a **point-in-time restore to any timestamp within the retention window (≥ 7 days)** is demonstrated end-to-end.
- [ ] **RPO ≤ 5 minutes** (worst-case data loss on total-primary loss), evidenced by WAL archive cadence + a restore test that loses ≤ 5 min of writes.
- [ ] **RTO ≤ 30 minutes** for full control-plane recovery from backup into a clean environment, measured in the game-day.
- [ ] Backups are encrypted at rest and their restore is verified automatically (a scheduled restore-check job), not just taken.

**Control-plane redundancy**
- [ ] Management runs with **≥ 2 replicas** across ≥ 2 nodes/zones; killing one replica causes **zero failed API requests** over a 2-minute load test (PDB `minAvailable ≥ 1` holds during a drain).
- [ ] The service is verified stateless: sessions validate against the DB (not in-process memory), the local PVC is removed or made non-load-bearing in Postgres mode, and every background singleton (change-log sweeper, `main.rs:144-173`) is leader-elected or idempotent under N replicas.
- [ ] Concurrent replica startup does not corrupt or race migrations (advisory-lock or pre-deploy Job proven by starting 3 replicas simultaneously against an empty DB).

**Fleet upgrade without eval downtime**
- [ ] A documented runbook upgrades the control plane (rolling, `maxUnavailable=0`/surge) with **zero 5xx** on the management API during the roll.
- [ ] Agents are upgraded via rolling restart while **continuously serving `/api/v1/check`** with **zero denied-by-outage responses** — proven by a load generator running throughout, relying on the atomic bundle hot-swap and the readiness/cold-start gates.
- [ ] The rollout only reports "complete" after **agent-confirmed convergence** (the existing confirmation loop), not optimistic completion.

**DR game-day**
- [ ] A written game-day plan exists in `docs/deployment/` and has been executed at least once, with recorded actual RPO/RTO vs targets and a remediation list.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

### Step 1 — Decide and document the Postgres HA posture (ADR)
- **What:** Choose managed HA Postgres (RDS/Aurora Multi-AZ, Cloud SQL HA, Azure Flexible Server zone-redundant) as the *recommended* production path, and a self-hosted operator cluster (CloudNativePG or Zalando `postgres-operator`, streaming replication) as the *portable* path. DATA_PLANE_PLAN.md "Persistence decision (any-cloud)" already commits to managed Postgres — make HA explicit.
- **Where:** New `docs/deployment/CONTROL_PLANE_HA_DR.md`; ADR block (see §8).
- **Verify:** Doc reviewed; both paths give the same connection contract to `connection.rs` (a single primary URL + optional replica URL), so no code fork.

### Step 2 — Backup + PITR + restore verification
- **What:** Managed path — enable automated backups + PITR + cross-AZ/region snapshot copy via infra (Terraform/console), retention ≥ 7 days. Self-hosted path — add `pgBackRest` (or the operator's built-in `Backup`/`ScheduledBackup` CRDs) with WAL archiving to object storage, plus a **scheduled restore-check** CronJob that restores the latest backup into a throwaway namespace and runs a smoke query.
- **Where:** `deploy/kubernetes/postgres.yaml` (replace the raw StatefulSet with operator CRDs or document that managed PG is used instead); `deploy/helm/reaper/values.yaml:346-363` (add `postgresql.backup.*`, or switch the subchart to an operator-backed values block); new `deploy/kubernetes/postgres-backup-cronjob.yaml`.
- **Verify:** Take a backup, write a known row, restore to a timestamp before the row, confirm absence; restore to after, confirm presence. Record wall-clock (RTO) and write-loss window (RPO).

### Step 3 — Replace single-replica DB with an HA cluster
- **What:** Managed path — point `DATABASE_URL` at the managed HA endpoint; nothing else changes. Self-hosted path — deploy a 3-node operator cluster (1 primary + 2 sync/async standbys) with automatic failover and a stable primary Service the app connects to.
- **Where:** `deploy/kubernetes/postgres.yaml` (retire `replicas: 1` StatefulSet in favor of operator CRD or a documented managed endpoint); `deploy/helm/reaper/values.yaml:346-363`.
- **Verify:** `kubectl delete pod <primary>`; confirm a standby is promoted ≤ 60s and the management API recovers writes with only transient errors. Confirm the app's pool reconnects (see step 4).

### Step 4 — Make the management pool failover-aware
- **What:** Add optional read-replica URL + connection retry/backoff so a primary failover surfaces as a brief retry, not a cascade of 500s. Set a conservative `test_before_acquire`/health check and shorter `acquire_timeout`. Keep the existing single-URL default (managed endpoints already hide failover behind one DNS name, so this is mostly resilience hardening + optional read-scaling).
- **Where:** `services/reaper-management/src/db/connection.rs:140-153` (`new_postgres`): add pool health-check-on-acquire and a reconnect-tolerant retry wrapper; `src/config` `DatabaseConfig` to accept an optional `replica_url`. Do **not** change the `AnyPool` query codebase — SQLite dev path stays identical.
- **Verify:** Unit/integration test that kills the DB mid-load and asserts the pool recovers without process restart. Confirm SQLite path unaffected (`connection.rs:84-137`).

### Step 5 — Guard migrations under N concurrent replicas
- **What:** Wrap `run_pg_migrations` in a Postgres advisory lock (`pg_advisory_lock`) so only one booting replica migrates at a time, OR move migrations to a Helm pre-install/pre-upgrade Job and have the app assert-schema-only at boot. The checksum drift guard (`connection.rs:231-244`) already prevents divergence; this prevents a startup race.
- **Where:** `services/reaper-management/src/db/connection.rs:206-278`; new `deploy/helm/reaper/templates/management-migrate-job.yaml` (optional path).
- **Verify:** Start 3 replicas simultaneously against an empty DB; exactly one applies migrations, none error, schema is correct.

### Step 6 — Prove and enforce management statelessness
- **What:** Audit the service for in-process state that breaks with N replicas: (a) sessions — confirm `rst_` sessions validate against the DB, not memory (they are DB-backed per the Product review's `sessions` table); (b) remove or make non-load-bearing the local PVC mount in Postgres mode (`management-deployment.yaml:60-62,83-90`, `management-pvc.yaml`), since `ReadWriteOnce` pins scheduling; (c) make the change-log retention sweeper (`main.rs:144-173`) safe under N replicas via a `pg_advisory_lock`-based leader election or fully-idempotent delete-by-range.
- **Where:** `services/reaper-management/src/main.rs:144-173`; `deploy/helm/reaper/templates/management-deployment.yaml`, `management-pvc.yaml`, `values.yaml:75-95` (gate `persistence.enabled` off when `database.type=postgres`).
- **Verify:** Run 3 replicas; confirm no double-execution of the sweeper (log/metric), sessions work across replicas (login on replica A, authenticated call served by replica B), no pod is unschedulable due to RWO storage.

### Step 7 — Turn on redundancy via the existing HPA/PDB and anti-affinity
- **What:** Set `management.autoscaling.enabled: true`, `minReplicas: 2`, `podDisruptionBudget.minAvailable: 1`, and add pod anti-affinity so replicas spread across nodes/zones (`values.yaml` already exposes `affinity`). Configure the Deployment rollout as `maxUnavailable: 0, maxSurge: 1` for zero-gap upgrades.
- **Where:** `deploy/helm/reaper/values.yaml:16,55-64` and `management-deployment.yaml:10-13` (add `strategy.rollingUpdate`); reuse `management-hpa.yaml`, `management-pdb.yaml` unchanged.
- **Verify:** `kubectl drain` a node; PDB blocks eviction of the last replica; 2-minute API load test shows zero failures.

### Step 8 — Author the fleet-upgrade runbook (control plane + agents)
- **What:** Document the ordered procedure: (1) DB backup/PITR checkpoint; (2) run migration Job / advisory-locked migrate; (3) rolling-upgrade management (`maxUnavailable=0`); (4) roll agents one wave at a time, relying on the **atomic bundle hot-swap** (agent keeps serving during restart) and the **cold-start/staleness gates** (`REAPER_DATA_REQUIRE_SYNC`, `/ready` 503 keeps un-synced pods out of rotation); (5) drive the agent roll through the **confirmed-convergence rollout** so completion means every agent acked the target bundle+data version, not optimism. Include rollback: version-pin the previous bundle and re-converge; DB rollback = restore-to-timestamp (destructive, last resort).
- **Where:** `docs/deployment/CONTROL_PLANE_HA_DR.md` (or a dedicated `FLEET_UPGRADE_RUNBOOK.md`); cross-reference `docs/deployment/OPERATIONS_GUIDE.md`.
- **Verify:** Execute the runbook in staging with a load generator hitting `/api/v1/check` throughout; assert zero outage-denied responses and rollout reports complete only after agent confirmation.

### Step 9 — DR game-day plan + first execution
- **What:** Write a game-day script: simulate total primary loss and total control-plane loss, restore from backup/PITR into a clean environment, measure actual RPO/RTO, and record gaps. Schedule it recurring (quarterly).
- **Where:** `docs/deployment/CONTROL_PLANE_HA_DR.md` (game-day section).
- **Verify:** First run completed; actual RPO ≤ 5 min and RTO ≤ 30 min, or a remediation backlog filed.

---

## 5. Dependencies

- **Infra/platform:** a managed HA Postgres offering *or* permission to run a Postgres operator (CloudNativePG/Zalando) + object storage for WAL/backups.
- **Session storage confirmation:** sessions must be DB-backed (they are, per the SSO/`sessions` table) — a prerequisite for multi-replica management. If any session state is in-process, that must be fixed first.
- **Confirmed-convergence rollout (already shipped):** `deployment/service/helpers.rs:240 require_agent_confirmation` and the strategies/waves machinery (Product review "done well" #1) — the fleet-upgrade runbook depends on it, no new build.
- **Agent fail-safe behavior (already shipped):** cold-start gate + staleness budget (DATA_PLANE_PLAN §7), bundle hot-swap — the runbook leans on these.
- **Adjacent, not blocking:** SSO/SCIM (plan for identity), supply-chain gates. This plan is orthogonal to the auth P0s but should not be sequenced ahead of them.

---

## 6. Testing & verification (incl. DR game-day)

1. **DB failover test:** kill primary, assert standby promotion ≤ 60s and app recovery (step 3/4 verify).
2. **PITR restore test:** write-known-row → restore-to-before/after → assert (step 2 verify); record RPO/RTO.
3. **Scheduled restore-check:** automated CronJob restores latest backup nightly into a throwaway namespace and smoke-queries — a backup that can't restore is not a backup.
4. **Replica-kill under load:** 2-min load test on the management API while deleting a replica; zero failed requests (PDB + anti-affinity hold).
5. **Concurrent-startup migration race:** 3 replicas against empty DB; exactly one migrates (step 5).
6. **Statelessness cross-replica test:** login on A, authenticated call on B; sweeper runs once across N (step 6).
7. **Fleet-upgrade zero-downtime test:** load generator on `/api/v1/check` across a full control-plane + agent roll; zero outage-denied responses; rollout completes only on agent confirmation (step 8).
8. **DR game-day (quarterly):** full simulated loss + restore into clean env; measured RPO/RTO recorded vs targets (step 9). This is the capstone acceptance test for the whole plan.

---

## 7. Effort & phasing — S/M/L

- **Phase A (M) — Data durability first:** steps 1, 2, 3 (HA topology + backup + PITR + restore verification). This alone closes the "restore last Tuesday's policy set" compliance question and removes the single-DB SPOF. Largest infra lift; mostly ops/manifests, minimal code.
- **Phase B (S–M) — Control-plane redundancy:** steps 4, 5, 6, 7. Small code changes in `connection.rs`/`main.rs`, mostly Helm/values wiring; the HPA/PDB already exist. Statelessness audit (step 6) is the main risk of surprise work.
- **Phase C (S) — Runbook + game-day:** steps 8, 9. Docs + a rehearsed procedure leaning on machinery that already exists (confirmed rollout, hot-swap). Low code, high compliance value.

Overall: **M** for the arc. Sequence A → B → C; C validates A and B.

---

## 8. Key decisions (ADR-style)

**ADR-1: Managed vs self-hosted Postgres HA.**
- *Context:* Single `replicas: 1` StatefulSet (`postgres.yaml:49-51`) is a SPOF over the policy/audit source-of-truth; DATA_PLANE_PLAN already names managed Postgres as canonical.
- *Options:* (a) **Managed HA** (RDS/Aurora Multi-AZ, Cloud SQL HA, Azure zone-redundant) — automatic failover, PITR, backups, encryption as platform features; near-zero ops; per-hour cost + cloud lock-in of the DB tier. (b) **Self-hosted operator** (CloudNativePG/Zalando) — portable, air-gap-capable, no per-hour managed premium; you own failover/backup correctness and on-call.
- *Decision:* **Recommend managed HA as the default production path; ship the operator path as the portable/self-hosted option.** Keep `connection.rs` seeing a single primary URL (+ optional replica) so there is no code fork between the two. Document both in `CONTROL_PLANE_HA_DR.md`.
- *Consequence:* Fastest path to RPO/RTO for the target buyer; self-host remains a first-class citizen (matches how Reaper already ships both SQLite dev and Postgres prod).

**ADR-2: Migration execution model under N replicas.**
- *Options:* (a) in-process advisory-locked migrator (`connection.rs:206-278` + `pg_advisory_lock`); (b) pre-deploy Helm Job, app boots assert-only.
- *Decision:* **Advisory lock now (smallest change, keeps the shipped checksum-drift guard), offer the Job as the enterprise/GitOps path.**
- *Consequence:* Safe concurrent boot without a hard dependency on Helm hooks; the Job path is available for teams that require migrations gated outside the app image.

**ADR-3: Local PVC on management in Postgres mode.**
- *Decision:* **Disable `management.persistence` when `database.type=postgres`** so the `ReadWriteOnce` PVC (`management-pvc.yaml`) stops pinning the pod and blocking clean multi-replica scale-out; keep it only for SQLite/dev.

---

## 9. Risks & rollback

- **Risk: multi-replica exposes hidden in-process state** (a background singleton double-runs, or session state is memory-local). *Mitigation:* step 6 statelessness audit is a hard gate before enabling `replicaCount ≥ 2`; leader-elect the sweeper. *Rollback:* scale management back to 1 replica (`replicaCount: 1`, `autoscaling.enabled: false`) — instantly restores today's behavior with no data change.
- **Risk: failover causes a burst of write errors** if the pool isn't reconnect-tolerant. *Mitigation:* step 4 retry/health-check. *Rollback:* revert `connection.rs` pool changes; single-URL pool behaves exactly as today.
- **Risk: PITR restore is destructive / loses recent writes up to RPO.** *Mitigation:* restore is the last-resort rollback; prefer roll-forward (version-pin previous bundle) for policy issues. Always snapshot before a restore. *Rollback of a bad restore:* restore again to a later timestamp within retention.
- **Risk: operator-based self-host adds operational surface** (failover bugs, backup misconfig). *Mitigation:* scheduled restore-check (step 2) catches silent backup failure; recommend managed for teams without DB on-call.
- **Risk: fleet upgrade still drops evals if an agent comes up un-synced.** *Mitigation:* the runbook mandates `REAPER_DATA_REQUIRE_SYNC` + `/ready` 503 gate so un-synced agents stay out of rotation, and rollout waits for confirmed convergence. *Rollback:* version-pin the prior bundle and re-converge; agents keep serving the last good bundle throughout.
- **Overall rollback posture:** every step is independently revertible to today's single-Postgres/single-replica state; the plan changes deployment topology and adds resilience, not the query codebase or data model, so there is no schema migration to unwind.
