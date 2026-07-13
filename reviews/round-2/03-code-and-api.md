# Review 03 (Round 2) — Code Quality & API Design

**Reviewer:** Staff engineer, API governance — paid external auditor, re-review.
**Lenses:** (1) maintainable by a team for a decade; (2) publishable to enterprise customers.
**Scope this round:** verification of Plan 07 (OpenAPI + parity gate, `/api/v1`, ETag/If-Match, Idempotency-Key, keyset pagination, RFC 9457) closures, plus the fast-written Plan 10/12 roadmap code (migrations, change requests, ServiceNow, datastore apply). Auth correctness is deferred to the security/auth reviewer; I note only that the round-1 P0 auth/panic findings now have mechanism in place (see Absence checks).

---

## VERDICT: CONDITIONAL

Plan 07 landed for real, not cosmetically: a single-sourced utoipa-axum OpenAPI tree with a blocking parity gate (`tests/api_contract.rs` + `openapi-spec-validator` in CI), an RFC 9457 `application/problem+json` envelope with correct constraint→409/422 mapping and no message leaks, a genuine SQL-guarded optimistic-concurrency path, a database-arbitrated idempotency claim/complete, and keyset pagination with a clean sentinel-row envelope. The round-1 P0s (agent inbound auth, mgmt default-deny router layer, `panic="abort"` removed + `CatchPanicLayer`, `/debug/datastore` compiled out of release) all have mechanism present.

It is **not yet READY** for two reasons. First, the "every list endpoint is paginated" claim (Plan 07 Phase E, DoD) is **false for the ABAC/ReBAC datastore** — `list_entities`, `list_tuples`, and `list_bindings` return the whole table unbounded, and those are the largest tables in any real deployment (P1). Second, the concurrency-control closure (API-5) is **present but dormant by default** (`require_if_match=false`) and, for policies, the ETag does not cover metadata edits — so silent lost-updates remain reachable in the shipping default configuration (P2 cluster). New Plan 12 migration-apply code reintroduces the exact lost-update/idempotency gaps Plan 07 fixed elsewhere.

The engineering underneath is strong and consistent; the defects are closure-completeness and new-code parity gaps, not rot.

---

## Executive summary (≤10)

1. **P1 — Unbounded biggest-table lists.** `list_entities`/`list_tuples`/`list_bindings` (`api/datastore.rs:293,553,449`) have no limit; repo SQL has no `LIMIT` (`db/repositories/datastore.rs:438,643,537`). Contradicts Plan 07 Phase E "every list endpoint."
2. **P2 — Optimistic concurrency is off by default.** `require_if_match=false` (`config/server.rs:39`); in the shipped default a `PUT` without `If-Match` runs **unguarded** (`preconditions.rs:72-79`). API-5's lost-update is closed in mechanism, dormant in practice.
3. **P2 — Policy ETag doesn't cover metadata.** ETag = `content_hash` (`policies.rs:59`), which does not change on name/description/is_active edits; those run version-unguarded even with enforcement on (`db/repositories/policy.rs:365-408`). Two representations share one ETag — violates RFC 9110 §8.8.1.
4. **P2 — `apply_migration` has no model-version concurrency guard.** `UPDATE datastores SET model=…, model_version=model_version+1 WHERE id=$3` (`db/repositories/datastore.rs:801`) — no `AND model_version=$expected`. Concurrent migrations (or migration racing entity writes) silently clobber the model / rewrite entities from a stale plan snapshot.
5. **P2 — Migration apply lacks Idempotency-Key** though it publishes a new data version and fans out to the fleet (`api/datastore.rs:730,767`). Inconsistent with promote/rollback/rollout/org-create, which have it.
6. **P2 — Contract gate proves presence, not completeness.** It forbids raw `.route(` and validates structural OpenAPI 3.1 (`tests/api_contract.rs`; `openapi-spec-validator` in `ci.yml`) but enforces no request/response schemas, examples, or error-response coverage. Many handlers return untyped `Json<Value>` (datastore/decisions/replay/audit) and `ProblemDetails` lacks `ToSchema` (`api/error.rs:62`) — the error model is absent from the spec; generated clients get untyped bodies.
7. **P2 — Outbound HTTP without timeouts.** `reqwest::Client::new()` (no timeout) in `api/oauth/github.rs:124,412`, `sync/github_app.rs:119`, and the ClickHouse client `decisions/mod.rs:218`. A hung upstream stalls the handling task. (ServiceNow is correctly bounded at 10s — `integrations/servicenow.rs:62-63`.)
8. **P3 — RFC 9457 `instance` member missing** from `ProblemDetails` (`api/error.rs:63-75`); the DoD explicitly listed it.
9. **P3 — `#[non_exhaustive]`/`missing_docs` still largely open.** Only `ApiError` is `#[non_exhaustive]` (2 hits workspace-wide); `PolicyLanguage` and SDK/core public enums are not; no `#[deny(missing_docs)]` anywhere. Round-1 API-10 only partially closed.
10. **P3 — Bundle ETag is a timestamp.** `updated_at.to_rfc3339()` as the version token (`api/bundles.rs:218,258`) — two writes within the stored timestamp's resolution share a tag and defeat the `WHERE updated_at=$expected` guard. A monotonic counter is safer than a clock.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| R2-01 | P1 | `api/datastore.rs:293,449,553`; `db/repositories/datastore.rs:438,537,643` | ABAC entities + ReBAC bindings/tuples listed unbounded (`count: len()`, no `LIMIT`) | OOM / multi-MB responses / DB scan on the largest tables; fleet-scale DoS; false Phase-E closure | Route through `PageQuery`/`Paginated` keyset like `policies`; keyset over `(created_at, entity_id)` |
| R2-02 | P2 | `config/server.rs:39`; `preconditions.rs:60-87`; `policies.rs:336-342` | `require_if_match` defaults false → unguarded PUT on missing `If-Match` | API-5 lost-update still reachable in default config; "READY" must not ship warn-only | Flip default to true for the GA release, or gate READY on it; keep env override for migration |
| R2-03 | P2 | `policies.rs:59-72`; `db/repositories/policy.rs:365-408` | Policy ETag = content_hash; metadata-only edits neither change it nor bump version | Silent lost-update on name/description/is_active even with enforcement on; ETag ≠ representation (RFC 9110) | Derive ETag from a row-version/updated_at that every write bumps; guard metadata UPDATE on it |
| R2-04 | P2 | `db/repositories/datastore.rs:801-810`; `api/datastore.rs:730-760` | `apply_migration` UPDATE has no `AND model_version=$expected`; entities rewritten from plan snapshot | Concurrent migrations / migration-vs-write races silently clobber model + corrupt entity rows | Add optimistic guard on `model_version` (the plan already carries `model_before`); 409 on mismatch |
| R2-05 | P2 | `api/datastore.rs:714-780` (no `idempotency::run`) | Migration apply triggers publish+fan-out but takes no `Idempotency-Key` | Retried timeout double-applies → spurious model version + redundant fleet propagation | Wrap in `idempotency::run` scope `datastore.migrate` like promote/rollout |
| R2-06 | P2 | `tests/api_contract.rs:105-158`; `ci.yml:228-235`; `api/error.rs:62` | Parity gate checks presence + structural validity only; untyped `Json<Value>` handlers; `ProblemDetails` not `ToSchema` | Publishable contract has untyped bodies + no documented error model; client codegen degraded | Add `ToSchema` DTOs for datastore/decisions/replay/audit + `ProblemDetails`; lint descriptions/error responses (redocly ruleset) |
| R2-07 | P2 | `api/oauth/github.rs:124,412`; `sync/github_app.rs:119`; `decisions/mod.rs:218` | Outbound `reqwest::Client::new()` with no timeout | Hung GitHub/ClickHouse stalls the request task indefinitely | Build clients via `ClientBuilder::timeout(...)`; centralize a `http_client()` helper |
| R2-08 | P3 | `api/error.rs:63-75` | RFC 9457 `instance` member omitted | Consumers can't correlate a problem to the failing request URI; DoD said it would ship | Add `instance` (correlation-id or request path) to `ProblemDetails` |
| R2-09 | P3 | workspace (2 `#[non_exhaustive]` hits, both `api/error.rs`); no `deny(missing_docs)` | Growable public enums (`PolicyLanguage`, SDK/core) not sealed; SDK undocumented | Adding a variant is a breaking change; published SDK has no doc-coverage gate | `#[non_exhaustive]` on growable public enums; `#[deny(missing_docs)]` on `reaper-sdk`/`reaper-core` |
| R2-10 | P3 | `api/bundles.rs:218,258-281`; `db/repositories/policy.rs:341-347` | Bundle version token is a wall-clock timestamp; content-path VersionConflict returned even when `expected_version` is None (deleted row) | Sub-resolution rapid bundle edits defeat the guard; 412 masks a 404 on concurrent delete | Use a monotonic bundle version column; distinguish deleted-row (404) from stale-version (412) |
| R2-11 | P3 | `api/datastore.rs:795`; `db/repositories/datastore.rs:881` | `list_migrations`/`list_model_versions` unpaginated | Slow-growing but same unbounded pattern; large-history stores return everything | Paginate for consistency (lower urgency than R2-01) |

---

## Detailed findings

### R2-01 (P1) — The ABAC/ReBAC datastore lists are unbounded; Phase-E "every list endpoint" is false

`list_entities` (`api/datastore.rs:293-305`) calls `DatastoreRepository::list_entities(store.id, entity_type)` and returns `{"entities": entities, "count": entities.len()}`. The repository (`db/repositories/datastore.rs:438-466`) issues `SELECT … FROM adm_entities WHERE datastore_id = $1 [AND entity_type = $2] ORDER BY entity_id` with **no `LIMIT`** and `fetch_all`. The same holds for `list_bindings` (`api/datastore.rs:449` → repo `:537`) over `adm_role_bindings` and `list_tuples` (`api/datastore.rs:553` → repo `:643`) over `adm_tuples`.

These three tables are the entity attribute store and the relationship-tuple store — for an enterprise ABAC/ReBAC deployment they are, by design, the biggest tables in the product (the CLAUDE.md scale examples run 10k–100k+ entities; production is larger). An operator listing entities pulls the entire set into memory, serializes it to one JSON array, and ships it in a single response. This is the persona's explicit P1 ("unpaginated biggest-table list = P1") and it directly contradicts the Plan 07 STATUS banner ("keyset cursor pagination on **every** list endpoint") and DoD checkbox ("Every list endpoint … enforces a default page size"). The pagination primitives already exist and are used correctly on `policies`/`agents`/`teams`; the datastore handlers were simply not migrated. `changes_since` (repo `:246`) *is* bounded (`LIMIT $3`, clamp 1..2000), which shows the author knew — the entity/tuple/binding lists were missed.

**Remediation:** thread `PageQuery::validate()` → keyset over `(created_at, entity_id)` (add the column to the ORDER BY and cursor) and return `Paginated<AdmEntity>`; same for tuples/bindings. Add a seeded 500-row no-drift test mirroring the policies pagination test.

### R2-02 + R2-03 (P2 cluster) — Concurrency control is real but dormant, and doesn't cover policy metadata

The SQL guard is genuine and correct where it applies: `db/repositories/policy.rs:306-347` runs `UPDATE policies … WHERE id=$6 AND current_version=$7`, rolls back and returns `DatabaseError::VersionConflict` (→ 412, `api/error.rs:122-126`) on `rows_affected()==0`, and inserts the immutable version row in the same transaction. The handler fast-fails a stale `If-Match` before touching the DB (`policies.rs:335-342`). This is a proper, atomic optimistic-concurrency implementation.

Two gaps keep the lost-update reachable:

1. **Off by default (R2-02).** `ServerConfig::require_if_match` defaults `false` (`config/server.rs:39`), documented as ADR-3's one-release warn-only rollout. In that mode a `PUT` with **no** `If-Match` returns `Ok(false)` and runs unguarded (`preconditions.rs:72-79`, `expected_version = None` at `policies.rs:342`). So in the shipping default, an automation client that never learned to send `If-Match` still silently clobbers. The mechanism is present; the protection is not on. A release that claims READY must flip this (or gate the readiness call on it) — otherwise the round-1 P1 is closed only on paper.

2. **Metadata isn't covered (R2-03).** The policy ETag is the current version's `content_hash` (`policies.rs:59-72`). Metadata-only edits (name/description/is_active) do **not** create a new version and do **not** change `content_hash` — the code path at `db/repositories/policy.rs:365-408` bumps neither. So the ETag is stable across metadata changes: two GETs returning different `name`s yield the *same* ETag (an RFC 9110 §8.8.1 violation), and two concurrent metadata editors both send the matching `If-Match`, both pass the guard (`WHERE current_version=$expected` with an unchanged version), and the later write wins silently — even with `require_if_match=true`. The code comments acknowledge this ("last-write-wins there," `policy.rs:366-369`) but it defeats the feature's purpose for the common case of editing a policy's description. Bundles do **not** have this gap: their tag is `updated_at`, bumped by every write (`api/bundles.rs:216-218`). Policies should adopt the same row-version source.

### R2-04 + R2-05 (P2) — New Plan 12 migration-apply reintroduces the lost-update and idempotency gaps

`apply_migration` (`db/repositories/datastore.rs:694-840`) is otherwise a careful single-transaction commit (records + model + version bump + append-only history + outbox — the D2 invariant is respected). But:

- **No optimistic concurrency on the model.** The model UPDATE (`:801-809`) is `SET model=$1, model_version=model_version+1 WHERE id=$3` with no `AND model_version=$expected`. The plan is recomputed server-side from transforms (good — a stale client plan can't be smuggled), but the plan's `entities_after` and `model_after` are materialized against a snapshot read at `prepare_migration` time (`api/datastore.rs:738`). Two concurrent applies, or an apply racing ordinary entity writes, both commit: the second's wholesale entity rewrite (`:782-798`, `UPDATE adm_entities … WHERE entity_id=$5`) and model overwrite reflect a stale snapshot and silently drop the other's effect. This is precisely the API-5 class Plan 07 fixed for policies, re-created in the new code. The plan already records `model_before`/`model_before_hash` (`:720,826`) — check the current model against it inside the transaction and 409 on drift.

- **No idempotency (R2-05).** `apply_migration` the handler (`api/datastore.rs:730-780`) publishes a new data version (`:767`) and fans out to the fleet (`notify_published`, `:768`) — it is exactly the "propagation-triggering POST" category Plan 07 Phase D targeted, yet it is not wrapped in `idempotency::run`. A retried timeout re-applies the transforms (rename→rename may no-op, but each success bumps `model_version` and re-publishes), producing a spurious model version and a redundant fleet convergence. Wrap it with scope `datastore.migrate` keyed on the transform fingerprint, consistent with `bundles.promote`/`bundles.rollback`/rollout/org-create.

### R2-06 (P2) — The contract gate proves the surface exists, not that it's usable

`tests/api_contract.rs` does two real things well: `no_undocumented_raw_routes` forbids any `.route(` outside a 3-entry allowlist (forcing every handler through `routes!` + `#[utoipa::path]`, so a route cannot be served without appearing in the single-sourced spec), and `openapi_spec_is_populated` asserts a valid OpenAPI 3.1 doc, ≥30 paths, anchor paths, non-empty `responses`, and **unique operationIds** (the collision check the persona asked about — good). CI adds `openapi-spec-validator` on both dumped specs (`ci.yml:228-235`).

What it does **not** enforce, and what an enterprise consumer needs:
- **No schema completeness.** A `responses` map need only be non-empty. Handlers returning `-> ApiResult<Json<Value>>` (all of `api/datastore.rs`, `api/decisions.rs`, `api/replay.rs`, `api/audit.rs`) document an untyped body; a generated client gets `object`/`any`. That is the bulk of the Plan 10/12 surface.
- **No error-response coverage.** Most `#[utoipa::path]` blocks document only success (e.g. `list_entities` has no responses beyond 200-shaped `Json`); 401/403/409/412/422/428 are not declared per operation.
- **No error-model schema at all.** `ProblemDetails` is `#[derive(Debug, Serialize)]` only (`api/error.rs:62`) — no `ToSchema`, so the RFC 9457 envelope never enters `components.schemas`. The very error contract Plan 07 built is invisible to the published spec.
- `openapi-spec-validator` is a structural validator, not a style linter — the DoD's alternative (`redocly lint`) would have caught missing descriptions/operation metadata. The weaker of the two options was chosen.

This is not a drift risk (the parity gate handles drift); it is a contract-quality ceiling. Add `ToSchema` DTOs for the `Json<Value>` handlers and `ProblemDetails`, declare the common error responses via a shared `responses(...)` fragment, and add a redocly ruleset (or an in-Rust assertion that every operation documents ≥1 4xx and has typed bodies).

### R2-07 (P2) — Outbound calls without timeouts can wedge a request task

ServiceNow is done right — `reqwest::Client::builder().timeout(Duration::from_secs(10))` (`integrations/servicenow.rs:62-63`). But `reqwest::Client::new()` (which has **no** default timeout) is used for the GitHub OAuth token exchange and user fetch (`api/oauth/github.rs:124,412`), GitHub App token minting (`sync/github_app.rs:119`), and — most concerning for an audit path — the **ClickHouse decision-query client** (`decisions/mod.rs:218`). A hung or slow upstream leaves the awaiting task parked with no deadline; under load these accumulate. Note also the `bundle_url.rs:71`/`s3.rs:57`/`api.rs:38` pattern `builder…timeout(…).build().unwrap_or_else(|_| reqwest::Client::new())` silently degrades to a no-timeout client if the builder ever fails — low probability, but the fallback drops the safety property. Centralize an `http_client(timeout)` helper and forbid bare `Client::new()` in non-test code via a grep lint.

---

## Absence checks performed

- **`require_if_match` default (prior open thread):** `config/server.rs:30,39` → field defaults `false`; env `REAPER_REQUIRE_IF_MATCH` override at `config/mod.rs:98-101`; enforcement branch `preconditions.rs:68-79`. **Confirmed warn-only by default** (ADR-3, intentional). Contract test for the matrix exists (`preconditions.rs:118-141`). The concurrency **integration** test (two writers, same base version → one 2xx/one 412) promised by DoD line 60 — I searched `services/reaper-management/tests/` for a two-writer policy test and did not locate one; the guarantee is unit-tested at the `check_precondition` and repo level but I found no end-to-end concurrent-writer test. Recommend adding it (R2-02/R2-03 make it load-bearing).
- **Idempotency coverage:** `idempotency::run` callers = `bundles.promote` (`api/bundles.rs:498`), `bundles.rollback` (`:549`), rollout-create (`deployments/rollouts.rs:75`), org-create (`orgs.rs:143`). Migration apply is **absent** (R2-05). Failed-op claim release + fingerprint separator-safety are unit-tested (`api/idempotency.rs:150-169`).
- **Pagination coverage:** present on `teams/sources/policies/agents/bundles/namespaces/change_requests/decisions/orgs`; **absent** on `datastore` entities/tuples/bindings (R2-01) and migration history (R2-11). `decisions` uses ClickHouse with its own bound.
- **RFC 9457:** `application/problem+json` content-type set (`api/error.rs:202-205`), `type/title/status/detail/code` present, `instance` **missing** (R2-08). Internal errors collapse to a generic detail and log the real cause (`:105-112`) — no stack/path leak. sqlx 23505/23514/23503 → 409/422/422 (`:157-186`) — correct.
- **Agent inbound auth (round-1 API-2):** default-deny middleware now mounted (`reaper-agent/src/main.rs:694-700`) and `/debug/datastore` compiled out of release unless `REAPER_DEBUG_ENDPOINTS` (`:680-688`); `CatchPanicLayer` outermost (`:710-712`); `panic="abort"` removed. Mechanism present — correctness deferred to the auth/security reviewer (note: the layer is only mounted when `AgentAuthVerifier::from_config` returns `Some`; whether "no auth config" fails open should be confirmed by that reviewer).
- **Mgmt default-deny + single surface (round-1 API-1/API-6):** router-level auth layer at `main.rs:297-305`; single `/api/v1` with `serve_root_alias` default-off (`config/server.rs:20-21`, `main.rs:282-293`). Addressed.
- **`#[non_exhaustive]`/`missing_docs`:** 2 hits workspace-wide, both `ApiError` (`api/error.rs:19`). `PolicyLanguage` and SDK/core enums unsealed; no `deny(missing_docs)`. Round-1 API-10 partially open (R2-09).
- **Outbound timeouts:** audited all `reqwest` client construction — ServiceNow/jwks/webhook/sso/sync builders set timeouts; oauth-github/github-app/clickhouse do not (R2-07).
- **New-code hygiene:** grepped `domain/migration.rs`, `domain/impact.rs`, `api/change_requests.rs`, `integrations/servicenow.rs` for `TODO/FIXME/unimplemented!/todo!/allow(dead_code)` — **none**. `migration.rs` (1058 lines) is well-shaped: typed `ModelTransform`/`RecordOp`/`PlanBlocker`, `inverse`/`compose_rollback`, fail-closed `applyable()`, with unit tests.
- **CI:** `api-contract` job runs both contract tests + spec validation; `cargo-deny` + `cargo-audit` blocking (`ci.yml:112-149`, 4 documented RUSTSEC ignores incl. the unfixable rsa Marvin advisory); `management-tests-postgres` real-PG job (`:352`); `mutation.yml` + `fuzz.yml` nightly. No coverage-gate job observed in `ci.yml` (tarpaulin is a make target only) — acceptable but worth noting.

## What's done well (≤5)

1. **The parity gate is structurally sound** — single utoipa-axum tree means router and spec cannot diverge on *presence*, and `no_undocumented_raw_routes` + unique-operationId enforcement make the guarantee durable (`tests/api_contract.rs`, `api/openapi.rs`).
2. **RFC 9457 done properly** — problem+json envelope, no message leaks, correct SQLSTATE 23505/23514/23503 → 409/422 mapping, `VersionConflict` → 412 (`api/error.rs`).
3. **Idempotency is DB-arbitrated, not advisory** — claim-then-complete under a unique constraint, fingerprint-bound (different body → 422), failed-op claim release for safe retry (`api/idempotency.rs`).
4. **Keyset pagination is correct** — `LIMIT n+1` sentinel, opaque `(created_at,id)` cursors, hard-max rejection (not silent clamp), with round-trip and no-drift tests (`api/pagination.rs`).
5. **The Plan 12 migration engine is disciplined** — server-recomputed plans (no client-smuggled state), fail-closed blockers, append-only history + inverse rollback, all record+model+outbox mutations in one transaction (`domain/migration.rs`, `db/repositories/datastore.rs:694-840`). The two gaps (R2-04/R2-05) are guard/idempotency omissions, not design flaws.
