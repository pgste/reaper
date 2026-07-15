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

// Warmup (JIT + caches), then measured single-threaded replay.
//
// TIMING WINDOW: only the `engine.evaluate(...)` call — the wasm boundary
// crossing + policy evaluation — is inside the per-request clock. Parsing
// the returned decision JSON is *harness* accounting work, not embedding
// cost, and at microsecond scale JSON.parse would dominate and inflate
// every percentile; decisions are therefore classified after the timestamp
// is taken. Decision accounting itself matches the Rust benchmark's reaper
// row (send_reaper_request): "allow" counts as Allow, anything else —
// including an evaluation error such as an unknown principal entity —
// counts as Deny (the agent's fail-closed posture).
for (let i = 0; i < Math.min(warmup, requests.length); i++) {
  const r = requests[i];
  try {
    engine.evaluate(policyId, r.principal, r.action, r.resource);
  } catch {
    /* warmup only */
  }
}

const latenciesNs = new Float64Array(requests.length);
let allowed = 0;
let denied = 0;
const runStart = process.hrtime.bigint();
for (let i = 0; i < requests.length; i++) {
  const r = requests[i];
  let decisionJson = null;
  const t0 = process.hrtime.bigint();
  try {
    decisionJson = engine.evaluate(policyId, r.principal, r.action, r.resource);
  } catch {
    decisionJson = null; // fail-closed: counted as deny below
  }
  latenciesNs[i] = Number(process.hrtime.bigint() - t0);

  // Outside the timed window: classify the decision.
  if (decisionJson !== null && JSON.parse(decisionJson).decision.toLowerCase() === "allow") {
    allowed++;
  } else {
    denied++;
  }
}
// Throughput derives from the SAME timed window as the latencies (the sum
// of per-request eval times), so rps and percentiles describe one number
// system; wall-clock (incl. harness accounting) is reported separately.
const wallNs = Number(process.hrtime.bigint() - runStart);
const totalNs = latenciesNs.reduce((a, b) => a + b, 0);

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

// Human-readable output mirroring the Rust benchmark's table/CSV renderers
// (same columns as BenchmarkRow / the csv arm in src/main.rs). Table goes to
// stderr like the Rust tool's, so stdout stays clean for jq consumers.
const pctOf = (n) => ((n / requests.length) * 100).toFixed(1);
const cells = {
  Engine: "Reaper-wasm",
  Scenario: scenario,
  Requests: String(requests.length),
  Success: "100.0%",
  Allow: `${allowed} (${pctOf(allowed)}%)`,
  Deny: `${denied} (${pctOf(denied)}%)`,
  RPS: String(Math.round(row.throughput_rps)),
  "P50 (μs)": row.latency_p50_us.toFixed(0),
  "P95 (μs)": row.latency_p95_us.toFixed(0),
  "P99 (μs)": row.latency_p99_us.toFixed(0),
  "Max (μs)": row.latency_max_us.toFixed(0),
};

if (args.output === "csv") {
  console.log(
    "Engine,Scenario,Requests,Success,Failed,Duration(s),RPS,P50(μs),P95(μs),P99(μs),Max(μs)",
  );
  console.log(
    [
      "Reaper-wasm", scenario, requests.length, requests.length, 0,
      (totalNs / 1e9).toFixed(2), Math.round(row.throughput_rps),
      row.latency_p50_us.toFixed(0), row.latency_p95_us.toFixed(0),
      row.latency_p99_us.toFixed(0), row.latency_max_us.toFixed(0),
    ].join(","),
  );
} else {
  console.log(JSON.stringify(row, null, 2));
}

const keys = Object.keys(cells);
const widths = keys.map((k) => Math.max(k.length, cells[k].length));
const line = (l, m, r) => l + widths.map((w) => "─".repeat(w + 2)).join(m) + r;
console.error("\n📈 Benchmark Results (wasm, in-process)");
console.error("=".repeat(80));
console.error(line("┌", "┬", "┐"));
console.error("│ " + keys.map((k, i) => k.padEnd(widths[i])).join(" │ ") + " │");
console.error(line("├", "┼", "┤"));
console.error("│ " + keys.map((k, i) => cells[k].padEnd(widths[i])).join(" │ ") + " │");
console.error(line("└", "┴", "┘"));
console.error(
  `  evaluator tier: ${tier} · entities: ${entityCount} (loaded in ${Math.round(loadMs)} ms) · ` +
    `timed eval: ${(totalNs / 1e9).toFixed(2)} s · wall (incl. harness): ${(wallNs / 1e9).toFixed(2)} s`,
);

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
