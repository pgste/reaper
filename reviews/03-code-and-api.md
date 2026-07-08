# Review 03 — Code Quality & API Design

**Reviewer persona:** Staff engineer, API governance. Two lenses: (1) is this Rust
codebase maintainable by a team for a decade, and (2) are these APIs publishable to
enterprise customers without embarrassment?
**Scope covered:** control-plane API surface (`reaper-management` 19 route files),
agent enforcement API, error model, Rust error handling, panic discipline, testing/CI,
DSL spec, supply chain. **Not deeply covered:** platform (legacy), SDK ergonomics,
billing/oauth handler internals, sync engine internals (deferred to distribution reviewer).

---

## VERDICT: NOT READY

Two P0s block: the control-plane bundle API and the entire agent enforcement API ship
with **no authentication and no tenant scoping** — the `{org}` path segment on bundle
routes is decorative (`let _org_id = …`) and the agent's `/api/v1/*` surface (policy
deploy, bundle load, entity writes, decision read, `/debug/datastore`) has no auth layer
at all. Authorization correctness is formally the Security reviewers' call, but the code
evidence is unambiguous and it is also an API-governance failure (opt-in per-handler auth
with no default-deny). Even discounting authz, the product would be **CONDITIONAL** at
best: no API spec, no concurrency control on policy/bundle edits, and `panic = "abort"`
turning any reachable `unwrap()` into a full enforcement-node crash.

The engineering fundamentals underneath are better than the surface suggests (typed
errors, an I/O-free engine crate, guarded hot-path unwraps, differential property tests).
The failures are at the API boundary and in operational hardening, not in the core.

---

## Findings table

| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| API-1 | P0 | `api/bundles.rs` all handlers; `bundle/service.rs:113` | Bundle endpoints have no `RequireAuth`; `org` resolved then discarded (`let _org_id`); `bundle_service.get(bundle_id)` not org-scoped | Unauthenticated cross-tenant read/update/promote/download/delete of any bundle by UUID | Add default-deny auth layer; scope every bundle lookup by `org_id` |
| API-2 | P0 | `reaper-agent/src/main.rs:490–538` | Agent enforcement API has zero inbound auth; `/debug/datastore` exposed; policy-deploy/bundle-load/entity-write all open | Anyone reaching the agent can deploy policies, overwrite entity data, and dump the datastore (PII for ABAC) | Require mTLS/token on all agent routes; remove `/debug/*` from prod router |
| API-3 | P1 | `Cargo.toml:90` + ~957 `unwrap/expect` in prod src | `panic = "abort"` in release + no `CatchPanicLayer` | Any reachable panic aborts the whole process; sidecar crash = authz outage or fail-open bypass | Drop `panic=abort` for services, add `CatchPanicLayer`, deny `unwrap` in reachable paths via clippy |
| API-4 | P1 | repo-wide (searched `*openapi*`,`utoipa`,`swagger`) | No OpenAPI/API spec of any kind | No contract, no generated clients, no drift detection, no publishable reference | Adopt `utoipa`/`aide` generated spec; gate CI on handler↔spec parity |
| API-5 | P1 | `api/policies.rs:195` `update_policy`; `api/bundles.rs:119` `update_bundle`; `db/repositories/policy.rs:229` | No ETag/`If-Match`/expected-version guard on updates | Lost update: concurrent policy edits silently clobber each other | Add `If-Match`/version column with `WHERE version = $expected`, return 409 on mismatch |
| API-6 | P1 | `main.rs:215` | API mounted at **both** root and `/api/v1`; `api::api_v1_routes` is dead code | Unversioned root surface is permanent; no deprecation policy; breaking changes have no path | Serve only under `/api/v1`; document versioning + deprecation policy |
| API-7 | P2 | `api/error.rs` | Error body is custom `{error:{code,message}}`, not RFC 9457 problem+json; `sqlx::Error`→500 hides 409s | Non-standard errors; unique-constraint conflicts surface as 500 not 409 | Emit `application/problem+json`; map DB constraint errors to 409/422 |
| API-8 | P2 | `api/agents.rs:231`, `namespaces.rs:169`, `sources.rs:120`, `teams.rs:66` | List endpoints return all rows, no limit/pagination | Unbounded responses at fleet scale (thousands of agents) | Enforce default+max page size on every list endpoint |
| API-9 | P2 | `.github/workflows/` (searched) | No `cargo audit` / `cargo deny` / SBOM | Vulnerable/yanked deps ship undetected; no license governance | Add `cargo-audit` + `cargo-deny` gates |
| API-10 | P2 | no `#[non_exhaustive]`/`#[deny(missing_docs)]` anywhere (searched) | Public enums (`ApiError`, error types, `PolicyLanguage`) not `#[non_exhaustive]`; SDK/core undocumented | Adding an enum variant is a breaking change; no doc-coverage gate on published SDK | `#[non_exhaustive]` on growable public types; `#[deny(missing_docs)]` on `reaper-sdk`/`reaper-core` |
| API-11 | P2 | `api/bundles.rs:253` | `Response::builder()…body().unwrap()` with user-controlled `bundle.name` in `Content-Disposition` | Crafted bundle name → invalid header → panic → process abort (API-3) | Return `ApiError::Internal` on builder error; sanitize filename |
| API-12 | P3 | `api/bundles.rs:420–460` | `get_bundle_diff` does `policy_repo.get_by_id` in a loop (N+1) | Slow diffs on large bundles | Batch fetch policies |
| API-13 | P3 | `api/policies.rs`, `api/decisions.rs:486` | Offset pagination, not cursor | Deep-page drift/perf on the biggest tables | Prefer keyset/cursor pagination |
| API-14 | P3 | no `Idempotency-Key` handling (searched) | Mutation endpoints (promote, rollout, org create) lack idempotency keys | Automation retries after timeouts double-apply (partly mitigated by `ActiveRolloutExists`→409) | Accept `Idempotency-Key` on propagation-triggering POSTs |

---

## Detailed findings

### API-1 (P0) — Control-plane bundle API is unauthenticated and cross-tenant

Every handler in `services/reaper-management/src/api/bundles.rs` (`get_bundle`,
`update_bundle`, `delete_bundle`, `promote_bundle`, `stage_bundle`, `download_bundle`,
`add_policies`, …) takes **no `RequireAuth` extractor**. Compare with `api/decisions.rs`,
`api/agents.rs`, and `api/deployments/rollouts.rs`, which all take `RequireAuth(user)` and
call `user.has_permission(...)`. Auth in this service is **opt-in per handler** — there is
no global auth middleware. The middleware stack in `main.rs:219–232` is
`security_headers → correlation_id → request_metrics → body_size_limit → access_log →
TraceLayer` (+ optional rate limit). No auth layer.

Worse, the `{org}` path segment is resolved and then thrown away:

```rust
// api/bundles.rs:109
async fn get_bundle(State(state)…, Path((org, bundle_id)): Path<(String, Uuid)>) … {
    let _org_id = parse_org_id(&org, &state).await?;   // discarded
    let bundle = state.bundle_service.get(bundle_id).await?;  // not org-scoped
    Ok(Json(bundle))
}
```

`bundle_service.get` (`bundle/service.rs:113`) is `repo.get_by_id(bundle_id)` with no
`org_id` predicate. Any caller can read/update/promote/download/delete **any org's
bundle** by guessing/enumerating UUIDs — and, because there is no auth, without
credentials at all. `promote_bundle` broadcasts `BundlePromoted` over SSE, so this is also
a cross-tenant policy-injection primitive.

**Remediation:** (1) wrap `build_api_router()` in a default-deny auth layer so a handler
that forgets `RequireAuth` fails closed; (2) thread `org_id` into every bundle repository
call (`get_by_id_scoped(org_id, bundle_id)`); (3) add a test asserting org B cannot fetch
org A's bundle. Security reviewers own the definitive exploitability call, but the code
path is unconditional.

### API-2 (P0) — Agent enforcement API has no auth and ships a debug datastore dump

`services/reaper-agent/src/main.rs:490–538` builds the full agent router. The only layer
applied is `DefaultBodyLimit` (line 538). Grepping the agent for `Authorization`, `Bearer`,
`ApiKey`, `RequireAuth`, `AuthenticatedUser` finds hits **only** in
`management/client.rs` and `management/sse.rs` — i.e. the agent authenticating *outbound*
to management. Nothing authenticates *inbound* requests. Exposed unauthenticated:

- `POST /api/v1/policies/deploy`, `/api/v1/bundles/load` — anyone can hot-swap policy.
- `POST /api/v1/entities`, `/api/v1/data/apply-deltas` — anyone can rewrite ABAC data.
- `GET /api/v1/decisions*` — audit log read.
- `GET /debug/datastore` (line 532) — dumps the in-memory datastore (entities/relationships,
  potentially PII) in a **production** router.

This may be defensible for a localhost/UDS-only sidecar, but the agent binds TCP :8080 and
the k8s/Helm manifests expose it. **Remediation:** enforce mTLS or a shared token on all
agent routes; compile `/debug/*` out of release builds (`#[cfg(debug_assertions)]`) or gate
behind an explicit env flag that defaults off.

### API-3 (P1, borderline P0) — `panic = "abort"` weaponizes every reachable `unwrap`

`Cargo.toml:90` sets `panic = "abort"` for `[profile.release]`. There is no
`tower_http::catch_panic::CatchPanicLayer` on either service router (searched). With abort
semantics a panic cannot even be caught per-request — **one panic terminates the whole
process**. For a policy-enforcement sidecar that is a host availability incident (map's own
framing), and if the calling app fails-open on agent unavailability it becomes an authz
bypass.

The raw ~957 `unwrap/expect` count is inflated by inline `#[cfg(test)]` modules (62 files
in `policy-engine` alone). The **eval hot path is disciplined** — e.g. `evaluate.rs:581`
`v.as_i64().unwrap()` is guarded by an `is_i64()` check on the line above, and the other
three unwraps in that file are in tests. Good. But the exposure is the *breadth* of the
network surface, not the hot path: the reap parser (`reap/parser/*` ~36 non-test unwraps on
`pest` pair navigation, reachable via the management compile/validate endpoints), and the
agent data handlers (`handlers/data.rs`, `entities.rs` — 61 unwraps incl. inline tests) run
on attacker-supplied input. A single parser/grammar mismatch or a malformed data payload
that reaches one of these aborts the process.

**Remediation:** set `panic = "unwind"` for the service binaries (keep abort only for the
engine microbench if desired), add `CatchPanicLayer` to both routers so a handler panic
becomes a 500 instead of a crash, and turn on `clippy::unwrap_used`/`expect_used` (allow in
`#[cfg(test)]`) to stop the bleed.

### API-4 (P1) — No API specification exists

Confirmed the map's finding: searches for `openapi`, `swagger`, `utoipa`, `aide` across
`*.rs` and `*.toml` return nothing. A 37k-LOC control plane with 19 route files, plus the
agent API, has **no machine-readable contract**. Consequences: customers can't generate
clients, there's no way to detect handler drift, no reference docs to ship, and no
contract tests are possible. For an enterprise security product this is a credibility
problem in the first customer security review. **Remediation:** annotate handlers with
`utoipa` (or derive from `aide`), publish `/openapi.json`, and add a CI job asserting every
route in the router appears in the spec.

### API-5 (P1) — Lost-update on policy and bundle edits

`update_policy` (`api/policies.rs:195`) and `update_bundle` (`api/bundles.rs:119`) accept a
body and write unconditionally. The policy repo (`db/repositories/policy.rs:205–229`) reads
`current_version`, computes `current_version + 1`, and writes — server-managed monotonic
versioning for *history*, but with **no optimistic-concurrency guard**: there is no
`WHERE … AND current_version = $expected`. Two concurrent editors both read version N, both
write N+1, and the later write silently wins; there is no `If-Match`/ETag on the request
either (searched — only S3/webhook `etag` fields exist, unrelated). In a regulated setting
"who changed this policy and did they see the version they edited" must hold. This is the
persona's explicit P1 ("lost-update anywhere in policy editing"). **Remediation:** return an
ETag (content hash already exists as `content_hash`) on GET, require `If-Match` on
PUT, and make the UPDATE conditional on the expected version → 409 on mismatch.

### API-6 (P1) — Unversioned surface baked in permanently

`main.rs:214–215`:

```rust
let api_router =
    api::build_api_router().merge(Router::new().nest("/api/v1", api::build_api_router()));
```

Every route is served at **both** the bare root (`/orgs/...`) and `/api/v1/orgs/...`. The
comment says the root form exists for "existing consumers/tests." The bare-root surface is
now a permanent compatibility obligation with no version namespace, and `api::api_v1_routes`
(`api/mod.rs:56`, `#[allow(dead_code)]`) is dead. There is no documented versioning or
breaking-change policy anywhere. **Remediation:** serve only `/api/v1`, migrate internal
callers/tests, and write a one-page compatibility policy (what's stable, deprecation window,
sunset headers).

---

## Absence checks performed

- **OpenAPI/spec:** searched `openapi`, `swagger`, `utoipa`, `aide` in `*.rs`/`*.toml` — none. (API-4)
- **Global auth middleware:** read `main.rs:206–243` layer stack (mgmt) and `main.rs:490–538` (agent) — none; auth is per-handler extractor only. (API-1/2)
- **Idempotency keys:** searched `Idempotency-Key`/`idempotency` — only a test comment; no header handling. (API-14)
- **Concurrency control:** searched `If-Match`/`ETag`/`expected_version` — none on mutation handlers; repo UPDATEs are unconditional. (API-5)
- **Panic isolation:** searched `CatchPanic`/`catch_panic` — none; `panic="abort"` present (`Cargo.toml:90`). (API-3)
- **`anyhow` in libraries:** grepped `policy-engine`/`reaper-core` src — **zero**; libraries use `thiserror`. (positive)
- **Engine I/O purity:** read `policy-engine/Cargo.toml [dependencies]` — no tokio/reqwest/axum/sqlx in non-dev deps. (positive)
- **`#[non_exhaustive]` / `#[deny(missing_docs)]`:** grepped `crates/`,`services/` — only one unrelated `finish_non_exhaustive` Debug helper. (API-10)
- **Fuzzing:** no `fuzz/` or `fuzz_targets` dir; but `proptest` differential harnesses exist (`differential_parity_tests.rs`, `check_mode_differential_tests.rs`, `delta_sync_differential_tests.rs`). (partial positive)
- **Supply chain:** searched `.github/` for `cargo audit`/`cargo deny` — none. (API-9)
- **Decision list bound:** `decisions/mod.rs:486` caps `limit.min(1000)` with parameterized ClickHouse query — bounded and injection-safe. (positive)
- **DSL spec:** `docs/reference/reap-language.md` (656 lines) + `docs/development/DSL_V2_DESIGN.md` (176) exist — the language is specified in docs, not only in the parser, so the persona's "unspecified language = P1" does **not** apply. Did not audit the spec for normative completeness vs. the parser grammar; recommend a follow-up parity check.

---

## What's done well (≤5)

1. **Typed errors, no `anyhow` soup** in `policy-engine`/`reaper-core`; boundary errors use
   `thiserror` with `#[from]` conversions (`api/error.rs`, `bundle.rs:285`).
2. **Engine crate is I/O-free** — `policy-engine`'s production `[dependencies]` carry no
   async runtime, HTTP, or DB crate; a clean ports/adapters boundary between engine and the
   services that host it.
3. **Eval hot-path unwraps are guarded**, not reckless — the raw ~957 count is dominated by
   inline test modules; the request-parsing fast path checks before it unwraps
   (`evaluate.rs:581`).
4. **Differential property testing** of the parser/evaluator (`differential_parity`,
   `check_mode`, `delta_sync` proptest suites) — stronger than the "no fuzz targets" line
   alone implies, though `cargo-fuzz` on the parser is still worth adding.
5. **Decision query API is bounded and parameterized** (`decisions/mod.rs:486`) — capped
   limit, no string interpolation into SQL, with a test asserting the cap.
