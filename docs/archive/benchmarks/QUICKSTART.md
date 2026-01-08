# Quick Start Guide - Reaper vs OPA Benchmark

## 🚀 Fastest Way to Run

```bash
cd /workspaces/reaper/benchmarks/reaper-vs-opa

# Option 1: Fully Automated Quick Test (recommended for first run)
./quick-test.sh

# Option 2: Manual Local (requires Reaper + OPA running)
./run-benchmark.sh

# Option 3: Docker (fully automated)
DOCKER=1 ./run-benchmark.sh
```

## 📋 Prerequisites

### Local Mode
- Reaper Agent running on port 8080
- OPA running on port 8181

### Docker Mode
- Docker and docker-compose installed
- That's it!

## 🎯 Example Commands

### Quick Test (1000 requests)
```bash
./run-benchmark.sh --requests 1000 --concurrency 10
```

### Full Benchmark (10K requests)
```bash
./run-benchmark.sh --requests 10000 --concurrency 50
```

### Stress Test (100K requests)
```bash
./run-benchmark.sh --requests 100000 --concurrency 100
```

### All Scenarios
```bash
./run-benchmark.sh --scenario all
```

### Save Results
```bash
./run-benchmark.sh --save results.json --output json
```

## 📊 Expected Output

```
🚀 Reaper vs OPA Benchmark
================================================================================
  Requests:     10000
  Concurrency:  50
  Scenario:     rbac
  Reaper URL:   http://localhost:8080
  OPA URL:      http://localhost:8181
================================================================================

🔍 Testing connectivity...
  ✓ Reaper is reachable
  ✓ OPA is reachable

📊 Running scenario: rbac

  Testing Reaper...
  [########################################] 10000/10000
  ✓ Reaper - 15234 req/s, p99: 3421μs

  Testing OPA...
  [########################################] 10000/10000
  ✓ OPA - 8932 req/s, p99: 7234μs

📈 Benchmark Results
================================================================================
[TABLE WITH RESULTS]

🏆 Performance Comparison
================================================================================

rbac Scenario:
  Throughput: Reaper is 70.5% faster (15234 vs 8932 req/s)
  P99 Latency: Reaper is 52.7% lower (3421μs vs 7234μs)

✅ Benchmark complete!
```

## 🐛 Troubleshooting

### "Cannot reach Reaper"
```bash
# Start Reaper Agent
cd /workspaces/reaper
cargo run --bin reaper-agent
```

### "Cannot reach OPA"
```bash
# Download and install OPA
curl -L -o opa https://openpolicyagent.org/downloads/latest/opa_linux_amd64
chmod +x opa
sudo mv opa /usr/local/bin/

# Start OPA
cd /workspaces/reaper/benchmarks/reaper-vs-opa
opa run --server --addr=0.0.0.0:8181 policies/opa/
```

### "Port already in use"
```bash
# Kill existing processes
pkill reaper-agent
pkill opa

# Or use different ports
./run-benchmark.sh \
  --reaper-url http://localhost:8090 \
  --opa-url http://localhost:8191
```

## 📁 Output Files

Results can be saved in multiple formats:

```bash
# JSON (for programmatic analysis)
./run-benchmark.sh --output json --save results.json

# CSV (for spreadsheets)
./run-benchmark.sh --output csv > results.csv

# Table (human-readable, default)
./run-benchmark.sh --output table
```

## 🔄 Next Steps

1. Run your first benchmark
2. Try different scenarios (rbac, abac, all)
3. Adjust request count and concurrency
4. Compare results across runs
5. Share your findings!

## 📚 More Information

See [README.md](README.md) for detailed documentation.
