#!/bin/bash
# Benchmark summary script for Reaper
# Runs benchmarks and creates a nice summary report

set -euo pipefail

BENCH_OUTPUT=$(mktemp)
BENCH_SUMMARY=$(mktemp)

echo "🚀 Running Reaper Benchmarks..."
echo ""

# Run benchmarks and capture output
cargo bench --workspace 2>&1 | tee "$BENCH_OUTPUT"

# Parse results and create summary
echo ""
echo "╔═══════════════════════════════════════════════════════════════════════════╗"
echo "║                     📊 REAPER BENCHMARK SUMMARY                           ║"
echo "╚═══════════════════════════════════════════════════════════════════════════╝"
echo ""

# Extract key metrics
echo "🎯 Core Performance Metrics:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Policy evaluation benchmarks
if grep -q "policy_evaluation/simple_policy" "$BENCH_OUTPUT"; then
    SIMPLE_TIME=$(grep "policy_evaluation/simple_policy" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Simple Policy Evaluation:     $SIMPLE_TIME"
fi

if grep -q "policy_evaluation/complex_policy" "$BENCH_OUTPUT"; then
    COMPLEX_TIME=$(grep "policy_evaluation/complex_policy" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Complex Policy Evaluation:    $COMPLEX_TIME"
fi

# Hot-swap benchmarks
if grep -q "policy_hot_swap/deploy_policy" "$BENCH_OUTPUT"; then
    HOTSWAP_TIME=$(grep "policy_hot_swap/deploy_policy" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Policy Hot-Swap (Zero-Down):  $HOTSWAP_TIME"
fi

if grep -q "policy_hot_swap/concurrent_lookup" "$BENCH_OUTPUT"; then
    LOOKUP_TIME=$(grep "policy_hot_swap/concurrent_lookup" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Concurrent Policy Lookup:     $LOOKUP_TIME"
fi

echo ""
echo "⚡ Advanced Benchmarks:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Concurrent access
if grep -q "concurrent_access/concurrent_evaluations" "$BENCH_OUTPUT"; then
    CONCURRENT_TIME=$(grep "concurrent_access/concurrent_evaluations" -A 2 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    CONCURRENT_THRPT=$(grep "concurrent_access/concurrent_evaluations" -A 2 "$BENCH_OUTPUT" | grep "thrpt:" | awk '{print $4, $5}')
    echo "  Concurrent Evaluations:       $CONCURRENT_TIME"
    echo "  Throughput:                   $CONCURRENT_THRPT"
fi

# Memory efficiency
if grep -q "memory_efficiency/policy_storage/100" "$BENCH_OUTPUT"; then
    MEM_100=$(grep "memory_efficiency/policy_storage/100" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  100 Policy Storage:           $MEM_100"
fi

if grep -q "memory_efficiency/policy_storage/1000" "$BENCH_OUTPUT"; then
    MEM_1000=$(grep "memory_efficiency/policy_storage/1000" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  1000 Policy Storage:          $MEM_1000"
fi

# Realistic workloads
if grep -q "realistic_workloads/microservice_auth_pattern" "$BENCH_OUTPUT"; then
    REALISTIC_TIME=$(grep "realistic_workloads/microservice_auth_pattern" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Microservice Auth Pattern:    $REALISTIC_TIME"
fi

# Latency targets
if grep -q "latency_targets/policy_evaluation_performance" "$BENCH_OUTPUT"; then
    LATENCY_TIME=$(grep "latency_targets/policy_evaluation_performance" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4, $5}')
    echo "  Latency Target Test:          $LATENCY_TIME"
fi

echo ""
echo "📈 Performance Status:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check for performance improvements/regressions
IMPROVED_COUNT=$(grep -c "Performance has improved" "$BENCH_OUTPUT" || echo "0")
REGRESSED_COUNT=$(grep -c "Performance has regressed" "$BENCH_OUTPUT" || echo "0")
NO_CHANGE_COUNT=$(grep -c "No change in performance" "$BENCH_OUTPUT" || echo "0")

echo "  ✅ Improved:                    $IMPROVED_COUNT benchmark(s)"
echo "  ⚠️  Regressed:                   $REGRESSED_COUNT benchmark(s)"
echo "  ➖ No Change:                   $NO_CHANGE_COUNT benchmark(s)"

echo ""
echo "🎯 Performance Goals vs Actual:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Extract simple policy time in nanoseconds and compare to target
if grep -q "policy_evaluation/simple_policy" "$BENCH_OUTPUT"; then
    SIMPLE_NS=$(grep "policy_evaluation/simple_policy" -A 1 "$BENCH_OUTPUT" | grep "time:" | awk '{print $4}')
    TARGET_NS=1000
    if (( $(echo "$SIMPLE_NS < $TARGET_NS" | bc -l) )); then
        echo "  ✅ Sub-microsecond latency:     PASSED ($SIMPLE_NS ns < 1000 ns)"
    else
        echo "  ❌ Sub-microsecond latency:     FAILED ($SIMPLE_NS ns >= 1000 ns)"
    fi
fi

echo ""
echo "📊 Benchmark Artifacts:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  HTML Reports:                 target/criterion/"
echo "  Baseline Data:                target/criterion/*/base/"
echo ""

# Count total benchmarks run
TOTAL_BENCHES=$(grep -c "^Benchmarking " "$BENCH_OUTPUT" || echo "0")
echo "✨ Total Benchmarks Executed: $TOTAL_BENCHES"
echo ""
echo "═══════════════════════════════════════════════════════════════════════════"
echo ""

# Cleanup
rm -f "$BENCH_OUTPUT" "$BENCH_SUMMARY"
