# Review 03 (Round 3) — Code Quality & API Design

**Reviewer:** Staff engineer, API governance / code standards — paid external auditor, third pass.
**Lenses:** (1) is this Rust codebase maintainable by a *team* for a decade; (2) are these
APIs publishable to enterprise customers without embarrassment.
**Scope covered this round:** verification of every round-1 and round-2 finding closure
(auth mechanism, panic discipline, OpenAPI/parity gate, RFC 9457, ETag/If-Match,
idempotency, pagination, migration-apply guards); fresh review of the control-plane
resource model / action endpoints, outbound-I/O hygiene, list-endpoint pagination coverage
across **all 55 API files**, published-but-stub surfaces (billing), public-enum/semver
discipline, and CLI test coverage.
**Not covered (out of scope or deferred):** authz *correctness* of the auth layer (security
reviewers own it — I confirm only that the mechanism is mounted fail-closed); eval hot-path
internals (persona 06); SSO/SCIM protocol conformance; sync/replication internals
(distribution reviewer); Cedar evaluator internals. UI is out of scope per brief.

---

## VERDICT: CONDITIONAL

Every blocking finding from rounds 1 and 2 is now **closed in real mechanism, not
cosmetically** — I verified each at the code level (see Absence checks). The round-1 P0s
(agent inbound auth, mgmt default-deny, `panic="abort"` removed + `CatchPanicLayer`,
`/debug/datastore` compiled out) are backed by a genuine fail-closed exposure gate that
`anyhow::bail!`s before serving (`reaper-agent/src/main.rs:229-235`). The round-2 P1 (unbounded
ABAC/ReBAC datastore lists) is fixed with real keyset SQL (`LIMIT`, `db/repositories/datastore.rs:541,702,869`),
`require_if_match` now defaults **true** (`config/server.rs:71-73`), the policy ETag is now a
`row_version` that bumps on every write including metadata edits (`db/repositories/policy.rs:380-404`),
and migration-apply gained both the optimistic guard (`... AND model_version = $4`,
`db/repositories/datastore.rs:1021-1023`) and idempotency (`api/datastore.rs:1054`).

It is **CONDITIONAL, not READY**, on a residual P2 cluster that should land before a
regulated deploy: (1) the **audit read path (ClickHouse) and GitHub OAuth/App outbound
calls still have no request timeout** (round-2 R2-07 only partially fixed); (2) **Plan 07's
"every list endpoint is paginated" is still false** — `/orgs/{org}/pins` and several
config-cardinality lists return unbounded `Vec`s; and (3) **`reaper-cli` — the CI/CD entry
point customers script against — has zero integration tests.** No P0/P1 remains open in my
scope. The engineering underneath is strong and consistent; the residue is closure-completeness
and edge hygiene, not rot.

---

## Executive summary (≤10)

1. **All round-1 P0 / round-2 P1 findings verified closed at code level.** Auth fail-closed
   gate, pagination on the big tables, optimistic concurrency (policy + bundle + migration),
   idempotency, RFC 9457 `instance` — all present and correct.
2. **P2 — Audit-path + GitHub outbound calls still lack timeouts (R2-07 partial).** The
   ClickHouse decision-query client (`decisions/mod.rs:255`) and its `run()` set no
   per-request timeout; `oauth/github.rs:124,412` and `sync/github_app.rs:119` use bare
   `reqwest::Client::new()`. A hung upstream parks the handling task with no deadline.
3. **P2 — Not every list endpoint is paginated.** `/orgs/{org}/pins` returns an unbounded
   `Json<Vec<PinResponse>>` (`api/deployments/pins.rs:152`; repo query has no `LIMIT`,
   `db/repositories/deployment/pins.rs:85`). Pins scale with fleet size. `environments`,
   `webhooks`, `strategies`, `capabilities`, `revocations`, `agent_subscriptions` are also
   unpaginated. Plan 07 Phase-E "every list endpoint" remains overstated.
4. **P2 — `reaper-cli` has 0 integration tests.** Only 2 `#[cfg(test)]` modules and a
   `cli_bdd_tests.rs.backup` (disabled). The CLI's `eval/test/test-suite/bundle` output
   contracts — which customers gate CI/CD on — can change silently.
5. **P3 — Billing API is a published stub.** `/orgs/{org}/billing/checkout` and `/portal`
   are mounted (`api/mod.rs:86`), OpenAPI-documented, and return fabricated
   `cs_placeholder_*` sessions (`billing/service.rs:204-210`); `/webhooks/stripe` is a no-op
   that returns 200 without verifying the signature (`billing/service.rs:298-323`).
6. **P3 — `#[non_exhaustive]` / `missing_docs` still largely open (R2-09 carried).** Only
   `ApiError` is sealed (2 hits workspace-wide). `PolicyLanguage` (`engine/types.rs:38`), SDK
   `Decision`/`Source`/`Transport`, and `reaper-core` public enums are unsealed; no
   `#[deny(missing_docs)]` anywhere, including the published SDK.
7. **P3 — Contract-quality ratchet baselines are still high (R2-06 partial).** The
   `contract_is_publishable` gate now exists and hard-fails on an undocumented error model,
   but the shipping spec still carries **94 untyped success bodies** and **129 operations
   with no documented 4xx** (`tests/api_contract.rs:226-227`). Mechanism to drive these down
   exists; the current published contract has a large untyped surface.
8. **P3 — Bundle ETag is still a wall-clock timestamp (R2-10 carried).**
   `updated_at.to_rfc3339()` (`api/bundles.rs:218,281`) — sub-resolution rapid edits share a
   tag. Low probability, but a monotonic column is the correct source.
9. **Resource model is clean.** Actions are consistently modeled as `noun/{id}/verb`
   sub-resources under an `/orgs/{org}` hierarchy (promote/rollback/approve/reject/apply); no
   top-level verb endpoints. Pluralization and ID formats are consistent. No finding.
10. **Error model is now exemplary.** RFC 9457 problem+json with `instance` + `request_id`
    stamped by middleware, `ProblemDetails` is `ToSchema` (in the published spec), SQLSTATE
    23505/23514/23503 → 409/422, `VersionConflict` → 412, no message leaks.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R3-01 | P2 | `decisions/mod.rs:255,275-298`; `api/oauth/github.rs:124,412`; `sync/github_app.rs:119` | Outbound `reqwest::Client::new()` with no client- or request-level timeout (R2-07 still open) | Hung ClickHouse (audit query path) / GitHub stalls the awaiting task indefinitely; accumulates under load | Build all clients via `ClientBuilder::timeout(...)`; centralize `http_client()`; grep-lint bare `Client::new()` out of non-test code |
| R3-02 | P2 | `api/deployments/pins.rs:152`; `db/repositories/deployment/pins.rs:85`; also `environments.rs:44`, `webhook_subscriptions.rs:133`, `deployments/strategies.rs:32`, `capabilities.rs`, `revocations.rs` | List endpoints return unbounded `Vec`/`Json<Value>`; no `LIMIT`, no `PageQuery`. Plan 07 "every list paginated" false | `/orgs/{org}/pins` grows with fleet size → multi-thousand-row responses; DB scan; contradicts Phase-E DoD | Route pins through `PageQuery`/`Paginated` keyset like `agents`; paginate the config-cardinality lists for consistency |
| R3-03 | P2 | `tools/reaper-cli` (0 `tests/`; 2 `#[cfg(test)]`; `tests/cli_bdd_tests.rs.backup`) | The customer-facing CI/CD entry point has no integration tests | `eval`/`test`/`test-suite`/`bundle` exit codes and output shapes can regress silently, breaking customer pipelines | Add an integration suite asserting exit codes + JSON/table output for each subcommand; re-enable the disabled BDD file |
| R3-04 | P3 | `api/billing.rs:126,189,281` + `billing/service.rs:158,198,229,305` | Mounted, OpenAPI-documented billing endpoints return placeholder sessions; Stripe webhook is a no-op with no signature verification | Enterprise contract advertises a billing flow that fabricates checkout/portal URLs and silently 200s unverified webhooks | Feature-gate the billing surface off by default and exclude from the published spec until implemented, or clearly mark `x-experimental` |
| R3-05 | P3 | `engine/types.rs:38`; `reaper-sdk/src/types.rs:38,48`, `transport.rs:11`; `reaper-core` enums; no `deny(missing_docs)` (R2-09) | Growable public enums unsealed; published SDK has no doc-coverage gate | Adding a variant is a breaking change for downstream matchers; SDK ships undocumented | `#[non_exhaustive]` on growable public enums; `#[deny(missing_docs)]` on `reaper-sdk`/`reaper-core` |
| R3-06 | P3 | `tests/api_contract.rs:225-227` (`MISSING_4XX_BASELINE=129`, `UNTYPED_SUCCESS_BASELINE=94`) | Contract-quality gate exists but tolerates a large untyped/undocumented-error surface | Generated clients get `object`/`any` bodies and no error typing for ~94/129 operations | Drive the ratchets to 0 (typed DTOs + shared `responses(...)` 4xx fragment); add a redocly style lint |
| R3-07 | P3 | `api/bundles.rs:218,281` (R2-10) | Bundle ETag is `updated_at.to_rfc3339()` wall-clock | Sub-resolution rapid bundle edits share a tag, defeating `WHERE updated_at=$expected` | Use a monotonic `row_version` column like policies now have |

---

## Detailed findings

### R3-01 (P2) — Outbound calls without timeouts remain on the audit and GitHub paths

Round-2 R2-07 flagged this; it is **partially** fixed. Timeouts were added / already present
for ServiceNow (`integrations/servicenow.rs:62`), JWKS (`auth/jwks.rs:149`), SSO
(`api/auth/sso.rs:557`), webhook (`webhook/service.rs:61`), SIEM (`siem/mod.rs:69`), and the
sync builders (`sync/bundle_url.rs`, `s3.rs`, `api.rs` — though those still degrade to a
no-timeout `Client::new()` on builder failure via `unwrap_or_else`). But three remain
unbounded:

- **`decisions/mod.rs:255`** — the ClickHouse `DecisionStore` client is `reqwest::Client::new()`
  (no default timeout), and `run()` (`:270-298`) sets no per-request `.timeout(...)`. This is
  the **audit read/query path**; a slow or wedged ClickHouse leaves every
  `/orgs/{org}/decisions*` request task parked indefinitely. (It does not lose audit *writes*
  — those go through the agent ring buffer / export — so this is a query-availability defect,
  not audit loss; hence P2 not P1.)
- **`api/oauth/github.rs:124` and `:412`** — the OAuth token exchange and user fetch use bare
  `reqwest::Client::new()`.
- **`sync/github_app.rs:119`** — GitHub App installation-token minting, same.

**Remediation:** a single `http_client(timeout: Duration)` helper used everywhere, and a grep
lint (`ci.yml`) forbidding `reqwest::Client::new()` outside `#[cfg(test)]`. Fix the
`unwrap_or_else(|_| reqwest::Client::new())` fallbacks in the sync builders too — they silently
drop the timeout property if the builder ever errors.

### R3-02 (P2) — "Every list endpoint is paginated" is still not true

The big-table lists that dominated round 2 (ABAC entities, ReBAC tuples/bindings, decisions)
are now correctly keyset-paginated — verified. But the audit of **all** list handlers turned
up a fleet-scaling one that was missed:

`list_pins` (`api/deployments/pins.rs:152`) returns `Json<Vec<PinResponse>>` with no page
query; it calls `DeploymentService::list_pins(org_id)` → `PinRepository::list(org_id)`
(`db/repositories/deployment/pins.rs:85`) which is `SELECT ... FROM version_pins ... ORDER BY
... fetch_all` with **no `LIMIT`**. A version pin exists per pinned agent; at the fleet scale
the product targets (thousands of agents), `/orgs/{org}/pins` returns the whole set in one
array. This is the same class as round-2 R2-01 for a fleet-cardinality table, and it directly
contradicts the Plan 07 Phase-E DoD ("Every list endpoint enforces a default page size").

Also unpaginated (lower urgency — config-cardinality, bounded by org configuration rather than
runtime data): `list_environments` (`environments.rs:44`, `Json<Vec<Environment>>`),
`list_webhooks` (`webhook_subscriptions.rs:133`), `list_strategies`
(`deployments/strategies.rs:32`), `list_agent_subscriptions` (`namespaces.rs:505`),
`capabilities`, `revocations`. **Remediation:** route pins through the existing
`PageQuery`/`Paginated` keyset (the primitives are proven on `agents`/`policies`); paginate the
rest for contract consistency and to make the Phase-E claim honest.

### R3-03 (P2) — The CLI, a customer CI/CD dependency, is untested end-to-end

`tools/reaper-cli` is documented as the CI/CD integration surface (CLAUDE.md §CLI:
`reaper-cli test ... --expect allow|deny`, `test-suite`, `bundle validate`). Customers wire
these exit codes and output into their pipelines. Yet the crate has **no `tests/` integration
directory** — only 2 inline `#[cfg(test)]` modules in `src/`, and a
`tests/cli_bdd_tests.rs.backup` that has been renamed out of the build. Nothing exercises the
compiled binary's argument parsing, exit codes, or the JSON/table/`--verbose` output contracts.
A refactor that changes `test`'s non-zero exit on `deny`, or the `test-suite` YAML schema, or
the `--output json` shape, breaks every customer pipeline **silently** — no CI signal. For a
tool whose entire value proposition is "gate your CI on policy decisions," this is the highest-
leverage test gap in the tree. **Remediation:** an `assert_cmd`/`trycmd` integration suite
covering each subcommand's success + failure exit codes and serialized output; restore the BDD
file.

---

## Absence checks performed (closure verification)

- **Round-1 P0 agent auth:** fail-closed exposure gate `validate_exposure(...)` →
  `anyhow::bail!` before serving (`reaper-agent/src/main.rs:229-235`); default-deny layer mounted
  when a verifier exists (`:729-735`); non-loopback-without-auth requires explicit
  `allow_unauthenticated` opt-out with a loud warn (`:241-248`); `/debug/datastore` compiled out
  unless `debug_assertions` or `REAPER_DEBUG_ENDPOINTS` (`:715-723`). **Closed** (correctness is
  the security reviewer's call; mechanism is fail-closed).
- **Round-1 P0 panic discipline:** `panic="abort"` deliberately absent, documented (`Cargo.toml:114-121`);
  `CatchPanicLayer` outermost on the agent router (`main.rs:745-747`). **Closed.**
- **Round-1/2 mgmt auth + single surface:** router-level default-deny; `serve_root_alias` defaults
  **false** (`config/server.rs:20-21`). **Closed.**
- **R2-01 datastore list pagination (P1):** `list_entities/bindings/tuples` now return
  `Paginated<PageRow<...>>` (`api/datastore.rs:374,601,750`); repo has `... ORDER BY created_at,
  id LIMIT $n` (`db/repositories/datastore.rs:541,702,869`). **Closed.**
- **R2-02 `require_if_match` default (P2):** now `default_require_if_match() -> true`
  (`config/server.rs:71-73`). **Closed.**
- **R2-03 policy metadata ETag (P2):** ETag is `{content_tag}.r{row_version}`
  (`api/policies.rs:64-70`); metadata-only UPDATE guarded on `WHERE id=$5 AND row_version=$6`
  (`db/repositories/policy.rs:380-404`), `row_version` bumps on every write. **Closed.**
- **R2-04 migration model-version guard (P2):** `UPDATE datastores SET ... model_version =
  model_version + 1 WHERE id = $3 AND model_version = $4` with 409 on mismatch
  (`db/repositories/datastore.rs:1021-1045`). **Closed.**
- **R2-05 migration idempotency (P2):** `apply_migration` wraps `idempotency::run(...)`
  (`api/datastore.rs:1040-1087`); `Idempotency-Key` documented in the utoipa path. **Closed.**
- **R2-06 contract completeness (P2):** `contract_is_publishable` gate added
  (`tests/api_contract.rs:194-399`): hard-fails on missing `ProblemDetails` schema/members and
  on untyped errors in typed groups, and ratchets the rest. `ProblemDetails` is `ToSchema`
  (`api/error.rs:79`). **Partially closed** — baselines still high (R3-06).
- **R2-07 outbound timeouts (P2):** **Partially closed** — three call sites still bare (R3-01).
- **R2-08 RFC 9457 `instance` (P3):** `problem_instance` middleware stamps `instance` +
  `request_id` on every problem+json response (`api/error.rs:259-301`). **Closed.**
- **R2-09 `non_exhaustive`/`missing_docs` (P3):** still 2 `#[non_exhaustive]` hits workspace-wide,
  both `ApiError`; no `deny(missing_docs)`. **Open** (R3-05).
- **R2-10 bundle ETag (P3):** still `updated_at.to_rfc3339()` (`api/bundles.rs:218,281`).
  **Open** (R3-07).
- **Resource modeling / verb endpoints:** surveyed all `#[utoipa::path]` sites — actions are
  `noun/{id}/{verb}` sub-resources (`bundles/{id}/promote|rollback`, `rollouts/{id}/approve|cancel`,
  `change-requests/{id}/approve|reject`, `migrations/apply`), never top-level verbs. Consistent
  `/orgs/{org}/...` hierarchy. **No finding.**
- **`anyhow` in libraries:** re-grepped `policy-engine`/`reaper-core` src — none in non-dev deps;
  boundaries use `thiserror`. **Still clean.**
- **Idempotency callers:** `bundles.promote/rollback`, `rollouts.create`, `orgs.create`,
  `audit` (`:719`), `datastore.migrate` — all propagation-triggering POSTs covered.
- **Versioning/deprecation:** `/api/v1` is the single served surface; root alias off by default
  with `Deprecation`/`Sunset` headers when enabled. A written breaking-change/deprecation policy
  doc was not located in `docs/api/` — recommend one exists before GA (not re-flagged; low).

## What's done well (≤5)

1. **The remediation is real, not cosmetic.** Every round-1 P0 and round-2 P1/P2 I re-checked
   is closed with correct mechanism at the SQL/middleware level — this is the rare re-review
   where the closures hold up under adversarial reading.
2. **Error model is exemplary** — RFC 9457 problem+json with middleware-stamped `instance` +
   `request_id`, `ProblemDetails` in the published schema, correct SQLSTATE→4xx mapping, no
   internal-message leaks (`api/error.rs`).
3. **Optimistic concurrency is now uniform and atomic** — policies (row-version, covers
   metadata), bundles, and migrations all guard their UPDATE in-transaction and return 412/409,
   with a fail-fast handler pre-check.
4. **Resource model would not embarrass in an enterprise security review** — clean noun
   hierarchy, actions as sub-resources with justification, consistent pluralization, single
   versioned surface.
5. **The parity + publishability gate is durable** — single utoipa-axum tree makes drift
   structurally impossible, `no_undocumented_raw_routes` + unique-operationId + the
   `ProblemDetails`-required check make the contract self-defending as the surface grows.
