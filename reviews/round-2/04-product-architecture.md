# Reaper — Product Architecture & Enterprise-Readiness Review (Persona 4, Round 2)

**Reviewer:** Principal Product Architect (dev-infra → enterprise GA), acting as paid external auditor
**Scope:** Re-review after the 12-plan remediation roadmap shipped (PRs #10–#48). Verify round-1 closures are real (not cosmetic); find what REMAINS or is newly gapped. Same scope as round 1: control-plane journey, Git link, distribution, environments/promotion, multi-tenancy, data-fork lifecycle, audit-as-a-product, enterprise table stakes, competitive frame. UI out of scope. Review-only; no code modified.
**Method:** Read `reviews/round-2/00-repo-map.md` and `reviews/04-product-architecture.md` (round-1 + D-A..D-E mini-designs), then traced current code at HEAD across `services/reaper-management/src/{api,auth,sync,billing,domain,decisions,replay,audit}`, `services/reaper-agent/src/handlers`, `crates/policy-engine/src/{decision_log,replay}`, `tools/reaper-cli`, `docs/deployment`. Trusted the pre-gathered evidence from prior verification passes but re-cited every load-bearing claim myself.

---

## VERDICT: READY WITH CONDITIONS (conditional GA for a regulated UK bank)

The roadmap is **real, not cosmetic.** Round 1's single P0 (no SSO/SCIM) is closed by a genuine IdP-agnostic session broker + SCIM 2.0 with deprovision-revokes-sessions (`auth/sso/broker.rs`, `api/scim/users.rs`). Every round-1 P1 — dead GitOps, wrong-shape Git link, no environment model, no HA/DR, no migration engine, reproduction-only replay — is now materially addressed with well-factored code. **No P0 remains.** What blocks unconditional bank deployment is a tight cluster of P1 completions on otherwise-strong pillars: the flagship distribution pillar can detect but not **autonomously act** on a bad rollout (auto-rollback is operator-poll only); audit egress is still NDJSON/JSON-only with **no native SIEM connectors**; there is **no GDPR erasure endpoint** (a UK DPA-2018 hard gate); and the **air-gap sneakernet workflow is unsigned/missing** despite the signing primitives existing. These are finishing work on capabilities that are 70–90% built, not new pillars — hence *conditional*, not *not ready*.

**Counts:** P0 = 0 · P1 = 4 · P2 = 7 · P3 = 5

---

## Executive summary (≤10)

1. **Round-1 P0 closed.** SSO is real and correctly shaped: one broker funnels all protocols, find-or-creates by `(issuer, subject)`, reconciles org role from IdP groups on every login, and — structurally — no IdP group can confer platform `admin` (`auth/sso/broker.rs:50-141,150-169,243-262`). SCIM 2.0 deprovision removes membership **and revokes sessions** (`api/scim/users.rs:5,300-341`). This activates the already-comprehensive management-action audit log.
2. **GitOps is now wired and correctly shaped.** The sync loop is actually spawned (`main.rs:143-148`), `trigger_sync` really triggers (`api/sources.rs:410-447`), auth is a **GitHub App installation token**, not a user's `repo`-scoped PAT (`sync/github_app.rs:6-14,45-62`; `domain/source.rs:125-133`), and there is a genuine **UI↔git conflict model** (`ConflictMode::{CommitBack,ReadOnly,LastWriterWins}`, default commit-back, `domain/source.rs:148-163`, `api/policies.rs:272-329`) plus first-class **drift detection** (`sync/drift.rs`).
3. **Environments + promotion are first-class.** Env objects with `tier_order`, a `can_promote_to` guard, `POST /environments/{env}/promote` with `from_env`, and approve/reject change requests (`api/change_requests.rs:92-131,221,291`; `domain/{environment,promotion,change_request}.rs`). Regulated dev→stage→prod with distinct approvers is modeled, not conventional.
4. **Control-plane HA/DR shipped as an operational reality.** RPO ≤5min / RTO ≤30min, managed-vs-CloudNativePG ADR, PITR, and nightly restore-verification (`docs/deployment/CONTROL_PLANE_HA_DR.md:21-32,37-57,93-153`; `deploy/kubernetes/postgres-cnpg.yaml`, `postgres-restore-check.yaml`). First game-day is honestly deferred.
5. **Counterfactual replay is solid** — real policy/data counterfactual through the actual serving path (`engine.evaluate_set`), full-request fidelity from `replay_input`, flip diff (`crates/policy-engine/src/replay/mod.rs:54-126,456`). Caveats: capture-time opt-in (`decision_log.rs:640-641`), 100k scan cap, **jobs are ephemeral in-memory** (lost on control-plane restart, `replay/mod.rs:22,173-174`), DSL replay fails closed.
6. **P1 — distribution can see a bad rollout but not stop it.** Auto-rollback config + `check_rollback_trigger` exist (`api/deployments/rollback_config.rs:274-364`) but **nothing invokes them autonomously**; the four background loops in `main.rs` are pg-bridge, source-sync, change-log sweep, audit-retention, idempotency-sweep — no rollout supervisor. A bad policy spiking denials at a bank waits for a human to poll an endpoint.
7. **P1 — audit egress is single-shape.** Export is NDJSON/JSON only (`handlers/decisions.rs:232-260`); Vector→ClickHouse with the S3 sink commented out (`deploy/decision-logs/vector.toml:57-105`); no native Kafka / Splunk-HEC / CEF / OCSF. A bank SOC integrates by name-brand connector.
8. **P1 — no GDPR/DPA subject-erasure endpoint** (`grep erasure|gdpr|forget` → 0 product hits). Retention + legal-hold are solid (`api/audit.rs`, migration 014) but erase-by-subject-on-demand is unbuilt — a UK legal hard gate.
9. **P1 — air-gap sneakernet is effectively missing.** Signing primitives and `keygen` exist (`reaper-core::bundle_signing`, CLI `main.rs:158-164`) but `compile` doesn't sign (`main.rs:689-738`) and there is no signed `bundle export`/`import` CLI. Distribution assumes network reachability to the plane.
10. **Net:** the enterprise wrapper that was ~30% in round 1 is now ~80%. Remaining work is finishing 4 P1s on built pillars + inventory/hardening P2s — a weeks-to-a-quarter list, not a rebuild.

---

## Journey walkthrough (org create → propagation)

| Step | Status | Location |
|---|---|---|
| Org create + tenancy root | exists-solid | `api/orgs.rs` (`resolve_org`, create); org-scope enforced per-handler `user.org_id == org.id` (e.g. `api/environments.rs:1-6`) |
| Human identity / SSO login | exists-solid | `auth/sso/broker.rs:50-141`; SCIM lifecycle `api/scim/users.rs:300-341` |
| Org RBAC for Reaper's own admin surface | exists-solid (coarse) | `OrgRole {Viewer,Developer,Admin,Owner}` → scopes; `auth/scopes.rs:11-58`; **no per-namespace/per-env role binding** (deferred, rejected loudly at API) |
| Policy source setup (BYO git / managed) | exists-solid | `api/sources.rs` create + `trigger_sync:410-447`; GitHub App tokens `sync/github_app.rs:45-62`; scheduler `main.rs:143-148` |
| Branch/dir-per-env mapping | exists-weak | `domain/source.rs:101-102` single `branch` per source; no explicit dir-per-env or branch→env map field — modeled as one-source-per-env by convention |
| Model the data (ADM: RBAC/ABAC/ReBAC) | exists-solid | `domain/datastore.rs:171 seed_model`; ADM CRUD `api/datastore.rs`; multi-model store `crates/policy-engine/src/data/{rbac,relationships,store}.rs` |
| Model migration (rename/type-change/relation) | exists-solid | Plan 12: `domain/migration.rs` typed transforms + planner + `compose_rollback`; `api/datastore.rs` plan/apply/history/rollback; migration 0015 model-version provenance |
| Author policy (Simple/Cedar/DSL) | exists-solid | `evaluators/{simple,cedar,reaper_dsl}`; hot path `agent/handlers/evaluate.rs` |
| Deploy to data plane (publish + sync) | exists-solid | data full deploy `agent/handlers/data.rs deploy-version`; delta `apply-deltas` (contiguity-enforced 409 self-heal); outbox `db/repositories/datastore.rs:199,218-271` |
| Rollout strategy / canary / waves / approval | exists-solid | `api/deployments/*`, `deployment.rs:15-95,245-257`; agent-confirmed convergence `service/mod.rs:64-199` (`require_agent_confirmation` default true) |
| Promote env→env with approval | exists-solid | `api/change_requests.rs:92-131` (`can_promote_to` tier guard :128), approve/reject :221/:291; freeze windows + ServiceNow `integrations/servicenow.rs` |
| Propagation to fleet (push + fallback) | exists-weak | SSE + `pg_notify` bridge `events_pg.rs:115` **only propagates `datastore_published`** cross-instance; bundle/rollout events not bridged; SSE fan-out unsharded (`api/events.rs`) → poll-reliant at fleet scale |
| Autonomous safety (auto-rollback on error spike) | **missing** | config + `check_rollback_trigger` `api/deployments/rollback_config.rs:274-364` but no supervising loop (`main.rs:131-234`) |
| Fleet inventory (what version is everything running) | exists-weak | bundle inventory solid `landscape/service.rs:42-97,222-259`; **data-version convergence per-agent only** `api/landscape.rs:134-139`, no fleet-wide data-version histogram, `version_pins` count TODO `service.rs:269` |
| Air-gap export/import (sneakernet) | **missing** | signing exists but `compile` unsigned `cli main.rs:689-738`; no `bundle export/import` |
| Subject erasure (GDPR) | **missing** | no endpoint; retention/legal-hold only |

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|---|---|---|---|---|---|
| R2-1 | **P1** | `api/deployments/rollback_config.rs:274-364`; `main.rs:131-234` | Auto-rollback is operator-poll only; no autonomous supervisor invokes `check_rollback_trigger` | Bad policy/data version spiking denials is not self-reverted; the flagship pillar is safe-to-detect but not safe-to-act | Build a supervised rollout control loop (design T-3) |
| R2-2 | **P1** | `handlers/decisions.rs:232-260`; `deploy/decision-logs/vector.toml:57-105` | Audit export is NDJSON/JSON only; no native Kafka/Splunk-HEC/CEF/OCSF; S3 sink commented out | Bank SOC onboarding blocked; "send to Splunk" is DIY glue | Ship Vector sink configs + OCSF mapping + push export API (design T-1) |
| R2-3 | **P1** | `crates/policy-engine`, `services/reaper-management` (grep 0 hits) | No GDPR/DPA subject-erasure-on-demand endpoint | UK DPA-2018 right-to-erasure gap; legal cannot honor a DSAR through the product | Build erase-by-subject over ClickHouse + DataStore + legal-hold guard (design T-5) |
| R2-4 | **P1** | `cli main.rs:689-738,158-164`; `reaper-core::bundle_signing` | No signed air-gap export/import; `compile` doesn't sign, deploy sends no signature field | Air-gapped bank enclaves cannot receive policy via sneakernet with verifiable provenance | Add `bundle export --sign` / `import --verify` + agent checksum report (design T-4) |
| R2-5 | **P2** | `api/landscape.rs:134-139`; `landscape/service.rs:269` | Fleet data-version convergence is per-agent, not a fleet-wide histogram; `version_pins` count is TODO; `current_bundle_version` is a checksum not semver | "Is every agent on the approved data version?" is not a first-class compliance answer | Build a fleet version inventory read model (design T-2) |
| R2-6 | **P2** | `bundles.rs:1082-1142` (`get_bundle_diff` preview-only) | Policy bundle distribution is full-deploy only; delta exists for DATA but not for POLICY bundles over the wire | Large policy sets re-ship in full to every agent; costly at fleet scale | Extend the data-plane delta idea to policy bundles (keyed by bundle SHA) |
| R2-7 | **P2** | `events_pg.rs:115`; `api/events.rs` | Cross-instance bridge only propagates `datastore_published`; bundle/rollout SSE not bridged; fan-out unsharded | At multi-instance fleet scale, bundle/rollout push degrades to poll; convergence slower but correct | Bridge all `ServerEvent` variants; shard SSE fan-out |
| R2-8 | **P2** | `replay/mod.rs:22,173-174` | Replay jobs are ephemeral in-memory; lost on control-plane restart | A long "impact of policy v7 over last month" run is not durable/resumable; poor for audit evidence | Persist replay jobs + results (table + object store) |
| R2-9 | **P2** | `billing/service.rs:274-306` | Plan-limit quotas are **advisory only** — `UsageMetrics` is hardcoded to 0, so `exceeded_limits` never trips; no create-time enforcement; no noisy-neighbor control | Multi-tenant SaaS offering has no real quota/isolation ceiling (single-tenant enterprise unaffected) | Wire real usage counts + enforce at agent-register / policy-create; add per-tenant rate ceilings |
| R2-10 | **P2** | `api/audit.rs` (chain/checkpoints); no shipped verifier | Audit hash-chain + checkpoints are real but the verifier is library-only; no external TSA/notarization | Bank auditor cannot independently verify tamper-evidence without running Rust code; no third-party time anchor | Ship a `reaper audit verify` CLI + optional RFC-3161 TSA anchoring |
| R2-11 | **P2** | `api/policies.rs:281-316`; `broker`/user email | Commit-back pushes **directly to the tracked branch** (no PR interposed) and attributes commits to synthetic `{user.id}@reaper` | UI edit lands on prod branch without PR review unless external branch protection catches it; git blame not tied to corp identity | Optional PR-mode commit-back; use SSO email for author (design T-6) |
| R2-12 | **P3** | `data fork` — none in `api/datastore.rs`/`domain/datastore.rs` beyond `seed_model:171` | No datastore backup/restore/PITR or test-data fork for lower envs (migration engine is solid, but data lifecycle isn't) | Seeding staging from a masked prod snapshot, or restoring last-Tuesday's data, is manual | Build datastore snapshot/fork/seed with masking (design T-7) |
| R2-13 | **P3** | `auth/scopes.rs:11-58`; `OrgRole` | Admin RBAC is org-wide role→scope; no per-namespace/per-environment scoped role bindings | A "prod-only approver" cannot be expressed; approver holds it org-wide | Scoped role bindings (already rejected loudly at API; revisit as real feature) |
| R2-14 | **P3** | `docs/` (SLO grep → perf/DB only) | No product-level control-plane SLO / error-budget doc; no support/diagnostics bundle tool | Enterprise support contracts and incident triage lack a defined SLO + one-shot diag capture | Define control-plane SLOs; add `reaper support-bundle` |
| R2-15 | **P3** | `crates/policy-engine/src/replay` (DSL path) | DSL replay fails closed | Counterfactual coverage is narrower for the flagship DSL than for other languages | Close the DSL replay path to parity |

---

## Detailed findings

### R2-1 (P1) — Distribution detects but does not act
The rollout machinery is the product's strongest pillar and it is genuinely fleet-grade: strategies, waves, approval gates, and **agent-confirmed convergence** (`deployment.rs:245-257`, `service/mod.rs:64-199`). Auto-rollback is *configured* — per-org and per-namespace error-rate thresholds, window, min-requests (`rollback_config.rs:108-114,231-237`) — and `check_rollback_trigger` computes `should_rollback = error_rate > threshold` from a real `summary.failure_rate()` (`:346-347`). But it is an **HTTP handler an operator must call.** I enumerated every `tokio::spawn` in `main.rs` (`:132,145,167,220`) plus `spawn_retention_sweeper` (`:211`): pg-event bridge, source-sync, change-log sweep, audit-retention, idempotency-sweep. There is no rollout supervisor. So the safety story is "we can tell you it's on fire if you ask us." For a bank running zero-downtime policy deploys, autonomous revert on a denial spike is table stakes — and here it is ~90% built (thresholds, window, the trigger evaluation, and the rollback action all exist), missing only the loop that ties them together. This is the highest risk-reduction-per-effort item in the review.

### R2-2 (P1) — Audit egress is single-shape
Capture is correctly off the hot path and analytics are solid (stats/timeseries/facets/top-denied, `decisions/mod.rs:700-787`). But egress is NDJSON/JSON export (`handlers/decisions.rs:232-260`) and a Vector→ClickHouse pipeline whose S3 sink is commented out (`vector.toml:57-105`). A regulated SOC does not accept "write NDJSON and build your own shipper" — they ask for a Splunk HEC token, a Kafka topic, or CEF/OCSF-normalized events into their SIEM. None exist (grep found only docs mentioning them). This is integration glue Vector can largely provide, plus an OCSF field map — medium effort, high sales impact.

### R2-3 (P1) — No subject erasure
Retention and legal-hold are real and correctly interact with the retention sweeper (`main.rs:208-211`, `api/audit.rs`, migration 014). But there is no "erase everything about data-subject X" operation across the decision store and DataStore. Under UK DPA 2018 / UK GDPR Art. 17 a controller must honor a DSAR erasure; a bank's DPO will ask for it in the questionnaire. It must be legal-hold-aware (a subject under hold is exempt) and itself audited. Buildable on existing provenance.

### R2-4 (P1) — Air-gap sneakernet missing
`reaper-core::bundle_signing` provides ed25519/ECDSA, and the CLI can generate keypairs (`main.rs:158-164`). Agents verify signed bundles on the push path (`policies.rs:370-399`). But `compile` produces an **unsigned** `.rbb` (`handle_compile`, `main.rs:689-738`) and CLI deploy sends no signature field (`main.rs:1476-1480`), and there is no `bundle export`/`import` verb. So the one workflow that matters for an air-gapped bank enclave — sign on the connected side, carry media across the gap, verify+import on the isolated side, report active checksum on reconnect — cannot be performed with the shipped tooling despite every primitive existing.

### R2-5 (P2) — Fleet version inventory is half-built
Bundle distribution inventory is solid: `BundleDistribution` histogram + `pending_update` drift (`landscape/service.rs:42-97,222-259`). The gap is **data-version convergence**: it is exposed per-agent (`api/landscape.rs:134-139`) with no fleet-wide histogram, `version_pins` count is a TODO (`service.rs:269`), and `current_bundle_version` is a checksum not a human/semver version. "Prove every agent is on approved data version N" is a compliance question that today requires scraping per-agent rows.

### R2-8 / R2-9 / R2-11 (P2)
- **Replay durability:** jobs live in an in-memory map (`replay/mod.rs:22,173-174`); a control-plane restart mid-run loses the job and its result — unacceptable as audit evidence of "we assessed the blast radius before shipping."
- **Quotas advisory:** `get_billing_summary` computes `exceeded_limits`, but `UsageMetrics` is hardcoded to all-zero (`billing/service.rs:274-284`) with a `// in production, query from metrics/database` comment, and Stripe is placeholder (`:157-165,197-209,320-337`). Limits never trip; there is no create-time enforcement and no per-tenant rate ceiling. Fine for single-tenant enterprise; a real gap for a multi-tenant SaaS SKU.
- **Commit-back shape:** `ConflictMode::CommitBack` does `commit_and_push` straight to the source branch (`policies.rs:294-306`), attributed to `{user.id}@reaper` (`:293`). For regulated change control you generally want the UI edit to open a **PR** for review, not land on `main`; and the author should be the SSO-verified corporate identity for git blame.

---

## Gap register

| Gap | Why it blocks enterprise | Proposed solution | Build/Buy/Integrate | Effort | Priority |
|---|---|---|---|---|---|
| Autonomous auto-rollback | Bad rollout not self-reverted; safety gap on flagship pillar | Supervised control loop consuming existing threshold + trigger + rollback action | Build (90% exists) | S | **P1** |
| SIEM/audit connectors | SOC onboarding by name-brand connector blocked | Vector Kafka/Splunk-HEC/S3 sinks + OCSF map + push export API | Integrate (Vector) + Build (OCSF/API) | M | **P1** |
| GDPR subject erasure | UK DPA-2018 legal hard gate | Erase-by-subject across ClickHouse + DataStore, legal-hold-aware, audited | Build | M | **P1** |
| Signed air-gap export/import | Air-gapped bank enclaves can't receive policy with provenance | `bundle export --sign` / `import --verify` + agent checksum report | Build (primitives exist) | S–M | **P1** |
| Fleet data-version inventory | Can't prove fleet convergence to approved version | Read model: per-agent + histogram of bundle/data versions, pins | Build | S–M | **P2** |
| Policy-bundle wire delta | Full re-ship to every agent at scale | Extend data-plane delta to policy bundles keyed by SHA | Build | M | **P2** |
| Cross-instance event bridge completeness | Bundle/rollout push degrades to poll at multi-instance scale | Bridge all `ServerEvent`s; shard SSE fan-out | Build | M | **P2** |
| Durable replay jobs | Impact runs lost on restart; weak as audit evidence | Persist jobs + results (table + object store) | Build | S | **P2** |
| Real quota enforcement | Multi-tenant SaaS has no isolation ceiling | Wire usage counts; enforce at register/create; per-tenant rate | Build | M | **P2** |
| Independent audit verifier + TSA | Auditor can't verify tamper-evidence without running code | `reaper audit verify` CLI + optional RFC-3161 anchoring | Build + Integrate | S–M | **P2** |
| Datastore backup/fork/seed | No PITR/test-data fork for lower envs | Snapshot/fork/seed with masking | Build | M–L | **P3** |
| Scoped role bindings | Can't express "prod-only approver" | Per-namespace/env role bindings | Build | M | **P3** |
| Control-plane SLOs + support bundle | Support contract + incident triage lack primitives | Define SLOs; `reaper support-bundle` | Build + Docs | S | **P3** |

---

## Proposed tooling mini-designs (remaining capabilities)

### T-1. Audit export & SIEM connectors (closes R2-2 / P1)
**Goal:** deliver decision logs into a bank's SIEM by native connector, not DIY glue, with a normalized schema auditors recognize.
**API sketch:**
- `POST /orgs/{org}/decisions/export` `{format: ndjson|cef|ocsf, sink: s3|kafka|splunk_hec, time_range, filter}` → async export job; returns `job_id`.
- `GET /orgs/{org}/decisions/export/{job_id}` → `{state, delivered, sink_ack}`.
- `CRUD /orgs/{org}/audit/sinks` → durable sink configs (endpoint, credential-ref, format).
**Data-model touchpoints:** new `audit_sinks`, `audit_export_jobs`; reuse `DecisionLogEntry` provenance fields; OCSF field-map table (Reaper decision → OCSF `Authorization Result` class 3002). Continuous streaming stays in Vector; add `kafka`, `splunk_hec`, and re-enable the `s3` sink in `vector.toml`.
**Composition:** the push API and Vector sinks read the same ClickHouse rows; OCSF mapping is a pure transform shared by both. Credentials are secret-refs, never inline.
**ADR trade-off:** *Vector sinks (integrate)* — fast, battle-tested, ops-owned — **vs in-process emitters (build)** — one less moving part but re-implements delivery/retry/backpressure. **Recommend Vector for streaming + a thin push API for on-demand/bounded exports;** OCSF mapping is ours either way.

### T-2. Fleet data-version inventory (closes R2-5 / P2)
**Goal:** a first-class, auditable answer to "what bundle AND data version is every agent running right now, and is the fleet converged?"
**API sketch:**
- `GET /orgs/{org}/fleet/versions` → `{bundle: histogram[{version, count}], data: histogram[{data_version, count}], converged: bool, stragglers: [agent_id]}`.
- `GET /orgs/{org}/fleet` → per-agent `{agent_id, env, active_bundle_version, active_data_version, target, drift, pins, last_seen, health}`.
**Data-model touchpoints:** denormalize `active_bundle_version` + `active_data_version` onto `agents` from the existing ack/confirm callback (truth already flows via `require_agent_confirmation`); resolve the `version_pins` TODO (`landscape/service.rs:269`); map checksum→named version so output is human-readable.
**Composition:** pure read model over `agent_deployments`, `agents`, pins, and data-plane `applied_seq`; no new capture. Feeds T-3's decision (converged vs stragglers) and the compliance export.
**ADR:** denormalize-on-ack (fast reads, tiny write cost) **vs** compute-on-query by scanning acks (always fresh, heavier). **Recommend denormalize** — fleet views are read-hot.

### T-3. Autonomous auto-rollback control loop (closes R2-1 / P1) — the flagship fix
**Goal:** a bad rollout self-reverts within a bounded window without a human in the loop, using the thresholds and trigger logic already built.
**API sketch (mostly internal):**
- New supervised task (leader-elected like the change-log sweeper, `main.rs:181-192`): for each in-flight rollout with `auto_rollback.enabled`, on each tick evaluate the **existing** `check_rollback_trigger` logic; if `should_rollback`, invoke the existing rollback action, freeze the rollout, emit `ServerEvent::AutoRollback`, and write an audit entry.
- `GET /orgs/{org}/deployments/{id}/rollback-status` → `{monitoring, current_error_rate, threshold, window_remaining, last_action}`.
- Config gains `auto_rollback.mode: monitor|enforce` so a bank can dry-run before arming.
**Data-model touchpoints:** reuse `rollback_configs`; add `rollout_supervisions` (rollout_id, window_start, samples, last_decision) for durability across restarts; new audit action `deployment.auto_rollback`.
**Composition:** the loop is the *only* new code — thresholds (`rollback_config.rs:108-114`), rate (`summary.failure_rate()`), trigger (`:346-347`), and the rollback action all exist. Leader-election pattern is already in the codebase. Error-rate signal comes from agent-confirmed metrics / decision buffer.
**ADR:** control-plane-driven supervisor (central, auditable, one brain) **vs** agent-side self-rollback (faster, survives partition, but decentralized policy). **Recommend control-plane supervisor** with `monitor` default → arm to `enforce` per namespace; it composes with the existing confirmation loop and keeps one decision authority.

### T-4. Signed air-gap export/import (closes R2-4 / P1)
**Goal:** move an approved, signed policy+data bundle across an air gap and verify provenance on the isolated side.
**API/CLI sketch:**
- `reaper bundle export <bundle_id|files> --sign --key <key> --data <snapshot> -o pkg.rbbx` → self-describing package: bundle + data snapshot + manifest + detached ed25519 signature + key_id.
- `reaper bundle import pkg.rbbx --verify --trust <pubkey>` → verifies signature+checksum, stages locally; agent reports active checksum on next reconnect (truth reconciles via T-2).
- Sign `compile` output too: add `--key`/`--key-id` to the `Compile` command (`cli main.rs:92-93`) so the artifact is signed at build.
**Data-model touchpoints:** none server-side for the offline path; on reconnect the agent's checksum report flows into the existing acknowledge callback and T-2 inventory. Package manifest records source bundle_id, data_version, signer key_id, created_at.
**Composition:** reuses `reaper-core::bundle_signing` (already used on the online push path, `policies.rs:370-399`) and the existing verified-load path — this is packaging + CLI, not new crypto.
**ADR:** one signed super-package (bundle+data together, simplest sneakernet) **vs** separately-signed bundle and data (independent rotation). **Recommend the super-package** for the air-gap UX, with per-part checksums inside the manifest so parts remain individually verifiable.

### T-5. GDPR/DPA subject erasure (closes R2-3 / P1)
**Goal:** honor a right-to-erasure DSAR through the product, safely and auditably.
**API sketch:**
- `POST /orgs/{org}/privacy/erasure` `{subject: {principal_id | attribute selector}, scope: decisions|data|both, reason}` → async job; **rejects if the subject is under legal hold** (409 with the hold id).
- `GET /orgs/{org}/privacy/erasure/{job_id}` → `{state, decisions_erased, data_entities_erased, holds_skipped}`.
- `GET /orgs/{org}/privacy/erasure` → history (the erasure is itself an audit event, retained even as the subject data goes).
**Data-model touchpoints:** ClickHouse `ALTER TABLE decisions DELETE WHERE principal = ...` (mutation) gated by legal-hold check; DataStore entity delete via existing `delete_entity` delta op (so it propagates to the fleet like any data change); new `erasure_jobs` table; new audit actions `privacy.erasure_requested|completed`. Erasure emits a data-plane change so agents converge.
**Composition:** reuses the retention sweeper's legal-hold interaction (`main.rs:208-211`) and the data-plane delta path. Erasure of an entity is just a delete propagated through the existing outbox.
**ADR:** hard-delete (true erasure, irreversible) **vs** crypto-shred (delete the per-subject key, leave ciphertext). **Recommend hard-delete for DataStore** (small, structured) and **crypto-shred option for the decision store** (append-only, high-volume) where physical deletion is expensive — both satisfy erasure, the second is cheaper at ClickHouse scale.

### T-6. UI↔git reconciliation hardening (closes R2-11 / P2)
**Goal:** make commit-back regulator-grade: reviewed, and attributed to a real identity.
**API sketch:**
- `ConflictMode::CommitBackPr` (new variant): a UI edit opens a branch + commit + **PR** via the GitHub App (`sync/github_app.rs`) instead of pushing to the tracked branch; deployment still only happens when the merged commit is synced (one lineage preserved).
- Author identity: use the SSO-verified email from `users` (`broker.rs` already resolves it) instead of `{user.id}@reaper` (`policies.rs:293`).
- `GET /orgs/{org}/sources/{id}/pending-edits` → open PRs raised by UI edits.
**Data-model touchpoints:** `sources.conflict_mode` gains `commit_back_pr`; `policy_edit_prs` (policy_id, pr_number, branch, author_user_id, state). Reuse `audit_log`.
**Composition:** slots beside the existing `CommitBack` arm (`policies.rs:281-316`); the App can already mint installation tokens, so PR creation is one API call. Drift detection (`sync/drift.rs`) and the sync materialization path are unchanged.
**ADR:** direct commit-back (immediate, relies on external branch protection) **vs** PR commit-back (review interposed, slower). **Recommend offering both**, default to PR for env tiers whose `ApprovalPolicy` requires a change record — reuse the environment approval policy already in `domain/environment.rs`.

### T-7. Datastore backup / fork / seed (closes R2-12 / P3)
**Goal:** PITR restore of the authorization data plane and masked test-data forks for lower envs.
**API sketch:**
- `POST /orgs/{org}/datastore/{ns}/snapshots` → point-in-time snapshot (reuses the binary bundle snapshot format `data/bundle.rs`); `POST .../restore {snapshot_id, at?}`.
- `POST /orgs/{org}/datastore/fork {from_ns, to_env, mask_profile}` → copy a namespace's data into a lower env with a masking transform applied.
**Data-model touchpoints:** `datastore_snapshots` (checksum, data_version, created_at, object-store ref); reuse `seed_model` (`domain/datastore.rs:171`) as the empty-fork baseline; masking profile references the existing `decision_privacy` masking primitives.
**Composition:** snapshots reuse the materialize + checksum machinery already used for agent full-deploy; fork = snapshot + masking transform + publish into the target env's namespace, then normal propagation.
**ADR:** logical snapshot (portable, maskable, model-aware) **vs** physical PG dump (fast, exact, not maskable/portable across model versions). **Recommend logical** for forks/seed (masking needs it) and lean on the **control-plane PITR** (already in HA/DR doc) for disaster restore.

---

## Sequenced NEXT roadmap (previous 12-plan roadmap is DONE)

**#1 most important next move — close the autonomous auto-rollback loop (T-3).** It is the single remaining safety gap on the product's strongest, most-differentiated pillar; it is ~90% built (thresholds, error-rate signal, trigger evaluation, and the rollback action all exist — only the supervising loop is missing); it has **no operational workaround** (a bad policy at a bank cannot wait for a human to poll an endpoint); and the leader-election pattern it needs already exists in `main.rs`. Highest risk-reduction per unit effort in the entire review. Ship it in `monitor` mode first, then arm `enforce` per namespace.

1. **T-3 Autonomous auto-rollback (P1, S).** Safety on the flagship pillar. Do first.
2. **T-5 GDPR erasure (P1, M).** Legal hard gate; parallelizable with T-3 (different subsystem).
3. **T-1 SIEM connectors (P1, M).** Unblocks SOC onboarding; mostly Vector config + OCSF map.
4. **T-4 Signed air-gap export/import (P1, S–M).** Primitives exist; packaging + CLI.
5. **T-2 Fleet version inventory (P2, S–M).** Feeds T-3 and compliance; cheap read model.
6. **T-6 PR commit-back + identity (P2, S).** Regulator-grade change control; one App call.
7. **R2-8 durable replay + R2-9 real quotas + R2-10 audit verifier/TSA (P2).** Hardening for evidence, SaaS SKU, and independent verification.
8. **T-7 datastore fork/seed + R2-6 policy-bundle delta + R2-7 event-bridge completeness + R2-14 SLOs/support-bundle (P2–P3).** Scale + lifecycle polish.

**Competitive frame (blunt, post-roadmap):** Reaper's *combination* is now genuinely hard to match. Against **OPA + OPAL / Styra DAS**: Reaper ships the managed multi-model data plane OPA makes your problem, *plus* env→env promotion with approvals, *plus* a model-migration engine none of them have — and now has the SSO/SCIM/HA-DR wrapper Styra uses to win enterprise deals. Against **OpenFGA / SpiceDB**: they give you ReBAC tuples but no attributes/roles and no policy-as-code DSL; Reaper spans RBAC+ABAC+ReBAC in one typed model with counterfactual replay. Against **Cedar / AWS Verified Permissions**: AVP passes entities per-request with no fleet sync, no off-path audit at this depth, and no distribution machinery; Reaper owns the whole loop (managed data → sub-µs enforcement → off-path audit → confirmation-driven distribution). **Where Reaper is now genuinely ahead:** the migration engine + counterfactual replay + agent-confirmed fleet convergence is a combination *no single competitor holds*. **Where it is still weaker:** operational maturity artifacts the incumbents ship as standard — native SIEM connectors (T-1), autonomous rollback (T-3), independently-verifiable audit (R2-10), and the multi-tenant SaaS controls (real quotas, R2-9). These are finishing work, not architecture — which is exactly why the verdict is *conditional GA*, not *not ready*.

---

## What's done well (≤5)

1. **The roadmap closed real gaps, not cosmetic ones.** SSO broker with a structural "no IdP group → platform admin" invariant (`broker.rs:150-169,243-262`) and SCIM deprovision-revokes-sessions (`scim/users.rs:300-341`) are the correct shapes, not checkbox stubs.
2. **The Git link was re-shaped correctly:** GitHub App installation tokens (no stored PAT), a first-class UI↔git conflict model with commit-back default, and queryable drift (`github_app.rs`, `source.rs:148-163`, `policies.rs:272-329`, `drift.rs`).
3. **Environments + promotion are a genuine governance spine** layered on the existing rollout machinery — tier-ordered promotion with a `can_promote_to` guard and approve/reject, reusing (not rewriting) the strong distribution core (`change_requests.rs:92-131`).
4. **Counterfactual replay runs through the real serving path** (`engine.evaluate_set`) with full-request fidelity and a flip diff — the "how many allows flip if we ship v7" question is now answerable (`replay/mod.rs:54-126,456`).
5. **HA/DR is treated as an operational reality, not a slide** — RPO/RTO targets, a managed-vs-self-hosted ADR, PITR, and *nightly restore verification* ("a backup that has not been restored is not a backup"), with the first game-day honestly marked deferred (`CONTROL_PLANE_HA_DR.md`).

---

## Coverage statement
**Covered:** full journey traced through current routes (org→SSO→source→model→data-plane deploy→env promotion→propagation); Git link (App auth, sync wiring, drift, commit-back conflict model); distribution (SSE + pg bridge + confirmation + rollback config + pins); environments/promotion/approvals; multi-tenancy (org→namespace→environment, coarse RBAC, advisory quotas); data-fork lifecycle (migration engine solid; backup/fork missing); audit-as-product (capture→ship→query→replay→export→retention/legal-hold→erasure gap); enterprise table stakes (SSO/SCIM/HA-DR verified real; SLO/support-bundle absent); competitive frame.
**Not covered (other personas / out of scope):** eval-engine internals and hot-path perf; deep auth/crypto/injection soundness (relied on repo map + prior passes where it intersected product); UI. Findings are from source inspection at current HEAD, cross-checked against the pre-gathered verification evidence; services not compiled/run.
