# Reaper — Product Architecture & Enterprise-Readiness Review (Persona 4, Round 3)

**Reviewer:** Principal Product Architect (dev-infra → enterprise GA), paid external auditor
**Scope:** Third pass. Verify round-2's four remaining P1s actually shipped (not cosmetic); find what remains, what regressed, and what is *newly* wrong. Same product lens: control-plane journey, the Git link, distribution, environments/promotion, multi-tenancy, data-fork lifecycle, audit-as-a-product, enterprise table stakes, competitive frame. UI out of scope (journey assessed through APIs/backend). Review-only; no code modified.
**Method:** Read `reviews/round-3/00-repo-map.md`, `reviews/04-product-architecture.md`, `reviews/round-2/04-product-architecture.md`, `reviews/round-2/06-future-architecture.md`. Then traced current code at HEAD: `services/reaper-management/src/{deployment/supervisor.rs,siem,decisions/purge.rs,db/repositories/audit_erasure.rs,quota,replay,events_pg.rs,landscape,api/{audit,connectors,landscape,sources}.rs,domain/{agent_deployment,source}.rs}`, `services/reaper-agent/src/{handlers/evaluate.rs,capability_gate.rs}`, `crates/reaper-core/src/capability.rs`, `tools/reaper-cli/src/{airgap.rs,main.rs}`. Did not re-verify SSO/SCIM/environments/migration internals (proven real in round 2; spot-confirmed present, no regression signal); did not compile or run services.

---

## VERDICT: CONDITIONAL (conditional GA for a regulated bank)

The round-2 roadmap is, again, **real and not cosmetic.** All four round-2 P1s materially shipped: the autonomous auto-rollback supervisor is wired (`deployment/supervisor.rs`, `main.rs:215`), SIEM connectors exist (`siem/mod.rs`), signed air-gap export/import is a real CLI verb (`tools/reaper-cli/src/airgap.rs`), and GDPR subject-erasure is a thorough, legal-hold-aware, pseudonym-aware endpoint (`api/audit.rs:501+`, `db/repositories/audit_erasure.rs`). Multi-tenant quota enforcement (round-2 R2-9) is now wired at the create paths (`quota/mod.rs`, `api/policies.rs:209`, `api/agents.rs:180`). And — the headline of this round — the round-2 *future*-review's #1 strategic bet, **authorization for AI/agentic actors, has been built and wired into the hot eval path** (`crates/reaper-core/src/capability.rs`, `agent/handlers/evaluate.rs:203-219`, `capabilities` API, MCP gate).

**No P0 remains. No round-2 P1 remains open.** What holds the verdict at *conditional* rather than *ready* is a single **new P1 that the round-2 closure created**: the autonomous auto-rollback the product now advertises as its safety story fires on **bundle-apply failure rate** (agents that couldn't fetch/verify/install a bundle), **not on runtime decision quality** (`service/mod.rs:582`, `domain/agent_deployment.rs:141-146`). A syntactically-valid policy that deploys cleanly to 100% of agents and then denies every legitimate request — the most likely and most damaging failure at a bank — is invisible to the trigger and will *not* self-revert. The loop was wired; the wrong signal was connected to it. Everything else is P2/P3 finishing work on 80–95%-built capabilities.

**Counts:** P0 = 0 · P1 = 1 · P2 = 4 · P3 = 5

---

## Executive summary (≤10 lines)

1. **Round-2 P1s all closed, verified real.** Supervisor loop, SIEM connectors, signed air-gap, GDPR erasure — each is well-factored code wired into the running service, not a stub.
2. **NEW P1 — auto-rollback watches the wrong signal.** `evaluate_rollback_trigger` uses `DeploymentSummary::failure_rate()` = *failed agents / total agents* on the **deploy path** (`agent_deployment.rs:141`). Runtime allow/deny/eval-error data — which the decision buffer already captures — is never fed to the rollback trigger. Safe-to-act covers distribution failure, not policy-quality failure.
3. **Agentic authz shipped and hot-path-wired** — attenuated, expiring, signed capabilities (`capability.rs`), a hot-path `capability_gate::enforce` (`evaluate.rs:207`), context taint labels, a `capabilities` management API, and an MCP gate (`tools/reaper-mcp`). This lands Reaper on the one axis the round-2 future review said was its biggest opening. Genuinely differentiating.
4. **Quota enforcement is now real** (round-2 R2-9 closed): tier limits + per-org overrides enforced at agent-register and policy-create (`quota/mod.rs`, `api/{agents,policies}.rs`).
5. **P2 — SIEM streaming is still Vector-only.** The in-process connector (`siem/mod.rs`) is **on-demand/test push only** — no scheduler continuously ships new decisions; `ConnectorDeliveryService` is invoked only from the export/test handlers (`api/connectors.rs:499`), never from `main.rs`. No native Kafka connector (Vector-commented only). Continuous SOC feed is still DIY Vector.
6. **P2 — replay jobs still ephemeral** (round-2 R2-8 unclosed): `replay/mod.rs:21,188` — in-memory registry, lost on control-plane restart. Notably, erasure got a durable receipt table while replay did not.
7. **P2 — cross-instance event bridge still single-event** (round-2 R2-7 unclosed): `events_pg.rs:115` bridges only `datastore_published`; bundle/rollout SSE is not bridged, so at multi-instance scale those pushes degrade to poll.
8. **P2 — fleet convergence answer still half-built** (round-2 R2-5): bundle distribution histogram exists, but **no fleet-wide data-version convergence** view and `version_pins` count is still a TODO (`landscape/service.rs:269`). "Prove every agent is on approved version N" still requires scraping per-agent rows.
9. **P3 residue:** branch/dir-per-env still one-branch-one-path per source (`domain/source.rs:102,105`); PR commit-back mode still absent (R2-11); datastore backup/fork/seed still absent (R2-12); scoped per-env admin role bindings still absent (R2-13); no control-plane SLO doc or support-bundle tool (R2-14).
10. **Net:** the enterprise wrapper that was ~30% (r1) → ~80% (r2) is now ~90%. The one thing between here and a clean regulated deploy is making auto-rollback react to *policy behaviour*, not just *distribution success*.

---

## Journey walkthrough (org create → propagation)

| Step | Status | Evidence / change since round 2 |
|---|---|---|
| Org create + tenancy root | exists-solid | `api/orgs.rs`; org-scope enforced per handler |
| Human SSO login + SCIM lifecycle | exists-solid | `auth/sso/broker.rs`, `api/scim/users.rs` (verified real r2; no regression) |
| Org RBAC for Reaper's own admin surface | exists-solid (coarse) | `auth/scopes.rs`; **still no per-namespace/per-env role binding** (R2-13 open); note new dedicated `audit:erase` scope for separation of duties (`api/audit.rs:76-88`) |
| Multi-tenant quota / noisy-neighbour | **now exists** | `quota/mod.rs` real usage counts + tier limits + overrides; enforced at create (`api/policies.rs:209`, `api/agents.rs:180`) — **R2-9 closed** |
| Policy source setup (BYO git / managed) | exists-solid | GitHub App tokens, wired sync loop (`main.rs:143-148`), `trigger_sync` (verified r2) |
| Branch/dir-per-env mapping | **exists-weak** | `domain/source.rs:102` single `branch`, `:105` single `path` — still one-source-per-env by convention, no branch→env or dir→env map |
| UI↔git conflict model | exists-solid (direct-commit only) | `ConflictMode::{CommitBack,ReadOnly,LastWriterWins}`; **no PR mode** (R2-11 open) |
| Model data (ADM RBAC/ABAC/ReBAC) + migration engine | exists-solid | `domain/{datastore,migration}.rs` (verified r2) |
| Author policy + agentic capability grants | **exists-solid, expanded** | evaluators unchanged; **new**: attenuated capabilities as derived principals (`capability.rs`), verified in hot path (`evaluate.rs:203-219`) |
| Deploy to data plane (publish + delta sync) | exists-solid | `agent/handlers/data.rs`; outbox delta feed |
| Rollout strategy / canary / waves / approval | exists-solid | `api/deployments/*`; agent-confirmed convergence |
| Promote env→env with approval + freeze | exists-solid | `api/change_requests.rs`, `integrations/servicenow.rs` |
| **Autonomous safety (auto-revert bad rollout)** | **exists-weak (wrong signal)** | Supervisor loop wired (`supervisor.rs`, `main.rs:215`) but trigger reads deploy-apply failure, not runtime decisions (`service/mod.rs:582`) — **NEW P1** |
| SIEM egress | exists (on-demand) / weak (streaming) | On-demand OCSF/CEF/HEC push (`siem/mod.rs`, `api/connectors.rs`); continuous streaming still Vector-only; no in-process Kafka |
| GDPR subject erasure | **exists-solid** | `api/audit.rs:501+`, legal-hold-aware, pseudonym-aware, durable receipt (`audit_erasure.rs`) — **R2-3/T-5 closed** |
| Air-gap signed export/import | **exists-solid** | `tools/reaper-cli/src/airgap.rs`, CLI `Export`/`Import` (`main.rs:366,399`) — **R2-4/T-4 closed** |
| Fleet inventory (what version is everything on) | exists-weak | bundle histogram yes; **data-version convergence + pins count no** (`landscape/service.rs:269`) — R2-5 half-open |
| Replay durability | exists-weak | ephemeral in-memory jobs (`replay/mod.rs:21,188`) — R2-8 open |
| Cross-instance push completeness | exists-weak | only `datastore_published` bridged (`events_pg.rs:115`) — R2-7 open |

---

## Findings table

| ID | Sev | Location | Finding | Impact |
|---|---|---|---|---|
| R3-1 | **P1** | `deployment/service/mod.rs:582`; `domain/agent_deployment.rs:141-146` | Auto-rollback trigger fires on **bundle-apply failure rate** (failed agents / total agents), not on **runtime decision quality**. Live allow/deny/eval-error data (already in the decision buffer) is not wired to the trigger. | The flagship "safe-to-act" pillar does not protect against the most dangerous bank failure: a valid policy that deploys cleanly but denies (or allows) wrongly. That never trips the supervisor. |
| R3-2 | **P2** | `siem/mod.rs`; `api/connectors.rs:499`; `main.rs` (no spawn) | SIEM connector delivery is **on-demand/test only** — no background scheduler continuously ships new decisions; no native Kafka connector (Vector-commented). | Continuous SOC feed still requires customer-run Vector; "stream decisions to Splunk/Kafka" is push-a-batch or DIY, not a managed sink. |
| R3-3 | **P2** | `replay/mod.rs:21,188,218` | Replay jobs remain an ephemeral in-memory registry; lost on control-plane restart. | A long "impact of policy v7 over last quarter" run is not durable/resumable — weak as pre-change audit evidence. (Erasure got a durable receipt table; replay did not.) |
| R3-4 | **P2** | `events_pg.rs:115-136` | Cross-instance pg bridge propagates only `datastore_published`; bundle/rollout `ServerEvent`s are not bridged. | At multi-instance control-plane scale, bundle/rollout push degrades to poll for agents connected to a non-originating instance. Convergence still correct, just slower. |
| R3-5 | **P2** | `landscape/service.rs:269`; `api/landscape.rs` | Fleet **data-version** convergence is not a first-class fleet-wide answer; `version_pins` count is still a `TODO` returning 0. | "Prove every agent is on approved data version N and none are pinned off it" is a compliance question that still requires per-agent row scraping. |
| R3-6 | **P3** | `domain/source.rs:102,105` | Source carries a single `branch` + single `path`; no branch→env or dir→env mapping field. | GitOps "one repo, promote through envs" requires one source per env by convention; regulated buyers expect branch-per-env or dir-per-env as a first-class config. |
| R3-7 | **P3** | `sync/*`, `api/policies.rs` (grep: no `CommitBackPr`) | UI↔git commit-back still pushes directly to the tracked branch; no PR-interposed mode; author still synthetic, not SSO identity. | UI edit lands on the tracked (possibly prod) branch without a reviewed PR unless external branch protection catches it; git blame not tied to corp identity. |
| R3-8 | **P3** | `api/datastore.rs` (no snapshot/fork/restore routes) | No datastore backup/PITR or masked test-data fork for lower envs (migration engine is solid; *data lifecycle* isn't). | Seeding staging from a masked prod snapshot, or restoring last-Tuesday's authz data, is manual. |
| R3-9 | **P3** | `auth/scopes.rs`; `OrgRole` | Admin RBAC is org-wide role→scope; no per-namespace/per-env scoped role bindings. | A "prod-only approver" cannot be expressed; the new `audit:erase` scope is org-wide, so an erasure approver holds it everywhere. |
| R3-10 | **P3** | `docs/` (no control-plane SLO doc); no `support-bundle` tool | No product-level control-plane SLO/error-budget doc; no one-shot diagnostics/support bundle. | Enterprise support contracts and incident triage lack a defined SLO + a capture tool. |

---

## Detailed findings

### R3-1 (P1) — Auto-rollback reacts to distribution failure, not policy behaviour
This is the one finding that should block a clean regulated deploy, and it is subtle precisely because round 2's fix *looks* complete. The supervisor loop is genuinely well-built: leader-elected per tick, loop-guarded against re-rolling-back its own remediation, `monitor` default with `enforce` opt-in, audited and metered (`supervisor.rs:52,100-129,145-244`). The wiring gap round 2 flagged (T-3) is truly closed. **But the signal it consumes is the wrong one for the advertised purpose.**

`evaluate_rollback_trigger` computes `should_rollback = summary.failure_rate() > threshold` (`service/mod.rs:602-603`), where `summary` is `AgentDeploymentRepository::get_summary(rollout.id)` and `failure_rate()` is `failed / total_agents` over the `DeploymentSummary` statuses `{pending, deploying, deployed, failed, acknowledged}` (`agent_deployment.rs:111-146`). Those statuses describe whether an agent **successfully fetched, verified, and installed the bundle** — a *distribution* outcome. They say nothing about what the bundle *decides* once installed.

So the failure mode the supervisor catches is: "agents can't apply the new bundle" (bad signature, corrupt bundle, incompatible schema). That is real and worth catching. But the failure mode a bank actually fears — and the one "auto-rollback on error rate" *implies to a buyer* — is: "the new policy installed perfectly on every agent and now denies every payment / allows every unauthorized read." That policy shows `failed = 0`, `deployed = 100%`, `failure_rate = 0.0`, and the supervisor happily reports "within threshold" while production burns. The decision buffer on every agent already has the live allow/deny/eval-error counts (`decision_buffer.rs`, exposed via agent `/metrics` and decision stats); none of it is plumbed into the rollback trigger. **The loop is right; connect it to the decision-quality signal, not the deploy-apply signal.** Until then, the safety story is "we auto-revert bundles that fail to install," which is not what the pillar claims.

### R3-2 (P2) — SIEM egress: on-demand push shipped, continuous streaming still DIY
`siem/mod.rs` is a solid delivery transport — Splunk HEC + generic HTTP, OCSF/CEF/NDJSON shaping (shaping correctly lives in `policy-engine`), HMAC signing, exponential-backoff retries with 5xx/timeout classification. It closes the "no native connector" half of R2-2. **But it is only ever invoked from request handlers** — `test_connector` and the bounded `export` endpoint (`api/connectors.rs:499`, `:43` hard cap). There is no scheduler in `main.rs` that tails the decision store and continuously ships new decisions to enabled connectors. So a bank's "live feed to our SIEM" is still served by the customer running Vector (`deploy/decision-logs/vector-siem-sinks.toml`, where Kafka is *commented out*). The in-process path is on-demand/bounded export + a test button — useful, but not the managed continuous sink the connector's existence implies. Add a leader-elected connector-shipper loop (the pattern already exists three times in `main.rs`) and a native Kafka connector variant.

### R3-3 / R3-4 / R3-5 (P2) — durability and fleet-truth residue
- **Replay still ephemeral** (`replay/mod.rs:21,188`): the module comment is candid ("replay is an ephemeral analysis, not durable state"), but for a regulated buyer the whole *value* of counterfactual replay is producing durable evidence — "here is the assessed blast radius we reviewed before shipping v7." A control-plane restart mid-run loses that. The team persisted erasure receipts (`audit_erasure.rs`) but not replay results; the asymmetry is the tell that replay was left as a P2.
- **Event bridge single-event** (`events_pg.rs:115`): only `datastore_published` crosses instances. Bundle/rollout pushes to agents on a sibling instance fall back to poll. Correct, slower; a scale wart, not a safety bug.
- **Fleet convergence half-built** (`landscape/service.rs:269`): `BundleDistribution` histogram exists (`:90,228`), but there is no fleet-wide *data-version* histogram and `pinned` is still hardcoded `0 // TODO: Count from version_pins table`. The compliance question "is every agent on approved version N, and is anything pinned off it?" is not a first-class answer.

### R3-6..R3-10 (P3)
- **Branch/dir-per-env** (`source.rs:102,105`): single branch + single path per source. The regulated GitOps expectation (branch-per-env `main→prod`/`staging→staging`, or dir-per-env) is still convention, not config.
- **PR commit-back** (R2-11 unchanged): no `CommitBackPr` variant; UI edits push to the tracked branch, attributed to a synthetic identity. Round-2 design T-6 not taken.
- **Datastore backup/fork/seed** (R2-12 unchanged): migration engine is strong, but there is no snapshot/PITR/masked-fork for the authorization data itself.
- **Scoped admin RBAC** (R2-13 unchanged): org-wide roles only. The new `audit:erase` scope is a good separation-of-duties addition but is org-wide, so it can't be granted "prod only."
- **SLO/support-bundle** (R2-14 unchanged): no control-plane SLO/error-budget doc; no `reaper support-bundle` diagnostics capture.

---

## Absence checks (where I looked and found nothing)

- **Runtime-decision signal into auto-rollback:** `grep -rn "decision|deny_rate|denial|eval_error" deployment/` → only type/service files; the trigger consumes `DeploymentSummary` (deploy statuses) exclusively. No path from decision metrics to `evaluate_rollback_trigger`.
- **Continuous SIEM shipper:** `grep "ConnectorDeliveryService|siem::" main.rs decisions/*.rs` → 0; delivery only from `api/connectors.rs` handlers.
- **Native in-process Kafka:** `ConnectorType` has only `SplunkHec` + `Http` (`siem/mod.rs:81-105`); Kafka exists only as commented Vector config.
- **Durable replay:** `grep "replay_jobs|INSERT INTO replay|CREATE TABLE.*replay" replay/mod.rs` → in-memory `HashMap` registry only.
- **Bundle/rollout cross-instance bridge:** `events_pg.rs` sends only `ServerEvent::DatastorePublished`.
- **PR commit-back:** `grep "CommitBackPr|create_pull|pull_request" sync/ api/policies.rs domain/source.rs` → 0.
- **Branch/dir-per-env map:** `grep "env_mapping|branch_map|dir_per_env|path_prefix" domain/source.rs api/sources.rs` → 0.
- **Datastore snapshot/fork/restore routes:** none in `api/datastore.rs` (migration/rollback only).
- **Control-plane SLO doc / support bundle:** no SLO/error-budget doc in `docs/deployment/`; no support-bundle tool in `tools/`.
- **Consistency tokens/zookies:** `grep "zookie|consistency.token|snapshot.token|zedtoken"` → 0 (known-deferred, correctly).

---

## What's done well (≤5)

1. **The round-2 P1s are genuinely closed, not stubbed.** Supervisor loop (leader-elected, loop-guarded, monitor/enforce), signed air-gap export/import, and a GDPR erasure endpoint that handles legal holds, pseudonymized columns, and discloses immutable append-only surfaces with their lawful basis — this is careful, regulator-aware work (`supervisor.rs`, `airgap.rs`, `api/audit.rs:501+`).
2. **Agentic authorization is now first-class and hot-path-wired** — attenuated, expiring, ancestry-recording capabilities as derived principals (`capability.rs`), verified before evaluation with context-taint labels (`evaluate.rs:203-219`), a `capabilities` management API, revocation integration, and an MCP gate. This is the round-2 future review's single highest-leverage recommendation, shipped. It re-values every existing strength (sub-µs eval, ReBAC, provenance) for the agent era.
3. **Quota enforcement went from advisory to real** — tier limits + per-org JSON overrides, counted against actual usage and enforced at agent-register and policy-create (`quota/mod.rs`, `api/{agents,policies}.rs`). Round-2 R2-9 closed cleanly with no new table.
4. **SIEM record shaping is correctly layered** — OCSF/CEF/NDJSON shaping lives in `policy-engine` (`DecisionLogEntry::export`), the API orchestrates, and `siem/` is purely the wire. The transport reuses the webhook delivery discipline (retry classification, HMAC).
5. **Separation-of-duties instincts are maturing** — a dedicated `audit:erase` scope gates the irreversible erasure op (`api/audit.rs:76-88`), and the erasure receipt is recorded from inside the idempotency-guarded op *after* the irreversible step, so a write hiccup never fails an erasure that already ran.

---

## Gap register

| Gap | Why it blocks enterprise adoption | Proposed solution | Build/Buy/Integrate | Effort | Priority |
|---|---|---|---|---|---|
| Auto-rollback watches deploy-apply, not decision quality | Named safety pillar misses the likeliest bank failure (valid-but-wrong policy) | Feed live allow/deny/eval-error rate from the decision buffer into `evaluate_rollback_trigger`; add a policy-behaviour trigger alongside the deploy-failure trigger | Build (signal exists, trigger exists) | S–M | **P1** |
| No continuous SIEM shipper / no native Kafka | "Live feed to our SOC" is DIY Vector; Kafka absent in-process | Leader-elected connector-shipper loop tailing the decision store; add `ConnectorType::Kafka` | Build (transport exists) | M | **P2** |
| Ephemeral replay jobs | Impact-assessment evidence lost on restart; weak for audit | Persist jobs + results (table + object store), like the erasure receipt | Build | S | **P2** |
| Event bridge single-event | Bundle/rollout push degrades to poll at multi-instance scale | Bridge all `ServerEvent` variants over pg_notify | Build | M | **P2** |
| Fleet data-version convergence not first-class | Can't cleanly prove fleet is on approved version N | Fleet-wide data-version histogram + resolve `version_pins` count | Build (read model) | S–M | **P2** |
| Branch/dir-per-env mapping | Regulated GitOps expects env→branch/dir config, not convention | Add `env_mapping` (branch→env or dir→env) to source | Build | S | **P3** |
| PR commit-back + SSO author | UI edit lands on tracked branch without reviewed PR; blame not tied to identity | `ConflictMode::CommitBackPr` via GitHub App; author = SSO email | Build (App can already open PRs) | S | **P3** |
| Datastore backup/fork/seed | No PITR/masked test-data fork for authz data | Logical snapshot + masked fork + restore (design T-7 from r2) | Build | M–L | **P3** |
| Scoped admin role bindings | Can't express prod-only approver/eraser | Per-namespace/env role bindings | Build | M | **P3** |
| Control-plane SLO + support bundle | Support contract + triage lack primitives | Define SLOs; `reaper support-bundle` | Build + Docs | S | **P3** |

---

## Proposed tooling mini-designs (this round's new/residual gaps)

### T-A. Decision-quality auto-rollback trigger (closes R3-1 / P1) — the one required fix
**Goal:** the supervisor self-reverts a rollout when the *deployed policy behaves badly*, not only when it *fails to install*.
**API/internal sketch:**
- Extend `RollbackConfig` with a second trigger family: `decision_error_rate_threshold`, `denial_rate_threshold` (optional; a sudden deny-rate spike vs a baseline window), `min_decisions`, `window_seconds`.
- `evaluate_rollback_trigger` gains a decision-quality arm: read per-rollout (or per-namespace) live counts from the decision store / agent metrics — `evaluated`, `errors`, `denies` over the window — and compute `eval_error_rate` and `denial_delta` against the pre-rollout baseline. `should_rollback = deploy_failure_trip OR decision_quality_trip`.
- Keep the existing deploy-apply trigger; this is additive.
**Data-model touchpoints:** none new for the signal (decision buffer + `decisions/mod.rs` stats already expose allow/deny/error by policy/time). Store the pre-rollout baseline on the rollout row so "spike" is well-defined. New audit reason `decision_quality`.
**Composition:** the supervisor loop, leader election, monitor/enforce, and rollback action are all unchanged — only the *trigger's inputs* change. `monitor` mode lets a bank observe the decision-quality trip for a release before arming `enforce`.
**ADR:** absolute deny-rate threshold (simple, but legitimate policy changes *should* change deny rates) **vs** baseline-delta (catches "this deploy flipped behaviour" without punishing intended changes). **Recommend baseline-delta for denials + absolute threshold for eval errors** (an eval-error spike is never intended). This is the single highest risk-reduction-per-effort item in the review, and both the loop and the signal already exist — they are simply not connected.

### T-B. Continuous SIEM shipper + Kafka connector (closes R3-2 / P2)
**Goal:** a managed, continuous decision feed to a bank's SIEM by native connector, not customer-run Vector.
**Sketch:** a leader-elected `spawn_connector_shipper(state)` (identical pattern to the three existing sweepers in `main.rs`) that, per enabled `SiemConnector`, tails the decision store from a persisted `last_shipped_seq`, shapes via `DecisionLogEntry::export`, and `deliver()`s batches; add `ConnectorType::Kafka` (rdkafka) beside `SplunkHec`/`Http`. Persist per-connector cursor + delivery receipts.
**Composition:** reuses the entire `siem/mod.rs` transport; only the *drive* (a loop + a cursor) and one connector variant are new. Vector remains an option for shops that prefer it; the in-process path becomes a real managed sink.

### T-C. Durable replay (closes R3-3 / P2)
**Goal:** replay results survive restart and stand as audit evidence.
**Sketch:** mirror the erasure-receipt pattern — a `replay_jobs` table (params, state, progress, flip-diff summary) + result rows/object-store blob; the in-memory registry becomes a cache over it. `GET /orgs/{org}/replay/{job_id}` reads durable state.
**Composition:** the counterfactual engine (`policy-engine/replay`) is unchanged; this is persistence around the existing job runner. The precedent (`audit_erasure.rs`) shows the team already knows the shape.

*(Round-2 designs T-6 PR-commit-back and T-7 datastore fork/seed remain valid for the P3 items and are not repeated.)*

---

## Sequenced roadmap to "deployable in a regulated enterprise"

**#1 most important next move — wire the decision-quality signal into auto-rollback (T-A).** It is the only P1, it has **no operational workaround** (a valid-but-wrong policy at a bank cannot wait for a human to notice a denial storm), and it is a *small* build: the supervisor loop, leader election, monitor/enforce, and the rollback action all already exist, and the live allow/deny/eval-error signal already exists in the decision buffer — the two are simply not connected. Ship it in `monitor` mode first, arm `enforce` per namespace. This converts the safety story from "we revert bundles that fail to install" (true but narrow) into "we revert policies that misbehave" (what the pillar claims).

1. **T-A — decision-quality auto-rollback (P1, S–M).** The one gate. Do first.
2. **T-B — continuous SIEM shipper + Kafka (P2, M).** Turns the on-demand connector into a managed live feed; unblocks SOC onboarding by name-brand sink.
3. **R3-5 — fleet data-version convergence + pins count (P2, S–M).** Cheap read model; a direct compliance answer and feeds T-A's monitor view.
4. **T-C — durable replay (P2, S).** Makes counterfactual replay usable as pre-change audit evidence.
5. **R3-4 — bridge all ServerEvents (P2, M).** Removes the multi-instance poll-degradation.
6. **P3 cluster — branch/dir-per-env (R3-6), PR commit-back (R3-7), datastore fork/seed (R3-8), scoped RBAC (R3-9), SLO+support-bundle (R3-10).** Regulated-GitOps and support polish.

**Competitive frame (blunt).** Reaper's *combination* is now the widest in the category and, with capabilities shipped, it has planted a flag where no incumbent is strong. Against **OPA + OPAL / Styra DAS**: Reaper has the managed multi-model data plane OPA makes your problem, env→env promotion with approvals, a migration engine, *and now* first-class attenuated/expiring capabilities for agentic actors — none of which the OPA ecosystem ships. Against **OpenFGA / SpiceDB**: they own ReBAC tuples and, crucially, **consistency tokens (zookies)** — still Reaper's one genuine deficit against the category's marquee correctness feature (staleness-budget ≠ causal token; the case for the budget model is defensible but still undocumented). Reaper answers with RBAC+ABAC+ReBAC in one typed model, counterfactual replay, and agentic capabilities they lack. Against **Cedar / AWS Verified Permissions**: AVP passes entities per-request with no fleet sync, no off-path audit at this depth, no distribution machinery, and no agent-capability model — Reaper owns the whole loop. **Where Reaper is now genuinely ahead:** managed data plane + migration engine + counterfactual replay + agent-confirmed fleet convergence + **hot-path agentic capabilities** is a combination *no single competitor holds*. **Where it is still weaker:** consistency tokens (deferred), the auto-rollback signal being distribution-only (R3-1), continuous SIEM streaming being Vector-DIY (R3-2), and multi-region/HA-active-active (deferred). These are finishing work and one deliberate deferral, not architecture — which is exactly why the verdict is *conditional*, one P1 short of ready.

---

## Coverage statement

**Covered:** verification that all four round-2 P1s shipped (supervisor, SIEM connectors, air-gap, erasure) and are wired into the running service; round-2 P2/P3 residue (replay durability, event bridge, fleet inventory, quota, PR commit-back, branch/dir-per-env, datastore fork/seed, scoped RBAC, SLO/support-bundle); the full journey re-traced through current routes; the new agentic-capability surface (core primitive + hot-path gate + management API + MCP); competitive frame incl. the still-open consistency-token deficit.
**Not covered (other personas / out of scope):** eval-engine internals, DSL semantics, hot-path performance numbers (perf/testing personas); deep crypto/auth/injection soundness of the new capability signing and SIEM HMAC paths (security persona — I confirmed the capability gate is *invoked* in the hot path but did not audit its verification soundness); UI. Findings are from source inspection at current HEAD; services not compiled or run. Where round 2 verified a capability real and I saw no regression signal (SSO/SCIM, environments, migration engine), I spot-confirmed presence rather than re-tracing internals.
