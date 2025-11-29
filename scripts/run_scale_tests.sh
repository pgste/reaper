#!/bin/bash
# Run all scaling and performance tests
# This script generates test data, runs scale tests, and collects performance metrics

set -e

echo "========================================="
echo "Reaper Policy Engine - Scale Tests"
echo "========================================="
echo ""
echo "Starting scale tests at $(date)"
echo ""

# Create results directory
RESULTS_DIR="/tmp/scale_test_results"
mkdir -p "$RESULTS_DIR"
RESULTS_FILE="$RESULTS_DIR/summary.txt"
METRICS_FILE="$RESULTS_DIR/metrics.json"

# Clear results file
> "$RESULTS_FILE"
echo "{" > "$METRICS_FILE"
echo "  \"timestamp\": \"$(date -Iseconds)\"," >> "$METRICS_FILE"
echo "  \"tests\": [" >> "$METRICS_FILE"

# Data generation examples (run these first)
DATA_GEN_TESTS=(
    "generate_rbac_data"
    "generate_abac_data"
    "generate_rebac_data"
    "generate_multilayer_data"
)

# Scale test examples (run these after data generation)
SCALE_TESTS=(
    "benchmark_comprehensions"
    "test_rbac_10k"
    "test_abac_10k"
    "test_rebac_10k"
    "test_multilayer_10k"
)

# Track results
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
FIRST_TEST=true

echo "========================================="
echo "Step 1: Generating Test Data"
echo "========================================="
echo ""

# Generate test data
for test in "${DATA_GEN_TESTS[@]}"; do
    echo "Generating data with: $test"
    if cargo run --release --example "$test" 2>&1 | tee -a "$RESULTS_FILE"; then
        echo "✅ Generated: $test"
    else
        echo "⚠️  Skipped: $test (may already exist)"
    fi
    echo ""
done

echo "========================================="
echo "Step 2: Running Scale Tests"
echo "========================================="
echo ""

# Run each scale test
for test in "${SCALE_TESTS[@]}"; do
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo "========================================="
    echo "[$TOTAL_TESTS/${#SCALE_TESTS[@]}] Running: $test"
    echo "========================================="

    # Start timing
    START_TIME=$(date +%s)

    # Run the test and capture output
    OUTPUT_FILE="$RESULTS_DIR/${test}_output.txt"
    if cargo run --release --example "$test" 2>&1 | tee "$OUTPUT_FILE" | tee -a "$RESULTS_FILE"; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
        STATUS="passed"
        echo "✅ PASSED: $test"
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
        STATUS="failed"
        echo "❌ FAILED: $test"
    fi

    # End timing
    END_TIME=$(date +%s)
    DURATION=$((END_TIME - START_TIME))

    # Extract performance metrics from output if available
    AVG_TIME=$(grep -oP '(?<=Average: )[0-9.]+' "$OUTPUT_FILE" || echo "N/A")
    P99_TIME=$(grep -oP '(?<=p99: )[0-9.]+' "$OUTPUT_FILE" || echo "N/A")
    THROUGHPUT=$(grep -oP '(?<=Throughput: )[0-9.]+' "$OUTPUT_FILE" || echo "N/A")

    # Add to JSON metrics
    if [ "$FIRST_TEST" = false ]; then
        echo "    }," >> "$METRICS_FILE"
    fi
    FIRST_TEST=false

    echo "    {" >> "$METRICS_FILE"
    echo "      \"name\": \"$test\"," >> "$METRICS_FILE"
    echo "      \"status\": \"$STATUS\"," >> "$METRICS_FILE"
    echo "      \"duration_seconds\": $DURATION," >> "$METRICS_FILE"
    echo "      \"avg_time_us\": \"$AVG_TIME\"," >> "$METRICS_FILE"
    echo "      \"p99_time_us\": \"$P99_TIME\"," >> "$METRICS_FILE"
    echo "      \"throughput\": \"$THROUGHPUT\"" >> "$METRICS_FILE"

    echo ""
done

# Close JSON
echo "    }" >> "$METRICS_FILE"
echo "  ]," >> "$METRICS_FILE"
echo "  \"summary\": {" >> "$METRICS_FILE"
echo "    \"total\": $TOTAL_TESTS," >> "$METRICS_FILE"
echo "    \"passed\": $PASSED_TESTS," >> "$METRICS_FILE"
echo "    \"failed\": $FAILED_TESTS" >> "$METRICS_FILE"
echo "  }" >> "$METRICS_FILE"
echo "}" >> "$METRICS_FILE"

echo "========================================="
echo "Scale Test Summary"
echo "========================================="
echo "Total tests:  $TOTAL_TESTS"
echo "Passed:       $PASSED_TESTS"
echo "Failed:       $FAILED_TESTS"
echo ""
echo "Results directory: $RESULTS_DIR"
echo "Summary:          $RESULTS_FILE"
echo "Metrics (JSON):   $METRICS_FILE"
echo ""
echo "Completed at $(date)"

# Display summary metrics if jq is available
if command -v jq &> /dev/null; then
    echo ""
    echo "========================================="
    echo "Performance Metrics Summary"
    echo "========================================="
    jq -r '.tests[] | "\(.name): \(.status) (duration: \(.duration_seconds)s, avg: \(.avg_time_us)µs)"' "$METRICS_FILE"
fi

# Exit with error if any tests failed
if [ $FAILED_TESTS -gt 0 ]; then
    exit 1
fi
