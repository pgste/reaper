# Reaper vs OPA Fair Comparison Benchmarks

Clean, simple benchmarking framework for comparing Reaper and OPA performance.

## Directory Structure

```
benchmarks/reaper-vs-opa/
├── bin/
│   ├── benchmark.sh       # Main entry point
│   ├── deploy-reaper.sh   # Deploy to Reaper
│   ├── deploy-opa.sh      # Deploy to OPA
│   └── cleanup.sh         # Cleanup between runs
├── data/
│   ├── 10k/               # 10K entity datasets
│   │   ├── rbac.json
│   │   ├── abac.json
│   │   ├── rebac.json
│   │   └── multilayer.json
│   └── 100k/              # 100K entity datasets (generated, not committed)
│       └── (same structure)
├── policies/
│   ├── reaper/            # Reaper policies
│   │   ├── rbac.reap
│   │   ├── abac.reap
│   │   ├── rebac.reap
│   │   └── multilayer.reap
│   └── opa/               # OPA policies
│       ├── rbac.rego
│       ├── abac.rego
│       ├── rebac.rego
│       └── multilayer.rego
└── results/               # Generated benchmark results
```

## Prerequisites

**Start both services locally:**

```bash
# Terminal 1: Reaper
cd /workspaces/reaper
./target/release/reaper-agent

# Terminal 2: OPA
opa run --server --addr localhost:8181
```

## Quick Start

```bash
cd /workspaces/reaper/benchmarks/reaper-vs-opa

# Run single scenario
./bin/benchmark.sh --scenario multilayer --scale 10k

# Run all scenarios at 10K scale
./bin/benchmark.sh --scenario all --scale 10k

# Run comprehensive test (all scenarios, all scales)
./bin/benchmark.sh --scenario all --scale both
```

## Usage

```bash
./bin/benchmark.sh [OPTIONS]

OPTIONS:
  -s, --scenario SCENARIO    rbac, abac, rebac, multilayer, or all
  -n, --scale SCALE         10k, 100k, or both
  -r, --requests NUM        Number of requests (default: 50000)
  -c, --concurrency NUM     Concurrent requests (default: 50)
  -h, --help                Show help

EXAMPLES:
  # Single scenario
  ./bin/benchmark.sh --scenario multilayer --scale 10k

  # All scenarios, 10K entities
  ./bin/benchmark.sh --scenario all --scale 10k

  # RBAC with 100K entities, 100K requests
  ./bin/benchmark.sh --scenario rbac --scale 100k --requests 100000

  # Complete test suite
  ./bin/benchmark.sh --scenario all --scale both
```

### Comparing over a Unix domain socket (UDS)

Both engines serve the **same** HTTP API over a Unix socket as over TCP, so the
`benchmark` binary can measure either transport. Pass a `unix:<path>` endpoint
instead of an `http://` URL and it dials the socket (via a hyper Unix connector)
with no other change — an apples-to-apples TCP-vs-UDS comparison.

```bash
# Start each engine bound to BOTH transports at once:
#   Reaper: TCP (8080) always + a socket when UDS is enabled
REAPER_UDS_ENABLED=1 REAPER_UDS_PATH=/tmp/reaper-bench/agent.sock \
  ./target/release/reaper-agent &
#   OPA accepts multiple --addr, so it binds TCP and a socket together
mkdir -p /tmp/opa-bench
opa run --server --addr localhost:8181 --addr unix:///tmp/opa-bench/opa.sock &

# Deploy over TCP (state is shared across both listeners), then benchmark over UDS:
./bin/deploy-reaper.sh rbac 100k
./bin/deploy-opa.sh rbac 100k
./bin/benchmark --scenario rbac --requests 10000 --concurrency 50 \
  --reaper-url unix:/tmp/reaper-bench/agent.sock \
  --opa-url unix:/tmp/opa-bench/opa.sock
```

UDS bypasses the TCP/IP stack, so on the same host it typically trims tail
latency vs TCP. The `uds-comparison` CI job runs each scenario back-to-back over
TCP then UDS on one runner and reports the per-transport throughput and the
Reaper UDS-vs-TCP p99 reduction.

## What It Tests

### Scenarios
- **RBAC** - Role-Based Access Control
- **ABAC** - Attribute-Based Access Control
- **ReBAC** - Relationship-Based Access Control
- **Multilayer** - Complex multi-layer policies (most comprehensive)

### Scales
- **10k** - ~12,000 entities (realistic)
- **100k** - ~102,000 entities (stress test)

### Metrics
- **Throughput** - Requests per second (RPS)
- **Latency** - p50, p95, p99 in microseconds
- **Memory** - Peak memory usage during benchmark
- **Accuracy** - Allow/Deny decision counts

## Results

Results are saved to `results/{scale}/{scenario}/`:
- `results.json` - Raw JSON results
- `report.txt` - Human-readable comparison report

Example output:
```
═══════════════════════════════════════════════════════════════
BENCHMARK RESULTS: multilayer @ 10k
═══════════════════════════════════════════════════════════════

PERFORMANCE
───────────────────────────────────────────────────────────────
                    Reaper          OPA             Improvement
Throughput:         26429 req/s     9403 req/s       2.81x
p99 Latency:        9055μs          32863μs          3.63x faster
Allow:              533             0
Deny:               1467            2000

MEMORY
───────────────────────────────────────────────────────────────
                    Reaper          OPA             Ratio
Peak Memory:        108 MB          97 MB            1.11x
```

## SLO Harness

`slo-harness` (in this crate) measures the **served-path SLO table** from
`plans/08-engine-performance-to-sla.md` §3 against a real reaper-agent over
HTTP — request-total latency (client-observed send→response, an upper bound on
the server-side request-total incl. deser+serialize), recorded in an HDR
histogram at nanosecond resolution and reported as p50/p99/p999. It needs no
OPA; it is Reaper-only.

### Scenarios (one per §3 row)

| Scenario | Path | §3 row (p50/p99/p999) |
|----------|------|-----------------------|
| `slo-targeted` | `POST /api/v1/messages` with `policy_id`, 10k DSL policies | ≤2µs / ≤10µs / ≤50µs |
| `slo-evaluate-all` | `POST /api/v1/messages`, no policy, pruning index | ≤5µs / ≤25µs / ≤100µs |
| `slo-rebac` | `POST /api/v1/messages`, rebac policy + 10k-entity data | ≤15µs / ≤75µs / ≤300µs |
| `slo-batch` | `POST /api/v1/batch-messages`, 100 requests/call (per-call limits) | ≤200µs / ≤1ms / ≤5ms |

### Running locally

```bash
cargo build --release   # builds benchmark, generate-data, slo-harness

# 1. Generate the policy sets (10k distinct policies each):
./target/release/generate-data --count 10000 --output /tmp/slo-simple-10k.json \
    policy-set --language simple --resource-prefix /slo/eval
./target/release/generate-data --count 10000 --output /tmp/slo-dsl-10k.json \
    policy-set --language dsl --resource-prefix /slo/targeted

# 2. Start a RELEASE agent. slo-evaluate-all additionally needs:
REAPER_ALLOW_EVALUATE_ALL=true ../../target/release/reaper-agent &
#    (REAPER_USE_PRUNING_INDEX defaults to true; leave it on.)

# 3. Run scenarios. Ordering matters for `all`: evaluate-all runs FIRST,
#    because it requires a PRUNABLE policy set.
#
#    As of D2 (review finding R2-P2-1 closed) the pruning index extracts
#    resource literals from compiled DSL policies too — a DSL policy whose rule
#    is `allow if resource == "…"` is now bucketed by resource, so the DSL set
#    generated with a distinct `resource == …` per policy is prunable and can be
#    used for slo-evaluate-all. The Simple set below still works and is kept as
#    the low-variance default; pass the DSL set to --evaluate-all-policy-set
#    instead to exercise the mandated language end to end. (DSL policies with
#    attribute/dynamic-resource predicates remain unprunable and, past the
#    candidate cap, would still deny — so the evaluate-all set must be one whose
#    rules bind the resource to a literal.)
./target/release/slo-harness --scenario all \
    --evaluate-all-policy-set /tmp/slo-simple-10k.json \
    --policy-set /tmp/slo-dsl-10k.json \
    --requests 20000 --concurrency 4 --save slo-results.json
```

The harness deploys everything it needs (policies via
`/api/v1/policies/compile` / `/api/v1/policies/deploy`, principal entities and
the rebac dataset via `/api/v1/data/stream`) and probes each scenario for a
correct decision before measuring, so a mis-deployed set fails loudly instead
of benchmarking a deny storm.

### Asserting the SLO (`slo.yaml` + multiplier)

```bash
./target/release/slo-harness --scenario all ... \
    --assert-slo slo.yaml --slo-multiplier 250   # or env SLO_MULTIPLIER
```

`slo.yaml` (checked in next to this README) is the single source of truth for
the §3 table. Every threshold is scaled by the multiplier; any violated cell
is listed and the process exits non-zero.

- **Multiplier 1.0 = the real SLA.** Only meaningful on dedicated, isolated
  hardware: the µs-scale rows are below the TCP-loopback floor (~100-190µs
  observed for a full HTTP round trip), so 1.0 will fail on any ordinary box —
  by design, the file carries the true SLA numbers.
- **Shared CI runners use a documented larger multiplier** (the nightly
  `slo-harness.yml` workflow uses 250 — an observed starting point: the local
  worst-cell ratio was ~100x, targeted p50 ≈ 200µs vs the 2µs cell at
  concurrency 4, so 250 leaves ~2.5x headroom for slower runners) to catch
  order-of-magnitude regressions while absorbing loopback + runner variance.
- **True-SLA measurement** requires dedicated hardware, a pinned agent, and
  ideally a kernel-bypass or UDS transport; run with `--concurrency 1..4` and
  `--slo-multiplier 1.0` there.

Concurrency note: request-total latency includes client-side queueing at high
`--concurrency`; use low concurrency (1-4) to approximate service latency, and
higher values to measure under load. The achieved rps is reported alongside —
the table's 5k rps load column is a target for dedicated hardware, not a
harness parameter.

### CI wiring

- **Nightly absolute run** — `.github/workflows/slo-harness.yml` builds a
  release agent, generates both policy sets, runs all four scenarios against a
  fresh agent each, asserts `slo.yaml × SLO_MULTIPLIER` (default 250), and
  uploads the HDR JSON as an artifact. Schedule/dispatch-only; never blocks PRs.
- **Paired A/B on PRs** — the `http-slo-ab` job in
  `.github/workflows/perf-gate.yml` runs `slo-targeted` at 1k policies against
  a merge-base agent AND a PR-head agent, interleaved on the same runner, and
  gates request-total p99 via `scripts/perf_ab_gate.py --http-ab` (median
  ratio > 1.25x AND disjoint samples). This is what actually catches served-path
  regressions (deser/serialize/cache/audit-capture) in PR CI.

## Fair Comparison

Both systems are tested identically:
- ✅ Same entity dataset loaded
- ✅ Same policy logic (Reaper DSL vs Rego)
- ✅ Same benchmark workload
- ✅ Both perform entity lookups by ID
- ✅ Memory tracked during benchmark

## Cleanup

The benchmark automatically cleans up between runs. To manually cleanup:

```bash
./bin/cleanup.sh
```

This:
- Clears OPA data and policies
- Restarts Reaper for clean state
- Ensures no state leakage between tests

## Troubleshooting

### Services not running
```bash
# Check services
curl http://localhost:8080/health  # Reaper
curl http://localhost:8181/health  # OPA

# Check processes
ps aux | grep reaper-agent
ps aux | grep opa
```

### Data files missing
```bash
# Verify data files exist
ls -lh data/10k/
ls -lh data/100k/   # generate first: ./data/generate_all_data.sh
```

### Benchmark fails
```bash
# Check Reaper logs
tail -f /tmp/reaper-agent.log

# Rebuild if needed
cd /workspaces/reaper
cargo build --release
```

## Tips

1. **Start small**: Test one scenario at 10k before running full suite
2. **Watch memory**: Both services should return to baseline between tests
3. **Clean state**: Use `./bin/cleanup.sh` if results seem inconsistent
4. **Check logs**: Tail logs during benchmarks to catch issues early

## Next Steps

After benchmarking:
1. Review results in `results/` directory
2. Compare throughput and latency between engines
3. Analyze memory usage patterns
4. Document findings in reports
