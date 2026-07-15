# reaper-wasm

The Reaper policy evaluation core compiled to WebAssembly (Workstream F2,
`plans/round-2/F2-wasm-target.md`). The same sub-microsecond DSL engine the
agent serves, embeddable in a browser, an edge worker, or a Node process —
without the agent.

## Build (Node target)

```bash
cargo build -p reaper-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target nodejs --out-dir crates/reaper-wasm/pkg-node \
    target/wasm32-unknown-unknown/release/reaper_wasm.wasm
```

(CI runs exactly this in the `wasm-build` job and uploads the bindings as the
`reaper-wasm-node` artifact; wasm-bindgen-cli is pinned to the `wasm-bindgen`
crate version in `Cargo.lock`.)

## Use

```js
const { ReaperEngine } = require("./pkg-node/reaper_wasm.js");

const engine = new ReaperEngine();
engine.loadEntitiesJson(JSON.stringify({
  entities: [{ id: "alice", type: "User", attributes: { roles: ["admin"] } }],
}));
const policyId = engine.deployPolicy("rbac", `
policy rbac {
    default: deny,
    rule admins { allow if "admin" in user.roles }
}
`);

const decision = JSON.parse(engine.evaluate(policyId, "alice", "read", "doc-1"));
// -> { decision: "Allow", policy_id: "...", evaluation_time_ns: ..., ... }

// Any-deny-wins across every deployed policy:
const all = JSON.parse(engine.evaluateAll("alice", "read", "doc-1"));

// Deterministic/replayable time for DSL `time::*` builtins (wasm only):
engine.setNowUnixNs(1_700_000_000_000_000_000n);

// Compiled-primary, same as the agent: "reaper_dsl" = compiled DSL v2
// evaluator; "ReapAstEvaluator" = AST fallback (identical decisions, only
// speed differs). The parity suite asserts the tier cross-target.
engine.evaluatorType(policyId); // -> "reaper_dsl"
```

Semantics are pinned to the agent's serving path: the principal is injected
as `context["principal"]` (nothing else), scalar context values are coerced
to strings, nested values are dropped. The three-leg parity suite (AST +
compiled evaluators, this wrapper natively, the wasm artifact in Node) runs
every `policy-library/*/manifest.json` case on all legs in CI — a decision
divergence anywhere is a red build.

## Check mode (conftest/gatekeeper style)

```js
const result = JSON.parse(engine.checkDocument(policySource, inputJson, "check", "deploy.yaml"));
// -> { allowed: false, violations: [{ rule: "no_latest_tag", message: "..." }] }
```

All violated deny rules are collected (not first-match) with rendered
messages — same semantics as the CLI/agent check endpoints. Parity for all
17 policy-library document cases is asserted on both legs.

## Browser (web target)

```bash
wasm-bindgen --target web --out-dir crates/reaper-wasm/pkg-web \
    target/wasm32-unknown-unknown/release/reaper_wasm.wasm
# then serve the crate dir and open demo/index.html
```

`demo/index.html` is a self-contained page (policy + entities + evaluate,
no server round-trips); CI smoke-tests it in headless Chrome through the
real click handlers (`?autorun`).

## npm packaging

`scripts/package-npm.mjs <dir> --target nodejs|web` stamps a
publishable-shaped `package.json` (version from Cargo.toml, `private: true`
mirroring the workspace publish policy) into the bindings dir. CI uploads
both packages plus the demo as the `reaper-wasm-node` artifact.
