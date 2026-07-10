# Availability & Resilience

**Readiness gate:** Operational resilience (Bank of England SS2/21, DORA Art. 11–12). Blocks enterprise/regulated deployment.
**Priority:** P1 (Synthesis #8) — one finding is borderline P0 (`panic = "abort"` on an enforcement sidecar).
**Findings closed:** Synthesis #8; Code API-3, API-11; Security P2-1; Perf P1-2.

---

## 1. Goal

Make a single Reaper enforcement node survive hostile or malformed input without aborting the process, and make every failure mode **fail closed** (deny / 500), never fail open (silent allow) and never crash-loop. Concretely:

1. A reachable `panic!`/`unwrap()`/`expect()` on any handler becomes an HTTP 500 for that one request, not a process abort that takes down every co-located workload trusting the sidecar.
2. The Reaper DSL is **total and terminating by construction**: a crafted deeply-nested policy cannot stack-overflow the parser, compiler, or evaluator.
3. The batch evaluation endpoint cannot let one request monopolize a runtime worker or drive unbounded CPU; batch size is bounded independently of the body-size limit and CPU work runs off the async reactor.
4. Every failure mode of the enforcement path (policy load failure, control-plane unreachable, audit sink down, evaluator error) has a **documented, tested** fail-open-vs-fail-closed decision.

Non-goal: control-plane HA/DR/backup (that is Product F5, a separate plan). This plan is the *single-node* resilience surface.

---

## 2. Current state (evidence) — file:line

- **`panic = "abort"` in the release profile** — `Cargo.toml:87-91` (`[profile.release]` sets `panic = "abort"`, with a comment justifying it for the hot path). This applies to the **service binaries** too, not just the engine microbench. `[profile.bench]` (`Cargo.toml:95-98`) deliberately omits it because criterion needs unwinding — so the pattern for a per-target override already exists.
- **No `CatchPanicLayer` on the agent router** — `services/reaper-agent/src/main.rs:490-539`. The router applies exactly one layer, `DefaultBodyLimit::max(256 * 1024 * 1024)` (`main.rs:538`), before `.with_state(state)`. No `tower_http::catch_panic::CatchPanicLayer`. With `panic = "abort"`, a panic cannot even be unwound per-request — it aborts the process. The same router is cloned for the UDS listener (`main.rs:542-543`), so both transports carry the exposure.
- **DSL recursion is not depth-bounded** — the grammar is directly left/right recursive: `reap.pest` `not_expr = { "!" ~ not_expr | primary_expr }` (`reap.pest:66-69`) and `primary_expr = { "(" ~ condition_expr ~ ")" | … }` (`reap.pest:71-72`). Pest builds the parse tree by recursive descent, so `!!!!…` or `((((…))))` recurses at **parse** time. The AST walk recurses again: `ReapAstEvaluator::evaluate_expr` (`reap/ast_evaluator/expr_eval.rs:18`) and the compiler `compile_condition` / `compile_expr_condition` (`reap/compiler/mod.rs:73,171`) are unbounded recursive descents over the same tree. A grep for `depth|MAX_DEPTH|recursion|stacker` over `reap/parser/*` and `reap/ast_evaluator/*` finds only the ReBAC traversal `max_depth` (a graph arg), not a syntactic-nesting bound (confirmed in Security P2-1's absence check). The parser entry point is `ReapParser::parse` (`reap/parser/mod.rs:35`); the compile entry point is `compile_policy` (`reap/compiler/mod.rs:36`). These endpoints are reachable over the network via the agent `/api/v1/policies/compile` and the management compile/validate endpoints.
- **Batch endpoint runs a synchronous unbounded CPU loop on the tokio worker** — `services/reaper-agent/src/handlers/evaluate.rs:766-890`. The doc comment (`evaluate.rs:768`) claims "in parallel using rayon", but the body is a sequential `requests.iter().enumerate().map(|(i, req)| { … evaluate … }).collect()` (`evaluate.rs:838-890`) running inline on the async worker — no `spawn_blocking`, no `par_iter` (grep confirms). Batch size is bounded only by the global 256 MB body limit (`main.rs:538`); there is no per-endpoint request-count cap. A 256 MB payload of tiny requests is millions of synchronous evals on one thread → head-of-line blocking on a 1–2 vCPU sidecar.
- **`Content-Disposition` header builder `unwrap` on attacker-controlled name** — `services/reaper-management/src/api/bundles.rs:229-253`. `filename = format!("{}-{}.rbb", bundle.name, bundle_id)` (`bundles.rs:229`) is interpolated raw into the `CONTENT_DISPOSITION` header value (`bundles.rs:234-237`), and the response is finalized with `builder.body(Body::from(download.data)).unwrap()` (`bundles.rs:253`). A `bundle.name` containing a newline/control char (or otherwise invalid header byte) makes `HeaderValue` construction fail, poisoning the builder so `.body().unwrap()` panics → process abort under `panic = "abort"`.
- **No documented failure-mode matrix** — Perf's audit note ("there is **no** fail-closed 'deny if audit unavailable' mode … capture is best-effort") confirms fail-open-vs-closed behavior is undocumented and partly unintended.

---

## 3. Definition of Done — testable checkboxes

- [x] `Cargo.toml` no longer sets `panic = "abort"` for the **service** binaries (agent, management, platform); the engine/bench may keep abort only where no network surface exists. `cargo metadata`/build confirms unwind for the service targets.
- [x] Both the agent router (`reaper-agent/src/main.rs`) and the management router carry a `CatchPanicLayer` that converts a handler panic into an HTTP **500** (never a 2xx), and the panic is logged with the correlation id. An integration test that hits a deliberately-panicking test route asserts a 500 and a still-alive process.
- [x] A network-reachable panic on a real handler (malformed bundle name, malformed policy) returns 500 and the process stays up — verified by a test that fires the request then a follow-up `/health` and gets 200.
- [x] The DSL rejects input whose syntactic nesting depth exceeds a configured limit (default e.g. 64) with a typed `InvalidPolicy` error at **parse or compile time** — before evaluation. A test feeds `"(" * 100_000` and `"!" * 100_000` and asserts a clean error, not a crash, on `ReapParser::parse` and on `compile_policy`.
- [x] The evaluator/compiler carries an independent runtime depth guard (defense-in-depth) so any tree that reaches it (e.g. via YAML/JSON policy formats bypassing the pest grammar) is bounded too. A test constructs a deep AST directly and asserts bounded behavior.
- [x] The DSL depth guarantee is documented in `docs/reference/reap-language.md` (or DSL_V2_DESIGN.md) as a normative "the language is total/terminating; max nesting depth = N" statement.
- [x] The batch endpoint enforces a `max_batch_requests` cap (config, default e.g. 1000) checked **before** evaluation, returning 400/413 on exceed; a test with `cap+1` requests asserts rejection.
- [x] Batch CPU work runs via `tokio::task::spawn_blocking` (and optionally rayon) so it cannot starve the async reactor; the eval endpoints carry a **smaller** `DefaultBodyLimit` than the bulk data-load endpoints (per-route layer), verified by a test.
- [x] The `download_bundle` header path sanitizes/escapes `bundle.name` and returns `ApiError::Internal` (500) instead of `unwrap()` on builder error; a test with a bundle name containing `"\r\n"` and quotes asserts a well-formed response or a clean 500, never a panic.
- [x] A committed **failure-mode matrix** (in `docs/deployment/OPERATIONS_GUIDE.md`) documents fail-open vs fail-closed for: policy load failure, control-plane/sync unreachable, audit sink down/full, evaluator error, and no-policy-loaded — each with the code path that enforces it and a test reference.
- [x] A clippy gate denies `unwrap_used`/`expect_used` in non-test reachable code (allowed in `#[cfg(test)]`), wired into `ci.yml`'s `lint-and-analyze` job.

---

## 4. Critical steps — ordered; per step what/where(files)/verify

**Step 1 — Stop weaponizing panics: drop `panic = "abort"` for services + add `CatchPanicLayer`.**
- *What:* Change the panic strategy for the service binaries to `unwind`, and wrap both service routers in a panic-catch layer that maps a panic to a fail-closed 500. Keep abort (if desired) only for pure-engine/bench targets with no inbound network surface.
- *Where:* `Cargo.toml:87-91` — remove `panic = "abort"` from `[profile.release]` (it inherits the LTO/codegen settings that matter for perf; abort was the only availability-hostile part). If the microbench truly needs abort, move it to a dedicated profile for that target only. Add `tower_http::catch_panic::CatchPanicLayer::custom(...)` to the layer stack in `reaper-agent/src/main.rs:490-539` (outermost, so it wraps all routes) and to the management router's layer stack. The custom handler must return a 500 (deny-equivalent), never a 2xx, and log with the correlation id.
- *Verify:* Add a `#[cfg(test)]`-gated `/panic-test` route (or use `tower::service_fn`) that panics; integration test asserts (a) response is 500, (b) a subsequent `/health` returns 200 (process alive). Confirm the perf micro-benchmark numbers are unchanged within noise (LTO/codegen-units retained; only landing pads added — measure with the existing criterion eval bench).

**Step 2 — Make the DSL total/terminating: compile-time nesting-depth limit.**
- *What:* Add a syntactic-nesting-depth bound enforced before evaluation, plus a runtime depth guard for non-pest policy formats. Prefer a cheap pre-parse scan (count max bracket/`!`/`&&`/`||` nesting) rejecting over-limit input, **and** a threaded `depth` counter in the recursive descents.
- *Where:* Parse entry `reap/parser/mod.rs:35` (`ReapParser::parse`) — reject input whose nesting exceeds `MAX_NESTING_DEPTH` before/after pest parse. Compiler recursion `reap/compiler/mod.rs:73` (`compile_condition`) and `:171` (`compile_expr_condition`) — thread a `depth: usize` param, return `ReaperError::InvalidPolicy` on exceed. AST evaluator `reap/ast_evaluator/expr_eval.rs:18` (`evaluate_expr`) — same guard for the interpreted path. Grammar sites to bound: `reap.pest:66` (`not_expr`), `:71` (`primary_expr`), `:45-63` (`condition`/`or_expr`/`and_expr`). Consider the `stacker` crate only as a fallback for legitimately-deep-but-bounded trees; a hard cap is simpler and sufficient.
- *Verify:* Unit tests feeding `"(".repeat(100_000)`, `"!".repeat(100_000)`, and a deeply right-nested `a || (a || (a || …))` assert a typed error (not SIGSEGV) from both `ReapParser::parse` and `compile_policy`. Run the test under a small thread stack (e.g. spawn a thread with 256 KB stack) to prove no overflow. Add to the existing proptest differential suites a max-depth generator.

**Step 3 — Bound and offload the batch endpoint.**
- *What:* (a) Reject batches over a configured request-count cap before any evaluation; (b) run the CPU loop in `spawn_blocking` (optionally rayon `par_iter`); (c) apply a tighter per-route body limit to eval endpoints than to bulk data-load endpoints.
- *Where:* `reaper-agent/src/handlers/evaluate.rs:776-890` — add `if payload.requests.len() > cfg.max_batch_requests { return 413 }` before the loop at `:824`; wrap `:838-890` in `tokio::task::spawn_blocking`. In `reaper-agent/src/main.rs:497-502` split the layer at `:538`: apply a small `DefaultBodyLimit` to `/api/v1/{messages,fast-messages,batch-messages,check}` and keep 256 MB only on `/api/v1/data*` and `/api/v1/entities*` via `.route_layer(...)` per route group. Add `max_batch_requests` to the agent config.
- *Verify:* Test that `max_batch_requests + 1` requests returns the cap error before evaluation; a load test with a large batch confirms unrelated `/health` and `/api/v1/messages` latency is not starved (measure p99 of a concurrent single-eval stream during a large batch). Update the now-accurate doc comment at `evaluate.rs:768`.

**Step 4 — Sanitize the bundle download header builder.**
- *What:* Percent-encode / strip control characters from `bundle.name` before it enters the `Content-Disposition` value (RFC 6266 `filename*`), and replace `.unwrap()` with a mapped `ApiError::Internal`.
- *Where:* `services/reaper-management/src/api/bundles.rs:229-253` — sanitize at `:229`, build the header defensively, and change `:253` `builder.body(...).unwrap()` to `.map_err(|e| ApiError::Internal(...))?`.
- *Verify:* Test `download_bundle` with a bundle whose name contains `"\r\nSet-Cookie: x"`, embedded quotes, and Unicode — assert a valid single-header response (no header injection) or a clean 500, and no panic. This also closes a header-injection/response-splitting vector.

**Step 5 — Author and test the fail-open-vs-fail-closed matrix.**
- *What:* Enumerate every enforcement failure mode and its intended behavior; make the code match; document it.
- *Where:* Document in `docs/deployment/OPERATIONS_GUIDE.md`. Verify against code: no-policy-loaded and evaluator error already deny (`evaluate.rs:858,868` map `Err → PolicyAction::Deny` — fail closed, good); policy-load failure at deploy should reject and keep the last-good policy (hot-swap is atomic per Perf's RCU note); audit-sink-down is currently best-effort drop-and-count (Perf absence check) — document this as fail-**open on audit** and add a config'd optional "deny if audit unavailable" mode for regulated deployments; control-plane unreachable → agent keeps serving last-good bundle (document as intentional availability choice, with anti-rollback deferred to the distribution plan).
- *Verify:* One integration test per row of the matrix asserting the documented behavior (e.g. kill the decision writer queue and assert requests still deny-on-error / or deny-all if the mandatory-audit flag is set).

**Step 6 — Stop the bleed: clippy gate on `unwrap`/`expect` in reachable code.**
- *What:* Turn on `clippy::unwrap_used` and `clippy::expect_used` (allow within `#[cfg(test)]`) so new reachable panics don't reappear.
- *Where:* Workspace lint config (`Cargo.toml` `[workspace.lints]` or crate-level `#![deny(...)]`), enforced by the existing `ci.yml:37` `lint-and-analyze` job (`cargo clippy --workspace --all-targets -- -D warnings`, `ci.yml:65`).
- *Verify:* CI fails on a newly-introduced reachable `unwrap()`; existing test-only unwraps are unaffected.

---

## 5. Dependencies

- `tower-http` `catch-panic` feature (already a dependency via axum stack — confirm the feature flag is enabled).
- Agent config plumbing for `max_batch_requests` and the optional `mandatory_audit`/`deny_if_audit_unavailable` flag (reuse the existing `agent_config` in `AgentState`).
- Coordinates with **Plan 06 (Supply Chain)**: the DSL fuzz targets there exercise the same parser/compiler this plan bounds — build them together so the depth limit is fuzz-verified.
- Loosely coupled to the distribution/anti-rollback work (Security P1-1): the "control-plane unreachable" row of the matrix references it but does not block on it.
- No schema/DB migration required; no external service dependency.

---

## 6. Testing & verification

- **Panic isolation:** integration test with a panicking route → 500 + process-alive assertion (Step 1). Fuzz-style: feed malformed JSON/bundle to real handlers and assert no process exit.
- **DSL totality:** unit + proptest with adversarial-depth generators; run under a constrained thread stack to force overflow if the guard is absent (Step 2). Reuse the existing `differential_parity_tests` / `compiled_ast_equivalence_tests` harness (invoked in `ci.yml:108`).
- **Batch bounds:** cap-rejection test + concurrent-latency starvation test (Step 3).
- **Header safety:** injection/panic test on `download_bundle` (Step 4).
- **Matrix:** one test per failure mode (Step 5).
- **Regression fence:** clippy `unwrap_used`/`expect_used` gate (Step 6); add a criterion run to confirm the `panic=abort`→`unwind` change does not regress the hot path beyond noise (the `eval-microbench` job, `ci.yml:776`).
- **Falsifiable acceptance:** `grep -R "panic = \"abort\"" Cargo.toml` returns nothing under a service profile; `grep -R "CatchPanicLayer" services/` returns both routers; `grep -R "MAX_NESTING_DEPTH\|max_batch_requests" crates/ services/` returns the new guards.

---

## 7. Effort & phasing — S/M/L

| Phase | Scope | Size |
|-------|-------|------|
| **P0 hotfixes** | Step 1 (drop abort + CatchPanicLayer) and Step 4 (header unwrap) — highest risk, smallest diff | **S** |
| **DSL hardening** | Step 2 (parse + compile + eval depth bound, docs, proptest) | **M** |
| **Batch + limits** | Step 3 (cap, spawn_blocking, per-route body limits, config) | **M** |
| **Governance** | Step 5 (matrix + per-mode tests) and Step 6 (clippy gate) | **M** |

Recommended order: P0 hotfixes (ship immediately) → DSL hardening (co-develop with Plan 06 fuzz) → Batch → Governance. Total ~ one focused sprint; the S phase is a day.

---

## 8. Key decisions (ADR-style)

- **ADR-1: Drop `panic = "abort"` for service binaries; keep LTO/codegen-units.** *Context:* abort turns any reachable panic into a host-availability incident; the perf comment conflates abort with the (retained) LTO win. *Decision:* services build with `unwind`; abort is confined (if at all) to pure-engine bench targets, mirroring the existing `[profile.bench]` carve-out. *Consequence:* landing pads add negligible size/latency; panics become recoverable 500s. *Alternative rejected:* keep abort + audit every unwrap — infeasible at ~957 call sites and fragile.
- **ADR-2: Enforce a hard nesting-depth cap rather than growing the stack (`stacker`).** *Decision:* a policy language for authorization should be total/terminating by construction; a bounded cap (default 64, config'd) is simpler, cheaper, and a documentable guarantee. *Consequence:* pathologically deep (but "legitimate") policies are rejected — acceptable and far safer than unbounded recursion. *Alternative rejected:* `stacker` merely delays overflow and hides the DoS.
- **ADR-3: Fail closed on evaluation, but treat audit-availability as a configurable posture.** *Decision:* eval errors and no-policy → deny (already true); audit-sink-down defaults to best-effort (availability) but offers an opt-in "deny if audit unavailable" mode for regulated tenants (ties to Security P1-2). *Consequence:* the operator, not the code, chooses the availability/compliance trade-off — and it is documented.
- **ADR-4: Batch size is bounded by an explicit request count, decoupled from the 256 MB body limit.** *Decision:* the body limit exists for bulk entity loads and must not double as a batch bound. *Consequence:* eval endpoints get a tighter body limit and a request-count cap; data endpoints keep 256 MB.

---

## 9. Risks & rollback

- **Risk: dropping `panic = "abort"` regresses the sub-µs hot path.** *Mitigation:* LTO + codegen-units=1 retained; only unwind tables added. Measure with the `eval-microbench` criterion gate before/after; expected delta is within runner noise. *Rollback:* revert the one-line profile change (CatchPanicLayer is independent and can stay).
- **Risk: depth cap rejects a real customer policy.** *Mitigation:* default generous (64), configurable, and the error is a clear typed message, not a silent failure. *Rollback:* raise/disable the cap via config without a redeploy of logic.
- **Risk: `spawn_blocking` for batch changes ordering/latency characteristics.** *Mitigation:* results already carry an explicit `index` field (`evaluate.rs:884`), so order is preserved regardless of parallelism. *Rollback:* the cap alone (without offloading) already removes the unbounded-CPU DoS; offloading can be reverted independently.
- **Risk: CatchPanicLayer masks a bug that should crash-and-restart.** *Mitigation:* every caught panic is logged at error with correlation id and increments a metric; alerting on that metric restores the "loud failure" signal without the outage. *Rollback:* the layer is a single `.layer()` call, trivially removable.
- **General rollback:** every step is an isolated, independently-revertable change (profile line, one layer, one guard param, one handler edit, one doc + tests). No data migration, no wire-format change, so rollback is code-only and instant.
