#!/usr/bin/env node
// In-process reaper-wasm leg of the Reaper-vs-OPA benchmark.
//
// Replays the EXACT request stream the HTTP/UDS benchmark sends (produced by
// `benchmark --emit-requests --scenario <s> --requests <n>` — same
// generate_request source, zero drift) through the wasm artifact, and prints
// one results row in the same shape as the Rust benchmark's JSON output.
//
// In-process = no wire, no serialization beyond the JSON boundary of the
// wasm ABI; this measures the embedded-gate deployment model (MCP tool
// server / edge worker), not a transport.
//
// Usage:
//   node bin/wasm-bench.mjs --scenario rbac \
//     --policy policies/reaper/rbac.reap --data data/100k/rbac.json \
//     --requests-file /tmp/requests.ndjson [--pkg <dir>] [--warmup 1000]

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith("--")) acc.push([a.slice(2), all[i + 1]]);
    return acc;
  }, []),
);

const scenario = args.scenario ?? "rbac";
const warmup = Number(args.warmup ?? 1000);
const pkgDir = resolve(args.pkg ?? join(here, "..", "..", "..", "crates", "reaper-wasm", "pkg-node"));

const require = createRequire(import.meta.url);
const { ReaperEngine } = require(join(pkgDir, "reaper_wasm.js"));

const policySrc = readFileSync(resolve(args.policy), "utf8");
const dataJson = readFileSync(resolve(args.data), "utf8");
const requests = readFileSync(resolve(args["requests-file"]), "utf8")
  .split("\n")
  .filter(Boolean)
  .map((l) => JSON.parse(l));

const engine = new ReaperEngine();
const loadStart = process.hrtime.bigint();
const entityCount = engine.loadEntitiesJson(dataJson);
const loadMs = Number(process.hrtime.bigint() - loadStart) / 1e6;
const policyId = engine.deployPolicy(`${scenario}-policy`, policySrc);
const tier = engine.evaluatorType(policyId);

// Same decision accounting as the Rust benchmark's reaper row
// (send_reaper_request): "allow" counts as Allow, anything else — including
// an evaluation error such as an unknown principal entity — counts as Deny
// (the agent's fail-closed posture).
function evalOnce(r) {
  try {
    return JSON.parse(engine.evaluate(policyId, r.principal, r.action, r.resource))
      .decision.toLowerCase() === "allow";
  } catch {
    return false;
  }
}

// Warmup (JIT + caches), then measured single-threaded replay.
for (let i = 0; i < Math.min(warmup, requests.length); i++) {
  evalOnce(requests[i]);
}

const latenciesNs = new Float64Array(requests.length);
let allowed = 0;
let denied = 0;
const runStart = process.hrtime.bigint();
for (let i = 0; i < requests.length; i++) {
  const t0 = process.hrtime.bigint();
  const isAllow = evalOnce(requests[i]);
  latenciesNs[i] = Number(process.hrtime.bigint() - t0);
  if (isAllow) allowed++;
  else denied++;
}
const totalNs = Number(process.hrtime.bigint() - runStart);

const sorted = Array.from(latenciesNs).sort((a, b) => a - b);
const pct = (p) => sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];

const row = {
  engine: "reaper-wasm",
  scenario,
  transport: "in-process",
  evaluator_tier: tier,
  entity_count: entityCount,
  data_load_ms: Math.round(loadMs),
  total_requests: requests.length,
  successful: requests.length,
  failed: 0,
  allowed,
  denied,
  throughput_rps: requests.length / (totalNs / 1e9),
  latency_p50_us: pct(50) / 1e3,
  latency_p95_us: pct(95) / 1e3,
  latency_p99_us: pct(99) / 1e3,
  latency_max_us: sorted[sorted.length - 1] / 1e3,
};

console.log(JSON.stringify(row, null, 2));

// Degenerate-workload guard: a benchmark where every request allows (or
// every request denies) is measuring one code path — likely a broken
// dataset/request-id mismatch — and any "decision parity" against it is
// vacuous. Fail loudly instead of going green on a meaningless run.
// Opt-out (--allow-degenerate) for scenarios that are legitimately one-sided.
if (!("allow-degenerate" in args) && (allowed === 0 || denied === 0)) {
  console.error(
    `DEGENERATE WORKLOAD: allowed=${allowed} denied=${denied} over ${requests.length} requests — ` +
      "every request took the same decision path; dataset/request mismatch?",
  );
  process.exit(2);
}
