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
│   └── 100k/              # 100K entity datasets
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
ls -lh data/100k/
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
