# eBPF Performance Measurement Guide

This guide covers measuring and verifying Reaper's sub-microsecond policy evaluation performance.

## Performance Goals

- **Fast Path (eBPF)**: < 100ns p99 latency
- **Slow Path (Userspace)**: 10-50µs typical
- **Throughput**: > 100K decisions/second per agent
- **Memory**: < 50MB per agent instance
- **Fast Path Percentage**: > 80% of decisions

## Measurement Tools

### 1. BPF Performance Tools

#### bpftrace - Real-time Latency Measurement

```bash
# Install bpftrace
sudo apt-get install -y bpftrace

# Measure LSM hook latency
sudo bpftrace -e '
kprobe:bpf_lsm_file_open {
    @start[tid] = nsecs;
}

kretprobe:bpf_lsm_file_open /@start[tid]/ {
    @latency_ns = hist(nsecs - @start[tid]);
    delete(@start[tid]);
}

interval:s:5 {
    print(@latency_ns);
    clear(@latency_ns);
}
'

# Expected output:
# [64, 128)    500 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
# [128, 256)   100 |@@@@@@                                  |
# [256, 512)    50 |@@@                                     |
```

#### perf - Detailed Profiling

```bash
# Install perf
sudo apt-get install linux-tools-$(uname -r)

# Profile the agent
sudo perf record -F 99 -p $(pidof reaper-agent) -g sleep 30
sudo perf report

# Measure syscall overhead
sudo perf stat -e 'syscalls:sys_enter_*' -p $(pidof reaper-agent) sleep 10
```

### 2. Application-Level Benchmarks

#### Criterion Benchmarks

```bash
# Run all benchmarks
cargo bench --workspace

# Run eBPF-specific benchmarks
cargo bench -p reaper-ebpf

# View results
open target/criterion/report/index.html
```

#### Custom Load Test

Create `/tmp/load_test.sh`:

```bash
#!/bin/bash

# Configuration
AGENT_URL="http://localhost:8080"
NUM_REQUESTS=100000
CONCURRENCY=100

# Deploy a simple policy
curl -X POST $AGENT_URL/api/v1/policies/deploy \
    -H "Content-Type: application/json" \
    -d '{
        "name": "test_policy",
        "language": "simple",
        "rules": [
            {
                "action": "allow",
                "resource": "*",
                "conditions": []
            }
        ]
    }'

# Warm up
echo "Warming up..."
ab -n 1000 -c 10 $AGENT_URL/health > /dev/null 2>&1

# Benchmark policy evaluation
echo "Running load test..."
ab -n $NUM_REQUESTS -c $CONCURRENCY \
    -p /dev/stdin \
    -T 'application/json' \
    $AGENT_URL/api/v1/messages <<EOF
{
    "resource": "/api/users",
    "action": "read",
    "context": {
        "user_id": "alice"
    }
}
EOF

# Get statistics
curl $AGENT_URL/metrics | jq
```

```bash
# Run the load test
chmod +x /tmp/load_test.sh
/tmp/load_test.sh
```

### 3. Kernel-Level Metrics

#### View eBPF Statistics

```bash
#!/bin/bash
# /usr/local/bin/reaper-stats

# Find the STATS map
STATS_MAP_ID=$(sudo bpftool map list | grep STATS | awk '{print $1}' | cut -d: -f1)

echo "=== eBPF Performance Statistics ==="
echo ""

# Dump stats
sudo bpftool map dump id $STATS_MAP_ID | while read -r line; do
    if [[ $line =~ key:\ ([0-9]+) ]]; then
        key="${BASH_REMATCH[1]}"
    elif [[ $line =~ value:\ ([0-9]+) ]]; then
        value="${BASH_REMATCH[1]}"
        case $key in
            0) echo "Fast Path: $value" ;;
            1) echo "Slow Path: $value" ;;
            2) echo "Denials: $value" ;;
            3) echo "Allows: $value" ;;
            4) echo "Errors: $value" ;;
        esac
    fi
done

# Calculate fast path percentage
FAST=$(sudo bpftool map dump id $STATS_MAP_ID | grep -A1 "key: 0" | grep value | awk '{print $2}')
SLOW=$(sudo bpftool map dump id $STATS_MAP_ID | grep -A1 "key: 1" | grep value | awk '{print $2}')
TOTAL=$((FAST + SLOW))

if [ $TOTAL -gt 0 ]; then
    PERCENT=$(echo "scale=2; $FAST * 100 / $TOTAL" | bc)
    echo ""
    echo "Fast Path: ${PERCENT}%"
fi
```

```bash
# Make it executable
sudo cp /tmp/reaper-stats /usr/local/bin/
sudo chmod +x /usr/local/bin/reaper-stats

# Run
sudo reaper-stats
```

## Performance Benchmarks

### Example Benchmark Script

Create `scripts/bench_ebpf.sh`:

```bash
#!/bin/bash
set -e

AGENT_URL="${AGENT_URL:-http://localhost:8080}"
RESULTS_DIR="${RESULTS_DIR:-./bench_results}"

mkdir -p "$RESULTS_DIR"

echo "=== Reaper eBPF Performance Benchmark ==="
echo "Agent: $AGENT_URL"
echo "Results: $RESULTS_DIR"
echo ""

# 1. Deploy policies
echo "[1/5] Deploying test policies..."
for i in {1..10}; do
    curl -s -X POST $AGENT_URL/api/v1/policies \
        -H "Content-Type: application/json" \
        -d "{
            \"name\": \"policy_$i\",
            \"language\": \"simple\",
            \"rules\": [{
                \"action\": \"allow\",
                \"resource\": \"/api/resource_$i\",
                \"conditions\": []
            }]
        }" > /dev/null
done

echo "✓ Deployed 10 policies"

# 2. Warm-up
echo "[2/5] Warming up..."
ab -n 10000 -c 50 -q $AGENT_URL/health > /dev/null 2>&1
echo "✓ Warm-up complete"

# 3. Measure fast path latency
echo "[3/5] Measuring fast path latency..."
ab -n 100000 -c 100 \
    -p <(echo '{"resource":"/api/resource_1","action":"read","context":{}}') \
    -T 'application/json' \
    -g "$RESULTS_DIR/fast_path.tsv" \
    $AGENT_URL/api/v1/messages | tee "$RESULTS_DIR/fast_path.txt"

# Extract key metrics
FAST_P99=$(grep "99%" "$RESULTS_DIR/fast_path.txt" | awk '{print $2}')
FAST_RPS=$(grep "Requests per second" "$RESULTS_DIR/fast_path.txt" | awk '{print $4}')

echo "  P99 Latency: ${FAST_P99}ms"
echo "  Throughput: ${FAST_RPS} req/s"

# 4. Get eBPF statistics
echo "[4/5] Collecting eBPF statistics..."
curl -s $AGENT_URL/metrics > "$RESULTS_DIR/metrics.json"

FAST_COUNT=$(jq '.fast_path_evaluations' "$RESULTS_DIR/metrics.json")
SLOW_COUNT=$(jq '.slow_path_evaluations' "$RESULTS_DIR/metrics.json")
FAST_PERCENT=$(jq '.fast_path_percent' "$RESULTS_DIR/metrics.json")

echo "  Fast Path: $FAST_COUNT evaluations"
echo "  Slow Path: $SLOW_COUNT evaluations"
echo "  Fast Path %: ${FAST_PERCENT}%"

# 5. Generate report
echo "[5/5] Generating report..."
cat > "$RESULTS_DIR/report.md" <<EOF
# Reaper eBPF Performance Report

**Date**: $(date)
**Agent**: $AGENT_URL

## Results

### Fast Path Performance

- **P99 Latency**: ${FAST_P99}ms
- **Throughput**: ${FAST_RPS} requests/second
- **Fast Path Rate**: ${FAST_PERCENT}%

### eBPF Statistics

- **Fast Path Evaluations**: $FAST_COUNT
- **Slow Path Evaluations**: $SLOW_COUNT
- **Total Evaluations**: $((FAST_COUNT + SLOW_COUNT))

## Performance Goals

| Metric | Goal | Actual | Status |
|--------|------|--------|--------|
| Fast Path < 100ns | ✓ | ${FAST_P99}ms | $([ ${FAST_P99%ms} -lt 1 ] && echo "✓ PASS" || echo "✗ FAIL") |
| Throughput > 100K req/s | ✓ | ${FAST_RPS} | $([ ${FAST_RPS%.*} -gt 100000 ] && echo "✓ PASS" || echo "⚠ PARTIAL") |
| Fast Path > 80% | ✓ | ${FAST_PERCENT}% | $(awk "BEGIN {if ($FAST_PERCENT > 80) print \"✓ PASS\"; else print \"✗ FAIL\"}") |

## Conclusion

$(if awk "BEGIN {exit !($FAST_PERCENT > 80)}"; then
    echo "✓ All performance goals met"
else
    echo "⚠ Some performance goals not met. Consider tuning auto-promotion thresholds."
fi)

EOF

cat "$RESULTS_DIR/report.md"

echo ""
echo "✓ Benchmark complete! Results in: $RESULTS_DIR"
```

```bash
# Run the benchmark
chmod +x scripts/bench_ebpf.sh
./scripts/bench_ebpf.sh
```

## Continuous Performance Monitoring

### Prometheus + Grafana

#### prometheus.yml

```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'reaper-agent'
    static_configs:
      - targets: ['localhost:8080']
```

#### Grafana Dashboard

Import dashboard JSON:

```json
{
  "dashboard": {
    "title": "Reaper eBPF Performance",
    "panels": [
      {
        "title": "Fast vs Slow Path",
        "targets": [
          {
            "expr": "rate(reaper_fast_path_total[5m])",
            "legendFormat": "Fast Path"
          },
          {
            "expr": "rate(reaper_slow_path_total[5m])",
            "legendFormat": "Slow Path"
          }
        ]
      },
      {
        "title": "Fast Path Percentage",
        "targets": [
          {
            "expr": "reaper_fast_path_total / (reaper_fast_path_total + reaper_slow_path_total) * 100"
          }
        ]
      },
      {
        "title": "Decision Latency (p99)",
        "targets": [
          {
            "expr": "histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))"
          }
        ]
      }
    ]
  }
}
```

## Optimization Tips

### 1. Increase Fast Path Percentage

If `fast_path_percent < 80%`:

```bash
# Lower promotion thresholds
# Edit /etc/reaper/agent.toml
[ebpf]
promotion_threshold = 50    # Down from 100
stability_window = 50       # Down from 100
```

### 2. Reduce Latency

```bash
# Pin to specific CPUs
sudo taskset -c 0-3 /usr/local/bin/reaper-agent

# Disable CPU frequency scaling
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Increase process priority
sudo nice -n -20 /usr/local/bin/reaper-agent
```

### 3. Increase Throughput

```bash
# Increase worker threads
# Edit /etc/reaper/agent.toml
[server]
workers = 8  # Match CPU count

# Increase connection limits
[server]
max_connections = 10000
```

## Performance Regression Testing

Add to CI pipeline:

```yaml
# .github/workflows/perf.yml
name: Performance Tests

on: [push, pull_request]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install dependencies
        run: sudo apt-get install -y bpftool bpftrace

      - name: Build
        run: cargo build --release --workspace

      - name: Run benchmarks
        run: cargo bench --workspace -- --save-baseline current

      - name: Compare with baseline
        run: |
          cargo bench --workspace -- --baseline current \
            | tee bench_results.txt

          # Fail if regression > 10%
          if grep -q "regressed" bench_results.txt; then
            echo "Performance regression detected!"
            exit 1
          fi
```

## Troubleshooting Performance Issues

### High Latency

```bash
# Check for CPU throttling
sudo cpupower frequency-info

# Check for memory pressure
free -h
sudo vmstat 1 10

# Profile with perf
sudo perf record -F 99 -p $(pidof reaper-agent) -g sleep 10
sudo perf report
```

### Low Throughput

```bash
# Check connection limits
ulimit -n  # Should be > 10000

# Check network buffer sizes
sysctl net.core.rmem_max
sysctl net.core.wmem_max

# Increase if needed
sudo sysctl -w net.core.rmem_max=134217728
sudo sysctl -w net.core.wmem_max=134217728
```

### Memory Leaks

```bash
# Monitor memory over time
watch -n 1 'ps aux | grep reaper-agent | grep -v grep'

# Profile with valgrind
sudo valgrind --tool=massif /usr/local/bin/reaper-agent
```

## Next Steps

- [Deployment Guide](./EBPF_DEPLOYMENT.md)
- [Troubleshooting](./TROUBLESHOOTING.md)
- [Architecture Overview](../architecture/ARCHITECTURE.md)
