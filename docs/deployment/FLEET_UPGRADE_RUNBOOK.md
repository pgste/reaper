# Fleet Upgrade Runbook — zero authorization downtime

The ordered, tested procedure for upgrading the whole Reaper fleet — control
plane and agents — **without ever dropping an authorization decision**. It
leans on machinery that already ships: the agent's atomic bundle hot-swap,
the cold-start/staleness readiness gates, and confirmed-convergence rollouts.

Companion documents: `CONTROL_PLANE_HA_DR.md` (HA topology, backup/PITR,
game-day), `OPERATIONS_GUIDE.md` (health checks, metrics, alerts).

## Why zero eval downtime is achievable

- **Agents never stop serving during their own upgrade.** A restarting agent
  pod's traffic shifts to its siblings; each agent hot-swaps bundles
  atomically in memory, and a **new** pod does not enter rotation until it
  has synced (`REAPER_DATA_REQUIRE_SYNC=true` keeps `/ready` at 503 until
  the first successful bundle+data sync).
- **Agents fail safe when the control plane is briefly away.** During the
  management roll, agents keep serving the last synced bundle; staleness is
  bounded by `REAPER_DATA_MAX_STALENESS_SECS`.
- **Rollouts complete on confirmation, not optimism.** The rollout engine
  reports complete only after every targeted agent has acknowledged the
  target bundle + data version.

## Pre-flight checklist (every upgrade)

1. **Durability checkpoint.** Managed DB: confirm the latest automated
   backup/PITR window covers "now" (take a manual snapshot for major
   upgrades). Self-hosted CNPG: `kubectl cnpg backup reaper-pg` (or apply an
   on-demand `Backup` CR) and confirm the nightly
   `reaper-pg-restore-check` Job last succeeded.
2. **Fleet health.** All management replicas Ready; `GET /health` green;
   agents heartbeating; no rollout currently in `in_progress` /
   `awaiting_approval` unless intentionally paused.
3. **Record versions.** Current management image tag, agent image tag,
   deployed bundle versions per namespace (`GET /api/v1/orgs/{org}/rollouts`)
   — this is the rollback line.
4. **Start the canary load.** Run a low-rate load generator against a
   representative agent (`POST /api/v1/messages` or `/api/v1/check`) for the
   entire window and alert on ANY non-2xx. This is the proof, not a
   formality.

## Step 1 — migrations

Migrations are embedded and run at boot behind a Postgres advisory lock, so
N replicas starting concurrently apply each migration exactly once (Plan 11
Phase B). Nothing to do beyond knowing the posture:

- Rolling a new image with pending migrations: the FIRST new pod migrates
  during startup; old pods keep serving on the old schema until they are
  replaced (write migrations to be backward-compatible one release back —
  additive columns/tables, no drops in the same release that stops using
  them).
- GitOps/enterprise alternative: run the migration in a pre-deploy Job from
  the same image (`reaper-management` migrates then exits via any command
  that constructs the DB), and gate the Deployment on Job success.

## Step 2 — roll the control plane

```bash
helm upgrade reaper deploy/helm/reaper -n reaper \
  --set management.image.tag=<NEW_TAG> --reuse-values
kubectl -n reaper rollout status deploy/<release>-management
```

The chart's defaults make this zero-gap: `maxUnavailable: 0 / maxSurge: 1`
(a new pod must be Ready before an old one terminates), PDB
`minAvailable: 1`, soft anti-affinity across nodes. **Acceptance: zero 5xx
on the management API for the whole roll** (watch your ingress/service
metrics or a second load generator on `GET /health` + a read endpoint).

Agents notice nothing: SSE connections reconnect to a surviving replica and
the Postgres LISTEN/NOTIFY bridge re-broadcasts publishes across instances.

## Step 3 — roll the agents, wave by wave

1. Ensure the readiness gates are on for the agent Deployment:
   `REAPER_DATA_REQUIRE_SYNC=true` (new pods stay out of rotation until
   synced) and a sane `REAPER_DATA_MAX_STALENESS_SECS`.
2. Rolling restart with surge, one wave at a time:
   ```bash
   kubectl -n reaper rollout restart deploy/<release>-agent
   kubectl -n reaper rollout status deploy/<release>-agent
   ```
   For large fleets, partition by node pool / zone labels and restart one
   partition at a time.
3. **Acceptance per wave:** the canary load generator records zero
   outage-denied responses; `kubectl get pods` shows new pods Ready (i.e.
   past the sync gate) before old ones terminated.

## Step 4 — converge and confirm

If the upgrade also ships a new bundle/policy version, drive it through the
normal rollout (or env promotion) machinery and wait for **confirmed
convergence** — the rollout is complete only when every targeted agent has
acked the target bundle + data version:

```bash
GET /api/v1/orgs/{org}/rollouts/{id}          # status: completed
GET /api/v1/orgs/{org}/rollouts/{id}/deployments  # per-agent acks
```

Do not declare the upgrade done on pod readiness alone.

## Rollback

- **Agents / policy:** version-pin the previous bundle (deployment pins API)
  or roll out the prior bundle version and wait for confirmed convergence.
  Agents keep serving throughout — rollback is also zero-downtime.
- **Control plane:** `helm rollback reaper <REV> -n reaper` — same zero-gap
  strategy applies on the way down. Schema note: because migrations are
  append-only and backward-compatible one release, the previous image runs
  fine against the migrated schema.
- **Database:** PITR restore-to-timestamp is the LAST resort (it is
  destructive for writes after the target time — see
  `CONTROL_PLANE_HA_DR.md` §4). Always snapshot before restoring; prefer
  roll-forward.

## Verification record

Keep a short record per upgrade (date, versions from pre-flight step 3,
canary-load result, rollout confirmation id, anomalies). The quarterly DR
game-day (`CONTROL_PLANE_HA_DR.md` §8) exercises the failure half of this
runbook; this file is the happy path.
