# Workstream F2 — Wasm Eval Build Target (scoping)

Strategic bet (`reviews/round-2/06-future-architecture.md` §"WebAssembly
everywhere", backlog `plans/round-2/00-NEXT-BACKLOG.md` Workstream F). Not
remediation.

## STATUS (2026-07-15) — Slices 1–3 LANDED (workstream complete)

**Slice 3 landed:**
- **Check mode on the wasm surface**: `checkDocument(source, input, action,
  resource)` wraps the AST evaluator's `check_with_input` (all violated deny
  rules + rendered messages, CLI/agent semantics). The 17 policy-library
  document cases now run on BOTH parity legs (native wrapper + wasm-in-Node)
  with allowed flags and exact violation sets asserted — the full 82-case
  library corpus is covered cross-target, nothing skipped.
- **Measurement correctness** (wasm-bench): the per-request timed window now
  contains ONLY the boundary crossing + evaluation (decision JSON parsing
  moved outside it — it was inflating µs-scale percentiles ~15-30%), and
  throughput derives from the same timed window as the latencies (wall time
  incl. harness reported separately). rbac reference: p50 7→6µs,
  116k→128k rps after the correction.
- **npm packaging**: `scripts/package-npm.mjs` stamps publishable-shaped
  `package.json` (version from Cargo.toml, private mirroring publish=false)
  into pkg-node (`@reaper/wasm`) and pkg-web (`@reaper/wasm-web`).
- **Browser**: web-target bindings built in CI + `demo/index.html`
  (self-contained; deploy/evaluate in-page) smoke-tested headless in
  Chromium through the real click handlers via `?autorun` — locally and as
  a blocking CI step.

## STATUS (earlier) — Slices 1–2

**Slice 2 landed** — `crates/reaper-wasm`: a cdylib+rlib wasm-bindgen wrapper
over the engine (`ReaperEngine`: `deployPolicy` / `loadEntitiesJson` /
`evaluate` / `evaluateAll` / `policyCount` / `setNowUnixNs`+`clearInjectedNow`
clock pinning on wasm). JSON at the boundary, matching the engine's
serialized `PolicyDecision`/`AllPoliciesEvaluationResult` shapes; request
building mirrors the agent exactly (principal-only context injection, agent
fast-path scalar coercion). **Three-leg parity contract in CI** (`wasm-build`
job): the policy-library manifests gate the AST+compiled evaluators
(pre-existing), the native wrapper (`tests/parity.rs`), and the actual wasm
artifact in Node (`tests/node/smoke.mjs`) — 10 scenarios / 65 authz cases per
leg, plus injected-clock determinism and error-surface checks through the
wasm boundary. wasm-bindgen-cli is pinned (0.2.126) to the crate version; the
Node bindings are a CI artifact (`reaper-wasm-node`), not committed.
Document/check-mode cases (17) are out of slice-2 scope → slice 3.

**Compiled-primary contract (wasm = native):** the wrapper deploys through
`build_preferred`, so the compiled DSL v2 evaluator is the PRIMARY tier on
wasm exactly as on the agent (AST interpreter only as feature fallback,
decision-identical by the equivalence differential). Enforced, not assumed:
`evaluatorType(policyId)` is exported; the native parity leg checks the
reported tier against independent compiler ground truth per scenario and
pins the fallback set in `tests/fixtures/ast-fallback-scenarios.json`
(currently **empty — all 10 library scenarios compile**), and the Node leg
asserts the same tier through the actual wasm artifact.

## STATUS (earlier) — Slice 1

**Slice 1 landed** (decisions confirmed: JS-first packaging, Cedar excluded
from wasm builds, host-injectable clock with JS fallback, crate at
`crates/reaper-wasm` when slice 2 lands):

- `policy-engine` gained its first `[features]` section:
  `default = ["cedar", "batch", "audit-buffer"]` — native consumers are
  unchanged; the wasm build selects down with `--no-default-features`.
  `cedar` gates the Cedar evaluator (a `PolicyLanguage::Cedar` deploy without
  it fails with a clear error), `batch` gates rayon + `BatchEvaluator`,
  `decision-privacy` gates aes-gcm/hmac crypto, `audit-buffer` (implies
  decision-privacy) gates the tokio/thread/fs decision buffer + export.
- New `policy-engine::clock` module: everything the eval path reaches
  (`PolicyEngine::evaluate`/`evaluate_set`/`evaluate_package`/`evaluate_all`
  latency probes, `DataLoader` timing, DSL `time::now*` builtins on both the
  AST and compiled paths) reads time through `clock::Stopwatch`/`now_unix_ns`.
  Native: `Instant`/`SystemTime` passthrough. wasm32: host-injected epoch
  (`set_injected_now_unix_ns`, deterministic/replayable) with `chrono`
  `wasmbind` (JS `Date`) fallback; builtins fail closed when no clock exists.
- getrandom wasm backends wired: `wasm_js` on getrandom 0.4 (reaper-core's
  keygen) + `js` on the 0.2 node pulled by the ed25519/p256 stack, and the
  `getrandom_backend="wasm_js"` rustflag in `.cargo/config.toml`.
- **CI gate**: blocking `wasm-build` job in `ci.yml` builds
  `cargo build -p policy-engine --no-default-features --target
  wasm32-unknown-unknown --locked` (debug + release). This is the regression
  firewall for the whole bet.
- Verified: native default-feature build byte-equivalent in behavior — full
  policy-engine suite green (637 lib + all integration suites), clippy
  `-D warnings`, fmt; wasm32 debug + release builds green locally.

**Remaining:** slice 2 (`reaper-wasm` cdylib + wasm-bindgen API + npm
packaging + Node smoke test), slice 3 (native↔wasm decision-parity
differential over the policy corpus, docs, demo).

## Goal (restated)

Ship the policy-eval core as a real WebAssembly artifact — not just
wasm-portable source. Concretely: `cargo build --target wasm32-unknown-unknown`
succeeds for the engine, a `reaper-wasm` cdylib exposes a minimal
"load policy + load data + evaluate" API, CI builds and gates the target, and
the artifact is embeddable in a browser, an edge worker (Cloudflare/Fastly), or
a Node/Python/Go host. This is the distribution vehicle for F1's agentic-authz
gate ("the in-process gate in an MCP tool server or an edge worker",
06-future-architecture.md §3).

The strategist's framing: the source tree is ~80% ready ("leading in source,
absent from the product"); OPA and Cedar both went to wasm for exactly this
surface expansion. Do it "before someone asks 'does it run in the browser?' and
you have to say 'almost.'"

## Inventory — reuse vs. build

### Already in place (reuse)

- **I/O-free engine core** — `policy-engine` prod deps carry no tokio-rt/sqlx/
  reqwest/axum; the eval path is deterministic and embeddable. This is the whole
  reason F2 is cheap.
- **JSON backend splits** — five `cfg(target_arch = "wasm32")` sites already
  swap `sonic-rs` (native SIMD) → `serde_json`: `fast_parse.rs` (request
  parsing, 3 entry points), `reap/ast_evaluator/builtin_functions/json.rs`
  (DSL `json::*` builtins), `data/loader.rs:104` (`load_json_batch`),
  `decision_log.rs:1058` (`to_ndjson`). Manifest gate at
  `crates/policy-engine/Cargo.toml:46-47`.
- **uuid wasm-ready** — workspace `uuid` already carries the `js` feature
  (root `Cargo.toml:47`), so `Uuid::new_v4()` sites are covered.
- **String-based load path** — the wasm-friendly entry points already exist:
  `impl FromStr for ReaperPolicy` (`reap/mod.rs:192`), `from_yaml_str` (`:55`),
  `from_json_str` (`:71`). No new parse surface needed; the `std::fs` variants
  (`from_file*`) simply stay native-only.
- **Marshallable DTOs** — `PolicyRequest` / `PolicyDecision` / `PolicyAction`
  (`engine/types.rs:210,218,16`) are Serialize/Deserialize — ready to cross the
  JS boundary as-is.
- **Single-request eval path is thread-free** — `PolicyEngine::evaluate`
  (`engine/mod.rs:420`) uses no rayon, no spawned threads, no RNG.
- **Precedent for feature-gating** — `reaper-ebpf` already uses
  `default = []` + opt-in features; same pattern, never applied to the engine.

### Must build (the real gaps)

1. **Feature flags — none exist.** `policy-engine` and `reaper-core` have no
   `[features]` section at all. `cedar-policy 4.2`, `rayon`, `tokio(sync,time)`,
   and `aes-gcm` are unconditional. A wasm build needs:
   - `cedar` (gates `evaluators/cedar*.rs`, and its uses in
     `partial_evaluation.rs` / `policy_compilation.rs`)
   - `batch` (gates `rayon` + `batch.rs` — rayon's pool doesn't exist on wasm)
   - `decision-privacy` (gates `aes-gcm`/`OsRng` in `decision_privacy.rs`)
   - buffer thread: `decision_buffer.rs:625` background writer needs gating or
     a wasm-safe no-op mode.
   Native default feature set = all-on (zero behavior change for every existing
   consumer); the wasm build selects down.
2. **Clock shim.** `wasm32-unknown-unknown` has no clock: `Instant::now()` in
   `PolicyEngine::evaluate` (`engine/mod.rs:425`, latency probe) and the DSL
   `time::*` builtins (`builtin_functions/time.rs:15-52`,
   `evaluators/reaper_dsl/time_eval.rs:132-153`) — which any policy may invoke —
   need a small clock abstraction (native = SystemTime/Instant; wasm =
   host-injected or js `Date.now()`; bare-wasm fallback = explicit eval error,
   never a silent wrong time).
3. **getrandom backend.** `reaper-core` calls `getrandom 0.4` directly
   (`bundle_signing.rs:219`); getrandom ≥0.3 requires the `wasm_js` backend via
   RUSTFLAGS + feature, and `.cargo/config.toml` has no wasm target config.
   Only needed if signing/keygen is *in* the wasm surface — verify-only avoids
   keygen; simplest is to feature-gate the keygen path out of the wasm build.
4. **`reaper-wasm` cdylib crate.** Nothing exists (only a docs sketch in
   `docs/deployment/ZERO_OVERHEAD_VISION.md:475+`). New thin crate:
   `crate-type = ["cdylib","rlib"]`, wasm-bindgen API over the §"minimal
   surface" below, wasm-pack packaging for npm.
5. **CI.** No workflow touches wasm32. Add a blocking job: `rustup target add
   wasm32-unknown-unknown` + `cargo build --target wasm32-unknown-unknown` for
   the gated engine + the cdylib (this is the deliverable's real gate).

### Minimal wasm API surface (slice-2 contract)

```
load_policy(text, format)  -> handle   // FromStr / from_yaml_str / from_json_str
load_data(json)            -> ()       // DataLoader::load_json_batch (split exists)
evaluate(principal, action, resource, context) -> PolicyDecision (JSON)
evaluate_all(request)      -> PolicyDecision (JSON)   // set-level, post-slice-2
```

## Proposed PR-sized slices (independently mergeable)

- **Slice 1 — Feature-gate the engine + wasm32 compile gate.** Add `[features]`
  to `policy-engine` (+ `reaper-core` if needed): `cedar`, `batch`,
  `decision-privacy`, all in `default`. Clock shim for the eval path + `time::*`
  builtins. Prove `cargo build --target wasm32-unknown-unknown -p policy-engine
  --no-default-features` (+ minimal features) and add the CI job so it can never
  regress. Native builds: default features unchanged, full test suite + clippy
  green. *No new crate, no bindgen yet — pure enablement, immediately gated.*
- **Slice 2 — `reaper-wasm` cdylib.** New crate under `crates/`, wasm-bindgen
  API per the contract above, wasm-pack build in CI producing an artifact.
  Node-based smoke test (load a real `.reap` + entities JSON, assert
  allow/deny parity with a native run of the same inputs — the *correctness*
  test, not just "it builds").
- **Slice 3 — Parity + packaging.** Differential test harness: run the existing
  BDD/example policy corpus through the wasm build and diff decisions against
  native (the delta≡rebuild-style determinism guarantee, extended cross-target).
  npm packaging metadata, a browser demo page, `docs/` entry. Optional stretch:
  WASI component-model (`wit`) exploration — explicitly *not* slice-1/2 scope.

Supply-chain note: slice 2 adds `wasm-bindgen` (+ `wasm-bindgen-test`,
`serde-wasm-bindgen`) — MIT/Apache-2.0, crates.io — must pass `cargo deny`
allow-lists before the PR is real.

## Open decisions (need confirmation)

1. **Packaging model:** wasm-bindgen/JS-first (browser + edge workers + Node —
   recommended first target, largest immediate surface) vs. WASI
   component-model (language-neutral, but tooling is younger). Proposal:
   bindgen now, component model as a follow-up.
2. **Cedar on wasm:** exclude from the wasm build (recommended — it's the
   heavyweight dep the strategist already flags as half-wired) or prove it
   compiles and include it? Native default keeps Cedar regardless.
3. **Clock semantics on wasm:** js `Date.now()` (via bindgen) vs. host-injected
   epoch per request (deterministic, replay-friendly). Proposal: host-injectable
   with js default — determinism matters for an authz engine.
4. **Where the crate lives:** `crates/reaper-wasm` (workspace member,
   `publish = false` like the rest) — any objection?
