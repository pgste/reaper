// Node leg of the wasm parity contract (F2 slice 2).
//
// Runs the SAME policy-library manifest cases that
// `crates/policy-engine/tests/policy_library_tests.rs` (AST + compiled) and
// `crates/reaper-wasm/tests/parity.rs` (native wrapper) enforce — but through
// the actual wasm32-unknown-unknown artifact, in Node. One shared oracle,
// three legs; a decision divergence anywhere fails the build.
//
// Prereq (CI does this; locally the same two commands):
//   cargo build -p reaper-wasm --target wasm32-unknown-unknown --release
//   wasm-bindgen --target nodejs --out-dir crates/reaper-wasm/pkg-node \
//     target/wasm32-unknown-unknown/release/reaper_wasm.wasm
//
// Run: node crates/reaper-wasm/tests/node/smoke.mjs

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
// wasm-bindgen --target nodejs emits CommonJS.
const { ReaperEngine } = require(join(here, "..", "..", "pkg-node", "reaper_wasm.js"));

const libraryRoot = join(here, "..", "..", "..", "..", "policy-library");
// Scenarios the compiler cannot yet handle (AST-interpreter fallback). The
// native parity leg verifies this list against compiler ground truth.
const astFallbackScenarios = JSON.parse(
  readFileSync(join(here, "..", "fixtures", "ast-fallback-scenarios.json"), "utf8"),
);

function findManifests(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) findManifests(p, out);
    else if (name === "manifest.json") out.push(p);
  }
  return out;
}

function decisionOf(json) {
  return JSON.parse(json).decision.toLowerCase();
}

// ---- 1. Manifest parity across the whole library ------------------------
const manifests = findManifests(libraryRoot, []);
assert.ok(manifests.length >= 8, `expected the full library, found ${manifests.length}`);

let scenarios = 0;
let casesRun = 0;
let skippedDocumentCases = 0;

for (const manifestPath of manifests) {
  const dir = dirname(manifestPath);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

  if (manifest.cases.every((c) => c.input !== undefined)) {
    skippedDocumentCases += manifest.cases.length;
    continue; // document/check mode: outside the slice-2 wasm surface
  }

  const engine = new ReaperEngine();
  if (manifest.data) {
    engine.loadEntitiesJson(readFileSync(join(dir, manifest.data), "utf8"));
  }
  const policyId = engine.deployPolicy(
    manifest.name,
    readFileSync(join(dir, manifest.policy), "utf8"),
  );
  assert.equal(engine.policyCount(), 1, `${manifest.name}: policyCount`);

  // Compiled-PRIMARY contract, wasm leg: the artifact must serve each policy
  // from the compiled DSL v2 evaluator ("reaper_dsl"), exactly like the
  // native engine, except scenarios pinned in the fixture (kept honest by
  // the native leg's independent compiler ground-truth check).
  const expectedTier = astFallbackScenarios.includes(manifest.name)
    ? "ReapAstEvaluator"
    : "reaper_dsl";
  assert.equal(
    engine.evaluatorType(policyId),
    expectedTier,
    `${manifest.name}: evaluator tier mismatch on wasm (compiled-primary contract)`,
  );

  for (const c of manifest.cases) {
    if (c.input !== undefined) {
      skippedDocumentCases += 1;
      continue;
    }
    casesRun += 1;
    const label = `[${manifest.name}] ${c.name}`;
    const ctx = c.context ? JSON.stringify(c.context) : undefined;

    const single = decisionOf(engine.evaluate(policyId, c.principal, c.action, c.resource, ctx));
    assert.equal(single, c.expect, `${label}: single-policy decision mismatch (wasm)`);

    const all = decisionOf(engine.evaluateAll(c.principal, c.action, c.resource, ctx));
    assert.equal(all, c.expect, `${label}: evaluateAll decision mismatch (wasm)`);
  }
  scenarios += 1;
}

// ---- 2. Injected-clock determinism through the wasm boundary ------------
{
  const engine = new ReaperEngine();
  engine.loadEntitiesJson(
    JSON.stringify({ entities: [{ id: "svc", type: "User", attributes: {} }] }),
  );
  const policyId = engine.deployPolicy(
    "clock-pin",
    `
policy clock_pin {
    default: deny,

    rule before_cutoff {
        allow if now := time::now_ns()
        && time::is_before(now, 1000000000)
    }
}
`,
  );

  // Pinned before the cutoff → allow; pinned after → deny. Same inputs,
  // different injected time, deterministic either way.
  engine.setNowUnixNs(BigInt(1));
  assert.equal(
    decisionOf(engine.evaluate(policyId, "svc", "read", "thing")),
    "allow",
    "pinned clock (before cutoff) must allow",
  );

  engine.setNowUnixNs(BigInt("2000000000"));
  assert.equal(
    decisionOf(engine.evaluate(policyId, "svc", "read", "thing")),
    "deny",
    "pinned clock (after cutoff) must deny",
  );

  // Unpinned → JS Date fallback (way past the 1s-epoch cutoff) → deny.
  engine.clearInjectedNow();
  assert.equal(
    decisionOf(engine.evaluate(policyId, "svc", "read", "thing")),
    "deny",
    "real JS clock is past the cutoff",
  );
}

// ---- 3. Error surface sanity ---------------------------------------------
{
  const engine = new ReaperEngine();
  assert.throws(
    () => engine.deployPolicy("bad", "this is not reap"),
    /parse/i,
    "invalid policy must throw",
  );
  assert.throws(
    () => engine.evaluate("not-a-uuid", "p", "a", "r"),
    /invalid policy id/i,
    "bad id must throw",
  );
}

console.log(
  `wasm node smoke: ${scenarios} scenarios, ${casesRun} authz cases verified through the ` +
    `wasm artifact (${skippedDocumentCases} document-mode cases out of slice-2 scope); ` +
    `clock injection + error surface OK`,
);
