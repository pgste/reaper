# Reaper Security Review — Round 2 (Offensive Security Engineer / External Auditor)

**VERDICT: CONDITIONAL** — Every Round-1 P0 is genuinely closed (verified, not
cosmetic). No open P0 in the default configuration. What remains is a cluster of
**P1 audit-integrity gaps that are real in the DEFAULT deploy** plus a
**least-privilege gap on fleet-propagation routes**. A bank board cannot sign
"READY" while an insider with store access can erase a boot's audit trail
undetectably and any org member can trigger a fleet-wide rollback — but these are
bounded, addressable, and do not reopen the anonymous-attacker P0s.

Counts: **P0: 0 · P1: 5 · P2: 3 · P3: 3**

---

## Threat model (assets → actors → boundaries)

**Assets:** live policy set on each agent (the allow/deny authority), ReBAC
entity/relationship graph, decision/audit logs + hash-chain checkpoints, ed25519
bundle signing keys, JWT/session/API-key secrets, git & DB creds, tenant
boundaries.
**Actors:** anonymous network attacker; malicious/curious tenant; **least-privilege
insider (org member holding a narrow-scope token)**; hostile operator; insider with
ClickHouse/S3 write access; compromised bundle store/CDN/on-path proxy; compromised
CI.
**Trust boundaries:** client→control plane; control plane→agent; bundle
store/CDN→agent; git→control plane; agent→audit sink; **intra-tenant role
boundary** (this round's weak point).

## Executive summary (≤10)

1. **Round-1 P0s CLOSED, verified.** Agent push-deploy now verifies signatures
   fail-closed *before* parse (`handlers/policies.rs:382-399`); control plane has a
   router-level **default-deny** auth gateway (`auth/gateway.rs`, wired
   `main.rs:297-299`, `GatewayMode::Enforcing` default `config/auth.rs:24-25`);
   `require_signed_bundles` defaults **true** (`config/settings.rs:587`).
2. **P1 — Rollout/rollback/pin routes enforce only org-membership-OR-Admin, no
   fine-grained scope.** Any authenticated org member (even a read-only token) can
   start/cancel/rollback rollouts and set pins fleet-wide, while the analogous
   `bundle:promote` correctly requires the `BundlePromote` scope. Least-privilege
   violation on the propagation-triggering surface.
3. **P1 — Audit chain is not verifiable from the queryable store, and no verifier
   ships.** `verify_chain`/`verify_checkpoint` exist and are correct but are called
   only from unit tests (`decision_log.rs:1048,1338`); no CLI/endpoint. ClickHouse
   `ORDER BY` doesn't preserve writer chain order, so the chain can't be replayed
   from where it lands.
4. **P1 (P0-adjacent in default deploy) — Checkpoints ship to the SAME ClickHouse
   as the decisions they attest, and the immutable S3/WORM sink is commented out**
   (`deploy/decision-logs/vector.toml:97-107`). An insider with store write access
   deletes a boot's decisions *and* its checkpoints together — undetectably. Closed
   only in a hardened deploy that enables WORM.
5. **P1 — Decisions are served before durable persistence** (async writer): a
   bounded window of served-allow decisions can be lost on sink failure even in
   mandatory-audit mode.
6. **P1 — PII redaction is entirely opt-in and the `resource` field is never
   redactable**, so the default audit stream can carry sensitive identifiers.
7. **P2 footguns (default-safe):** operator-selectable fail-open gateway modes
   (`Disabled`/`LogOnly`), agent `allow_unauthenticated`/`open_data_plane`. All
   default to the safe setting and warn, but each is a one-flag fail-open.
8. **Data plane clean.** All dynamic SQL binds values; only hardcoded column
   fragments/placeholder counters are interpolated. `unsafe` = 0 in `services/`,
   0 in `policy-engine`; the 12 `unsafe` blocks are all experimental `reaper-ebpf`
   (not audited for soundness — gate before that feature ships).
9. **Panics-from-network closed.** Every `unwrap()` on request-derived data in the
   eval hot path is type-guarded (`fast_parse.rs:73,79,148,154`;
   `evaluate.rs:652`); DSL nesting is depth-bounded at parse and eval
   (`reap/limits.rs`, `parser/mod.rs:40,97`, `ast_evaluator/mod.rs:317`).
10. **Supply chain CLOSED.** `cargo deny check advisories licenses bans sources`
    is a blocking PR gate (`ci.yml:130-131`), cargo-audit blocking (`:148-149`),
    Trivy `exit-code: 1` (`docker.yml:124-129`); `Cargo.lock` + `deny.toml` tracked.

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R2-1 | P1 | `api/deployments/rollouts.rs:97-103,244-250,284-290,333-339,380-386,427-433,491-497`; `pins.rs:44,99` | Rollout/rollback/approve-wave/cancel/pin gate only on `user.org_id == org.id \|\| Admin` — **no fine-grained scope**, unlike `bundle:promote` (`bundles.rs`) or every sibling (`sources.rs`/`namespaces.rs` require Policy/AgentWrite) | Any org member with *any* token scope (incl. read-only service tokens) triggers fleet-wide rollout/rollback/pin — intra-tenant privilege escalation on the propagation surface | Require a deploy scope (`BundlePromote`/new `DeploymentWrite`) `+ OrgAdmin` fallback, matching `authorize()` in `change_requests.rs:691-709` |
| R2-2 | P1 | `decision_log.rs:139,339,1048,1338`; no CLI/endpoint caller | Hash-chain + signed-checkpoint verifiers exist but are **only called from unit tests**; chain seq assigned pre-enqueue diverges from writer order and ClickHouse `ORDER BY` doesn't preserve write order → **chain not verifiable from the queryable store** | Tamper-evidence is unusable operationally; a regulator cannot be handed a "verify our audit" tool | Ship a verifier (CLI/endpoint) that reads the store; persist a monotonic writer-assigned seq and verify by it, not by query order |
| R2-3 | P1 | `deploy/decision-logs/vector.toml:76-91` (checkpoints→same ClickHouse), `:97-107` (S3 WORM commented out) | Checkpoints attesting the chain land in the **same mutable store** as the decisions; immutable WORM sink is disabled by default; no cross-boot `chain_id` linkage | Insider with store write access deletes a boot's decisions + checkpoints together, undetectably, in the **default** deploy | Enable the S3/WORM checkpoint sink by default; link boots via a signed genesis `chain_id`; ship checkpoints to a sink the data-plane insider can't rewrite |
| R2-4 | P1 | agent async decision writer (`handlers/decisions.rs`, `decision_buffer.rs`) | Decision is **served before durable persistence**; bounded served-allow loss window on sink failure even in mandatory mode | "Prove decision X at time T" can fail for a burst that was actually served | Add a durable, backpressuring path (or WAL) for mandatory-audit mode; count+alarm on drops |
| R2-5 | P1 | `decision_privacy.rs` (masking opt-in), `resource` field | PII redaction is **all opt-in**; `resource` is **never** redactable | Default audit stream carries raw principals/resources → PII/compliance exposure by default | Make redaction policy explicit at enable time; allow `resource` redaction; document a compliant default |
| R2-6 | P2 | `auth/gateway.rs:84-86,99-106`; `config/auth.rs:11-25` | Operator-selectable **fail-open** gateway modes `Disabled` (skip auth) and `LogOnly` (log-and-pass) | If any deployment sets these, the control plane serves anonymous callers — P0-equivalent in that deployment | Keep default `Enforcing`; gate `Disabled`/`LogOnly` behind a loud startup banner + refuse on non-loopback bind (mirror `validate_exposure`) |
| R2-7 | P2 | agent `config/settings.rs` (`allow_unauthenticated`, `open_data_plane`, `require_client_cert`) | Multiple one-flag fail-open agent footguns; default-safe + `validate_exposure` refuses unauth non-loopback bind (`settings.rs:186-204`) | Misconfiguration re-exposes the enforcement plane | Ship a hardened profile (mTLS on, data-plane gated); keep the exposure guard |
| R2-8 | P2 | repo getters `bundle.rs:67`, `change_request.rs:69`, `rollouts.rs:55` (by-UUID, unscoped) | No `org_id` in the WHERE clause — isolation depends entirely on the handler's pre-check (which is currently always present) | A future handler that forgets the pre-check is a cross-tenant IDOR with no second line of defense | Add `org_id` to the by-id queries (defense-in-depth), or a scoped `get_scoped` variant everywhere |
| R2-9 | P3 | `.github/workflows/ci.yml:148-149` | `cargo audit` ignores 4 RUSTSEC advisories incl. RSA Marvin `RUSTSEC-2023-0071` (no fix upstream) | Known-vuln dep tolerated; documented in VULN_RESPONSE.md | Track for upstream fix; confirm RSA path unreachable at runtime |
| R2-10 | P3 | `audit/mod.rs:447-448,563-564` | `LIKE` `action_prefix` doesn't escape `%`/`_` (value is bound, so injection-safe) | Over-broad audit filter match only | Escape LIKE metacharacters |
| R2-11 | P3 | `audit/mod.rs:516-529` (`AuditLog::query`/`for_resource`) | No mandatory `org_id` filter; currently **unreachable** (no callers outside module) | Latent cross-tenant read if wired up later | Add a mandatory org param before exposing |

## Detailed P0/P1

### R2-1 (P1) — Fleet-propagation routes lack least-privilege scope
`start_rollout_inner` (`rollouts.rs:97-103`) and its siblings (cancel `:380`,
approve-wave `:333`, rollback `:427,491`, list/get `:244,284`) and pins
(`pins.rs:44,99`) all gate with exactly:
```
if user.org_id != organization.id && !user.has_any_permission(&[Scope::Admin]) { Forbidden }
```
That admits **any authenticated member of the org, regardless of token scope**.
Contrast: `bundle:promote` requires the `BundlePromote` scope; `change_requests.rs`
`authorize()` (`:691-709`) requires the operation's scope *plus* the org/tenant
check; `sources.rs`/`namespaces.rs`/`agents.rs` all require Policy/Agent read/write
scopes. Rollout/rollback is a **propagation-triggering, fleet-wide** action
(`start_rollout` → SSE/pg_notify → agents pull). A least-privilege service token
minted with, say, `AgentRead` for a dashboard can roll the whole org's agents back
to an arbitrary prior bundle (`rollback_org` `:482`). This is not fail-open (auth
is enforced) and not cross-tenant (org-scoped), hence P1 not P0 — but it is a real
broken-access-control gap on the most dangerous verb in the API.
**Remediation:** introduce `DeploymentWrite` (or reuse `BundlePromote`) and require
it `+ OrgAdmin` fallback on all mutating rollout/pin handlers.

### R2-2 / R2-3 (P1, P0-adjacent) — Audit integrity is not defensible in the default deploy
The primitives are correct: `verify_chain_from` (`decision_log.rs:147`) detects
mutation, deletion, reorder, and insertion (tests `:1048-1074`);
`verify_checkpoint` (`:339`) checks an ed25519-signed checkpoint over the chain.
**But (a)** the only callers are unit tests — there is no shipped CLI or endpoint
that verifies the chain against the real store (grep of `crates/ services/ tools/`
for `verify_chain`/`verify_checkpoint` outside `decision_log.rs` = none). **(b)**
the chain seq is assigned before enqueue and ClickHouse `ORDER BY` does not
preserve writer order, so the sequence as stored can't be linearly re-verified.
**(c)** worst of all, checkpoints ship to `[sinks.clickhouse_checkpoints]`
(`vector.toml:76`) — the **same** mutable ClickHouse holding the decisions — and
the immutable archive `[sinks.s3_worm]` is **commented out** (`:97-107`). So the
one artifact that would let you detect deletion sits in the same place, writable by
the same insider, with no independent WORM copy and no cross-boot `chain_id`
linkage. In the default deployment, an insider with store write access truncates a
boot's decisions and its checkpoints together and nothing detects it. This is only
"P1" because it requires privileged store access and is closed by enabling the WORM
sink; treat it as a release blocker for any regulated tenant.
**Remediation:** enable the S3/WORM checkpoint sink by default; ship a
store-reading verifier; persist a monotonic writer-seq and a signed genesis
`chain_id` per boot so gaps between boots are detectable.

### R2-4 (P1) — Served-before-persisted
The hot path records the decision into a bounded in-memory buffer and returns the
allow/deny before the async writer has durably persisted it. Under sink outage a
burst of *served* allows can be lost, so the audit set is not a superset of served
decisions even in mandatory mode. **Remediation:** durable/backpressured write path
for mandatory-audit mode; counted+alarmed drops.

### R2-5 (P1) — PII by default
`decision_privacy.rs` masking/pseudonymization/encryption are all opt-in and the
`resource` field has no redaction path, so principals and resource identifiers flow
to the sink in clear by default. **Remediation:** force an explicit redaction
choice when decision logging is enabled; make `resource` redactable.

## Absence checks performed (falsifiable)

- **Global control-plane auth:** read `auth/gateway.rs` in full + wiring
  `main.rs:297-299`; default-deny confirmed, `GatewayMode::Enforcing` default
  (`config/auth.rs:24-25`). Public allowlist (`gateway.rs:33-75`) is health/metrics/
  openapi + genuine login/signup/refresh/reset + source-signed webhooks only.
- **Agent push-deploy signature bypass (Round-1 P0-2):** read
  `handlers/policies.rs:370-425` and `:488-532` — both `deploy_bundle` and
  `load_bundles_atomic` call `verify_push` *before* `from_bytes`/deploy, fail-closed
  (422). Standalone-agent unsigned path is explicit and warned (`verify.rs:208-215`).
- **`require_signed_bundles` default:** `config/settings.rs:587` = `true`.
- **Anti-rollback / revocation (Round-1 P1-1):** `anti_rollback.rs` persists a
  monotonic per-lineage floor, rejects downgrades, `force` never lowers the floor;
  `revocation.rs` checked in `verify_inner` (`verify.rs:182-183`) and **not**
  overridable by `force`. Closed.
- **`unsafe`:** grep `crates/ services/` — 12 blocks, **all** in `reaper-ebpf`
  (`loader.rs:51-54`, `slow_path.rs:259`, `types.rs:74`, kernel `lib.rs`);
  `services/` and `policy-engine` = **zero**. eBPF is experimental (Linux-only, off
  the eval path) — not audited for soundness; gate before GA.
- **Panics from network input:** eval-hot-path `unwrap()`s are all type-guarded
  (`fast_parse.rs:73/79/148/154`, `evaluate.rs:652`); the one non-test
  `#[allow(clippy::expect_used)]` (`bundle_signing.rs:217`) is a justified abort on
  OS-RNG failure during key generation. Workspace denies `unwrap_used`+`expect_used`
  (`Cargo.toml:31-32`).
- **DSL DoS (Round-1 P2-1):** nesting depth bounded at source, parse, and eval
  (`reap/limits.rs`, `parser/mod.rs:40,97`, `ast_evaluator/mod.rs:317`). Closed.
- **Two-person control on env promotion:** `ApprovalPolicy::evaluate`
  (`domain/environment.rs:122-137`) removes the requester from the distinct approver
  set when `distinct_from_requester` (default `true`, `:100`) and requires
  `have >= min_approvers`; approver scopes re-checked (`change_requests.rs:254-261`),
  applied only via `maybe_apply` (`:452-458`) with freeze-window + external-record
  re-checks at apply time. Self-approval is blocked when `min_approvers>=1`; the
  default `min_approvers=0` is documented single-control. Correct.
- **Hardcoded secrets:** grep non-test src — hits are all test fixtures
  (`agent/auth.rs:438,487`, `oauth/helpers.rs:186,200`, all `#[cfg(test)]`).
- **Supply-chain gates:** `ci.yml:130-131,148-149` (blocking deny+audit),
  `docker.yml:124-129` (Trivy exit-code 1). `Cargo.lock` + `deny.toml` tracked.

## What's done well (≤5)
1. The Round-1 P0 trio is genuinely fixed at the *structural* layer (router-level
   default-deny + single signature chokepoint on every load entry point), not
   patched per-handler — the right altitude.
2. Bundle trust chain is now complete: verify-before-parse, fail-closed default,
   persisted monotonic anti-rollback floor, and a signed revocation list that
   `force` cannot override.
3. `change_requests.rs` two-person control is carefully done: distinct-approver
   set, requester exclusion, scope re-check, and freeze/external re-validation at
   apply time (not just request time).
4. Data plane is clean: parameterized SQL throughout, zero `unsafe`, depth-bounded
   DSL, guarded hot-path parsing, and a caught-panic layer so a handler panic is a
   fail-closed 500.
5. Supply chain went from "nothing" (Round-1 P2-2) to blocking cargo-deny/audit +
   Trivy + tracked lockfile/deny.toml.
